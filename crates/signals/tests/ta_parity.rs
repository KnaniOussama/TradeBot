//! Numeric-parity tests: run the Rust TA signal against the same OHLCV CSV
//! fixtures as `tests/signals/test_ta.py` and assert each component score
//! (and the composite) matches the real Python implementation, which was
//! captured ahead of time into `tests/fixtures/ta_parity.json` by
//! `tests/fixtures/gen_ta_fixtures.py` (run via the project's Python venv;
//! see that script for how to regenerate).
//!
//! Tolerance: 1e-9 absolute, for every component and the composite, on all
//! three fixtures. The EMA/EWM recursions and rolling windows here are
//! implemented with the same operation order as pandas, so no indicator
//! needed a looser tolerance.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tradebot_signals::base::{MarketContext, Signal};
use tradebot_signals::ta::TASignal;
use tradebot_storage::Candle;

const TOLERANCE: f64 = 1e-9;

fn load_candles(path: &Path) -> Vec<Candle> {
    let text = std::fs::read_to_string(path).expect("read fixture csv");
    let mut lines = text.lines();
    let header = lines.next().expect("csv header");
    let cols: Vec<&str> = header.split(',').collect();
    let idx = |name: &str| cols.iter().position(|c| *c == name).expect("column");
    let (ts_i, o_i, h_i, l_i, c_i, v_i) = (
        idx("timestamp"),
        idx("open"),
        idx("high"),
        idx("low"),
        idx("close"),
        idx("volume"),
    );

    lines
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            Candle {
                timestamp: DateTime::parse_from_rfc3339(fields[ts_i])
                    .expect("parse timestamp")
                    .with_timezone(&Utc),
                open: fields[o_i].parse::<Decimal>().expect("parse open"),
                high: fields[h_i].parse::<Decimal>().expect("parse high"),
                low: fields[l_i].parse::<Decimal>().expect("parse low"),
                close: fields[c_i].parse::<Decimal>().expect("parse close"),
                volume: fields[v_i].parse::<Decimal>().expect("parse volume"),
            }
        })
        .collect()
}

fn assert_close(label: &str, actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff <= TOLERANCE,
        "{label}: actual={actual} expected={expected} diff={diff} exceeds tolerance {TOLERANCE}"
    );
}

#[tokio::test]
async fn ta_signal_matches_python_reference() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expected_json: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures_dir.join("ta_parity.json")).expect("read parity json"),
    )
    .expect("parse parity json");

    let csv_names = [
        "ohlcv_sol_uptrend.csv",
        "ohlcv_sol_downtrend.csv",
        "ohlcv_sol_choppy.csv",
    ];

    for name in csv_names {
        let candles = load_candles(&fixtures_dir.join(name));
        let ctx = MarketContext::new(
            "SOL/USDC",
            Utc::now(),
            HashMap::from([("1m".to_string(), candles)]),
        );
        let sig = TASignal::new("1m");
        let score = sig.score(&ctx).await;

        let expected = &expected_json[name];
        for key in ["rsi", "macd", "ema_cross", "bb", "atr_mom"] {
            let actual = score.components[key];
            let want = expected["components"][key].as_f64().unwrap();
            assert_close(&format!("{name}/{key}"), actual, want);
        }
        assert_close(
            &format!("{name}/composite"),
            score.score,
            expected["composite"].as_f64().unwrap(),
        );
    }
}
