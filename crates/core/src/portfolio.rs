//! Port of `tradebot/core/portfolio.py`: cash/position bookkeeping, fills,
//! equity, drawdown tracking, and SOL gas accounting.
//!
//! All amounts here (cash, balances, prices, fees) use `Money` (a
//! `rust_decimal::Decimal`) so fills and averaging are exact, matching the
//! Money-type decision for this dispatch. `drawdown_pct` returns `f64`
//! since it is a ratio, not an amount.

use std::collections::HashMap;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tradebot_common::{Mode, Money};
use tradebot_storage::{PortfolioState, Side};

/// SOL/USDC pair identifier, used to look up a mark for the free SOL gas
/// float when computing equity. Matches `SOL_PAIR` in portfolio.py.
pub const SOL_PAIR: &str = "SOL/USDC";

/// Fallback SOL/USDC price used when no live mark is available for
/// `SOL_PAIR`. Matches `DEFAULT_SOL_FALLBACK_PRICE` in portfolio.py.
pub fn default_sol_fallback_price() -> Money {
    Decimal::new(140, 0)
}

/// Amount-comparison epsilon for buy/sell guards, matching Python's `1e-9`.
fn amount_epsilon() -> Money {
    Decimal::new(1, 9)
}

/// Amount-comparison epsilon for gas guards, matching Python's `1e-12`.
fn gas_epsilon() -> Money {
    Decimal::new(1, 12)
}

/// Errors raised by `Portfolio` operations. Mirrors `PortfolioError` in
/// portfolio.py; the Python "unknown side" case does not exist here since
/// `side` is the `Side` enum, not a free-form string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioError {
    #[error("non-positive amounts: base={base}, quote={quote}")]
    NonPositiveAmounts { base: Money, quote: Money },
    #[error("insufficient cash: need {need}, have {have}")]
    InsufficientCash { need: Money, have: Money },
    #[error("no position to sell: {pair}")]
    NoPositionToSell { pair: String },
    #[error("oversell: trying {trying}, have {have}")]
    Oversell { trying: Money, have: Money },
    #[error("negative gas charge: {amount}")]
    NegativeGasCharge { amount: Money },
    #[error("insufficient SOL for gas: need {need}, have {have}")]
    InsufficientSol { need: Money, have: Money },
}

/// One open position within a portfolio.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub pair: String,
    pub base_amount: Money,
    pub avg_entry_price: Money,
    pub fees_paid_quote: Money,
}

/// Cash + position bookkeeping for one trading mode. Port of `Portfolio` in
/// portfolio.py.
#[derive(Debug, Clone)]
pub struct Portfolio {
    pub mode: Mode,
    pub starting_cash: Money,
    pub starting_sol_balance: Money,
    pub cash: Money,
    pub sol_balance: Money,
    pub sol_gas_paid_total: Money,
    pub realized_pnl_total: Money,
    pub equity_high: Money,
    positions: HashMap<String, Position>,
}

impl Portfolio {
    pub fn new(mode: Mode, starting_cash: Money, starting_sol_balance: Money) -> Self {
        Self {
            mode,
            starting_cash,
            starting_sol_balance,
            cash: starting_cash,
            sol_balance: starting_sol_balance,
            sol_gas_paid_total: Decimal::ZERO,
            realized_pnl_total: Decimal::ZERO,
            // Equity_high seeded from starting_cash only: SOL contribution
            // depends on a mark we don't have at construction time.
            // update_equity_high() catches up on first cycle.
            equity_high: starting_cash,
            positions: HashMap::new(),
        }
    }

    pub fn position_for(&self, pair: &str) -> Option<&Position> {
        self.positions.get(pair)
    }

    pub fn open_positions(&self) -> Vec<&Position> {
        self.positions.values().collect()
    }

    /// Deducts SOL spent on transaction fees. Errors if insufficient SOL.
    pub fn charge_gas(&mut self, sol_amount: Money) -> Result<(), PortfolioError> {
        if sol_amount < Decimal::ZERO {
            return Err(PortfolioError::NegativeGasCharge { amount: sol_amount });
        }
        if sol_amount == Decimal::ZERO {
            return Ok(());
        }
        if sol_amount > self.sol_balance + gas_epsilon() {
            return Err(PortfolioError::InsufficientSol {
                need: sol_amount,
                have: self.sol_balance,
            });
        }
        self.sol_balance -= sol_amount;
        self.sol_gas_paid_total += sol_amount;
        Ok(())
    }

    /// Applies a fill. Returns realized P&L for this fill (0 for buys,
    /// gain/loss for sells).
    pub fn apply_fill(
        &mut self,
        pair: &str,
        side: Side,
        base_amount: Money,
        quote_amount: Money,
        fee_quote: Money,
    ) -> Result<Money, PortfolioError> {
        if base_amount <= Decimal::ZERO || quote_amount <= Decimal::ZERO {
            return Err(PortfolioError::NonPositiveAmounts {
                base: base_amount,
                quote: quote_amount,
            });
        }
        let price = quote_amount / base_amount;

        match side {
            Side::Buy => {
                let total_out = quote_amount + fee_quote;
                if total_out > self.cash + amount_epsilon() {
                    return Err(PortfolioError::InsufficientCash {
                        need: total_out,
                        have: self.cash,
                    });
                }
                self.cash -= total_out;
                match self.positions.get_mut(pair) {
                    None => {
                        self.positions.insert(
                            pair.to_string(),
                            Position {
                                pair: pair.to_string(),
                                base_amount,
                                avg_entry_price: price,
                                fees_paid_quote: fee_quote,
                            },
                        );
                    }
                    Some(existing) => {
                        let new_base = existing.base_amount + base_amount;
                        existing.avg_entry_price =
                            (existing.avg_entry_price * existing.base_amount + price * base_amount)
                                / new_base;
                        existing.base_amount = new_base;
                        existing.fees_paid_quote += fee_quote;
                    }
                }
                Ok(Decimal::ZERO)
            }
            Side::Sell => {
                let realized;
                {
                    let existing = match self.positions.get_mut(pair) {
                        Some(p) => p,
                        None => {
                            return Err(PortfolioError::NoPositionToSell {
                                pair: pair.to_string(),
                            });
                        }
                    };
                    if base_amount > existing.base_amount + amount_epsilon() {
                        return Err(PortfolioError::Oversell {
                            trying: base_amount,
                            have: existing.base_amount,
                        });
                    }
                    let cost_basis = existing.avg_entry_price * base_amount;
                    realized = quote_amount - cost_basis - fee_quote;
                    existing.base_amount -= base_amount;
                }
                self.cash += quote_amount - fee_quote;
                self.realized_pnl_total += realized;
                let should_remove = self
                    .positions
                    .get(pair)
                    .map(|p| p.base_amount <= amount_epsilon())
                    .unwrap_or(false);
                if should_remove {
                    self.positions.remove(pair);
                }
                Ok(realized)
            }
        }
    }

    /// Total equity: cash + open positions marked at `marks` (falling back
    /// to average entry price when a mark is missing) + free SOL balance
    /// marked at `SOL_PAIR` (falling back to `default_sol_fallback_price`).
    pub fn equity(&self, marks: &HashMap<String, Money>) -> Money {
        let mut total = self.cash;
        for pos in self.positions.values() {
            let mark = marks.get(&pos.pair).copied().unwrap_or(pos.avg_entry_price);
            total += pos.base_amount * mark;
        }
        if self.sol_balance > Decimal::ZERO {
            let sol_mark = marks
                .get(SOL_PAIR)
                .copied()
                .unwrap_or_else(default_sol_fallback_price);
            total += self.sol_balance * sol_mark;
        }
        total
    }

    pub fn update_equity_high(&mut self, current_equity: Money) {
        if current_equity > self.equity_high {
            self.equity_high = current_equity;
        }
    }

    /// `max(0, (equity_high - current_equity) / equity_high)`, or `0.0` if
    /// `equity_high <= 0`. This is a ratio, so it returns `f64`.
    pub fn drawdown_pct(&self, current_equity: Money) -> f64 {
        if self.equity_high <= Decimal::ZERO {
            return 0.0;
        }
        let dd = (self.equity_high - current_equity) / self.equity_high;
        dd.to_f64().unwrap_or(0.0).max(0.0)
    }

    /// Rebuilds a `Portfolio` from a persisted `PortfolioState`.
    pub fn from_state(state: &PortfolioState) -> Self {
        let mut p = Self::new(state.mode, state.cash, state.sol_balance);
        p.realized_pnl_total = state.realized_pnl_total;
        p.equity_high = state.equity_high;
        p.sol_gas_paid_total = state.sol_gas_paid_total;
        for pr in &state.positions {
            p.positions.insert(
                pr.pair.clone(),
                Position {
                    pair: pr.pair.clone(),
                    base_amount: pr.base_amount,
                    avg_entry_price: pr.avg_entry_price,
                    fees_paid_quote: pr.fees_paid_quote,
                },
            );
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_storage::PositionRecord;

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn marks(pairs: &[(&str, &str)]) -> HashMap<String, Money> {
        pairs.iter().map(|(k, v)| (k.to_string(), dec(v))).collect()
    }

    #[test]
    fn new_portfolio_all_cash() {
        let p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        assert_eq!(p.cash, dec("50.0"));
        assert_eq!(p.equity(&HashMap::new()), dec("50.0"));
        assert!(p.open_positions().is_empty());
    }

    #[test]
    fn apply_buy_reduces_cash_creates_position() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        p.apply_fill("SOL/USDC", Side::Buy, dec("0.1"), dec("15.0"), dec("0.015"))
            .unwrap();
        assert_eq!(p.cash, dec("50.0") - dec("15.0") - dec("0.015"));
        let pos = p.position_for("SOL/USDC").unwrap();
        assert_eq!(pos.base_amount, dec("0.1"));
        assert_eq!(pos.avg_entry_price, dec("150"));
    }

    #[test]
    fn apply_buy_then_buy_averages_entry() {
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("20.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let pos = p.position_for("SOL/USDC").unwrap();
        assert_eq!(pos.base_amount, dec("0.2"));
        assert_eq!(pos.avg_entry_price, dec("150"));
    }

    #[test]
    fn apply_sell_realizes_pnl_and_closes() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let realized = p
            .apply_fill(
                "SOL/USDC",
                Side::Sell,
                dec("0.1"),
                dec("15.0"),
                Decimal::ZERO,
            )
            .unwrap();
        assert_eq!(realized, dec("5.0"));
        assert!(p.position_for("SOL/USDC").is_none());
        assert_eq!(p.cash, dec("50.0") - dec("10.0") + dec("15.0"));
        assert_eq!(p.realized_pnl_total, dec("5.0"));
    }

    #[test]
    fn partial_sell_keeps_position_and_realizes_pnl() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.2"),
            dec("20.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let realized = p
            .apply_fill(
                "SOL/USDC",
                Side::Sell,
                dec("0.1"),
                dec("15.0"),
                Decimal::ZERO,
            )
            .unwrap();
        assert_eq!(realized, dec("5.0"));
        let pos = p.position_for("SOL/USDC").unwrap();
        assert_eq!(pos.base_amount, dec("0.1"));
        assert_eq!(pos.avg_entry_price, dec("100"));
    }

    #[test]
    fn sell_without_position_raises() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let err = p
            .apply_fill(
                "SOL/USDC",
                Side::Sell,
                dec("0.1"),
                dec("10.0"),
                Decimal::ZERO,
            )
            .unwrap_err();
        assert!(matches!(err, PortfolioError::NoPositionToSell { .. }));
    }

    #[test]
    fn oversell_raises() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let err = p
            .apply_fill(
                "SOL/USDC",
                Side::Sell,
                dec("0.5"),
                dec("50.0"),
                Decimal::ZERO,
            )
            .unwrap_err();
        assert!(matches!(err, PortfolioError::Oversell { .. }));
    }

    #[test]
    fn equity_uses_marks() {
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let eq = p.equity(&marks(&[("SOL/USDC", "200.0")]));
        assert_eq!(eq, dec("110"));
    }

    #[test]
    fn equity_marks_missing_uses_avg_entry() {
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let eq = p.equity(&HashMap::new());
        assert_eq!(eq, dec("100"));
    }

    #[test]
    fn drawdown_tracked_from_high_water_mark() {
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.update_equity_high(dec("120.0"));
        let dd = p.drawdown_pct(dec("108.0"));
        assert!((dd - 0.10).abs() < 1e-9, "dd={dd}");
        p.update_equity_high(dec("108.0")); // noop, since 108 < 120
        assert_eq!(p.equity_high, dec("120.0"));
    }

    #[test]
    fn portfolio_seed_from_state() {
        let state = PortfolioState {
            mode: Mode::Demo,
            cash: dec("42.0"),
            realized_pnl_total: dec("8.0"),
            equity_high: dec("55.0"),
            sol_balance: Decimal::ZERO,
            sol_gas_paid_total: Decimal::ZERO,
            positions: vec![PositionRecord {
                pair: "SOL/USDC".to_string(),
                base_amount: dec("0.1"),
                avg_entry_price: dec("150.0"),
                fees_paid_quote: dec("0.01"),
            }],
        };
        let p = Portfolio::from_state(&state);
        assert_eq!(p.mode, Mode::Demo);
        assert_eq!(p.cash, dec("42.0"));
        assert_eq!(p.realized_pnl_total, dec("8.0"));
        assert_eq!(p.equity_high, dec("55.0"));
        let pos = p.position_for("SOL/USDC").unwrap();
        assert_eq!(pos.base_amount, dec("0.1"));
        assert_eq!(pos.avg_entry_price, dec("150.0"));
    }

    // --- SOL gas tracking ---

    #[test]
    fn sol_balance_seeded_from_constructor() {
        let p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.05"));
        assert_eq!(p.sol_balance, dec("0.05"));
        assert_eq!(p.sol_gas_paid_total, Decimal::ZERO);
    }

    #[test]
    fn charge_gas_deducts_and_tracks_total() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.01"));
        p.charge_gas(dec("0.000005")).unwrap();
        p.charge_gas(dec("0.000005")).unwrap();
        assert_eq!(p.sol_balance, dec("0.01") - dec("0.00001"));
        assert_eq!(p.sol_gas_paid_total, dec("0.00001"));
    }

    #[test]
    fn charge_gas_zero_is_noop() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.01"));
        p.charge_gas(Decimal::ZERO).unwrap();
        assert_eq!(p.sol_balance, dec("0.01"));
    }

    #[test]
    fn charge_gas_insufficient_raises() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.000001"));
        let err = p.charge_gas(dec("0.001")).unwrap_err();
        assert!(matches!(err, PortfolioError::InsufficientSol { .. }));
    }

    #[test]
    fn charge_gas_negative_raises() {
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.01"));
        let err = p.charge_gas(dec("-0.001")).unwrap_err();
        assert!(matches!(err, PortfolioError::NegativeGasCharge { .. }));
    }

    #[test]
    fn equity_includes_sol_balance_at_mark() {
        let p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.05"));
        let eq = p.equity(&marks(&[("SOL/USDC", "140.0")]));
        assert_eq!(eq, dec("50.0") + dec("0.05") * dec("140.0"));
    }

    #[test]
    fn equity_uses_fallback_sol_price_when_mark_missing() {
        let p = Portfolio::new(Mode::Demo, dec("50.0"), dec("0.05"));
        let eq = p.equity(&HashMap::new());
        assert!(eq > dec("50.0"));
    }

    #[test]
    fn portfolio_state_roundtrip_preserves_sol() {
        let state = PortfolioState {
            mode: Mode::Demo,
            cash: dec("42.0"),
            realized_pnl_total: Decimal::ZERO,
            equity_high: dec("50.0"),
            sol_balance: dec("0.0473"),
            sol_gas_paid_total: dec("0.0027"),
            positions: vec![],
        };
        let p = Portfolio::from_state(&state);
        assert_eq!(p.sol_balance, dec("0.0473"));
        assert_eq!(p.sol_gas_paid_total, dec("0.0027"));
    }
}
