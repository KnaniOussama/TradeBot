//! Port of `tests/backtest/test_regime_backtest.py` ("Phase 10 Task 3" smoke
//! test): regime filter ON vs OFF, and Kelly sizing ON vs OFF, both complete
//! and produce identical trade counts.
//!
//! NOTE: Regime gating is skipped in the backtest runner v1 (`regimes` is
//! never passed to `DecisionEngine::decide` -- see the doc comment on
//! `run_backtest`). This test validates that different `RiskConfig` settings
//! (`regime_filter_enabled` true/false, `use_kelly_sizing` true/false) passed
//! to `run_backtest` produce identical results in the current
//! implementation. A TODO for v2 is to wire per-bar regime computation into
//! the runner; this test documents that known omission, matching the Python
//! reference test's own documentation.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tradebot_backtest::{run_backtest, BacktestParams};
use tradebot_config::models::RiskConfig;
use tradebot_signals::{MarketContext, Signal, SignalScore};
use tradebot_storage::Candle;

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

fn base_ts() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap()
}

/// Clear uptrend OHLCV fixture, strong directional move.
fn make_ohlcv_uptrend(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let close = 100.0 + i as f64 * 1.0;
            Candle {
                timestamp: base_ts() + Duration::minutes(i as i64),
                open: Decimal::from_f64_retain(close - 0.1).unwrap(),
                high: Decimal::from_f64_retain(close + 0.8).unwrap(),
                low: Decimal::from_f64_retain(close - 0.8).unwrap(),
                close: Decimal::from_f64_retain(close).unwrap(),
                volume: Decimal::new(1000, 0),
            }
        })
        .collect()
}

fn weights(pairs: &[(&str, f64)]) -> IndexMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn params() -> BacktestParams {
    let mut p = BacktestParams::new("SOL/USDC");
    p.warmup_bars = 50;
    p.starting_cash = Decimal::new(100, 0);
    p
}

#[tokio::test]
async fn regime_filter_on_vs_off_both_complete() {
    let ohlcv = make_ohlcv_uptrend(80);

    let result_off = run_backtest(
        &ohlcv,
        params(),
        vec![Box::new(AlwaysBullSignal)],
        weights(&[("1m", 1.0)]),
        weights(&[("ta", 1.0)]),
        RiskConfig {
            regime_filter_enabled: false,
            ..RiskConfig::default()
        },
        "regime_off",
    )
    .await
    .unwrap();

    let result_on = run_backtest(
        &ohlcv,
        params(),
        vec![Box::new(AlwaysBullSignal)],
        weights(&[("1m", 1.0)]),
        weights(&[("ta", 1.0)]),
        RiskConfig {
            regime_filter_enabled: true,
            ..RiskConfig::default()
        },
        "regime_on",
    )
    .await
    .unwrap();

    assert_eq!(result_off.bars_processed, 30);
    assert_eq!(result_on.bars_processed, 30);
    assert!(result_off.final_equity > 0.0);
    assert!(result_on.final_equity > 0.0);
    // v1: regime skipped in runner -> same result; document with comment.
    // v2: update this to assert result_on.n_trades <= result_off.n_trades.
    assert_eq!(
        result_off.n_trades, result_on.n_trades,
        "Backtest runner v1 skips regime computation; both configs produce same trades. \
         Update when runner wires regime per-bar."
    );
}

#[tokio::test]
async fn kelly_on_vs_off_both_complete() {
    let ohlcv = make_ohlcv_uptrend(80);

    let result_kelly = run_backtest(
        &ohlcv,
        params(),
        vec![Box::new(AlwaysBullSignal)],
        weights(&[("1m", 1.0)]),
        weights(&[("ta", 1.0)]),
        RiskConfig {
            use_kelly_sizing: true,
            ..RiskConfig::default()
        },
        "kelly_on",
    )
    .await
    .unwrap();

    let result_linear = run_backtest(
        &ohlcv,
        params(),
        vec![Box::new(AlwaysBullSignal)],
        weights(&[("1m", 1.0)]),
        weights(&[("ta", 1.0)]),
        RiskConfig {
            use_kelly_sizing: false,
            ..RiskConfig::default()
        },
        "kelly_off",
    )
    .await
    .unwrap();

    assert_eq!(result_kelly.bars_processed, 30);
    assert_eq!(result_linear.bars_processed, 30);
    assert!(result_kelly.final_equity > 0.0);
    assert!(result_linear.final_equity > 0.0);
}
