//! Async engine wiring. Port of `_run` and `_run_loop` in `tradebot/main.py`:
//! builds the storage layer, portfolio, rate-limited data clients,
//! conditionally-instantiated signals, the aggregator/decision/risk stack,
//! and the trading loop, then runs it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;

use tradebot_common::{Mode, Money};
use tradebot_config::TradeBotConfig;
use tradebot_core::{
    DecisionEngine, ManualActionQueue, Portfolio, RiskManager, RiskState, SignalAggregator,
};
use tradebot_data::{
    BirdeyeClient, HeliusClient, JupiterClient, SolanaRpcClient, TokenBucketLimiter,
};
use tradebot_engine::{DashboardHub, TradingLoop, TradingLoopOptions};
use tradebot_execution::{DemoExecutor, Executor, RealExecutor};
use tradebot_signals::{
    MicrostructureSignal, OnChainSignal, Signal, TASignal, WhaleActivityTracker, WhaleFollowSignal,
};
use tradebot_storage::JsonStorage;
use tradebot_wallet::{log_findings, reconcile, BotKeypair, DEFAULT_MIN_SOL_FOR_FEES};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The three state files `--reset` wipes for `mode`. Mirrors the
/// `storage._portfolio_path` / `_risk_path` / `_mark_history_path` triple
/// main.py's `start_cmd` deletes directly; `JsonStorage` keeps those path
/// builders private, so this reproduces the same filename layout rather
/// than reaching into the crate.
pub fn state_file_paths(data_dir: &str, mode: Mode) -> Vec<PathBuf> {
    let root = Path::new(data_dir);
    vec![
        root.join(format!("portfolio.{mode}.json")),
        root.join(format!("risk_state.{mode}.json")),
        root.join(format!("mark_history.{mode}.json")),
    ]
}

fn money_from_f64(value: f64) -> Money {
    Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
}

/// Python's `str.isupper()`: true if there is at least one cased character
/// and every cased character is uppercase.
fn python_isupper(s: &str) -> bool {
    let mut has_cased = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            has_cased = true;
            if !c.is_uppercase() {
                return false;
            }
        }
    }
    has_cased
}

/// A config value "looks like an env var name" if, with underscores
/// stripped, it is non-empty and all-uppercase. Mirrors the
/// `cfg_value.replace("_", "").isupper() and len(cfg_value) > 0` check
/// repeated for both the Helius and Birdeye keys in `_run_loop`.
fn looks_like_env_var(value: &str) -> bool {
    !value.is_empty() && python_isupper(&value.replace('_', ""))
}

/// Resolves a config key value: if it looks like an env var name, reads
/// that env var; otherwise treats the value as the literal key. Either way,
/// an empty result becomes `None`. Mirrors
/// `(os.environ.get(cfg_value) if looks_like_env else cfg_value) or None`.
fn resolve_key(value: &str) -> Option<String> {
    let resolved = if looks_like_env_var(value) {
        std::env::var(value).ok()
    } else {
        Some(value.to_string())
    };
    resolved.filter(|s| !s.is_empty())
}

/// Builds and runs the engine for one `start` invocation. Mirrors `_run` in
/// main.py: resumes portfolio state (with SOL-migration seeding), builds
/// the shared Jupiter client, and dispatches to the demo or real executor
/// before handing off to [`run_loop`].
pub async fn run(
    mode: Mode,
    cfg: TradeBotConfig,
    bot_keypair: Option<BotKeypair>,
    max_cycles: Option<u64>,
) -> Result<(), BoxError> {
    let storage = JsonStorage::new(&cfg.data_dir)?;

    let portfolio = match storage.load_portfolio_state(mode) {
        Some(mut saved) => {
            // Migration: state file pre-dates SOL tracking -> seed from config.
            if saved.sol_balance == Decimal::ZERO && cfg.app.starting_sol_balance > 0.0 {
                saved.sol_balance = money_from_f64(cfg.app.starting_sol_balance);
            }
            let p = Portfolio::from_state(&saved);
            tracing::info!(
                cash = %p.cash,
                sol = %p.sol_balance,
                realized = %p.realized_pnl_total,
                positions = p.open_positions().len(),
                equity_high = %p.equity_high,
                "portfolio_resumed"
            );
            p
        }
        None => {
            let p = Portfolio::new(
                mode,
                money_from_f64(cfg.app.starting_capital_usd),
                money_from_f64(cfg.app.starting_sol_balance),
            );
            tracing::info!(capital = %p.cash, sol = %p.sol_balance, "portfolio_fresh");
            p
        }
    };
    tracing::info!(mode = %mode, data_dir = cfg.data_dir.as_str(), "startup");

    let base_mints: HashMap<String, (String, u32)> = cfg
        .watchlist
        .entries
        .iter()
        .map(|e| {
            (
                format!("{}/{}", e.symbol, cfg.watchlist.quote_symbol),
                (e.mint.clone(), e.decimals as u32),
            )
        })
        .collect();
    let pairs: Vec<String> = base_mints.keys().cloned().collect();
    let timeframes: Vec<String> = cfg.weights.timeframes.keys().cloned().collect();

    let jup_limiter = Arc::new(TokenBucketLimiter::new(
        cfg.app.jupiter_rate_limit_rps,
        cfg.app.jupiter_rate_limit_burst,
    ));
    let jup = JupiterClient::new(
        cfg.app.jupiter_base_url.clone(),
        Some(jup_limiter.clone()),
        cfg.app.jupiter_max_429_retries,
    );

    if mode == Mode::Real {
        let bot_kp = bot_keypair.expect("guarded by cmd_start: real mode always loads a keypair");
        let rpc = SolanaRpcClient::new(cfg.app.rpc_url.clone());

        let findings = reconcile(
            &portfolio,
            &rpc,
            &bot_kp.address,
            &base_mints,
            DEFAULT_MIN_SOL_FOR_FEES,
        )
        .await?;
        log_findings(&findings);

        let executor = RealExecutor::new(
            jup.clone(),
            rpc,
            &storage,
            &bot_kp,
            base_mints.clone(),
            cfg.watchlist.quote_mint.clone(),
            6,
            cfg.risk.max_slippage_pct,
            cfg.app.priority_fee_microlamports,
            cfg.app.confirmation_timeout_s,
            None,
        );
        run_loop(
            &cfg,
            mode,
            &storage,
            portfolio,
            jup,
            executor,
            base_mints,
            pairs,
            timeframes,
            Some(bot_kp.address.clone()),
            jup_limiter,
            max_cycles,
        )
        .await
    } else {
        let executor = DemoExecutor::new(
            jup.clone(),
            &storage,
            base_mints.clone(),
            cfg.watchlist.quote_mint.clone(),
            6,
            cfg.risk.max_slippage_pct,
            cfg.app.priority_fee_microlamports,
            cfg.app.simulated_confirm_latency_s,
        );
        run_loop(
            &cfg,
            mode,
            &storage,
            portfolio,
            jup,
            executor,
            base_mints,
            pairs,
            timeframes,
            None,
            jup_limiter,
            max_cycles,
        )
        .await
    }
}

/// Builds the aggregator/decision/risk stack and the trading loop, then
/// runs it. Mirrors `_run_loop` in main.py.
#[allow(clippy::too_many_arguments)]
async fn run_loop<E: Executor>(
    cfg: &TradeBotConfig,
    mode: Mode,
    storage: &JsonStorage,
    portfolio: Portfolio,
    jup: JupiterClient,
    executor: E,
    base_mints: HashMap<String, (String, u32)>,
    pairs: Vec<String>,
    timeframes: Vec<String>,
    real_address: Option<String>,
    jup_limiter: Arc<TokenBucketLimiter>,
    max_cycles: Option<u64>,
) -> Result<(), BoxError> {
    let risk = RiskManager::new(cfg.risk.clone());
    let state = storage
        .load_risk_state(mode)
        .map(|r| RiskState::from_record(&r))
        .unwrap_or_default();
    if state.kill_switch_active {
        tracing::warn!(
            reason = state.kill_switch_reason.as_str(),
            hint = "bot will not enter new positions until manually cleared",
            "kill_switch_resumed"
        );
    }
    let engine = DecisionEngine::new(
        RiskManager::new(cfg.risk.clone()),
        cfg.app.entry_threshold,
        cfg.app.exit_flip_threshold,
    );

    // Signals are only INSTANTIATED if their weight > 0. A weight of 0 in
    // the aggregator just zeroes the contribution, but the signal would
    // still make network calls every cycle, burning the rate-limit budget
    // for nothing.
    let sig_weights = &cfg.weights.signals;
    // NOTE: cfg.weights.timeframes is a BTreeMap (sorted key order), unlike
    // Python's insertion-ordered dict, so "the first timeframe" can differ
    // from the Python CLI whenever the declared and lexicographic orders
    // disagree (the shipped default config declares 5s/1m/15m/1h, but
    // BTreeMap iterates 15m/1h/1m/5s). This follows from the
    // tradebot-config crate's BTreeMap choice made in an earlier phase, not
    // something this wiring changes.
    let micro_tf = timeframes
        .first()
        .cloned()
        .unwrap_or_else(|| "1m".to_string());

    let mut signals: Vec<Box<dyn Signal>> = Vec::new();
    if sig_weights.get("ta").copied().unwrap_or(0.0) > 0.0 {
        for tf in &timeframes {
            signals.push(Box::new(TASignal::new(tf.clone())));
        }
    } else {
        tracing::info!(reason = "weight is 0", "ta_signal_disabled");
    }

    if sig_weights.get("microstructure").copied().unwrap_or(0.0) > 0.0 {
        for (pair, (mint, base_decimals)) in &base_mints {
            signals.push(Box::new(MicrostructureSignal::new(
                pair.clone(),
                micro_tf.clone(),
                jup.clone(),
                mint.clone(),
                cfg.watchlist.quote_mint.clone(),
                *base_decimals,
                6,
                10.0,
            )));
        }
        tracing::info!(pairs = base_mints.len(), "microstructure_signal_enabled");
    } else {
        tracing::info!(
            reason = "weight is 0: saves 2 Jupiter calls per pair per cycle",
            "microstructure_signal_disabled"
        );
    }

    // Helius key resolution: prefer env var if the config value looks like
    // a var name (uppercase + underscores), otherwise treat the value as
    // the literal API key.
    let helius_key = resolve_key(&cfg.app.helius_api_key_env);

    let onchain_weight = sig_weights.get("onchain").copied().unwrap_or(0.0);
    let whale_weight = sig_weights.get("whale_follow").copied().unwrap_or(0.0);
    let needs_helius = helius_key.is_some()
        && (onchain_weight > 0.0 || (whale_weight > 0.0 && cfg.whales.enabled));

    let mut helius: Option<HeliusClient> = None;
    if needs_helius {
        let key = helius_key.clone().expect("checked by needs_helius");
        let client = HeliusClient::with_base_url(key, cfg.app.helius_base_url.clone());
        if onchain_weight > 0.0 {
            let dex_addresses: HashSet<String> =
                cfg.app.onchain_dex_addresses.iter().cloned().collect();
            for (pair, (mint, _)) in &base_mints {
                signals.push(Box::new(
                    OnChainSignal::new(
                        pair.clone(),
                        micro_tf.clone(),
                        client.clone(),
                        mint.clone(),
                        dex_addresses.clone(),
                    )
                    .with_whale_min(cfg.app.onchain_whale_min),
                ));
            }
            tracing::info!(
                dex_addresses = cfg.app.onchain_dex_addresses.len(),
                "onchain_signal_enabled"
            );
        } else {
            tracing::info!(reason = "weight is 0", "onchain_signal_disabled");
        }
        helius = Some(client);
    } else if helius_key.is_none() {
        tracing::info!(reason = "no Helius key resolved", "helius_disabled");
    } else {
        tracing::info!(
            reason = "no consumer (onchain + whale weights are 0)",
            "helius_disabled"
        );
    }

    // Whale-follow: tracker fetches Helius once per cycle; per-pair signals
    // consume it. Skipped entirely if weight is 0, even if
    // `whales.enabled = true` in config.
    let mut whale_tracker: Option<Arc<WhaleActivityTracker>> = None;
    if cfg.whales.enabled
        && helius.is_some()
        && !cfg.whales.wallets.is_empty()
        && whale_weight > 0.0
    {
        let whale_addresses: Vec<String> = cfg
            .whales
            .wallets
            .iter()
            .map(|w| w.address.clone())
            .collect();
        let tracker = Arc::new(
            WhaleActivityTracker::new(
                helius.clone().expect("checked above"),
                whale_addresses.clone(),
            )
            .with_per_wallet_limit(cfg.whales.per_wallet_swap_limit),
        );
        for (pair, (mint, _)) in &base_mints {
            let mut sig = WhaleFollowSignal::new(
                pair.clone(),
                mint.clone(),
                cfg.watchlist.quote_mint.clone(),
                tracker.clone(),
            )
            .with_lookback_seconds(cfg.whales.lookback_minutes as i64 * 60)
            .with_decay_half_life_s(cfg.whales.decay_half_life_minutes * 60.0);
            sig.timeframe = micro_tf.clone();
            signals.push(Box::new(sig));
        }
        tracing::info!(
            wallets = whale_addresses.len(),
            lookback_min = cfg.whales.lookback_minutes,
            "whale_follow_enabled"
        );
        whale_tracker = Some(tracker);
    } else if cfg.whales.enabled {
        tracing::warn!(
            reason = "enabled but Helius key missing or no wallets configured",
            "whale_follow_skipped"
        );
    }

    // Birdeye: batched mark-price fetcher. Same env-var-or-literal-key
    // resolution as Helius. When present, a single Birdeye call per cycle
    // replaces N Jupiter mark probes, freeing the Jupiter rate-limit budget
    // for execution.
    let birdeye_key = resolve_key(&cfg.app.birdeye_api_key_env);
    let mut birdeye: Option<Arc<BirdeyeClient>> = None;
    if let Some(key) = birdeye_key {
        let birdeye_limiter = Arc::new(TokenBucketLimiter::new(
            cfg.app.birdeye_rate_limit_rps,
            cfg.app.birdeye_rate_limit_burst,
        ));
        let client = BirdeyeClient::with_options(
            key,
            cfg.app.birdeye_base_url.clone(),
            "solana",
            5.0,
            Some(birdeye_limiter),
            cfg.app.birdeye_max_429_retries,
        );
        tracing::info!(
            rps = cfg.app.birdeye_rate_limit_rps,
            burst = cfg.app.birdeye_rate_limit_burst,
            "birdeye_marks_enabled"
        );
        birdeye = Some(Arc::new(client));
    } else {
        tracing::info!(
            reason = "no Birdeye key configured",
            "birdeye_marks_disabled"
        );
    }

    let timeframe_weights: HashMap<String, f64> =
        cfg.weights.timeframes.clone().into_iter().collect();
    let signal_weights: HashMap<String, f64> = cfg.weights.signals.clone().into_iter().collect();
    let aggregator = SignalAggregator::new(signals, timeframe_weights, signal_weights);

    // The dashboard hub is created here and the loop publishes snapshots to
    // it below, but the axum HTTP server that would serve those snapshots
    // to a browser is Phase 8 and not implemented yet.
    // Phase 8: dashboard server subscribes to the hub here.
    let hub_enabled = cfg.dashboard.enabled;
    let hub = if hub_enabled {
        Some(Arc::new(DashboardHub::default()))
    } else {
        None
    };
    let manual_actions = if hub_enabled {
        Some(Arc::new(ManualActionQueue::new()))
    } else {
        None
    };
    let fast_interval_s = if birdeye.is_some() && hub_enabled && cfg.app.fast_tick_interval_s > 0.0
    {
        Some(cfg.app.fast_tick_interval_s)
    } else {
        None
    };

    let saved_history = storage.load_mark_history(mode);
    let history_pairs = saved_history.len();

    let mut loop_ = TradingLoop::new(
        storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        executor,
        jup,
        pairs,
        timeframes,
        cfg.watchlist.quote_mint.clone(),
        6,
        base_mints,
        TradingLoopOptions {
            hub,
            jup_limiter: Some(jup_limiter),
            manual_actions,
            whale_tracker,
            birdeye,
            mark_history_seed: Some(saved_history),
            ..TradingLoopOptions::default()
        },
    );
    if history_pairs > 0 {
        tracing::info!(pairs = history_pairs, "mark_history_resumed");
    }
    if let Some(interval) = fast_interval_s {
        tracing::info!(interval_s = interval, "fast_tick_enabled");
    }
    if let Some(address) = &real_address {
        tracing::warn!(
            address = address.as_str(),
            rpc = cfg.app.rpc_url.as_str(),
            "REAL_MODE_ACTIVE"
        );
    }

    if let Some(n) = max_cycles {
        // Test-only bounded run: skip the Ctrl-C-driven scheduler entirely
        // so integration tests get a deterministic number of cycles.
        for _ in 0..n {
            if let Err(e) = loop_.run_one_cycle(chrono::Utc::now()).await {
                tracing::error!(error = %e, "cycle_failed");
            }
        }
        return Ok(());
    }

    let stop_signal = loop_.stop_signal();
    let ctrlc_stop = stop_signal.clone();
    let ctrlc_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("shutdown_signal");
            ctrlc_stop.stop();
        }
    });

    run_combined(&mut loop_, cfg.app.decision_interval_s, fast_interval_s).await;
    ctrlc_task.abort();

    Ok(())
}

/// Drives the main decision cycle and (if configured) the fast-tick
/// republish on their own cadences from a single task, until stopped.
///
/// `TradingLoop::run_forever` and `TradingLoop::run_fast_ticks` both take
/// `&mut self`, so they cannot be spawned as two independently-scheduled
/// tasks against one `TradingLoop` the way loop.py's two `asyncio.Task`s
/// share one Python object. This reimplements their scheduling (fixed
/// cadence, each running back-to-back if its own interval is overrun) as a
/// single `tokio::select!` over both timers plus a short poll of the stop
/// signal, using `run_one_cycle` / `run_one_fast_tick` for the per-unit
/// work.
async fn run_combined<E: Executor>(
    loop_: &mut TradingLoop<'_, E>,
    cycle_interval_s: f64,
    fast_interval_s: Option<f64>,
) {
    const STOP_POLL: Duration = Duration::from_millis(200);

    let stop = loop_.stop_signal();
    let mut next_cycle = tokio::time::Instant::now();
    let mut next_fast = tokio::time::Instant::now();
    loop {
        if stop.is_stopped() {
            break;
        }
        let now_inst = tokio::time::Instant::now();
        let cycle_wait = next_cycle.saturating_duration_since(now_inst);
        let fast_wait = next_fast.saturating_duration_since(now_inst);
        tokio::select! {
            _ = tokio::time::sleep(cycle_wait) => {
                if let Err(e) = loop_.run_one_cycle(chrono::Utc::now()).await {
                    tracing::error!(error = %e, "cycle_failed");
                }
                next_cycle = tokio::time::Instant::now()
                    + Duration::from_secs_f64(cycle_interval_s.max(0.01));
            }
            _ = tokio::time::sleep(fast_wait), if fast_interval_s.is_some() => {
                loop_.run_one_fast_tick().await;
                next_fast = tokio::time::Instant::now()
                    + Duration::from_secs_f64(fast_interval_s.unwrap_or(1.0).max(0.01));
            }
            _ = tokio::time::sleep(STOP_POLL) => {}
        }
    }
}
