//! Rust port of `tradebot/backtest`: a self-contained OHLCV-replay harness
//! for the TA-only signal stack (no live network calls).
//!
//! Money amounts (cash, prices, fees, equity) use `Money` (a
//! `rust_decimal::Decimal`); ratios used only for reporting (Sharpe, max
//! drawdown, total return) are `f64`, matching the Money-vs-f64 split used
//! across the rest of the Rust port.

pub mod data_loader;
pub mod executor;
pub mod metrics;
pub mod runner;

pub use data_loader::{load_csv_bytes, load_csv_path, DataLoaderError};
pub use executor::SyntheticExecutor;
pub use metrics::{compute_metrics, Metrics, TradeOutcome};
pub use runner::{run_backtest, BacktestParams, BacktestResult, BacktestTrade, EquityPoint};
