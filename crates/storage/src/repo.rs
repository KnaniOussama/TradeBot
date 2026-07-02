//! `JsonStorage`: file layout and methods mirroring `tradebot/storage/repo.py`.

use crate::atomic::{atomic_write_json, read_json_or_default};
use crate::error::StorageError;
use crate::records::{
    Candle, EquitySnapshot, MarkHistory, MarkPoint, OhlcvCandle, OhlcvRecord, PortfolioState,
    RiskStateRecord, Trade,
};
use rust_decimal::prelude::ToPrimitive;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tradebot_common::Mode;

/// Replace `/` with `--` so a trading pair can be used as a filename
/// component. Mirrors `_safe_pair` in `repo.py`.
fn safe_pair(pair: &str) -> String {
    pair.replace('/', "--")
}

pub struct JsonStorage {
    root: PathBuf,
}

impl JsonStorage {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|source| StorageError::Io {
            path: root.clone(),
            source,
        })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- trades ---

    fn trades_path(&self) -> PathBuf {
        self.root.join("trades.json")
    }

    pub fn append_trade(&self, trade: &Trade) -> Result<(), StorageError> {
        let path = self.trades_path();
        let mut existing: Vec<Trade> = read_json_or_default(&path, Vec::new());
        existing.push(trade.clone());
        atomic_write_json(&path, &existing)
    }

    /// Trades for `mode`, newest first, capped at `limit`.
    pub fn list_trades(&self, mode: Mode, limit: usize) -> Vec<Trade> {
        let rows: Vec<Trade> = read_json_or_default(&self.trades_path(), Vec::new());
        let mut filtered: Vec<Trade> = rows.into_iter().filter(|t| t.mode == mode).collect();
        filtered.reverse();
        filtered.truncate(limit);
        filtered
    }

    /// Per-trade round-trip returns from trade history. Walks trades
    /// chronologically (storage is newest-first; we reverse). For each
    /// buy -> sell pair: `return = (sell_quote_net - buy_quote_cost) /
    /// buy_quote_cost` where `buy_quote_cost = quote_amount + fee_quote` and
    /// `sell_quote_net = quote_amount - fee_quote`. Partial sells are treated
    /// as full closes; this is a documented approximation, same as Python.
    pub fn round_trip_returns(&self, mode: Mode, pair: Option<&str>, limit: usize) -> Vec<f64> {
        let mut trades = self.list_trades(mode, limit.saturating_mul(2));
        if let Some(pair) = pair {
            trades.retain(|t| t.pair == pair);
        }
        trades.reverse(); // chronological order

        let mut returns = Vec::new();
        let mut last_buy_quote = None;
        for t in trades {
            match t.side {
                crate::records::Side::Buy => {
                    last_buy_quote = Some(t.quote_amount + t.fee_quote);
                }
                crate::records::Side::Sell => {
                    if let Some(buy_quote_cost) = last_buy_quote {
                        let sell_quote_net = t.quote_amount - t.fee_quote;
                        let ret = (sell_quote_net - buy_quote_cost) / buy_quote_cost;
                        returns.push(ret.to_f64().unwrap_or(0.0));
                        last_buy_quote = None;
                    }
                }
            }
        }
        returns
    }

    // --- portfolio state ---

    fn portfolio_path(&self, mode: Mode) -> PathBuf {
        self.root.join(format!("portfolio.{mode}.json"))
    }

    pub fn save_portfolio_state(&self, state: &PortfolioState) -> Result<(), StorageError> {
        atomic_write_json(&self.portfolio_path(state.mode), state)
    }

    pub fn load_portfolio_state(&self, mode: Mode) -> Option<PortfolioState> {
        read_json_or_default(&self.portfolio_path(mode), None)
    }

    // --- equity snapshots ---

    fn equity_path(&self, mode: Mode) -> PathBuf {
        self.root.join(format!("equity.{mode}.json"))
    }

    pub fn append_equity_snapshot(
        &self,
        mode: Mode,
        snapshot: &EquitySnapshot,
    ) -> Result<(), StorageError> {
        let path = self.equity_path(mode);
        let mut existing: Vec<EquitySnapshot> = read_json_or_default(&path, Vec::new());
        existing.push(snapshot.clone());
        atomic_write_json(&path, &existing)
    }

    /// Equity snapshots for `mode`, chronological (oldest first), tail last
    /// `limit`.
    pub fn list_equity_snapshots(&self, mode: Mode, limit: usize) -> Vec<EquitySnapshot> {
        let mut rows: Vec<EquitySnapshot> =
            read_json_or_default(&self.equity_path(mode), Vec::new());
        if limit > 0 && rows.len() > limit {
            rows = rows.split_off(rows.len() - limit);
        }
        rows
    }

    // --- ohlcv ---

    fn ohlcv_path(&self, pair: &str, timeframe: &str) -> PathBuf {
        self.root
            .join("ohlcv")
            .join(format!("{}__{timeframe}.json", safe_pair(pair)))
    }

    pub fn upsert_ohlcv(&self, candle: &OhlcvCandle) -> Result<(), StorageError> {
        let path = self.ohlcv_path(&candle.pair, &candle.timeframe);
        let mut existing: Vec<OhlcvRecord> = read_json_or_default(&path, Vec::new());

        let record = OhlcvRecord {
            bucket_start: candle.bucket_start,
            open: candle.open,
            high: candle.high,
            low: candle.low,
            close: candle.close,
            volume_quote: candle.volume_quote,
        };

        // Replace if the same bucket exists (search from the end, matching
        // the append-mostly assumption in repo.py).
        let idx = existing
            .iter()
            .rposition(|r| r.bucket_start == candle.bucket_start);
        match idx {
            Some(i) => existing[i] = record,
            None => existing.push(record),
        }
        atomic_write_json(&path, &existing)
    }

    /// The last `limit` candles for `pair`/`timeframe`, oldest first.
    pub fn load_ohlcv(&self, pair: &str, timeframe: &str, limit: usize) -> Vec<Candle> {
        let mut rows: Vec<OhlcvRecord> =
            read_json_or_default(&self.ohlcv_path(pair, timeframe), Vec::new());
        if limit > 0 && rows.len() > limit {
            rows = rows.split_off(rows.len() - limit);
        }
        rows.into_iter()
            .map(|r| Candle {
                timestamp: r.bucket_start,
                open: r.open,
                high: r.high,
                low: r.low,
                close: r.close,
                volume: r.volume_quote,
            })
            .collect()
    }

    pub fn load_ohlcv_for_pairs(
        &self,
        pairs: &[String],
        timeframes: &[String],
        limit: usize,
    ) -> BTreeMap<String, BTreeMap<String, Vec<Candle>>> {
        let mut out = BTreeMap::new();
        for pair in pairs {
            let mut per_tf = BTreeMap::new();
            for tf in timeframes {
                per_tf.insert(tf.clone(), self.load_ohlcv(pair, tf, limit));
            }
            out.insert(pair.clone(), per_tf);
        }
        out
    }

    // --- risk state ---

    fn risk_path(&self, mode: Mode) -> PathBuf {
        self.root.join(format!("risk_state.{mode}.json"))
    }

    pub fn save_risk_state(&self, mode: Mode, state: &RiskStateRecord) -> Result<(), StorageError> {
        atomic_write_json(&self.risk_path(mode), state)
    }

    pub fn load_risk_state(&self, mode: Mode) -> Option<RiskStateRecord> {
        read_json_or_default(&self.risk_path(mode), None)
    }

    // --- mark history (rolling chart cache) ---

    fn mark_history_path(&self, mode: Mode) -> PathBuf {
        self.root.join(format!("mark_history.{mode}.json"))
    }

    pub fn save_mark_history(&self, mode: Mode, history: &MarkHistory) -> Result<(), StorageError> {
        atomic_write_json(&self.mark_history_path(mode), history)
    }

    pub fn load_mark_history(&self, mode: Mode) -> MarkHistory {
        read_json_or_default(&self.mark_history_path(mode), MarkHistory::new())
    }
}

/// Convenience constructor for a single mark point, used by callers building
/// up a `MarkHistory` to pass to `save_mark_history`.
impl MarkPoint {
    pub fn new(t: chrono::DateTime<chrono::Utc>, p: tradebot_common::Money) -> Self {
        Self { t, p }
    }
}
