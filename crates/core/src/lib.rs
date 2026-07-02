//! Port of `tradebot/core`: portfolio bookkeeping, risk management, Kelly
//! position sizing, and ADX-based regime classification.
//!
//! This is the safety-critical heart of the trading engine: it moves money
//! and trips circuit breakers, so every formula here is ported to match the
//! Python reference exactly. Money amounts (cash, balances, prices, order
//! sizes) use `Money` (a `rust_decimal::Decimal`) for exact arithmetic;
//! ratios and scalar config values (confidence, slippage_pct, drawdown_pct,
//! ADX, Kelly fractions, all `_pct` fields) stay `f64`, matching the
//! Money-vs-f64 split used across the rest of the Rust port. See each
//! module's doc comments for tolerance notes where a value must cross
//! between the two representations.

pub mod portfolio;
pub mod regime;
pub mod risk;
pub mod sizing;

pub use portfolio::{Portfolio, PortfolioError, Position};
pub use regime::{classify_regime, classify_regime_default, Regime, RegimeLabel};
pub use risk::{KillAction, RiskDecision, RiskManager, RiskState};
pub use sizing::{compute_kelly_stats, kelly_size, KellySizeParams, KellyStats};

/// Clamp a confidence-like scalar into `[0, 1]`, matching the Python
/// `max(0.0, min(1.0, x))` idiom used throughout `risk.py` and `sizing.py`.
pub(crate) fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}
