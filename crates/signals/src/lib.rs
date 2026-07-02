//! Port of `tradebot/signals`: signal base types, technical-analysis,
//! microstructure, on-chain, and whale-follow signals.

pub mod base;
pub mod microstructure;
pub mod onchain;
pub mod ta;
pub mod whale_activity;
pub mod whale_follow;

pub use base::{clamp_score, rolling_zscore, MarketContext, ScoreOutOfRange, Signal, SignalScore};
pub use microstructure::MicrostructureSignal;
pub use onchain::OnChainSignal;
pub use ta::TASignal;
pub use whale_activity::WhaleActivityTracker;
pub use whale_follow::WhaleFollowSignal;
