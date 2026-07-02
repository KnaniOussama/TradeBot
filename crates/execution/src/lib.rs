//! Rust port of `tradebot/execution`: the `Executor` interface plus the demo
//! (paper-trading) and real (on-chain) implementations, and Solana gas
//! accounting.
//!
//! Money amounts (order sizes, fill amounts, prices, fees, gas cost in SOL)
//! use `Money` (a `rust_decimal::Decimal`) for exact arithmetic; lamports
//! and base-unit token amounts are integers; slippage and drift are `f64`
//! ratios. This matches the Money-vs-integer-vs-f64 split used across the
//! rest of the Rust port.

pub mod base;
pub mod demo;
pub mod gas;
pub mod real;
mod units;

pub use base::{ExecutionError, Executor, Fill, Order};
pub use demo::DemoExecutor;
pub use gas::{
    gas_cost_sol, gas_cost_sol_for_swap, NegativeGasInputs, DEFAULT_COMPUTE_UNITS_PER_SWAP,
    LAMPORTS_PER_SOL, SOLANA_BASE_FEE_LAMPORTS,
};
pub use real::{DefaultSigner, RealExecutor, TxSigner};
