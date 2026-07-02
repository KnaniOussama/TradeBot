//! Numeric-parity test: run the Rust `compute_metrics` against the same
//! fixed equity curve / trade list as the real Python
//! `tradebot.backtest.metrics.compute_metrics`, captured ahead of time into
//! `tests/fixtures/backtest_parity.json` by
//! `tests/fixtures/gen_backtest_fixtures.py` (run via the project's Python
//! venv; see that script to regenerate).
//!
//! Tolerance: 1e-9 absolute for total_return_pct and max_drawdown_pct, 1e-6
//! relative for sharpe (its magnitude here is in the hundreds, so this is
//! still far tighter than any observable difference), exact match for
//! final_equity/n_wins/n_losses.

use std::path::Path;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tradebot_backtest::{compute_metrics, TradeOutcome};
use tradebot_storage::Side;

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_parity() -> Value {
    let text = std::fs::read_to_string(fixtures_dir().join("backtest_parity.json"))
        .expect("read backtest_parity.json");
    serde_json::from_str(&text).expect("parse backtest_parity.json")
}

fn ts(i: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap() + Duration::minutes(i)
}

#[test]
fn compute_metrics_matches_python_golden() {
    let root = load_parity();
    let golden = &root["metrics_golden"];

    let equity_values: Vec<f64> = golden["equity_values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let starting_cash =
        Decimal::from_f64_retain(golden["starting_cash"].as_f64().unwrap()).unwrap();
    let bar_seconds = golden["bar_seconds"].as_u64().unwrap() as u32;

    let curve: Vec<(DateTime<Utc>, Decimal)> = equity_values
        .iter()
        .enumerate()
        .map(|(i, v)| (ts(i as i64), Decimal::from_f64_retain(*v).unwrap()))
        .collect();

    let trades = vec![
        TradeOutcome {
            side: Side::Buy,
            price: Decimal::new(1000, 1),
        },
        TradeOutcome {
            side: Side::Sell,
            price: Decimal::new(1080, 1),
        },
        TradeOutcome {
            side: Side::Buy,
            price: Decimal::new(1065, 1),
        },
        TradeOutcome {
            side: Side::Sell,
            price: Decimal::new(990, 1),
        },
        TradeOutcome {
            side: Side::Buy,
            price: Decimal::new(10125, 2),
        },
        TradeOutcome {
            side: Side::Sell,
            price: Decimal::new(11825, 2),
        },
    ];

    let result = compute_metrics(&curve, &trades, starting_cash, bar_seconds);

    let expected = &golden["result"];
    assert_eq!(
        result.final_equity,
        expected["final_equity"].as_f64().unwrap()
    );
    assert_eq!(result.n_wins, expected["n_wins"].as_i64().unwrap());
    assert_eq!(result.n_losses, expected["n_losses"].as_i64().unwrap());

    let want_dd = expected["max_drawdown_pct"].as_f64().unwrap();
    assert!(
        (result.max_drawdown_pct - want_dd).abs() < 1e-9,
        "max_drawdown_pct: actual={} expected={}",
        result.max_drawdown_pct,
        want_dd
    );

    let want_return = expected["total_return_pct"].as_f64().unwrap();
    assert!(
        (result.total_return_pct - want_return).abs() < 1e-9,
        "total_return_pct: actual={} expected={}",
        result.total_return_pct,
        want_return
    );

    let want_sharpe = expected["sharpe"].as_f64().unwrap();
    let sharpe_diff = (result.sharpe - want_sharpe).abs();
    assert!(
        sharpe_diff < want_sharpe.abs() * 1e-6 + 1e-9,
        "sharpe: actual={} expected={} diff={}",
        result.sharpe,
        want_sharpe,
        sharpe_diff
    );
}
