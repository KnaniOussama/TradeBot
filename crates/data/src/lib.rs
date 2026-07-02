//! Rust port of `tradebot/data`: rate limiting, the Jupiter aggregator
//! client (Phase 3a), and the Solana RPC, Helius, Birdeye, price feed, and
//! OHLCV aggregator clients (Phase 3b).

pub mod birdeye;
pub mod error;
pub mod helius;
pub mod jupiter;
pub mod rate_limiter;
pub mod rpc;

pub use birdeye::BirdeyeClient;
pub use error::{HeliusError, JupiterError, RpcError};
pub use helius::{
    filter_for_mint, get_recent_swaps_for_wallet, HeliusClient, TokenTransfer, WhaleSwap,
};
pub use jupiter::{JupiterClient, JupiterQuote, JupiterSwap};
pub use rate_limiter::{RateLimiterMetrics, TokenBucketLimiter};
pub use rpc::{LatestBlockhash, SignatureStatus, SolanaRpcClient};
