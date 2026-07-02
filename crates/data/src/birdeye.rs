//! Birdeye price client. Port of `tradebot/data/birdeye.py`.
//!
//! Free-tier endpoint we use:
//!   GET /defi/multi_price?list_address=mint1,mint2,...
//!   Headers: X-API-KEY: <key>, x-chain: solana
//!
//! Returns price_usd per mint. One call covers up to ~100 tokens, so the
//! entire watchlist's marks are a single HTTP round-trip rather than N
//! Jupiter quotes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use reqwest::StatusCode;
use serde_json::Value;
use tracing::{info, warn};

use crate::rate_limiter::TokenBucketLimiter;

pub const DEFAULT_BASE_URL: &str = "https://public-api.birdeye.so";
const DEFAULT_CHAIN: &str = "solana";
const DEFAULT_TIMEOUT_S: f64 = 5.0;
const DEFAULT_MAX_429_RETRIES: u32 = 2;

/// Async wrapper for Birdeye's public DeFi data API.
pub struct BirdeyeClient {
    api_key: String,
    base_url: String,
    chain: String,
    client: reqwest::Client,
    limiter: Option<Arc<TokenBucketLimiter>>,
    max_429_retries: u32,
    // Set to true once /defi/multi_price returns 401/403; we then use the
    // per-token /defi/price endpoint exclusively (free Standard tier).
    multi_price_locked: AtomicBool,
}

impl BirdeyeClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_options(
            api_key,
            DEFAULT_BASE_URL,
            DEFAULT_CHAIN,
            DEFAULT_TIMEOUT_S,
            None,
            DEFAULT_MAX_429_RETRIES,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_options(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        chain: impl Into<String>,
        timeout_s: f64,
        limiter: Option<Arc<TokenBucketLimiter>>,
        max_429_retries: u32,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .expect("failed to build reqwest client");
        Self {
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            chain: chain.into(),
            client,
            limiter,
            max_429_retries,
            multi_price_locked: AtomicBool::new(false),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.api_key) {
            headers.insert("X-API-KEY", v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.chain) {
            headers.insert("x-chain", v);
        }
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers
    }

    /// GET with shared rate limiter + 429 backoff + retry. On the final
    /// attempt this returns the 429 response itself rather than erroring;
    /// callers inspect the status code, matching Python's `_get_with_limit`.
    async fn get_with_limit(
        &self,
        url: &str,
        params: &[(&str, String)],
    ) -> Result<reqwest::Response, reqwest::Error> {
        let mut attempt: u32 = 0;
        loop {
            if let Some(limiter) = &self.limiter {
                limiter.acquire().await;
            }
            let resp = self
                .client
                .get(url)
                .query(params)
                .headers(self.headers())
                .send()
                .await?;
            if resp.status() != StatusCode::TOO_MANY_REQUESTS {
                return Ok(resp);
            }
            if let Some(limiter) = &self.limiter {
                limiter.record_429();
            }
            if attempt >= self.max_429_retries {
                return Ok(resp);
            }
            let jitter: f64 = rand::random::<f64>() * 0.25;
            let sleep_s = 2f64.powi(attempt as i32) + jitter;
            warn!(attempt = attempt + 1, sleep_s, "birdeye_429_retry");
            tokio::time::sleep(Duration::from_secs_f64(sleep_s)).await;
            attempt += 1;
        }
    }

    /// Fetch USD price for a batch of token mints. Returns `{mint: price_usd}`.
    ///
    /// Tries `/defi/multi_price` first (batched, one HTTP call, but requires
    /// paid Starter tier). On 401/403 (free-tier endpoint restriction) falls
    /// back to per-token `/defi/price`, which is on the free Standard tier.
    /// Switches mode permanently after the first 401 so we don't keep
    /// hitting the locked endpoint every cycle.
    pub async fn multi_price(&self, mints: &[String]) -> HashMap<String, f64> {
        if mints.is_empty() {
            return HashMap::new();
        }
        if !self.multi_price_locked.load(Ordering::SeqCst) {
            if let Some(batched) = self.try_multi_price(mints).await {
                return batched;
            }
            // try_multi_price set multi_price_locked when it hit 401/403.
        }
        self.single_prices(mints).await
    }

    /// Returns parsed result, or `None` if the endpoint is unavailable on
    /// this plan (401/403).
    async fn try_multi_price(&self, mints: &[String]) -> Option<HashMap<String, f64>> {
        let url = format!("{}/defi/multi_price", self.base_url);
        let list_address = mints.join(",");
        let params = [("list_address", list_address)];
        let resp = match self.get_with_limit(&url, &params).await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(endpoint = "multi_price", error = %e, "birdeye_request_failed");
                return Some(HashMap::new());
            }
        };
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            info!(
                status = status.as_u16(),
                hint = "free Standard tier only includes /defi/price (single)",
                "birdeye_multi_price_unavailable_falling_back_to_single"
            );
            self.multi_price_locked.store(true, Ordering::SeqCst);
            return None;
        }
        if status != StatusCode::OK {
            warn!(
                endpoint = "multi_price",
                status = status.as_u16(),
                "birdeye_http_error"
            );
            return Some(HashMap::new());
        }
        let payload: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => {
                warn!(endpoint = "multi_price", "birdeye_bad_json");
                return Some(HashMap::new());
            }
        };
        let obj = match payload.as_object() {
            Some(o) => o,
            None => return Some(HashMap::new()),
        };
        let success = obj.get("success").and_then(Value::as_bool).unwrap_or(false);
        if !success {
            warn!("birdeye_unsuccessful");
            return Some(HashMap::new());
        }
        let mut out = HashMap::new();
        if let Some(data) = obj.get("data").and_then(Value::as_object) {
            for (mint, info) in data {
                let Some(info_obj) = info.as_object() else {
                    continue;
                };
                let Some(value) = info_obj.get("value") else {
                    continue;
                };
                if let Some(f) = value_as_f64(value) {
                    out.insert(mint.clone(), f);
                }
            }
        }
        Some(out)
    }

    /// Fan out to `/defi/price` for each mint. One rate-limited HTTP call
    /// per token.
    async fn single_prices(&self, mints: &[String]) -> HashMap<String, f64> {
        let url = format!("{}/defi/price", self.base_url);
        let mut out = HashMap::new();
        for mint in mints {
            let params = [("address", mint.clone())];
            let resp = match self.get_with_limit(&url, &params).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!(mint = mint.as_str(), error = %e, "birdeye_single_price_failed");
                    continue;
                }
            };
            if resp.status() != StatusCode::OK {
                warn!(
                    mint = mint.as_str(),
                    status = resp.status().as_u16(),
                    "birdeye_single_price_http_error"
                );
                continue;
            }
            let payload: Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Some(obj) = payload.as_object() else {
                continue;
            };
            let success = obj.get("success").and_then(Value::as_bool).unwrap_or(false);
            if !success {
                continue;
            }
            if let Some(value) = obj.get("data").and_then(|d| d.get("value")) {
                if let Some(f) = value_as_f64(value) {
                    out.insert(mint.clone(), f);
                }
            }
        }
        out
    }
}

fn value_as_f64(v: &Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}
