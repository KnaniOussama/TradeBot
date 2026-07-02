//! Helius enhanced-transactions client. Port of `tradebot/data/helius.py`.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::error::HeliusError;

const DEFAULT_BASE_URL: &str = "https://api.helius.xyz";
const DEFAULT_TIMEOUT_S: f64 = 10.0;

/// One SPL token transfer leg parsed from a Helius enhanced transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenTransfer {
    pub signature: String,
    pub timestamp: i64,
    pub mint: String,
    pub from_addr: String,
    pub to_addr: String,
    pub amount: f64,
}

/// One parsed swap event for a watched wallet (whale-tracker view).
///
/// `in_mint` / `in_amount_raw` = what the whale spent.
/// `out_mint` / `out_amount_raw` = what the whale received.
#[derive(Debug, Clone, PartialEq)]
pub struct WhaleSwap {
    pub wallet: String,
    pub timestamp: DateTime<Utc>,
    pub signature: String,
    pub in_mint: String,
    pub out_mint: String,
    pub in_amount_raw: u64,
    pub out_amount_raw: u64,
}

pub fn filter_for_mint(transfers: &[TokenTransfer], mint: &str) -> Vec<TokenTransfer> {
    transfers
        .iter()
        .filter(|t| t.mint == mint)
        .cloned()
        .collect()
}

#[derive(Debug, Deserialize)]
struct RawTokenTransfer {
    #[serde(default)]
    mint: String,
    #[serde(default, rename = "fromUserAccount")]
    from_user_account: Option<String>,
    #[serde(default, rename = "toUserAccount")]
    to_user_account: Option<String>,
    #[serde(default, rename = "tokenAmount")]
    token_amount: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawEnhancedTx {
    #[serde(default)]
    signature: String,
    #[serde(default)]
    timestamp: i64,
    #[serde(default, rename = "tokenTransfers")]
    token_transfers: Vec<RawTokenTransfer>,
}

/// Client for the Helius enhanced-transactions API. Cheap to clone; wraps a
/// reusable `reqwest::Client`.
#[derive(Clone)]
pub struct HeliusClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl HeliusClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self::with_timeout(api_key, base_url, DEFAULT_TIMEOUT_S)
    }

    pub fn with_timeout(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        timeout_s: f64,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .expect("failed to build reqwest client");
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            client,
        }
    }

    /// Fetch the most recent transactions for `address` and flatten their
    /// token transfers into a list. Raises on any HTTP or parse error
    /// (strict, matching `resp.raise_for_status()` in Python).
    pub async fn recent_token_transfers(
        &self,
        address: &str,
        limit: u32,
    ) -> Result<Vec<TokenTransfer>, HeliusError> {
        let url = format!("{}/v0/addresses/{address}/transactions", self.base_url);
        let limit_str = limit.to_string();
        let query = [("api-key", self.api_key.as_str()), ("limit", &limit_str)];
        let resp = self.client.get(&url).query(&query).send().await?;
        let resp = resp.error_for_status()?;
        let data: Vec<RawEnhancedTx> = resp.json().await?;
        let mut out = Vec::new();
        for tx in data {
            for tt in tx.token_transfers {
                out.push(TokenTransfer {
                    signature: tx.signature.clone(),
                    timestamp: tx.timestamp,
                    mint: tt.mint,
                    from_addr: tt.from_user_account.unwrap_or_default(),
                    to_addr: tt.to_user_account.unwrap_or_default(),
                    amount: tt.token_amount.unwrap_or(0.0),
                });
            }
        }
        Ok(out)
    }
}

/// Best-effort SPL amount -> smallest-units integer.
///
/// Prefers `rawTokenAmount.tokenAmount` when present (lossless). Falls back
/// to `tokenAmount * 1e6` (works for USDC and most 6-decimal SPL tokens;
/// over/underestimates other decimals; used only for relative ranking, not
/// accounting).
fn amount_to_raw(transfer: &Value) -> i64 {
    if let Some(raw) = transfer.get("rawTokenAmount").and_then(Value::as_object) {
        if let Some(amt) = raw.get("tokenAmount") {
            if let Some(parsed) = parse_int_like(amt) {
                return parsed;
            }
        }
    }
    if let Some(ta) = transfer.get("tokenAmount").and_then(Value::as_f64) {
        return (ta * 1_000_000.0) as i64;
    }
    0
}

fn parse_int_like(v: &Value) -> Option<i64> {
    if let Some(s) = v.as_str() {
        return s.parse::<i64>().ok();
    }
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    v.as_f64().map(|f| f as i64)
}

/// Convert a Helius enhanced-tx JSON entry (type=SWAP) into a `WhaleSwap`.
///
/// Wallet-relative legs:
///   - outgoing transfer (fromUserAccount=wallet) = what the whale spent
///   - incoming transfer (toUserAccount=wallet)   = what the whale received
///
/// Returns `None` if either leg is missing or amounts are zero.
fn parse_swap_tx(wallet: &str, tx: &Value) -> Option<WhaleSwap> {
    let transfers = tx.get("tokenTransfers").and_then(Value::as_array)?;
    if transfers.is_empty() {
        return None;
    }
    let out_leg = transfers
        .iter()
        .find(|t| t.get("fromUserAccount").and_then(Value::as_str) == Some(wallet))?;
    let in_leg = transfers
        .iter()
        .find(|t| t.get("toUserAccount").and_then(Value::as_str) == Some(wallet))?;
    let spent_mint = out_leg
        .get("mint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let received_mint = in_leg
        .get("mint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if spent_mint.is_empty() || received_mint.is_empty() {
        return None;
    }
    let ts = tx.get("timestamp").and_then(Value::as_i64)?;
    let timestamp = Utc.timestamp_opt(ts, 0).single()?;
    let spent_raw = amount_to_raw(out_leg);
    let received_raw = amount_to_raw(in_leg);
    if spent_raw <= 0 || received_raw <= 0 {
        return None;
    }
    Some(WhaleSwap {
        wallet: wallet.to_string(),
        timestamp,
        signature: tx
            .get("signature")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        in_mint: spent_mint,
        out_mint: received_mint,
        in_amount_raw: spent_raw as u64,
        out_amount_raw: received_raw as u64,
    })
}

/// Fetch the most recent SWAP-type transactions for one wallet.
///
/// Tolerant of upstream failures: an HTTP error status or malformed JSON
/// body logs a warning and returns an empty list rather than raising,
/// matching Python's `get_recent_swaps_for_wallet`. A transport-level
/// failure (connection refused, timeout, ...) still propagates as `Err`.
pub async fn get_recent_swaps_for_wallet(
    client: &HeliusClient,
    address: &str,
    limit: u32,
) -> Result<Vec<WhaleSwap>, HeliusError> {
    let url = format!("{}/v0/addresses/{address}/transactions", client.base_url);
    let limit_str = limit.to_string();
    let query = [
        ("api-key", client.api_key.as_str()),
        ("type", "SWAP"),
        ("limit", &limit_str),
    ];
    let resp = client.client.get(&url).query(&query).send().await?;
    if resp.status() != reqwest::StatusCode::OK {
        warn!(address, status = %resp.status(), "helius_swaps_http_error");
        return Ok(Vec::new());
    }
    let payload: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => {
            warn!(address, "helius_swaps_bad_json");
            return Ok(Vec::new());
        }
    };
    let arr = match payload.as_array() {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for tx in arr {
        if !tx.is_object() {
            continue;
        }
        if let Some(swap) = parse_swap_tx(address, tx) {
            out.push(swap);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALLET: &str = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const SOL: &str = "So11111111111111111111111111111111111111112";

    fn swap_tx(sent_mint: &str, sent_raw: i64, received_mint: &str, received_raw: i64) -> Value {
        serde_json::json!({
            "signature": "S".repeat(88),
            "timestamp": 1_714_742_400i64,
            "tokenTransfers": [
                {
                    "mint": sent_mint,
                    "fromUserAccount": WALLET,
                    "toUserAccount": "Pool111",
                    "rawTokenAmount": {"tokenAmount": sent_raw.to_string(), "decimals": 6},
                    "tokenAmount": sent_raw as f64 / 1_000_000.0,
                },
                {
                    "mint": received_mint,
                    "fromUserAccount": "Pool111",
                    "toUserAccount": WALLET,
                    "rawTokenAmount": {"tokenAmount": received_raw.to_string(), "decimals": 9},
                    "tokenAmount": received_raw as f64 / 1_000_000_000.0,
                },
            ],
        })
    }

    #[test]
    fn parse_swap_tx_extracts_legs() {
        let tx = swap_tx(USDC, 10_000_000, SOL, 70_000_000);
        let out = parse_swap_tx(WALLET, &tx).expect("should parse");
        assert_eq!(out.in_mint, USDC);
        assert_eq!(out.out_mint, SOL);
        assert_eq!(out.in_amount_raw, 10_000_000);
        assert_eq!(out.out_amount_raw, 70_000_000);
        assert_eq!(out.wallet, WALLET);
    }

    #[test]
    fn parse_swap_tx_returns_none_when_wallet_not_in_transfers() {
        let tx = serde_json::json!({
            "signature": "S",
            "timestamp": 1,
            "tokenTransfers": [
                {"mint": USDC, "fromUserAccount": "OtherA", "toUserAccount": "OtherB",
                 "rawTokenAmount": {"tokenAmount": "1"}, "tokenAmount": 1.0},
            ],
        });
        assert!(parse_swap_tx(WALLET, &tx).is_none());
    }

    #[test]
    fn parse_swap_tx_returns_none_for_zero_amount() {
        let tx = swap_tx(USDC, 0, SOL, 10);
        assert!(parse_swap_tx(WALLET, &tx).is_none());
    }

    #[test]
    fn filter_for_mint_keeps_only_matching() {
        let transfers = vec![
            TokenTransfer {
                signature: "a".to_string(),
                timestamp: 1,
                mint: "MINT_A".to_string(),
                from_addr: "X".to_string(),
                to_addr: "Y".to_string(),
                amount: 10.0,
            },
            TokenTransfer {
                signature: "b".to_string(),
                timestamp: 2,
                mint: "MINT_B".to_string(),
                from_addr: "X".to_string(),
                to_addr: "Y".to_string(),
                amount: 20.0,
            },
        ];
        let only_a = filter_for_mint(&transfers, "MINT_A");
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].mint, "MINT_A");
    }
}
