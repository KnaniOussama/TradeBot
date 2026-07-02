//! Numeric-parity test: run the Rust `run_backtest` end-to-end against the
//! same OHLCV CSV fixture, real `TASignal`, and default `RiskConfig` as the
//! real Python `tradebot.backtest.runner.run_backtest`, captured ahead of
//! time into `tests/fixtures/backtest_parity.json` by
//! `tests/fixtures/gen_backtest_fixtures.py` (run via the project's Python
//! venv; see that script to regenerate).
//!
//! Two scenarios are checked:
//!   - `runner_parity`: default `entry_threshold=0.6` -- the real TASignal
//!     never crosses this threshold on this fixture, so both ports should
//!     report zero trades and an unchanged final equity.
//!   - `runner_parity_active`: `entry_threshold=0.2` -- crosses the
//!     threshold twice (an entry, then a take-profit-ladder partial exit),
//!     exercising the full fill/fee/slippage/PnL path with real Money
//!     amounts to compare against Python's floats.
//!
//! Tolerance: exact match for bars_processed/n_trades/n_wins/n_losses (these
//! are integers with no room for drift); 1e-9 absolute for
//! final_equity/total_return_pct/max_drawdown_pct/realized_pnl and for each
//! per-trade amount (base_amount/quote_amount/price/fee_quote). In practice
//! the observed diffs are far tighter than that (down to 1e-12), since the
//! Rust side computes the whole fill/fee/PnL chain in exact Decimal
//! arithmetic and only converts to f64 at the boundary (metrics, this
//! test's comparisons), while Python computes the same chain in float
//! throughout; 1e-9 leaves headroom without weakening the check.

use std::path::Path;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use tradebot_backtest::{load_csv_path, run_backtest, BacktestParams};
use tradebot_config::models::RiskConfig;
use tradebot_signals::TASignal;
use tradebot_storage::Side;

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_parity() -> Value {
    let text = std::fs::read_to_string(fixtures_dir().join("backtest_parity.json"))
        .expect("read backtest_parity.json");
    serde_json::from_str(&text).expect("parse backtest_parity.json")
}

fn dec(v: f64) -> Decimal {
    Decimal::from_f64_retain(v).unwrap()
}

async fn run_scenario(entry_threshold: f64) -> tradebot_backtest::BacktestResult {
    let ohlcv = load_csv_path(&fixtures_dir().join("backtest_sol_uptrend.csv")).unwrap();

    let mut params = BacktestParams::new("SOL/USDC");
    params.starting_cash = dec(100.0);
    params.fee_bps = 30;
    params.slippage_bps = 5;
    params.warmup_bars = 50;
    params.entry_threshold = entry_threshold;
    params.exit_flip_threshold = -0.3;
    params.bar_seconds = 60;

    run_backtest(
        &ohlcv,
        params,
        vec![Box::new(TASignal::new("1m"))],
        [("1m".to_string(), 1.0)].into_iter().collect(),
        [("ta".to_string(), 1.0)].into_iter().collect(),
        RiskConfig::default(),
        "parity_run",
    )
    .await
    .unwrap()
}

fn assert_close(label: &str, actual: f64, expected: f64, tol: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff <= tol,
        "{label}: actual={actual} expected={expected} diff={diff} exceeds tolerance {tol}"
    );
}

#[tokio::test]
async fn runner_matches_python_default_threshold() {
    let root = load_parity();
    let expected = &root["runner_parity"];

    let result = run_scenario(expected["entry_threshold"].as_f64().unwrap()).await;

    assert_eq!(
        result.bars_processed,
        expected["bars_processed"].as_u64().unwrap() as usize
    );
    assert_eq!(
        result.n_trades,
        expected["n_trades"].as_u64().unwrap() as usize
    );
    assert_eq!(result.n_wins, expected["n_wins"].as_i64().unwrap());
    assert_eq!(result.n_losses, expected["n_losses"].as_i64().unwrap());
    assert_close(
        "final_equity",
        result.final_equity,
        expected["final_equity"].as_f64().unwrap(),
        1e-9,
    );
    assert_close(
        "total_return_pct",
        result.total_return_pct,
        expected["total_return_pct"].as_f64().unwrap(),
        1e-9,
    );
}

#[tokio::test]
async fn runner_matches_python_active_threshold() {
    let root = load_parity();
    let expected = &root["runner_parity_active"];

    let result = run_scenario(expected["entry_threshold"].as_f64().unwrap()).await;

    assert_eq!(
        result.bars_processed,
        expected["bars_processed"].as_u64().unwrap() as usize
    );
    assert_eq!(
        result.n_trades,
        expected["n_trades"].as_u64().unwrap() as usize
    );
    assert_eq!(result.n_wins, expected["n_wins"].as_i64().unwrap());
    assert_eq!(result.n_losses, expected["n_losses"].as_i64().unwrap());

    assert_close(
        "final_equity",
        result.final_equity,
        expected["final_equity"].as_f64().unwrap(),
        1e-9,
    );
    assert_close(
        "total_return_pct",
        result.total_return_pct,
        expected["total_return_pct"].as_f64().unwrap(),
        1e-9,
    );
    assert_close(
        "max_drawdown_pct",
        result.max_drawdown_pct,
        expected["max_drawdown_pct"].as_f64().unwrap(),
        1e-9,
    );
    assert_close(
        "realized_pnl",
        result.realized_pnl.to_f64().unwrap(),
        expected["realized_pnl"].as_f64().unwrap(),
        1e-9,
    );

    let expected_trades = expected["trades"].as_array().unwrap();
    assert_eq!(result.trades.len(), expected_trades.len());
    for (actual, want) in result.trades.iter().zip(expected_trades) {
        let want_side = want["side"].as_str().unwrap();
        match actual.side {
            Side::Buy => assert_eq!(want_side, "buy"),
            Side::Sell => assert_eq!(want_side, "sell"),
        }
        assert_close(
            "trade.base_amount",
            actual.base_amount.to_f64().unwrap(),
            want["base_amount"].as_f64().unwrap(),
            1e-9,
        );
        assert_close(
            "trade.quote_amount",
            actual.quote_amount.to_f64().unwrap(),
            want["quote_amount"].as_f64().unwrap(),
            1e-9,
        );
        assert_close(
            "trade.price",
            actual.price.to_f64().unwrap(),
            want["price"].as_f64().unwrap(),
            1e-9,
        );
        assert_close(
            "trade.fee_quote",
            actual.fee_quote.to_f64().unwrap(),
            want["fee_quote"].as_f64().unwrap(),
            1e-9,
        );
    }
}
