//! Rust port of `tradebot/data`: rate limiting and the Jupiter aggregator
//! client (Phase 3a). RPC, Helius, Birdeye, prices, and OHLCV are ported in a
//! later phase.

pub mod error;
pub mod jupiter;
pub mod rate_limiter;

pub use error::JupiterError;
pub use jupiter::{JupiterClient, JupiterQuote, JupiterSwap};
pub use rate_limiter::{RateLimiterMetrics, TokenBucketLimiter};
