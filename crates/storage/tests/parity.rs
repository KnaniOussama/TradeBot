//! Golden-file parity tests: fixtures in `tests/fixtures/` were written by
//! the real Python `JsonStorage` (see the generator script referenced in the
//! phase notes). These tests confirm Rust loads them to the correct values,
//! and that data round-tripped through Rust (load -> save -> load) is
//! unchanged.

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use tradebot_common::Mode;
use tradebot_storage::records::{OhlcvCandle, Side};
use tradebot_storage::JsonStorage;

fn money(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fresh_tmp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tradebot_storage_parity_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    p
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn tmp_copy_of_fixtures(name: &str) -> PathBuf {
    let dst = fresh_tmp_dir(name);
    copy_dir(&fixtures_dir(), &dst);
    dst
}

// --- Python fixture loads correctly in Rust ---

#[test]
fn python_trades_fixture_loads_correctly() {
    let storage = JsonStorage::new(fixtures_dir()).unwrap();

    let demo = storage.list_trades(Mode::Demo, 10);
    assert_eq!(demo.len(), 2);
    // newest first
    assert_eq!(demo[0].side, Side::Sell);
    assert_eq!(demo[0].quote_amount, money("10.5"));
    assert_eq!(demo[0].tx_signature.as_deref(), Some("5abcXYZ"));
    assert_eq!(demo[0].notes.as_deref(), Some("tp hit"));
    assert_eq!(
        demo[0].opened_at,
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 5, 0).unwrap()
    );
    assert_eq!(demo[1].side, Side::Buy);
    assert_eq!(demo[1].confidence, Some(0.7));
    assert_eq!(demo[1].tx_signature, None);

    let real = storage.list_trades(Mode::Real, 10);
    assert_eq!(real.len(), 1);
    assert_eq!(real[0].pair, "JUP/USDC");
    assert_eq!(real[0].base_amount, money("12.0"));
    assert_eq!(real[0].confidence, None);
}

#[test]
fn python_portfolio_fixture_loads_correctly() {
    let storage = JsonStorage::new(fixtures_dir()).unwrap();
    let loaded = storage.load_portfolio_state(Mode::Demo).unwrap();

    assert_eq!(loaded.cash, money("42.5"));
    assert_eq!(loaded.realized_pnl_total, money("2.5"));
    assert_eq!(loaded.equity_high, money("50.0"));
    assert_eq!(loaded.sol_balance, money("1.25"));
    assert_eq!(loaded.sol_gas_paid_total, money("0.003"));
    assert_eq!(loaded.positions.len(), 2);
    assert_eq!(loaded.positions[0].pair, "SOL/USDC");
    assert_eq!(loaded.positions[0].fees_paid_quote, money("0.01"));
    assert_eq!(loaded.positions[1].pair, "JUP/USDC");
    assert_eq!(loaded.positions[1].avg_entry_price, money("0.8"));
}

#[test]
fn python_risk_state_fixture_loads_correctly() {
    let storage = JsonStorage::new(fixtures_dir()).unwrap();
    let loaded = storage.load_risk_state(Mode::Demo).unwrap();

    let d0503 = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
    let d0502 = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
    let week_start = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();

    assert_eq!(loaded.trades_per_day.get(&d0503), Some(&4));
    assert_eq!(loaded.trades_per_day.get(&d0502), Some(&1));
    assert_eq!(loaded.daily_start_equity.get(&d0503), Some(&money("50.0")));
    assert_eq!(
        loaded.weekly_start_equity.get(&week_start),
        Some(&money("45.0"))
    );
    assert_eq!(
        loaded.day_paused_until,
        Some(NaiveDate::from_ymd_opt(2026, 5, 4).unwrap())
    );
    assert_eq!(loaded.week_paused_until, None);
    assert!(loaded.kill_switch_active);
    assert_eq!(loaded.kill_switch_reason, "drawdown 16% >= 15%");
}

#[test]
fn python_ohlcv_fixture_loads_correctly() {
    let storage = JsonStorage::new(fixtures_dir()).unwrap();
    let candles = storage.load_ohlcv("SOL/USDC", "1m", 10);

    assert_eq!(candles.len(), 3);
    assert_eq!(
        candles[0].timestamp,
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
    );
    assert_eq!(candles[0].open, money("100.0"));
    assert_eq!(candles[0].volume, money("1000.0"));
    assert_eq!(candles[2].close, money("102.5"));
    assert_eq!(candles[2].high, money("103.0"));
}

// --- Round trip: load Python fixture in Rust, re-save, load back, unchanged ---

#[test]
fn round_trip_python_portfolio_through_rust() {
    let dir = tmp_copy_of_fixtures("portfolio_rt");
    let storage = JsonStorage::new(&dir).unwrap();

    let loaded = storage.load_portfolio_state(Mode::Demo).unwrap();
    storage.save_portfolio_state(&loaded).unwrap();
    let reloaded = storage.load_portfolio_state(Mode::Demo).unwrap();

    assert_eq!(loaded, reloaded);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn round_trip_python_risk_state_through_rust() {
    let dir = tmp_copy_of_fixtures("risk_rt");
    let storage = JsonStorage::new(&dir).unwrap();

    let loaded = storage.load_risk_state(Mode::Demo).unwrap();
    storage.save_risk_state(Mode::Demo, &loaded).unwrap();
    let reloaded = storage.load_risk_state(Mode::Demo).unwrap();

    assert_eq!(loaded, reloaded);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn round_trip_python_trades_through_rust() {
    let src = JsonStorage::new(fixtures_dir()).unwrap();
    let original = src.list_trades(Mode::Demo, 50); // newest first
    let mut chronological = original.clone();
    chronological.reverse();

    let dir = fresh_tmp_dir("trades_rt");
    let dst = JsonStorage::new(&dir).unwrap();
    for t in &chronological {
        dst.append_trade(t).unwrap();
    }
    let reloaded = dst.list_trades(Mode::Demo, 50);

    assert_eq!(reloaded, original);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn round_trip_python_ohlcv_through_rust() {
    let src = JsonStorage::new(fixtures_dir()).unwrap();
    let original = src.load_ohlcv("SOL/USDC", "1m", 50);

    let dir = fresh_tmp_dir("ohlcv_rt");
    let dst = JsonStorage::new(&dir).unwrap();
    for c in &original {
        dst.upsert_ohlcv(&OhlcvCandle {
            pair: "SOL/USDC".to_string(),
            timeframe: "1m".to_string(),
            bucket_start: c.timestamp,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume_quote: c.volume,
        })
        .unwrap();
    }
    let reloaded = dst.load_ohlcv("SOL/USDC", "1m", 50);

    assert_eq!(reloaded, original);
    std::fs::remove_dir_all(&dir).ok();
}
