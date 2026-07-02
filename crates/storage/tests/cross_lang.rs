//! Verifies the other parity direction: files written by Rust must be valid
//! JSON that Python's `JsonStorage` loader parses to the correct values.
//! Shells out to the repo's Python venv; skips (rather than fails) if the
//! venv or repo layout isn't available, so this doesn't break `cargo test`
//! in environments without the Python side checked out.

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::process::Command;
use tradebot_common::Mode;
use tradebot_storage::records::{OhlcvCandle, PositionRecord, RiskStateRecord, Side};
use tradebot_storage::{EquitySnapshot, JsonStorage, PortfolioState, Trade};

fn money(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn repo_root() -> PathBuf {
    // crates/storage -> crates -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn python_exe() -> Option<PathBuf> {
    let p = repo_root().join(".venv/Scripts/python.exe");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Render a path as a Python double-quoted string literal. Uses forward
/// slashes (Windows accepts them fine) so no backslash-escaping is needed.
fn py_str(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    format!("\"{}\"", s.replace('"', "\\\""))
}

fn fresh_tmp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tradebot_storage_cross_lang_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    p
}

#[test]
fn rust_written_state_loads_correctly_in_python() {
    let Some(python) = python_exe() else {
        eprintln!("skipping: .venv/Scripts/python.exe not found, cannot check Python-side load");
        return;
    };

    let dir = fresh_tmp_dir("rust_writes");
    let storage = JsonStorage::new(&dir).unwrap();

    let opened_at = Utc.with_ymd_and_hms(2026, 6, 1, 9, 30, 0).unwrap();
    storage
        .append_trade(&Trade {
            mode: Mode::Demo,
            pair: "SOL/USDC".to_string(),
            side: Side::Buy,
            base_amount: money("0.2"),
            quote_amount: money("20.0"),
            price: money("100.0"),
            fee_quote: money("0.02"),
            slippage_pct: 0.0015,
            tx_signature: Some("rustsig123".to_string()),
            opened_at,
            confidence: Some(0.65),
            notes: Some("written by rust".to_string()),
        })
        .unwrap();

    storage
        .save_portfolio_state(&PortfolioState {
            mode: Mode::Demo,
            cash: money("77.25"),
            realized_pnl_total: money("3.1"),
            equity_high: money("80.0"),
            sol_balance: money("0.5"),
            sol_gas_paid_total: money("0.001"),
            positions: vec![PositionRecord {
                pair: "SOL/USDC".to_string(),
                base_amount: money("0.2"),
                avg_entry_price: money("100.0"),
                fees_paid_quote: money("0.02"),
            }],
        })
        .unwrap();

    let mut risk_state = RiskStateRecord {
        trades_per_day: Default::default(),
        daily_start_equity: Default::default(),
        weekly_start_equity: Default::default(),
        day_paused_until: Some(NaiveDate::from_ymd_opt(2026, 6, 2).unwrap()),
        week_paused_until: None,
        kill_switch_active: false,
        kill_switch_reason: String::new(),
    };
    risk_state
        .trades_per_day
        .insert(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(), 2);
    risk_state
        .daily_start_equity
        .insert(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(), money("75.0"));
    storage.save_risk_state(Mode::Demo, &risk_state).unwrap();

    storage
        .upsert_ohlcv(&OhlcvCandle {
            pair: "SOL/USDC".to_string(),
            timeframe: "1m".to_string(),
            bucket_start: opened_at,
            open: money("100.0"),
            high: money("100.8"),
            low: money("99.5"),
            close: money("100.4"),
            volume_quote: money("5000.0"),
        })
        .unwrap();

    storage
        .append_equity_snapshot(
            Mode::Demo,
            &EquitySnapshot {
                snapshot_at: opened_at,
                equity: money("80.0"),
                cash: money("77.25"),
                positions_value: money("2.75"),
            },
        )
        .unwrap();

    let script = format!(
        r#"
import sys
sys.path.insert(0, {repo})
from tradebot.storage.repo import JsonStorage

storage = JsonStorage(root={data})

trades = storage.list_trades(mode="demo", limit=10)
assert len(trades) == 1, trades
t = trades[0]
assert t.pair == "SOL/USDC"
assert t.side == "buy"
assert t.base_amount == 0.2
assert t.quote_amount == 20.0
assert t.fee_quote == 0.02
assert t.tx_signature == "rustsig123"
assert t.confidence == 0.65
assert t.notes == "written by rust"
assert t.opened_at.isoformat() == "2026-06-01T09:30:00+00:00", t.opened_at.isoformat()

portfolio = storage.load_portfolio_state(mode="demo")
assert portfolio.cash == 77.25
assert portfolio.sol_balance == 0.5
assert len(portfolio.positions) == 1
assert portfolio.positions[0].pair == "SOL/USDC"

risk = storage.load_risk_state(mode="demo")
assert risk.kill_switch_active is False
from datetime import date
assert risk.trades_per_day[date(2026, 6, 1)] == 2
assert risk.day_paused_until == date(2026, 6, 2)
assert risk.week_paused_until is None

df = storage.load_ohlcv(pair="SOL/USDC", timeframe="1m", limit=10)
assert len(df) == 1
assert df["close"].iloc[0] == 100.4
assert df["open"].iloc[0] == 100.0

snaps = storage.list_equity_snapshots(mode="demo", limit=10)
assert len(snaps) == 1
assert snaps[0]["equity"] == 80.0

print("OK")
"#,
        repo = py_str(&repo_root()),
        data = py_str(&dir),
    );

    let script_path = dir.join("_verify.py");
    std::fs::write(&script_path, script).unwrap();

    let output = Command::new(&python)
        .arg(&script_path)
        .output()
        .expect("failed to run python");

    if !output.status.success() {
        panic!(
            "python verification failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(String::from_utf8_lossy(&output.stdout).contains("OK"));

    std::fs::remove_dir_all(&dir).ok();
}
