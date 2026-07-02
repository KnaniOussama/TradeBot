//! Port of `tradebot/backtest/executor.py`: the synthetic fill simulator
//! used by the backtest runner. Fills a buy/sell against the current bar's
//! close (`mark`), applying a flat slippage push and a flat fee, both in
//! basis points.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tradebot_common::Money;
use tradebot_core::{Portfolio, PortfolioError};
use tradebot_execution::{Fill, Order};
use tradebot_storage::Side;

fn bps(value: u32) -> Decimal {
    Decimal::from(value) / Decimal::from(10_000)
}

/// Fills orders against a supplied mark price with flat fee/slippage
/// haircuts. Mirrors `SyntheticExecutor` in executor.py.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticExecutor {
    pub fee_bps: u32,
    pub slippage_bps: u32,
}

impl Default for SyntheticExecutor {
    fn default() -> Self {
        Self {
            fee_bps: 30,
            slippage_bps: 5,
        }
    }
}

impl SyntheticExecutor {
    pub fn new(fee_bps: u32, slippage_bps: u32) -> Self {
        Self {
            fee_bps,
            slippage_bps,
        }
    }

    /// Fills `order` against `mark`, applying it to `portfolio`. Mirrors
    /// `SyntheticExecutor.execute` in executor.py: buys push the price up by
    /// `slippage_bps`, sells push it down; the fee is charged on the quote
    /// notional at `fee_bps`.
    pub async fn execute(
        &self,
        order: &Order,
        portfolio: &mut Portfolio,
        now: DateTime<Utc>,
        mark: Money,
    ) -> Result<Fill, PortfolioError> {
        let slippage = bps(self.slippage_bps);
        let (eff_price, base_amount, quote_amount) = match order.side {
            Side::Buy => {
                let eff_price = mark * (Decimal::ONE + slippage);
                let quote_amount = order.size_quote;
                let base_amount = if eff_price > Decimal::ZERO {
                    quote_amount / eff_price
                } else {
                    Decimal::ZERO
                };
                (eff_price, base_amount, quote_amount)
            }
            Side::Sell => {
                let eff_price = mark * (Decimal::ONE - slippage);
                let base_amount = order.size_base;
                let quote_amount = base_amount * eff_price;
                (eff_price, base_amount, quote_amount)
            }
        };
        let fee_quote = quote_amount * bps(self.fee_bps);

        portfolio.apply_fill(
            &order.pair,
            order.side,
            base_amount,
            quote_amount,
            fee_quote,
        )?;

        Ok(Fill {
            pair: order.pair.clone(),
            side: order.side,
            base_amount,
            quote_amount,
            price: eff_price,
            fee_quote,
            slippage_pct: self.slippage_bps as f64 / 10_000.0,
            tx_signature: None,
            filled_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_common::Mode;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn portfolio(cash: &str) -> Portfolio {
        Portfolio::new(Mode::Backtest, dec(cash), Decimal::ZERO)
    }

    #[tokio::test]
    async fn buy_reduces_cash_and_creates_position() {
        let exec_ = SyntheticExecutor::new(30, 5);
        let mut port = portfolio("100.0");
        let order = Order::buy("SOL/USDC", dec("50.0"));
        let mark = dec("100.0");

        let fill = exec_.execute(&order, &mut port, now(), mark).await.unwrap();

        let eff_price = dec("100.05");
        let base_amount = dec("50.0") / eff_price;
        let fee_quote = dec("50.0") * dec("0.003");

        assert_eq!(fill.side, Side::Buy);
        assert!((fill.price - eff_price).abs() < dec("0.000000001"));
        assert!((fill.base_amount - base_amount).abs() < dec("0.000000001"));
        assert!((fill.fee_quote - fee_quote).abs() < dec("0.000000001"));
        assert!((port.cash - (dec("100.0") - dec("50.0") - fee_quote)).abs() < dec("0.000001"));
        assert!(port.position_for("SOL/USDC").is_some());
    }

    #[tokio::test]
    async fn sell_realizes_pnl() {
        let exec_ = SyntheticExecutor::new(30, 5);
        let mut port = portfolio("100.0");

        let buy_order = Order::buy("SOL/USDC", dec("50.0"));
        let buy_fill = exec_
            .execute(&buy_order, &mut port, now(), dec("100.0"))
            .await
            .unwrap();

        let sell_order = Order::sell("SOL/USDC", buy_fill.base_amount);
        let sell_fill = exec_
            .execute(&sell_order, &mut port, now(), dec("110.0"))
            .await
            .unwrap();

        assert_eq!(sell_fill.side, Side::Sell);
        let eff_sell = dec("110.0") * dec("0.9995");
        assert!((sell_fill.price - eff_sell).abs() < dec("0.000001"));
        assert!(port.position_for("SOL/USDC").is_none());
        assert!(port.realized_pnl_total > Decimal::ZERO);
    }

    #[tokio::test]
    async fn fee_and_slippage_applied() {
        let exec_ = SyntheticExecutor::new(100, 50);
        let mut port = portfolio("1000.0");
        let order = Order::buy("SOL/USDC", dec("100.0"));
        let fill = exec_
            .execute(&order, &mut port, now(), dec("200.0"))
            .await
            .unwrap();

        assert!((fill.price - dec("201.0")).abs() < dec("0.000001"));
        assert!((fill.fee_quote - dec("1.0")).abs() < dec("0.000001"));
        assert!((fill.slippage_pct - 50.0 / 10_000.0).abs() < 1e-12);
    }
}
