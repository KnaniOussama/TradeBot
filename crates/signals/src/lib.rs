//! Port of `tradebot/signals`: signal base types and the technical-analysis
//! signal. Microstructure, onchain, and whale-follow signals are ported in a
//! later phase since they depend on data clients.

pub mod base;
pub mod ta;

pub use base::{clamp_score, rolling_zscore, MarketContext, ScoreOutOfRange, Signal, SignalScore};
pub use ta::TASignal;
