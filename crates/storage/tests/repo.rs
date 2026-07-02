//! Port of `tests/storage/test_repo.py` behaviors.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::path::PathBuf;
use tradebot_common::Mode;
use tradebot_storage::records::{OhlcvCandle, PositionRecord, Side};
use tradebot_storage::{
    EquitySnapshot, JsonStorage, MarkHistory, MarkPoint, PortfolioState, RiskStateRecord, Trade,
};

fn tmp_storage(name: &str) -> JsonStorage {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tradebot_storage_repo_test_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    JsonStorage::new(p).unwrap()
}

fn money(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
}

fn trade(side: Side, pair: &str) -> Trade {
    Trade {
        mode: Mode::Demo,
        pair: pair.to_string(),
        side,
        base_amount: money("0.1"),
        quote_amount: money("10.0"),
        price: money("100.0"),
        fee_quote: money("0.01"),
        slippage_pct: 0.001,
        tx_signature: None,
        opened_at: base_time(),
        confidence: Some(0.7),
        notes: None,
    }
}

#[test]
fn append_and_list_trades_newest_first() {
    let storage = tmp_storage("append_list");
    storage.append_trade(&trade(Side::Buy, "SOL/USDC")).unwrap();
    storage
        .append_trade(&trade(Side::Sell, "SOL/USDC"))
        .unwrap();
    let trades = storage.list_trades(Mode::Demo, 10);
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].side, Side::Sell);
}

#[test]
fn list_trades_filters_by_mode() {
    let storage = tmp_storage("filter_mode");
    storage.append_trade(&trade(Side::Buy, "SOL/USDC")).unwrap();
    let mut real = trade(Side::Buy, "SOL/USDC");
    real.mode = Mode::Real;
    storage.append_trade(&real).unwrap();
    assert_eq!(storage.list_trades(Mode::Demo, 10).len(), 1);
    assert_eq!(storage.list_trades(Mode::Real, 10).len(), 1);
}

#[test]
fn save_and_load_portfolio_state() {
    let storage = tmp_storage("portfolio");
    let state = PortfolioState {
        mode: Mode::Demo,
        cash: money("42.5"),
        realized_pnl_total: money("2.5"),
        equity_high: money("50.0"),
        sol_balance: Decimal::ZERO,
        sol_gas_paid_total: Decimal::ZERO,
        positions: vec![PositionRecord {
            pair: "SOL/USDC".to_string(),
            base_amount: money("0.1"),
            avg_entry_price: money("100.0"),
            fees_paid_quote: money("0.01"),
        }],
    };
    storage.save_portfolio_state(&state).unwrap();
    let loaded = storage.load_portfolio_state(Mode::Demo).unwrap();
    assert_eq!(loaded.cash, money("42.5"));
    assert_eq!(loaded.positions.len(), 1);
    assert_eq!(loaded.positions[0].pair, "SOL/USDC");
}

#[test]
fn load_portfolio_state_missing_returns_none() {
    let storage = tmp_storage("portfolio_missing");
    assert!(storage.load_portfolio_state(Mode::Demo).is_none());
}

#[test]
fn append_and_list_equity_snapshots_chronological() {
    let storage = tmp_storage("equity");
    let base = base_time();
    for i in 0..5i64 {
        storage
            .append_equity_snapshot(
                Mode::Demo,
                &EquitySnapshot {
                    snapshot_at: base + chrono::Duration::minutes(i),
                    equity: money("50.0") + Decimal::from(i),
                    cash: money("50.0") + Decimal::from(i),
                    positions_value: Decimal::ZERO,
                },
            )
            .unwrap();
    }
    let out = storage.list_equity_snapshots(Mode::Demo, 10);
    assert_eq!(out.len(), 5);
    assert_eq!(out[0].equity, money("50.0"));
    assert_eq!(out[4].equity, money("54.0"));
}

#[test]
fn append_and_load_ohlcv() {
    let storage = tmp_storage("ohlcv");
    let base = base_time();
    for i in 0..3i64 {
        storage
            .upsert_ohlcv(&OhlcvCandle {
                pair: "SOL/USDC".to_string(),
                timeframe: "1m".to_string(),
                bucket_start: base + chrono::Duration::minutes(i),
                open: money("100.0"),
                high: money("101.0"),
                low: money("99.0"),
                close: money("100.5"),
                volume_quote: money("1000.0"),
            })
            .unwrap();
    }
    let candles = storage.load_ohlcv("SOL/USDC", "1m", 10);
    assert_eq!(candles.len(), 3);
    assert_eq!(candles.last().unwrap().close, money("100.5"));
}

#[test]
fn upsert_ohlcv_replaces_same_bucket() {
    let storage = tmp_storage("ohlcv_replace");
    let base = base_time();
    storage
        .upsert_ohlcv(&OhlcvCandle {
            pair: "SOL/USDC".to_string(),
            timeframe: "1m".to_string(),
            bucket_start: base,
            open: money("100.0"),
            high: money("101.0"),
            low: money("99.0"),
            close: money("100.5"),
            volume_quote: money("1000.0"),
        })
        .unwrap();
    storage
        .upsert_ohlcv(&OhlcvCandle {
            pair: "SOL/USDC".to_string(),
            timeframe: "1m".to_string(),
            bucket_start: base,
            open: money("100.0"),
            high: money("105.0"),
            low: money("99.0"),
            close: money("104.0"),
            volume_quote: money("2000.0"),
        })
        .unwrap();
    let candles = storage.load_ohlcv("SOL/USDC", "1m", 10);
    assert_eq!(candles.len(), 1);
    assert_eq!(candles[0].close, money("104.0"));
    assert_eq!(candles[0].high, money("105.0"));
}

#[test]
fn filename_safe_for_pair_with_slash() {
    let storage = tmp_storage("safe_pair");
    storage
        .upsert_ohlcv(&OhlcvCandle {
            pair: "SOL/USDC".to_string(),
            timeframe: "1m".to_string(),
            bucket_start: base_time(),
            open: Decimal::ONE,
            high: Decimal::ONE,
            low: Decimal::ONE,
            close: Decimal::ONE,
            volume_quote: Decimal::ZERO,
        })
        .unwrap();
    let ohlcv_dir = storage.root().join("ohlcv");
    let files: Vec<PathBuf> = std::fs::read_dir(&ohlcv_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(files.iter().any(|f| f
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("SOL--USDC")));
}

#[test]
fn save_and_load_risk_state() {
    let storage = tmp_storage("risk_state");
    let d = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
    let week_start = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
    let mut state = RiskStateRecord {
        trades_per_day: Default::default(),
        daily_start_equity: Default::default(),
        weekly_start_equity: Default::default(),
        day_paused_until: Some(NaiveDate::from_ymd_opt(2026, 5, 4).unwrap()),
        week_paused_until: None,
        kill_switch_active: true,
        kill_switch_reason: "drawdown 16% >= 15%".to_string(),
    };
    state.trades_per_day.insert(d, 4);
    state.daily_start_equity.insert(d, money("50.0"));
    state.weekly_start_equity.insert(week_start, money("50.0"));

    storage.save_risk_state(Mode::Demo, &state).unwrap();
    let loaded = storage.load_risk_state(Mode::Demo).unwrap();
    assert_eq!(loaded.trades_per_day.get(&d), Some(&4));
    assert_eq!(
        loaded.day_paused_until,
        Some(NaiveDate::from_ymd_opt(2026, 5, 4).unwrap())
    );
    assert!(loaded.kill_switch_active);
    assert!(loaded.kill_switch_reason.starts_with("drawdown"));
}

#[test]
fn load_risk_state_missing_returns_none() {
    let storage = tmp_storage("risk_state_missing");
    assert!(storage.load_risk_state(Mode::Demo).is_none());
}

#[test]
fn save_and_load_mark_history() {
    let storage = tmp_storage("mark_history");
    let base = base_time();
    let mut history: MarkHistory = MarkHistory::new();
    history.insert(
        "SOL/USDC".to_string(),
        vec![
            MarkPoint::new(base, money("150.0")),
            MarkPoint::new(base + chrono::Duration::minutes(1), money("151.0")),
        ],
    );
    history.insert(
        "JUP/USDC".to_string(),
        vec![MarkPoint::new(base, money("0.85"))],
    );
    storage.save_mark_history(Mode::Demo, &history).unwrap();
    let loaded = storage.load_mark_history(Mode::Demo);
    assert!(loaded.contains_key("SOL/USDC"));
    assert_eq!(loaded["SOL/USDC"].len(), 2);
    assert_eq!(loaded["SOL/USDC"][0].p, money("150.0"));
    assert_eq!(loaded["JUP/USDC"][0].t, base);
}

#[test]
fn load_mark_history_missing_returns_empty() {
    let storage = tmp_storage("mark_history_missing");
    assert!(storage.load_mark_history(Mode::Demo).is_empty());
}

#[test]
fn round_trip_returns_basic() {
    let storage = tmp_storage("round_trip_basic");
    let buy = trade(Side::Buy, "SOL/USDC"); // quote_amount=10.0, fee_quote=0.01 -> cost=10.01
    let mut sell = trade(Side::Sell, "SOL/USDC");
    sell.quote_amount = money("10.5");
    sell.price = money("105.0");
    sell.opened_at = buy.opened_at;
    storage.append_trade(&buy).unwrap();
    storage.append_trade(&sell).unwrap();
    let returns = storage.round_trip_returns(Mode::Demo, Some("SOL/USDC"), 200);
    assert_eq!(returns.len(), 1);
    // sell_quote_net = 10.5 - 0.01 = 10.49; cost = 10.01; ret = (10.49-10.01)/10.01
    let expected = (10.49 - 10.01) / 10.01;
    assert!((returns[0] - expected).abs() < 1e-6);
}

#[test]
fn round_trip_returns_filters_by_pair() {
    let storage = tmp_storage("round_trip_filter");
    let buy_sol = trade(Side::Buy, "SOL/USDC");
    let sell_sol = trade(Side::Sell, "SOL/USDC");
    let buy_jup = trade(Side::Buy, "JUP/USDC");
    storage.append_trade(&buy_sol).unwrap();
    storage.append_trade(&sell_sol).unwrap();
    storage.append_trade(&buy_jup).unwrap();
    let returns_sol = storage.round_trip_returns(Mode::Demo, Some("SOL/USDC"), 200);
    let returns_jup = storage.round_trip_returns(Mode::Demo, Some("JUP/USDC"), 200);
    assert_eq!(returns_sol.len(), 1);
    assert_eq!(returns_jup.len(), 0); // no sell for JUP
}

#[test]
fn round_trip_returns_empty_when_no_trades() {
    let storage = tmp_storage("round_trip_empty");
    let returns = storage.round_trip_returns(Mode::Demo, None, 200);
    assert!(returns.is_empty());
}
