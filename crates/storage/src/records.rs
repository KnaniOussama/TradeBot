//! Serde record types for state persisted by `JsonStorage`. These mirror the
//! dataclasses in `tradebot/storage/repo.py` and the `RiskState` dataclass in
//! `tradebot/core/risk.py`. They are self-contained DTOs and do not depend on
//! a ported `core` crate.

use crate::time::rfc3339;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tradebot_common::{Mode, Money};

/// Trade side. Serializes as `"buy"` / `"sell"`, matching the Python
/// `Literal["buy", "sell"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

/// One recorded trade. Field order matches `dataclasses.asdict(Trade)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub mode: Mode,
    pub pair: String,
    pub side: Side,
    pub base_amount: Money,
    pub quote_amount: Money,
    pub price: Money,
    pub fee_quote: Money,
    pub slippage_pct: f64,
    #[serde(default)]
    pub tx_signature: Option<String>,
    #[serde(with = "rfc3339")]
    pub opened_at: DateTime<Utc>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// One open position within a portfolio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionRecord {
    pub pair: String,
    pub base_amount: Money,
    pub avg_entry_price: Money,
    #[serde(default)]
    pub fees_paid_quote: Money,
}

/// Full portfolio snapshot for one mode. Field order matches the dict built
/// by `JsonStorage.save_portfolio_state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioState {
    pub mode: Mode,
    pub cash: Money,
    pub realized_pnl_total: Money,
    pub equity_high: Money,
    #[serde(default)]
    pub sol_balance: Money,
    #[serde(default)]
    pub sol_gas_paid_total: Money,
    #[serde(default)]
    pub positions: Vec<PositionRecord>,
}

/// Input to `JsonStorage::upsert_ohlcv`. Mirrors the `OHLCVCandle` dataclass;
/// `pair` and `timeframe` select the target file and are not themselves
/// persisted in the record.
#[derive(Debug, Clone, PartialEq)]
pub struct OhlcvCandle {
    pub pair: String,
    pub timeframe: String,
    pub bucket_start: DateTime<Utc>,
    pub open: Money,
    pub high: Money,
    pub low: Money,
    pub close: Money,
    pub volume_quote: Money,
}

/// One candle as stored on disk in an `ohlcv/<pair>__<timeframe>.json` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OhlcvRecord {
    #[serde(with = "rfc3339")]
    pub bucket_start: DateTime<Utc>,
    pub open: Money,
    pub high: Money,
    pub low: Money,
    pub close: Money,
    pub volume_quote: Money,
}

/// A loaded candle, returned in place of the Python pandas DataFrame row.
#[derive(Debug, Clone, PartialEq)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: Money,
    pub high: Money,
    pub low: Money,
    pub close: Money,
    pub volume: Money,
}

/// One equity curve sample. Field order matches the dict built by
/// `JsonStorage.append_equity_snapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquitySnapshot {
    #[serde(with = "rfc3339")]
    pub snapshot_at: DateTime<Utc>,
    pub equity: Money,
    pub cash: Money,
    pub positions_value: Money,
}

/// Risk-manager state for one mode. Field order and shape matches
/// `JsonStorage.save_risk_state` / `load_risk_state`, keyed by calendar date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskStateRecord {
    #[serde(default)]
    pub trades_per_day: BTreeMap<NaiveDate, i64>,
    #[serde(default)]
    pub daily_start_equity: BTreeMap<NaiveDate, Money>,
    #[serde(default)]
    pub weekly_start_equity: BTreeMap<NaiveDate, Money>,
    #[serde(default)]
    pub day_paused_until: Option<NaiveDate>,
    #[serde(default)]
    pub week_paused_until: Option<NaiveDate>,
    #[serde(default)]
    pub kill_switch_active: bool,
    #[serde(default)]
    pub kill_switch_reason: String,
}

/// One timestamped mark price sample within a pair's rolling mark history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkPoint {
    #[serde(rename = "t", with = "rfc3339")]
    pub t: DateTime<Utc>,
    #[serde(rename = "p")]
    pub p: Money,
}

/// Rolling mark price history keyed by pair, oldest-first per pair.
pub type MarkHistory = BTreeMap<String, Vec<MarkPoint>>;
