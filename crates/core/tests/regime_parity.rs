//! Numeric-parity test: run the Rust regime classifier against the same
//! OHLCV CSV fixtures as `tests/core/test_regime.py`'s style of test, and
//! assert the label and ADX value match the real Python
//! `tradebot.core.regime.classify_regime`, captured ahead of time into
//! `tests/fixtures/regime_parity.json` by
//! `tests/fixtures/gen_regime_fixtures.py` (run via the project's Python
//! venv; see that script for how to regenerate).
//!
//! Tolerance: label must match exactly; ADX is compared with an absolute
//! tolerance of 1e-9. The ADX/DMI EWM recursion here (`ewm_adjust_false`)
//! reproduces pandas' `ewm(adjust=False)` NaN-hold semantics exactly (see
//! `regime.rs` module docs); measured diffs on all three fixtures are
//! ~1e-13 (float summation-order noise between this port's straight-line
//! recursion and pandas' internal Cython implementation), well inside the
//! 1e-9 contract.

use std::path::Path;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tradebot_core::regime::classify_regime;
use tradebot_storage::Candle;

const ADX_TOLERANCE: f64 = 1e-9;

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

#[test]
fn regime_classifier_matches_python_reference() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expected_json: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures_dir.join("regime_parity.json"))
            .expect("read parity json"),
    )
    .expect("parse parity json");

    let csv_names = [
        "ohlcv_sol_uptrend.csv",
        "ohlcv_sol_downtrend.csv",
        "ohlcv_sol_choppy.csv",
    ];

    for name in csv_names {
        let candles = load_candles(&fixtures_dir.join(name));
        let r = classify_regime(&candles, 14, 20, 50, 25.0, 20.0);

        let expected = &expected_json[name];
        let expected_label = expected["label"].as_str().unwrap();
        let expected_adx = expected["adx"].as_f64().unwrap();
        let expected_fast_above = expected["ema_fast_above_slow"].as_bool().unwrap();

        assert_eq!(r.label.as_str(), expected_label, "{name}: label mismatch");
        assert_eq!(
            r.ema_fast_above_slow, expected_fast_above,
            "{name}: ema_fast_above_slow mismatch"
        );
        let diff = (r.adx - expected_adx).abs();
        assert!(
            diff <= ADX_TOLERANCE,
            "{name}: adx actual={} expected={} diff={} exceeds tolerance {}",
            r.adx,
            expected_adx,
            diff,
            ADX_TOLERANCE
        );
    }
}
