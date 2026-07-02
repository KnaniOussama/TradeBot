//! Port of `tradebot/signals`: signal base types, technical-analysis,
//! microstructure, on-chain, and whale-follow signals.

pub mod base;
pub mod microstructure;
pub mod ta;

pub use base::{clamp_score, rolling_zscore, MarketContext, ScoreOutOfRange, Signal, SignalScore};
pub use microstructure::MicrostructureSignal;
pub use ta::TASignal;
