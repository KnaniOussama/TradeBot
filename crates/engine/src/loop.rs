//! Port of `tradebot/core/loop.py`: the trading loop that ties signals,
//! risk, decisions, execution, persistence, and the dashboard hub together.
//!
//! The module is named `loop` (a Rust keyword), so it is declared in
//! `lib.rs` as `pub mod r#loop;`; downstream code should use the crate-root
//! re-export (`tradebot_engine::TradingLoop`) rather than reaching into this
//! module directly.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tokio::sync::Notify;

use tradebot_common::Money;
use tradebot_core::{
    compute_kelly_stats, Action, ActionKind, AggregatedScore, DecisionEngine, ExitReason,
    KellyStats, ManualActionQueue, Observation, Portfolio, Regime, RiskManager, RiskState,
    SignalAggregator,
};
use tradebot_data::{BirdeyeClient, JupiterClient, TokenBucketLimiter};
use tradebot_execution::{ExecutionError, Executor, Order};
use tradebot_signals::{MarketContext, WhaleActivityTracker};
use tradebot_storage::{
    Candle, EquitySnapshot, JsonStorage, MarkHistory, MarkPoint, OhlcvCandle, PortfolioState,
    PositionRecord, StorageError,
};

use crate::hub::DashboardHub;
use crate::snapshot::{build_snapshot, SnapshotOptions, WhaleActivityEntry};

/// Errors that can abort a decision cycle. Execution failures are handled
/// per-action and logged (never abort the cycle, matching loop.py's
/// `except ExecutionError` inside `_execute_action`); the only thing that
/// can still fail the whole cycle is a state-persistence write.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Cap on the number of buffered `Observation`s the loop keeps for the
/// dashboard's decision log. Mirrors `deque(maxlen=200)` in loop.py.
const OBSERVATIONS_MAX: usize = 200;

/// Amount-comparison epsilon for the manual-exit position guard, matching
/// Python's `1e-9` in loop.py's `run_one_cycle`.
fn amount_epsilon() -> Money {
    Decimal::new(1, 9)
}

fn money_from_f64(value: f64) -> Money {
    Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
}

/// Pushes `item` onto the back of `buf`, dropping from the front until the
/// length is at most `max_len`. Mirrors Python's `collections.deque(maxlen=
/// ...)` auto-eviction, which Rust's `VecDeque` does not do natively.
fn push_bounded<T>(buf: &mut VecDeque<T>, item: T, max_len: usize) {
    buf.push_back(item);
    while buf.len() > max_len {
        buf.pop_front();
    }
}

/// Floors `ts` to the start of its bucket for `timeframe`. Falls back to
/// `ts` unchanged for an unrecognized timeframe token. Mirrors `_bucket_floor`
/// in loop.py (`_TIMEFRAME_SECONDS.get(timeframe)` returning `None`).
fn bucket_floor(ts: DateTime<Utc>, timeframe: &str) -> DateTime<Utc> {
    match tradebot_data::timeframe_seconds(timeframe) {
        Some(_) => tradebot_data::bucket_for(ts, timeframe),
        None => ts,
    }
}

/// A cheap, cloneable stop signal shared between a `TradingLoop` and any
/// external caller that needs to cancel `run_forever` / `run_fast_ticks`
/// from another task. Mirrors `asyncio.Event` in loop.py: `stop()` sets the
/// flag and wakes anyone currently sleeping between cycles.
#[derive(Clone)]
pub struct StopSignal {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl StopSignal {
    fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Sets the stop flag and wakes any pending inter-cycle sleep. Mirrors
    /// `TradingLoop.stop()` in loop.py.
    pub fn stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Sleeps for `dur`, or until `stop()` is called, whichever comes
    /// first. Mirrors `asyncio.wait_for(self._stop.wait(), timeout=sleep_s)`.
    async fn wait_timeout(&self, dur: Duration) {
        if self.is_stopped() {
            return;
        }
        let _ = tokio::time::timeout(dur, self.notify.notified()).await;
    }
}

impl Default for StopSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// The bot's trading loop: one decision cycle wires together live marks,
/// signal aggregation, regime classification, Kelly sizing, the decision
/// engine, order execution, and dashboard-snapshot publishing. Mirrors
/// `TradingLoop` in loop.py.
///
/// Generic over the executor implementation (`DemoExecutor` or
/// `RealExecutor`) rather than a `dyn Executor` trait object, since the
/// concrete executor type is always known at construction time in this
/// codebase; this avoids an unnecessary vtable indirection on the hot path.
pub struct TradingLoop<'s, E: Executor> {
    storage: &'s JsonStorage,
    portfolio: Portfolio,
    aggregator: SignalAggregator,
    engine: DecisionEngine,
    risk: RiskManager,
    state: RiskState,
    executor: E,
    jupiter: JupiterClient,
    pairs: Vec<String>,
    timeframes: Vec<String>,
    quote_mint: String,
    quote_decimals: u32,
    base_mints: HashMap<String, (String, u32)>,
    ohlcv_limit: usize,
    hub: Option<Arc<DashboardHub>>,
    chart_history_max: usize,
    jup_limiter: Option<Arc<TokenBucketLimiter>>,
    manual_actions: Option<Arc<ManualActionQueue>>,
    whale_tracker: Option<Arc<WhaleActivityTracker>>,
    birdeye: Option<Arc<BirdeyeClient>>,
    mark_history: HashMap<String, VecDeque<(DateTime<Utc>, Money)>>,
    observations: VecDeque<Observation>,
    stop_signal: StopSignal,
    // Latest marks observed by the fast-tick task (or the last decision
    // cycle). Used by the fast-tick republish to compute up-to-date
    // equity/P&L while waiting for the next decision cycle.
    latest_marks: HashMap<String, Money>,
    // Cached scored signals from the last decision cycle, reused by
    // fast-tick republishes so the dashboard's signal panel doesn't go
    // blank between cycles.
    latest_scored: Vec<AggregatedScore>,
}

/// Constructor parameters that have Python-side defaults (`ohlcv_limit=200`,
/// `hub=None`, `chart_history_max=120`, `jup_limiter=None`,
/// `manual_actions=None`, `whale_tracker=None`, `birdeye=None`). Bundled
/// here since Rust has no default-argument syntax; `Default::default()`
/// reproduces the Python defaults.
pub struct TradingLoopOptions {
    pub ohlcv_limit: usize,
    pub hub: Option<Arc<DashboardHub>>,
    pub chart_history_max: usize,
    pub jup_limiter: Option<Arc<TokenBucketLimiter>>,
    pub manual_actions: Option<Arc<ManualActionQueue>>,
    pub whale_tracker: Option<Arc<WhaleActivityTracker>>,
    pub birdeye: Option<Arc<BirdeyeClient>>,
    /// Rolling mark-price history to resume from (loaded by the caller via
    /// `JsonStorage::load_mark_history`). Only pairs already present in the
    /// loop's `pairs` list are seeded, mirroring the `if pair in
    /// loop._mark_history` guard in loop.py's startup wiring.
    pub mark_history_seed: Option<MarkHistory>,
}

impl Default for TradingLoopOptions {
    fn default() -> Self {
        Self {
            ohlcv_limit: 200,
            hub: None,
            chart_history_max: 120,
            jup_limiter: None,
            manual_actions: None,
            whale_tracker: None,
            birdeye: None,
            mark_history_seed: None,
        }
    }
}

impl<'s, E: Executor> TradingLoop<'s, E> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage: &'s JsonStorage,
        portfolio: Portfolio,
        aggregator: SignalAggregator,
        engine: DecisionEngine,
        risk: RiskManager,
        state: RiskState,
        executor: E,
        jupiter: JupiterClient,
        pairs: Vec<String>,
        timeframes: Vec<String>,
        quote_mint: String,
        quote_decimals: u32,
        base_mints: HashMap<String, (String, u32)>,
        options: TradingLoopOptions,
    ) -> Self {
        let mut mark_history: HashMap<String, VecDeque<(DateTime<Utc>, Money)>> = pairs
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    VecDeque::with_capacity(options.chart_history_max),
                )
            })
            .collect();
        if let Some(seed) = &options.mark_history_seed {
            for (pair, points) in seed {
                if let Some(buf) = mark_history.get_mut(pair) {
                    for point in points {
                        push_bounded(buf, (point.t, point.p), options.chart_history_max);
                    }
                }
            }
        }
        Self {
            storage,
            portfolio,
            aggregator,
            engine,
            risk,
            state,
            executor,
            jupiter,
            pairs,
            timeframes,
            quote_mint,
            quote_decimals,
            base_mints,
            ohlcv_limit: options.ohlcv_limit,
            hub: options.hub,
            chart_history_max: options.chart_history_max,
            jup_limiter: options.jup_limiter,
            manual_actions: options.manual_actions,
            whale_tracker: options.whale_tracker,
            birdeye: options.birdeye,
            mark_history,
            observations: VecDeque::new(),
            stop_signal: StopSignal::default(),
            latest_marks: HashMap::new(),
            latest_scored: Vec::new(),
        }
    }

    /// Signals `run_forever` / `run_fast_ticks` to stop after their current
    /// cycle/tick. Mirrors `TradingLoop.stop()` in loop.py.
    pub fn stop(&self) {
        self.stop_signal.stop();
    }

    /// A cloneable handle that can call `stop()` from another task, for
    /// callers that spawn `run_forever` and need to cancel it later.
    pub fn stop_signal(&self) -> StopSignal {
        self.stop_signal.clone()
    }

    pub fn portfolio(&self) -> &Portfolio {
        &self.portfolio
    }

    /// Buffered decision-cycle observations, newest last. Mirrors reading
    /// `loop._observations` directly in loop.py's test suite.
    pub fn observations(&self) -> &VecDeque<Observation> {
        &self.observations
    }

    /// Returns `(marks_per_pair, slippage_per_pair)`.
    ///
    /// Prefers Birdeye (one batched call covers the whole watchlist for a
    /// flat HTTP cost). Falls back to per-pair Jupiter probes if Birdeye is
    /// unavailable or returns nothing. The executor's slippage gate catches
    /// per-trade slippage at fill time, so when Birdeye is the source we
    /// report 0 here (Birdeye doesn't expose price impact). Mirrors
    /// `_live_marks` in loop.py.
    async fn live_marks(&self) -> (HashMap<String, Money>, HashMap<String, f64>) {
        if let Some(birdeye) = &self.birdeye {
            let mints: Vec<String> = self.base_mints.values().map(|(m, _)| m.clone()).collect();
            let prices = birdeye.multi_price(&mints).await;
            if !prices.is_empty() {
                let mut marks = HashMap::new();
                for (pair, (mint, _)) in &self.base_mints {
                    if let Some(p) = prices.get(mint) {
                        marks.insert(pair.clone(), money_from_f64(*p));
                    }
                }
                if !marks.is_empty() {
                    let slips = marks.keys().map(|p| (p.clone(), 0.0)).collect();
                    return (marks, slips);
                }
            }
            tracing::info!("birdeye_marks_empty_falling_back_to_jupiter");
        }

        let mut marks = HashMap::new();
        let mut slips = HashMap::new();
        let probe_quote = 1.0_f64;
        for (pair, (mint, base_decimals)) in &self.base_mints {
            let in_units = (probe_quote * 10f64.powi(self.quote_decimals as i32)) as u64;
            match self
                .jupiter
                .quote(&self.quote_mint, mint, in_units, 50)
                .await
            {
                Ok(q) => {
                    let out_human = q.out_amount as f64 / 10f64.powi(*base_decimals as i32);
                    if out_human > 0.0 {
                        marks.insert(pair.clone(), money_from_f64(probe_quote / out_human));
                    }
                    slips.insert(pair.clone(), q.price_impact_pct);
                }
                Err(e) => {
                    tracing::warn!(pair = pair.as_str(), error = %e, "mark_quote_failed");
                }
            }
        }
        (marks, slips)
    }

    /// Runs one full decision cycle. Mirrors `run_one_cycle` in loop.py;
    /// step numbers in the comments match the Python source.
    pub async fn run_one_cycle(&mut self, now: DateTime<Utc>) -> Result<(), EngineError> {
        // 0. Refresh whale activity (one Helius call per watched wallet,
        //    shared across every pair's WhaleFollowSignal this cycle).
        if let Some(tracker) = &self.whale_tracker {
            tracker.fetch_all().await;
        }

        // 1. Load OHLCV for all (pair, timeframe).
        let mut ohlcv =
            self.storage
                .load_ohlcv_for_pairs(&self.pairs, &self.timeframes, self.ohlcv_limit);

        // 2. Get live marks + slippages.
        let (marks, slippages) = self.live_marks().await;
        for (pair, price) in &marks {
            let buf = self.mark_history.entry(pair.clone()).or_default();
            push_bounded(buf, (now, *price), self.chart_history_max);
        }

        // 2b. Persist marks as OHLCV candles per timeframe so signals have
        //     data.
        for (pair, price) in &marks {
            for tf in &self.timeframes {
                let bucket = bucket_floor(now, tf);
                let existing_last = ohlcv
                    .get(pair)
                    .and_then(|m| m.get(tf))
                    .and_then(|v: &Vec<Candle>| v.last());
                let candle = match existing_last {
                    Some(last) if last.timestamp == bucket => OhlcvCandle {
                        pair: pair.clone(),
                        timeframe: tf.clone(),
                        bucket_start: bucket,
                        open: last.open,
                        high: last.high.max(*price),
                        low: last.low.min(*price),
                        close: *price,
                        volume_quote: last.volume,
                    },
                    _ => OhlcvCandle {
                        pair: pair.clone(),
                        timeframe: tf.clone(),
                        bucket_start: bucket,
                        open: *price,
                        high: *price,
                        low: *price,
                        close: *price,
                        volume_quote: Decimal::ZERO,
                    },
                };
                self.storage.upsert_ohlcv(&candle)?;
            }
        }
        // Reload OHLCV so signals see the freshly-appended candle.
        ohlcv = self
            .storage
            .load_ohlcv_for_pairs(&self.pairs, &self.timeframes, self.ohlcv_limit);

        // 3. Aggregate signals per pair.
        let mut scored: Vec<AggregatedScore> = Vec::with_capacity(self.pairs.len());
        for pair in &self.pairs {
            let pair_ohlcv: HashMap<String, Vec<Candle>> = ohlcv
                .get(pair)
                .cloned()
                .map(|btm| btm.into_iter().collect())
                .unwrap_or_default();
            let ctx = MarketContext::new(pair.clone(), now, pair_ohlcv);
            scored.push(self.aggregator.aggregate(&ctx).await);
        }

        // 4. Update risk state with current equity.
        let equity = self.portfolio.equity(&marks);
        self.risk
            .update_state(&mut self.portfolio, &mut self.state, equity, now);

        // 4b. Classify regime per pair (highest-weight timeframe).
        let regime_tf = self
            .aggregator
            .timeframe_weights()
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| k.clone())
            .expect("timeframe_weights must not be empty (mirrors Python's max() on {})");
        let mut regimes: HashMap<String, Regime> = HashMap::new();
        for pair in &self.pairs {
            if let Some(candles) = ohlcv.get(pair).and_then(|m| m.get(&regime_tf)) {
                if !candles.is_empty() {
                    regimes.insert(
                        pair.clone(),
                        tradebot_core::classify_regime_default(candles),
                    );
                }
            }
        }

        // 4c. Compute Kelly stats per pair from trade history.
        let mut kelly_stats: HashMap<String, KellyStats> = HashMap::new();
        for pair in &self.pairs {
            let returns =
                self.storage
                    .round_trip_returns(self.portfolio.mode, Some(pair.as_str()), 200);
            kelly_stats.insert(pair.clone(), compute_kelly_stats(&returns));
        }

        // 5. Decide.
        let (mut actions, mut observations) = self.engine.decide(
            &scored,
            &marks,
            &slippages,
            &self.portfolio,
            &self.state,
            now,
            Some(&regimes),
            Some(&kelly_stats),
        );

        // 5b. Drain manual exit requests (user clicked "sell now" in the
        //     dashboard).
        if let Some(manual_actions) = &self.manual_actions {
            let score_by_pair: HashMap<&str, &AggregatedScore> =
                scored.iter().map(|s| (s.pair.as_str(), s)).collect();
            for req in manual_actions.drain() {
                let (base_amount, avg_entry_price) = match self.portfolio.position_for(&req.pair) {
                    Some(p) if p.base_amount > amount_epsilon() => {
                        (p.base_amount, p.avg_entry_price)
                    }
                    _ => {
                        tracing::warn!(
                            pair = req.pair.as_str(),
                            reason = req.reason.as_str(),
                            "manual_exit_no_position"
                        );
                        continue;
                    }
                };
                // Insert at the front so manual exits run before any auto
                // exits/entries.
                actions.insert(
                    0,
                    Action {
                        kind: ActionKind::Exit,
                        pair: req.pair.clone(),
                        size_quote: Decimal::ZERO,
                        size_base: base_amount,
                        confidence: 0.0,
                        reason: Some(ExitReason::Manual),
                    },
                );
                let composite = score_by_pair
                    .get(req.pair.as_str())
                    .map(|s| s.composite)
                    .unwrap_or(0.0);
                let mark = marks.get(&req.pair).copied().unwrap_or(avg_entry_price);
                let regime = regimes.get(&req.pair).map(|r| r.label.as_str().to_string());
                observations.insert(
                    0,
                    Observation {
                        timestamp: now.to_rfc3339(),
                        pair: req.pair.clone(),
                        composite,
                        mark,
                        regime,
                        decision: "exit".to_string(),
                        reason: format!("manual sell: {}", req.reason),
                        size_quote: Decimal::ZERO,
                        size_base: base_amount,
                    },
                );
            }
        }

        for o in &observations {
            push_bounded(&mut self.observations, o.clone(), OBSERVATIONS_MAX);
            tracing::info!(
                pair = o.pair.as_str(),
                decision = o.decision.as_str(),
                reason = o.reason.as_str(),
                composite = o.composite,
                mark = o.mark.to_f64().unwrap_or(0.0),
                regime = o.regime.as_deref(),
                "cycle_observation"
            );
        }

        // 6. Execute.
        for action in &actions {
            self.execute_action(action, now).await;
        }

        // 7. Snapshot equity. Note: `equity` is the pre-trade value computed
        //    in step 4, reused as-is here even though `cash`/positions may
        //    have changed since then; this matches loop.py exactly.
        let positions_value: Money = self
            .portfolio
            .open_positions()
            .into_iter()
            .map(|p| p.base_amount * marks.get(&p.pair).copied().unwrap_or(p.avg_entry_price))
            .fold(Decimal::ZERO, |acc, v| acc + v);
        self.storage.append_equity_snapshot(
            self.portfolio.mode,
            &EquitySnapshot {
                snapshot_at: now,
                equity,
                cash: self.portfolio.cash,
                positions_value,
            },
        )?;

        // 7b. Persist resumable state.
        let positions_records: Vec<PositionRecord> = self
            .portfolio
            .open_positions()
            .into_iter()
            .map(|p| PositionRecord {
                pair: p.pair.clone(),
                base_amount: p.base_amount,
                avg_entry_price: p.avg_entry_price,
                fees_paid_quote: p.fees_paid_quote,
            })
            .collect();
        self.storage.save_portfolio_state(&PortfolioState {
            mode: self.portfolio.mode,
            cash: self.portfolio.cash,
            realized_pnl_total: self.portfolio.realized_pnl_total,
            equity_high: self.portfolio.equity_high,
            sol_balance: self.portfolio.sol_balance,
            sol_gas_paid_total: self.portfolio.sol_gas_paid_total,
            positions: positions_records,
        })?;
        self.storage
            .save_risk_state(self.portfolio.mode, &self.state.to_record())?;
        let mark_history_record: MarkHistory = self
            .mark_history
            .iter()
            .filter(|(_, buf)| !buf.is_empty())
            .map(|(pair, buf)| {
                (
                    pair.clone(),
                    buf.iter().map(|(t, p)| MarkPoint::new(*t, *p)).collect(),
                )
            })
            .collect();
        self.storage
            .save_mark_history(self.portfolio.mode, &mark_history_record)?;

        // Cache for fast-tick republishes.
        self.latest_marks = marks.clone();
        self.latest_scored = scored.clone();

        // 8. Publish dashboard snapshot.
        if self.hub.is_some() {
            self.publish_snapshot(&marks, &scored, now);
        }

        Ok(())
    }

    async fn execute_action(&mut self, action: &Action, now: DateTime<Utc>) {
        let order = match action.kind {
            ActionKind::Enter => Order::buy(action.pair.clone(), action.size_quote),
            ActionKind::Exit => Order::sell(action.pair.clone(), action.size_base),
        };
        match self
            .executor
            .execute(&order, &mut self.portfolio, now)
            .await
        {
            Ok(_) => {
                self.risk.record_trade(&mut self.state, now);
            }
            Err(ExecutionError::Invalid(msg)) => {
                tracing::warn!(pair = action.pair.as_str(), kind = ?action.kind, error = %msg, "execution_failed");
            }
            Err(e) => {
                tracing::warn!(pair = action.pair.as_str(), kind = ?action.kind, error = %e, "execution_failed");
            }
        }
    }

    /// Rolling mark-price history per pair with empty series dropped,
    /// ready to hand to `build_snapshot`. Mirrors the `mark_history_dict`
    /// comprehension repeated in loop.py's step 8 and `_publish_fast_snapshot`.
    fn mark_history_snapshot(&self) -> crate::snapshot::MarkHistorySnapshot {
        self.mark_history
            .iter()
            .filter(|(_, buf)| !buf.is_empty())
            .map(|(pair, buf)| (pair.clone(), buf.iter().cloned().collect()))
            .collect()
    }

    /// Whale swaps in tokens outside the watchlist, for the dashboard's
    /// Whale Watch panel. Mirrors the `unmatched_swaps` block repeated in
    /// loop.py's step 8 and `_publish_fast_snapshot`.
    fn unmatched_whale_swaps(&self) -> Vec<WhaleActivityEntry> {
        let Some(tracker) = &self.whale_tracker else {
            return Vec::new();
        };
        let mut watched_mints: HashSet<String> =
            self.base_mints.values().map(|(m, _)| m.clone()).collect();
        watched_mints.insert(self.quote_mint.clone());
        tracker
            .unmatched_swaps(&watched_mints, 30)
            .into_iter()
            .map(|s| WhaleActivityEntry {
                ts: s.timestamp.to_rfc3339(),
                wallet: s.wallet,
                in_mint: s.in_mint,
                out_mint: s.out_mint,
                in_amount_raw: s.in_amount_raw,
                out_amount_raw: s.out_amount_raw,
                signature: s.signature,
            })
            .collect()
    }

    /// Builds a snapshot from `marks`/`scored` and publishes it to the hub,
    /// if one is configured. Shared by `run_one_cycle`'s step 8 and
    /// `_publish_fast_snapshot` in loop.py.
    fn publish_snapshot(
        &mut self,
        marks: &HashMap<String, Money>,
        scored: &[AggregatedScore],
        now: DateTime<Utc>,
    ) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let mark_history = self.mark_history_snapshot();
        let whale_activity = self.unmatched_whale_swaps();
        let limiter_metrics = self.jup_limiter.as_ref().map(|l| l.metrics());
        let observations: Vec<Observation> = self.observations.iter().cloned().collect();
        let snap = build_snapshot(
            self.storage,
            &mut self.portfolio,
            &self.state,
            marks,
            scored,
            now,
            SnapshotOptions {
                mark_history: Some(mark_history),
                observations: Some(observations),
                limiter_metrics,
                whale_activity: Some(whale_activity),
                ..SnapshotOptions::default()
            },
        );
        hub.publish(snap);
    }

    /// Fixed-cadence loop: cycle N starts at `start_time + N * interval_s`.
    ///
    /// If a cycle takes longer than `interval_s` it runs back-to-back
    /// (limited only by the shared rate limiter); shorter cycles sleep for
    /// the remainder. This keeps wall-clock cadence predictable. Mirrors
    /// `run_forever` in loop.py.
    pub async fn run_forever(&mut self, interval_s: f64) {
        let mut next_start = tokio::time::Instant::now();
        while !self.stop_signal.is_stopped() {
            let cycle_started = tokio::time::Instant::now();
            if let Err(e) = self.run_one_cycle(Utc::now()).await {
                tracing::error!(error = %e, "cycle_failed");
            }
            let cycle_duration = cycle_started.elapsed().as_secs_f64();
            if cycle_duration > interval_s {
                tracing::warn!(
                    duration_s = cycle_duration,
                    interval_s,
                    hint = "rate-limited or slow network; cycles running back-to-back",
                    "cycle_overrun"
                );
            }
            next_start += Duration::from_secs_f64(interval_s);
            let now_inst = tokio::time::Instant::now();
            let sleep_dur = next_start.saturating_duration_since(now_inst);
            if sleep_dur > Duration::ZERO {
                self.stop_signal.wait_timeout(sleep_dur).await;
            } else {
                // Cycle overran: reset the schedule to "now" so we don't
                // burn CPU catching up.
                next_start = tokio::time::Instant::now();
            }
        }
    }

    /// Lightweight Birdeye-only loop that refreshes chart prices and
    /// republishes the dashboard snapshot at sub-second cadence between
    /// decision cycles. Mirrors `run_fast_ticks` in loop.py.
    ///
    /// Skips entirely when:
    ///   - no Birdeye client (would need Jupiter probes, too expensive at 1s)
    ///   - no dashboard hub (nothing to publish to)
    ///   - no cached scored signals yet (first decision cycle hasn't run)
    ///
    /// Unlike loop.py this has no per-tick try/catch: `BirdeyeClient::
    /// multi_price` and `DashboardHub::publish` are infallible in the Rust
    /// port (errors are already logged and swallowed inside them), so there
    /// is nothing left that can raise here.
    pub async fn run_fast_ticks(&mut self, interval_s: f64) {
        if self.birdeye.is_none() || self.hub.is_none() {
            tracing::info!(
                reason = "needs both Birdeye client and dashboard hub",
                "fast_tick_disabled"
            );
            return;
        }

        let mut next_start = tokio::time::Instant::now();
        while !self.stop_signal.is_stopped() {
            self.run_one_fast_tick().await;
            next_start += Duration::from_secs_f64(interval_s);
            let now_inst = tokio::time::Instant::now();
            let sleep_dur = next_start.saturating_duration_since(now_inst);
            if sleep_dur > Duration::ZERO {
                self.stop_signal.wait_timeout(sleep_dur).await;
            } else {
                next_start = tokio::time::Instant::now();
            }
        }
    }

    /// Runs a single fast-tick iteration (one Birdeye refresh + snapshot
    /// republish), or does nothing if Birdeye/hub are not configured or no
    /// decision cycle has produced cached scored signals yet. Factored out
    /// of `run_fast_ticks` so a caller that needs to interleave fast ticks
    /// with the main decision cycle on one `TradingLoop` can drive both from
    /// a single task (e.g. via `tokio::select!`) rather than needing two
    /// concurrent `&mut self` borrows, which `run_forever` and
    /// `run_fast_ticks` do not allow across separate tasks.
    pub async fn run_one_fast_tick(&mut self) {
        let (birdeye, _hub) = match (self.birdeye.clone(), self.hub.clone()) {
            (Some(b), Some(h)) => (b, h),
            _ => return,
        };
        if self.latest_scored.is_empty() {
            return;
        }
        let mints: Vec<String> = self.base_mints.values().map(|(m, _)| m.clone()).collect();
        let mint_to_pair: HashMap<String, String> = self
            .base_mints
            .iter()
            .map(|(pair, (m, _))| (m.clone(), pair.clone()))
            .collect();
        let prices = birdeye.multi_price(&mints).await;
        if prices.is_empty() {
            return;
        }
        let now = Utc::now();
        for (mint, price) in prices {
            let Some(pair) = mint_to_pair.get(&mint) else {
                continue;
            };
            let price = money_from_f64(price);
            self.latest_marks.insert(pair.clone(), price);
            let buf = self.mark_history.entry(pair.clone()).or_default();
            push_bounded(buf, (now, price), self.chart_history_max);
        }
        let marks_snapshot = self.latest_marks.clone();
        let scored_snapshot = self.latest_scored.clone();
        self.publish_snapshot(&marks_snapshot, &scored_snapshot, now);
    }
}
