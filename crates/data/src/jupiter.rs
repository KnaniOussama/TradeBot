//! Jupiter aggregator client. Port of `tradebot/data/jupiter.py`.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use tracing::warn;

use crate::error::JupiterError;
use crate::rate_limiter::TokenBucketLimiter;

const DEFAULT_TIMEOUT_S: f64 = 5.0;

/// Result of `JupiterClient::build_swap`.
#[derive(Debug, Clone)]
pub struct JupiterSwap {
    pub serialized_tx_b64: String,
    pub last_valid_block_height: u64,
    pub prioritization_fee_lamports: u64,
    pub raw: serde_json::Value,
}

/// Result of `JupiterClient::quote`.
#[derive(Debug, Clone)]
pub struct JupiterQuote {
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: u64,
    pub out_amount: u64,
    pub other_amount_threshold: u64,
    pub slippage_bps: u32,
    pub price_impact_pct: f64,
    pub route_labels: Vec<String>,
    pub raw: serde_json::Value,
}

impl JupiterQuote {
    pub fn implied_price(&self, in_decimals: u32, out_decimals: u32) -> f64 {
        let in_human = self.in_amount as f64 / 10f64.powi(in_decimals as i32);
        let out_human = self.out_amount as f64 / 10f64.powi(out_decimals as i32);
        if in_human == 0.0 {
            return 0.0;
        }
        out_human / in_human
    }
}

// Jupiter's quote endpoint returns lamport amounts and the price impact as
// JSON strings (to avoid float precision loss on large integers). Accept
// either string or numeric JSON representations to be robust.
fn u64_from_str_or_num<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Str(String),
        U64(u64),
        F64(f64),
    }
    match Repr::deserialize(deserializer)? {
        Repr::Str(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
        Repr::U64(n) => Ok(n),
        Repr::F64(f) => Ok(f as u64),
    }
}

fn f64_from_str_or_num<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Str(String),
        F64(f64),
    }
    match Repr::deserialize(deserializer)? {
        Repr::Str(s) => s.parse::<f64>().map_err(serde::de::Error::custom),
        Repr::F64(f) => Ok(f),
    }
}

fn default_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    u64_from_str_or_num(deserializer)
}

#[derive(Debug, Deserialize)]
struct RawSwapInfo {
    label: String,
}

#[derive(Debug, Deserialize)]
struct RawRoutePlanStep {
    #[serde(rename = "swapInfo")]
    swap_info: RawSwapInfo,
}

#[derive(Debug, Deserialize)]
struct RawQuoteResponse {
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
    #[serde(rename = "inAmount", deserialize_with = "u64_from_str_or_num")]
    in_amount: u64,
    #[serde(rename = "outAmount", deserialize_with = "u64_from_str_or_num")]
    out_amount: u64,
    #[serde(
        rename = "otherAmountThreshold",
        deserialize_with = "u64_from_str_or_num"
    )]
    other_amount_threshold: u64,
    #[serde(rename = "slippageBps")]
    slippage_bps: u32,
    #[serde(rename = "priceImpactPct", deserialize_with = "f64_from_str_or_num")]
    price_impact_pct: f64,
    #[serde(rename = "routePlan", default)]
    route_plan: Vec<RawRoutePlanStep>,
}

#[derive(Debug, Deserialize)]
struct RawSwapResponse {
    #[serde(rename = "swapTransaction")]
    swap_transaction: String,
    #[serde(
        rename = "lastValidBlockHeight",
        default,
        deserialize_with = "default_u64"
    )]
    last_valid_block_height: u64,
    #[serde(
        rename = "prioritizationFeeLamports",
        default,
        deserialize_with = "default_u64"
    )]
    prioritization_fee_lamports: u64,
}

/// Client for the Jupiter swap aggregator API. Cheap to clone; wraps a
/// reusable `reqwest::Client`.
#[derive(Clone)]
pub struct JupiterClient {
    base_url: String,
    client: reqwest::Client,
    limiter: Option<Arc<TokenBucketLimiter>>,
    max_429_retries: u32,
}

impl JupiterClient {
    pub fn new(
        base_url: impl Into<String>,
        limiter: Option<Arc<TokenBucketLimiter>>,
        max_429_retries: u32,
    ) -> Self {
        Self::with_timeout(base_url, DEFAULT_TIMEOUT_S, limiter, max_429_retries)
    }

    pub fn with_timeout(
        base_url: impl Into<String>,
        timeout_s: f64,
        limiter: Option<Arc<TokenBucketLimiter>>,
        max_429_retries: u32,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .expect("failed to build reqwest client");
        let base_url = base_url.into();
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            base_url,
            client,
            limiter,
            max_429_retries,
        }
    }

    async fn request_with_retry<F>(&self, build: F) -> Result<reqwest::Response, JupiterError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt: u32 = 0;
        loop {
            if let Some(limiter) = &self.limiter {
                limiter.acquire().await;
            }
            let resp = build().send().await?;
            if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Ok(resp);
            }
            if let Some(limiter) = &self.limiter {
                limiter.record_429();
            }
            if attempt >= self.max_429_retries {
                // Let the caller see the 429 as an HTTP status error.
                return Err(resp.error_for_status().unwrap_err().into());
            }
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let sleep_s = match retry_after {
                Some(s) => s
                    .parse::<f64>()
                    .unwrap_or_else(|_| 1.0 * 2f64.powi(attempt as i32)),
                None => {
                    let jitter: f64 = rand::random::<f64>() * 0.25;
                    1.0 * 2f64.powi(attempt as i32) + jitter
                }
            };
            let sleep_s = sleep_s.max(0.0);
            warn!(attempt = attempt + 1, sleep_s, "jupiter_429_retry");
            tokio::time::sleep(Duration::from_secs_f64(sleep_s)).await;
            attempt += 1;
        }
    }

    pub async fn quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u32,
    ) -> Result<JupiterQuote, JupiterError> {
        let url = format!("{}/quote", self.base_url);
        let query = [
            ("inputMint", input_mint.to_string()),
            ("outputMint", output_mint.to_string()),
            ("amount", amount.to_string()),
            ("slippageBps", slippage_bps.to_string()),
        ];
        let client = &self.client;
        let resp = self
            .request_with_retry(|| client.get(&url).query(&query))
            .await?;
        let resp = resp.error_for_status()?;
        let raw: serde_json::Value = resp.json().await?;
        let parsed: RawQuoteResponse = serde_json::from_value(raw.clone())?;
        Ok(JupiterQuote {
            input_mint: parsed.input_mint,
            output_mint: parsed.output_mint,
            in_amount: parsed.in_amount,
            out_amount: parsed.out_amount,
            other_amount_threshold: parsed.other_amount_threshold,
            slippage_bps: parsed.slippage_bps,
            price_impact_pct: parsed.price_impact_pct,
            route_labels: parsed
                .route_plan
                .into_iter()
                .map(|step| step.swap_info.label)
                .collect(),
            raw,
        })
    }

    pub async fn build_swap(
        &self,
        quote: &JupiterQuote,
        user_pubkey: &str,
        priority_fee_microlamports: u64,
        wrap_and_unwrap_sol: bool,
    ) -> Result<JupiterSwap, JupiterError> {
        let url = format!("{}/swap", self.base_url);
        let body = serde_json::json!({
            "quoteResponse": quote.raw,
            "userPublicKey": user_pubkey,
            "wrapAndUnwrapSol": wrap_and_unwrap_sol,
            "computeUnitPriceMicroLamports": priority_fee_microlamports,
            "asLegacyTransaction": false,
        });
        let client = &self.client;
        let resp = self
            .request_with_retry(|| client.post(&url).json(&body))
            .await?;
        let resp = resp.error_for_status()?;
        let raw: serde_json::Value = resp.json().await?;
        let parsed: RawSwapResponse = serde_json::from_value(raw.clone())?;
        Ok(JupiterSwap {
            serialized_tx_b64: parsed.swap_transaction,
            last_valid_block_height: parsed.last_valid_block_height,
            prioritization_fee_lamports: parsed.prioritization_fee_lamports,
            raw,
        })
    }
}
