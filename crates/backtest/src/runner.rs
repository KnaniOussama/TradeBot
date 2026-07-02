//! Port of `tradebot/backtest/runner.py`: the replay loop that steps through
//! candles, runs the (TA-only) signal aggregator + decision engine +
//! synthetic execution, and produces the backtest result (equity curve,
//! trades, metrics).
//!
//! v1 limitation preserved from the Python reference: the runner does not
//! compute per-bar regimes or Kelly stats, so `DecisionEngine::decide` is
//! always called with `regimes = None` and `kelly_stats = None` (see
//! `tests/backtest/test_regime_backtest.py`'s documented omission). Do not
//! wire per-bar regime classification into this runner without updating
//! that behavior deliberately.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tradebot_common::{Mode, Money};
use tradebot_config::models::RiskConfig;
use tradebot_core::{
    ActionKind, DecisionEngine, Portfolio, PortfolioError, RiskManager, RiskState, SignalAggregator,
};
use tradebot_execution::Order;
use tradebot_signals::{MarketContext, Signal};
use tradebot_storage::{Candle, Side};

use crate::executor::SyntheticExecutor;
use crate::metrics::{compute_metrics, TradeOutcome};

/// One fill recorded during the replay. Mirrors `BacktestTrade` in
/// runner.py.
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestTrade {
    pub timestamp: String,
    pub side: Side,
    pub base_amount: Money,
    pub quote_amount: Money,
    pub price: Money,
    pub fee_quote: Money,
}

/// One equity-curve sample. Mirrors the `{"t": ..., "e": ...}` dict runner.py
/// builds for `BacktestResult.equity_curve`.
#[derive(Debug, Clone, PartialEq)]
pub struct EquityPoint {
    pub t: String,
    pub e: Money,
}

/// Backtest run parameters. Mirrors `BacktestParams` in runner.py; `pair`
/// has no default (it is the one required dataclass field), so it is taken
/// by `BacktestParams::new` and the rest default to the Python dataclass
/// defaults.
#[derive(Debug, Clone)]
pub struct BacktestParams {
    pub pair: String,
    pub timeframe: String,
    pub starting_cash: Money,
    pub fee_bps: u32,
    pub slippage_bps: u32,
    pub warmup_bars: usize,
    pub entry_threshold: f64,
    pub exit_flip_threshold: f64,
    pub bar_seconds: u32,
}

impl BacktestParams {
    pub fn new(pair: impl Into<String>) -> Self {
        Self {
            pair: pair.into(),
            timeframe: "1m".to_string(),
            starting_cash: Decimal::new(500, 1), // 50.0
            fee_bps: 30,
            slippage_bps: 5,
            warmup_bars: 50,
            entry_threshold: 0.6,
            exit_flip_threshold: -0.3,
            bar_seconds: 60,
        }
    }
}

/// Full backtest output. Mirrors `BacktestResult` in runner.py.
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub id: String,
    pub pair: String,
    pub starting_cash: Money,
    pub final_equity: f64,
    pub total_return_pct: f64,
    pub realized_pnl: Money,
    pub n_trades: usize,
    pub n_wins: i64,
    pub n_losses: i64,
    pub max_drawdown_pct: f64,
    pub sharpe: f64,
    pub bars_processed: usize,
    pub equity_curve: Vec<EquityPoint>,
    pub trades: Vec<BacktestTrade>,
    pub params: BacktestParams,
    pub completed_at: String,
}

/// Runs a full backtest replay over `ohlcv`. Mirrors `run_backtest` in
/// runner.py: for each bar past `warmup_bars`, builds a `MarketContext` from
/// every candle up to and including that bar, scores it with the signal
/// aggregator, runs the decision engine (no regime/Kelly data in v1), fills
/// any resulting orders through the `SyntheticExecutor`, and records the
/// post-fill equity.
pub async fn run_backtest(
    ohlcv: &[Candle],
    params: BacktestParams,
    signals: Vec<Box<dyn Signal>>,
    timeframe_weights: IndexMap<String, f64>,
    signal_weights: IndexMap<String, f64>,
    risk: RiskConfig,
    backtest_id: impl Into<String>,
) -> Result<BacktestResult, PortfolioError> {
    let mut portfolio = Portfolio::new(Mode::Backtest, params.starting_cash, Decimal::ZERO);
    let mut state = RiskState::default();
    // `RiskManager` is stateless beyond its config, so the runner's own
    // manager (used for `update_state`/`record_trade`) and the decision
    // engine's manager (used for entry gating) are separate instances built
    // from the same config. In runner.py this is literally one shared
    // `RiskManager` object; the two are behaviorally identical here.
    let risk_mgr = RiskManager::new(risk.clone());
    let aggregator = SignalAggregator::new(signals, timeframe_weights, signal_weights);
    let mut engine = DecisionEngine::new(
        RiskManager::new(risk),
        params.entry_threshold,
        params.exit_flip_threshold,
    );
    let executor = SyntheticExecutor::new(params.fee_bps, params.slippage_bps);

    let mut trades: Vec<BacktestTrade> = Vec::new();
    let mut equity_curve: Vec<(DateTime<Utc>, Money)> = Vec::new();

    for i in params.warmup_bars..ohlcv.len() {
        let bar = &ohlcv[i];
        let now = bar.timestamp;
        let window: Vec<Candle> = ohlcv[..=i].to_vec();
        let ctx = MarketContext::new(
            params.pair.clone(),
            now,
            HashMap::from([(params.timeframe.clone(), window)]),
        );
        let agg_score = aggregator.aggregate(&ctx).await;
        let marks: HashMap<String, Money> = HashMap::from([(params.pair.clone(), bar.close)]);
        let slippages: HashMap<String, f64> =
            HashMap::from([(params.pair.clone(), params.slippage_bps as f64 / 10_000.0)]);

        let current_equity = portfolio.equity(&marks);
        risk_mgr.update_state(&mut portfolio, &mut state, current_equity, now);
        let (actions, _observations) = engine.decide(
            &[agg_score],
            &marks,
            &slippages,
            &portfolio,
            &state,
            now,
            None,
            None,
        );

        for action in &actions {
            let order = match action.kind {
                ActionKind::Enter => Order::buy(params.pair.clone(), action.size_quote),
                ActionKind::Exit => Order::sell(params.pair.clone(), action.size_base),
            };
            let mark = marks[&params.pair];
            let fill = executor.execute(&order, &mut portfolio, now, mark).await?;
            risk_mgr.record_trade(&mut state, now);
            trades.push(BacktestTrade {
                timestamp: now.to_rfc3339(),
                side: fill.side,
                base_amount: fill.base_amount,
                quote_amount: fill.quote_amount,
                price: fill.price,
                fee_quote: fill.fee_quote,
            });
        }

        equity_curve.push((now, portfolio.equity(&marks)));
    }

    let trade_outcomes: Vec<TradeOutcome> = trades
        .iter()
        .map(|t| TradeOutcome {
            side: t.side,
            price: t.price,
        })
        .collect();
    let metrics = compute_metrics(
        &equity_curve,
        &trade_outcomes,
        params.starting_cash,
        params.bar_seconds,
    );

    let bars_processed = ohlcv.len() - params.warmup_bars;
    let equity_curve_out: Vec<EquityPoint> = equity_curve
        .iter()
        .map(|(ts, eq)| EquityPoint {
            t: ts.to_rfc3339(),
            e: *eq,
        })
        .collect();

    Ok(BacktestResult {
        id: backtest_id.into(),
        pair: params.pair.clone(),
        starting_cash: params.starting_cash,
        final_equity: metrics.final_equity,
        total_return_pct: metrics.total_return_pct,
        realized_pnl: portfolio.realized_pnl_total,
        n_trades: trades.len(),
        n_wins: metrics.n_wins,
        n_losses: metrics.n_losses,
        max_drawdown_pct: metrics.max_drawdown_pct,
        sharpe: metrics.sharpe,
        bars_processed,
        equity_curve: equity_curve_out,
        trades,
        params,
        completed_at: Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{Duration, TimeZone};
    use tradebot_signals::SignalScore;

    struct AlwaysBullSignal;

    #[async_trait]
    impl Signal for AlwaysBullSignal {
        fn name(&self) -> &str {
            "ta"
        }
        fn timeframe(&self) -> &str {
            "1m"
        }
        async fn score(&self, ctx: &MarketContext) -> SignalScore {
            SignalScore::new("ta", ctx.pair.clone(), "1m", 1.0, ctx.now, HashMap::new())
                .expect("fixed score in range")
        }
    }

    fn make_ohlcv(n: usize, start_price: f64, trend: f64) -> Vec<Candle> {
        let base_ts = Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();
        (0..n)
            .map(|i| {
                let close = start_price + i as f64 * trend;
                let open = close - 0.1;
                let high = close + 0.5;
                let low = close - 0.5;
                Candle {
                    timestamp: base_ts + Duration::minutes(i as i64),
                    open: Decimal::from_f64_retain(open).unwrap(),
                    high: Decimal::from_f64_retain(high).unwrap(),
                    low: Decimal::from_f64_retain(low).unwrap(),
                    close: Decimal::from_f64_retain(close).unwrap(),
                    volume: Decimal::new(1000, 0),
                }
            })
            .collect()
    }

    fn weights(pairs: &[(&str, f64)]) -> IndexMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[tokio::test]
    async fn result_has_equity_curve_and_trades() {
        let ohlcv = make_ohlcv(80, 100.0, 0.5);
        let mut params = BacktestParams::new("SOL/USDC");
        params.starting_cash = Decimal::new(100, 0);
        params.warmup_bars = 50;

        let result = run_backtest(
            &ohlcv,
            params,
            vec![Box::new(AlwaysBullSignal)],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
            RiskConfig::default(),
            "test001",
        )
        .await
        .unwrap();

        assert_eq!(result.bars_processed, 30);
        assert_eq!(result.equity_curve.len(), 30);
        assert_eq!(result.id, "test001");
        assert_eq!(result.pair, "SOL/USDC");
        assert_eq!(result.starting_cash, Decimal::new(100, 0));
    }

    #[tokio::test]
    async fn warmup_bars_skipped() {
        let ohlcv = make_ohlcv(100, 100.0, 0.5);
        let mut params = BacktestParams::new("SOL/USDC");
        params.warmup_bars = 70;
        params.starting_cash = Decimal::new(100, 0);

        let result = run_backtest(
            &ohlcv,
            params,
            vec![Box::new(AlwaysBullSignal)],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
            RiskConfig::default(),
            "warmup_test",
        )
        .await
        .unwrap();

        assert_eq!(result.bars_processed, 30);
    }

    #[tokio::test]
    async fn strong_bull_signal_triggers_entry() {
        let ohlcv = make_ohlcv(80, 100.0, 0.01);
        let mut params = BacktestParams::new("SOL/USDC");
        params.starting_cash = Decimal::new(100, 0);
        params.warmup_bars = 50;

        let result = run_backtest(
            &ohlcv,
            params,
            vec![Box::new(AlwaysBullSignal)],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
            RiskConfig::default(),
            "bull_test",
        )
        .await
        .unwrap();

        assert!(result.n_trades >= 1);
    }

    #[tokio::test]
    async fn final_equity_reflects_trades() {
        let ohlcv = make_ohlcv(80, 100.0, 1.0);
        let mut params = BacktestParams::new("SOL/USDC");
        params.starting_cash = Decimal::new(100, 0);
        params.warmup_bars = 50;

        let result = run_backtest(
            &ohlcv,
            params,
            vec![Box::new(AlwaysBullSignal)],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
            RiskConfig::default(),
            "equity_test",
        )
        .await
        .unwrap();

        assert!(result.final_equity > 0.0);
        assert!(!result.completed_at.is_empty());
    }
}
