//! Rust port of `tradebot/data`: rate limiting, the Jupiter aggregator
//! client (Phase 3a), and the Solana RPC, Helius, Birdeye, price feed, and
//! OHLCV aggregator clients (Phase 3b).

pub mod error;
pub mod jupiter;
pub mod rate_limiter;
pub mod rpc;

pub use error::{JupiterError, RpcError};
pub use jupiter::{JupiterClient, JupiterQuote, JupiterSwap};
pub use rate_limiter::{RateLimiterMetrics, TokenBucketLimiter};
pub use rpc::{LatestBlockhash, SignatureStatus, SolanaRpcClient};
