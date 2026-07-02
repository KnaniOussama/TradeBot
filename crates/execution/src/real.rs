//! Port of `tradebot/execution/real.py`: signs and sends real Jupiter swap
//! transactions on Solana.

use std::collections::HashMap;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use tracing::info;
use tradebot_common::Money;
use tradebot_core::Portfolio;
use tradebot_data::{JupiterClient, SolanaRpcClient};
use tradebot_storage::{JsonStorage, Side, Trade};
use tradebot_wallet::BotKeypair;

use crate::base::{ExecutionError, Executor, Fill, Order};
use crate::gas::gas_cost_sol_for_swap;
use crate::units::{from_units, to_units};

/// Signs a base64-encoded, unsigned `VersionedTransaction` and submits it to
/// the RPC, returning the transaction signature. Injectable so tests can
/// stub out the signing/submission step, mirroring the `sign_and_send`
/// callback in real.py.
#[async_trait]
pub trait TxSigner: Send + Sync {
    async fn sign_and_send(
        &self,
        serialized_tx_b64: &str,
        keypair: &BotKeypair,
        rpc: &SolanaRpcClient,
    ) -> Result<String, ExecutionError>;
}

/// Default `TxSigner`: base64-decode the Jupiter-supplied unsigned
/// transaction, deserialize it as a `VersionedTransaction`, sign the message
/// with the bot keypair, re-serialize, and submit via `sendTransaction`.
///
/// Assumes exactly one required signer (the fee payer / bot wallet), which
/// holds for the Jupiter swap transactions this executor builds.
pub struct DefaultSigner;

#[async_trait]
impl TxSigner for DefaultSigner {
    async fn sign_and_send(
        &self,
        serialized_tx_b64: &str,
        keypair: &BotKeypair,
        rpc: &SolanaRpcClient,
    ) -> Result<String, ExecutionError> {
        let raw = BASE64
            .decode(serialized_tx_b64)
            .map_err(|e| ExecutionError::Signing(format!("invalid base64 transaction: {e}")))?;
        let mut tx: VersionedTransaction = wincode::deserialize(&raw).map_err(|e| {
            ExecutionError::Signing(format!("failed to deserialize transaction: {e}"))
        })?;

        let num_required = tx.message.header().num_required_signatures as usize;
        if num_required != 1 {
            return Err(ExecutionError::Signing(format!(
                "expected exactly 1 required signer, got {num_required}"
            )));
        }

        let message_bytes = tx.message.serialize();
        let sig_bytes = keypair
            .sign(&message_bytes)
            .map_err(|e| ExecutionError::Signing(e.to_string()))?;
        let signature = Signature::from(sig_bytes);
        if tx.signatures.is_empty() {
            tx.signatures.push(signature);
        } else {
            tx.signatures[0] = signature;
        }

        let signed_bytes = wincode::serialize(&tx).map_err(|e| {
            ExecutionError::Signing(format!("failed to serialize signed transaction: {e}"))
        })?;
        let signed_b64 = BASE64.encode(signed_bytes);
        Ok(rpc.send_raw_transaction(&signed_b64, false).await?)
    }
}

pub struct RealExecutor<'a> {
    jupiter: JupiterClient,
    rpc: SolanaRpcClient,
    storage: &'a JsonStorage,
    keypair: &'a BotKeypair,
    base_mints: HashMap<String, (String, u32)>,
    quote_mint: String,
    quote_decimals: u32,
    max_slippage_pct: f64,
    priority_fee_microlamports: u64,
    confirmation_timeout_s: f64,
    signer: Box<dyn TxSigner>,
}

impl<'a> RealExecutor<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        jupiter: JupiterClient,
        rpc: SolanaRpcClient,
        storage: &'a JsonStorage,
        keypair: &'a BotKeypair,
        base_mints: HashMap<String, (String, u32)>,
        quote_mint: impl Into<String>,
        quote_decimals: u32,
        max_slippage_pct: f64,
        priority_fee_microlamports: u64,
        confirmation_timeout_s: f64,
        signer: Option<Box<dyn TxSigner>>,
    ) -> Self {
        Self {
            jupiter,
            rpc,
            storage,
            keypair,
            base_mints,
            quote_mint: quote_mint.into(),
            quote_decimals,
            max_slippage_pct,
            priority_fee_microlamports,
            confirmation_timeout_s,
            signer: signer.unwrap_or_else(|| Box::new(DefaultSigner)),
        }
    }

    fn resolve(&self, pair: &str) -> Result<(&str, u32), ExecutionError> {
        self.base_mints
            .get(pair)
            .map(|(mint, decimals)| (mint.as_str(), *decimals))
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown pair: {pair}")))
    }
}

#[async_trait]
impl<'a> Executor for RealExecutor<'a> {
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
                let units = to_units(order.size_quote, self.quote_decimals)?;
                if units == 0 {
                    return Err(ExecutionError::Invalid(
                        "buy requires size_quote > 0".to_string(),
                    ));
                }
                (self.quote_mint.clone(), base_mint.clone(), units)
            }
            Side::Sell => {
                let units = to_units(order.size_base, base_decimals)?;
                if units == 0 {
                    return Err(ExecutionError::Invalid(
                        "sell requires size_base > 0".to_string(),
                    ));
                }
                (base_mint.clone(), self.quote_mint.clone(), units)
            }
        };

        let slippage_bps = ((self.max_slippage_pct * 10_000.0) as u32).max(1);
        let quote = self
            .jupiter
            .quote(&in_mint, &out_mint, in_units, slippage_bps)
            .await?;
        if quote.price_impact_pct > self.max_slippage_pct {
            return Err(ExecutionError::Invalid(format!(
                "slippage {:.4} > max {}",
                quote.price_impact_pct, self.max_slippage_pct
            )));
        }

        let swap = self
            .jupiter
            .build_swap(
                &quote,
                &self.keypair.address,
                self.priority_fee_microlamports,
                true,
            )
            .await?;
        let signature = self
            .signer
            .sign_and_send(&swap.serialized_tx_b64, self.keypair, &self.rpc)
            .await?;
        self.rpc
            .confirm_signature(&signature, self.confirmation_timeout_s, 1.0)
            .await?;

        let (base_amount, quote_amount) = match order.side {
            Side::Buy => (
                from_units(quote.out_amount, base_decimals),
                order.size_quote,
            ),
            Side::Sell => (
                order.size_base,
                from_units(quote.out_amount, self.quote_decimals),
            ),
        };

        // LP fees are already inside out_amount; SOL gas is charged below.
        let fee_quote = Money::ZERO;
        let price = if base_amount > Decimal::ZERO {
            quote_amount / base_amount
        } else {
            Decimal::ZERO
        };

        // Charge SOL gas (base + priority). On-chain tx already burned this;
        // we mirror it in the portfolio so equity reflects reality.
        let sol_gas = gas_cost_sol_for_swap(self.priority_fee_microlamports as i64)
            .map_err(|_| ExecutionError::Invalid("negative gas inputs".to_string()))?;
        portfolio.charge_gas(sol_gas)?;

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
            slippage_pct: quote.price_impact_pct,
            tx_signature: Some(signature.clone()),
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
            slippage_pct: quote.price_impact_pct,
            tx_signature: Some(signature.clone()),
            opened_at: now,
            confidence: None,
            notes: None,
        })?;

        info!(
            pair = %order.pair,
            side = ?order.side,
            signature = %signature,
            base = %base_amount,
            quote = %quote_amount,
            slippage = quote.price_impact_pct,
            "real_fill",
        );
        Ok(fill)
    }
}
