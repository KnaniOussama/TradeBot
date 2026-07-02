//! Port of `tradebot/execution/demo.py`: paper-trading executor using live
//! Jupiter quotes.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tracing::info;
use tradebot_common::Money;
use tradebot_core::Portfolio;
use tradebot_data::{JupiterClient, JupiterQuote};
use tradebot_storage::{JsonStorage, Side, Trade};

use crate::base::{ExecutionError, Executor, Fill, Order};
use crate::gas::gas_cost_sol_for_swap;
use crate::units::{from_units, to_units};

/// Paper-trading executor that mirrors `RealExecutor` as faithfully as
/// possible.
///
/// Cost model:
///   - Jupiter's `out_amount` is already net of LP fees + price impact, so
///     cash flow is `gross_in -> net_out` with no extra fee subtraction.
///   - Each fill burns SOL gas (base 5000 lamports + priority fee), deducted
///     from a tracked SOL balance just like real mode.
///   - The fill is re-quoted after `confirm_latency_s` to capture the
///     between-quote price drift real mode experiences during on-chain
///     confirmation. The decision uses the trigger quote; the fill uses the
///     later quote. If drift exceeds `max_slippage_pct`, the fill is
///     rejected (mirroring real-mode mid-flight slippage failure).
pub struct DemoExecutor<'a> {
    jupiter: JupiterClient,
    storage: &'a JsonStorage,
    base_mints: HashMap<String, (String, u32)>,
    quote_mint: String,
    quote_decimals: u32,
    max_slippage_pct: f64,
    priority_fee_microlamports: u64,
    confirm_latency_s: f64,
}

impl<'a> DemoExecutor<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        jupiter: JupiterClient,
        storage: &'a JsonStorage,
        base_mints: HashMap<String, (String, u32)>,
        quote_mint: impl Into<String>,
        quote_decimals: u32,
        max_slippage_pct: f64,
        priority_fee_microlamports: u64,
        confirm_latency_s: f64,
    ) -> Self {
        Self {
            jupiter,
            storage,
            base_mints,
            quote_mint: quote_mint.into(),
            quote_decimals,
            max_slippage_pct,
            priority_fee_microlamports,
            confirm_latency_s: confirm_latency_s.max(0.0),
        }
    }

    fn resolve(&self, pair: &str) -> Result<(&str, u32), ExecutionError> {
        self.base_mints
            .get(pair)
            .map(|(mint, decimals)| (mint.as_str(), *decimals))
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown pair: {pair}")))
    }

    async fn quote(
        &self,
        in_mint: &str,
        out_mint: &str,
        in_units: u64,
    ) -> Result<JupiterQuote, ExecutionError> {
        let slippage_bps = ((self.max_slippage_pct * 10_000.0) as u32).max(1);
        Ok(self
            .jupiter
            .quote(in_mint, out_mint, in_units, slippage_bps)
            .await?)
    }
}

#[async_trait]
impl<'a> Executor for DemoExecutor<'a> {
    async fn execute(
        &self,
        order: &Order,
        portfolio: &mut Portfolio,
        now: DateTime<Utc>,
    ) -> Result<Fill, ExecutionError> {
        let (base_mint, base_decimals) = self.resolve(&order.pair)?;
        let base_mint = base_mint.to_string();

        let (in_mint, out_mint, in_units) = match order.side {
            Side::Buy => {
                if order.size_quote <= Decimal::ZERO {
                    return Err(ExecutionError::Invalid(
                        "buy requires size_quote > 0".to_string(),
                    ));
                }
                let units = to_units(order.size_quote, self.quote_decimals)?;
                (self.quote_mint.clone(), base_mint.clone(), units)
            }
            Side::Sell => {
                if order.size_base <= Decimal::ZERO {
                    return Err(ExecutionError::Invalid(
                        "sell requires size_base > 0".to_string(),
                    ));
                }
                let units = to_units(order.size_base, base_decimals)?;
                (base_mint.clone(), self.quote_mint.clone(), units)
            }
        };

        // 1. Trigger quote: gates the decision (same role as RealExecutor's
        //    pre-check).
        let trigger_q = self.quote(&in_mint, &out_mint, in_units).await?;
        if trigger_q.price_impact_pct > self.max_slippage_pct {
            return Err(ExecutionError::Invalid(format!(
                "slippage {:.4} > max {}",
                trigger_q.price_impact_pct, self.max_slippage_pct
            )));
        }

        // 2. Simulate confirmation latency, then re-quote: the FILL uses
        //    this number.
        if self.confirm_latency_s > 0.0 {
            tokio::time::sleep(std::time::Duration::from_secs_f64(self.confirm_latency_s)).await;
        }
        let fill_q = self.quote(&in_mint, &out_mint, in_units).await?;

        // 3. Mid-flight drift gate. If the pool moved more than
        //    max_slippage_pct between trigger and fill, real mode's tx would
        //    have been rejected on-chain.
        if trigger_q.out_amount == 0 {
            return Err(ExecutionError::Invalid(
                "trigger quote returned zero out_amount".to_string(),
            ));
        }
        let drift_pct = (fill_q.out_amount as f64 - trigger_q.out_amount as f64).abs()
            / trigger_q.out_amount as f64;
        if drift_pct > self.max_slippage_pct {
            return Err(ExecutionError::Invalid(format!(
                "mid-flight drift {:.4} > max {} (trigger out={}, fill out={})",
                drift_pct, self.max_slippage_pct, trigger_q.out_amount, fill_q.out_amount
            )));
        }

        // 4. Compute realized amounts from the FILL quote.
        let (base_amount, quote_amount) = match order.side {
            Side::Buy => (
                from_units(fill_q.out_amount, base_decimals),
                order.size_quote,
            ),
            Side::Sell => (
                order.size_base,
                from_units(fill_q.out_amount, self.quote_decimals),
            ),
        };

        if base_amount <= Decimal::ZERO || quote_amount <= Decimal::ZERO {
            return Err(ExecutionError::Invalid(format!(
                "non-positive realized amounts: base={base_amount}, quote={quote_amount}"
            )));
        }

        let slippage_pct = fill_q.price_impact_pct;
        let fee_quote = Money::ZERO;
        let price = quote_amount / base_amount;

        // 5. Charge gas FIRST so a SOL-bankrupt wallet fails before mutating
        //    positions.
        let sol_gas = gas_cost_sol_for_swap(self.priority_fee_microlamports as i64)
            .map_err(|_| ExecutionError::Invalid("negative gas inputs".to_string()))?;
        portfolio.charge_gas(sol_gas)?;

        // 6. Apply the fill.
        portfolio.apply_fill(
            &order.pair,
            order.side,
            base_amount,
            quote_amount,
            fee_quote,
        )?;

        let fill = Fill {
            pair: order.pair.clone(),
            side: order.side,
            base_amount,
            quote_amount,
            price,
            fee_quote,
            slippage_pct,
            tx_signature: None,
            filled_at: now,
        };

        self.storage.append_trade(&Trade {
            mode: portfolio.mode,
            pair: order.pair.clone(),
            side: order.side,
            base_amount,
            quote_amount,
            price,
            fee_quote,
            slippage_pct,
            tx_signature: None,
            opened_at: now,
            confidence: None,
            notes: None,
        })?;

        info!(
            pair = %order.pair,
            side = ?order.side,
            base = %base_amount,
            quote = %quote_amount,
            price = %price,
            slippage = slippage_pct,
            drift = drift_pct,
            sol_gas = %sol_gas,
            sol_balance = %portfolio.sol_balance,
            "demo_fill",
        );
        Ok(fill)
    }
}
