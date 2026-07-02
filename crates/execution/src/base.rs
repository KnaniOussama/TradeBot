//! Port of `tradebot/execution/base.py`: the `Order`/`Fill` value types and
//! the `Executor` interface implemented by `DemoExecutor` and `RealExecutor`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tradebot_common::Money;
use tradebot_core::{Portfolio, PortfolioError};
use tradebot_data::{JupiterError, RpcError};
use tradebot_storage::{Side, StorageError};

/// Errors raised while executing an order. Wraps the lower-layer error types
/// (`PortfolioError`, `JupiterError`, `RpcError`, `StorageError`) rather than
/// re-stringifying them, so callers can match on the underlying cause the
/// way Python callers could catch the specific exception type re-raised
/// unchanged by demo.py/real.py.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("{0}")]
    Invalid(String),

    #[error(transparent)]
    Portfolio(#[from] PortfolioError),

    #[error(transparent)]
    Jupiter(#[from] JupiterError),

    #[error(transparent)]
    Rpc(#[from] RpcError),

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error("transaction signing failed: {0}")]
    Signing(String),
}

/// An order to fill. Mirrors the frozen `Order` dataclass in base.py:
/// `size_quote` is used for buys, `size_base` for sells.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub pair: String,
    pub side: Side,
    pub size_quote: Money,
    pub size_base: Money,
}

impl Order {
    pub fn buy(pair: impl Into<String>, size_quote: Money) -> Self {
        Self {
            pair: pair.into(),
            side: Side::Buy,
            size_quote,
            size_base: Money::ZERO,
        }
    }

    pub fn sell(pair: impl Into<String>, size_base: Money) -> Self {
        Self {
            pair: pair.into(),
            side: Side::Sell,
            size_quote: Money::ZERO,
            size_base,
        }
    }
}

/// The result of a filled order. Mirrors the frozen `Fill` dataclass in
/// base.py.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub pair: String,
    pub side: Side,
    pub base_amount: Money,
    pub quote_amount: Money,
    pub price: Money,
    pub fee_quote: Money,
    pub slippage_pct: f64,
    pub tx_signature: Option<String>,
    pub filled_at: DateTime<Utc>,
}

/// Port of the `Executor` protocol in base.py.
#[async_trait]
pub trait Executor {
    async fn execute(
        &self,
        order: &Order,
        portfolio: &mut Portfolio,
        now: DateTime<Utc>,
    ) -> Result<Fill, ExecutionError>;
}
