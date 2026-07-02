//! Rust port of `tradebot/storage`: atomic JSON persistence for trades,
//! portfolio state, equity snapshots, OHLCV candles, risk state, and mark
//! price history. File layout and JSON shapes match the Python `JsonStorage`
//! so state written by one implementation loads correctly in the other.

pub mod atomic;
pub mod error;
pub mod records;
pub mod repo;
pub mod time;

pub use atomic::{atomic_write_json, read_json_or_default};
pub use error::StorageError;
pub use records::{
    Candle, EquitySnapshot, MarkHistory, MarkPoint, OhlcvCandle, PortfolioState, PositionRecord,
    RiskStateRecord, Side, Trade,
};
pub use repo::JsonStorage;
