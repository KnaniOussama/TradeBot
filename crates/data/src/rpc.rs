//! Solana JSON-RPC client. Port of `tradebot/data/rpc.py`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::RpcError;

const DEFAULT_TIMEOUT_S: f64 = 10.0;
const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;

/// Result of `SolanaRpcClient::get_latest_blockhash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestBlockhash {
    pub blockhash: String,
    pub last_valid_block_height: u64,
}

/// One entry from `SolanaRpcClient::get_signature_statuses`. A missing
/// signature is represented as `None` in the outer `Vec`, matching Python's
/// `list[dict | None]`.
#[derive(Debug, Clone, Deserialize)]
pub struct SignatureStatus {
    #[serde(default)]
    pub slot: u64,
    #[serde(default)]
    pub confirmations: Option<u64>,
    #[serde(default)]
    pub err: Option<Value>,
    #[serde(default, rename = "confirmationStatus")]
    pub confirmation_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BalanceResult {
    value: u64,
}

#[derive(Debug, Deserialize)]
struct BlockhashValue {
    blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    last_valid_block_height: u64,
}

#[derive(Debug, Deserialize)]
struct BlockhashResult {
    value: BlockhashValue,
}

#[derive(Debug, Deserialize)]
struct TokenAccountsResult {
    #[serde(default)]
    value: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct SignatureStatusesResult {
    #[serde(default)]
    value: Vec<Option<SignatureStatus>>,
}

/// Client for the Solana JSON-RPC HTTP API. Cheap to clone; wraps a reusable
/// `reqwest::Client`. Each call gets an incrementing JSON-RPC request id,
/// matching Python's `itertools.count(1)`.
#[derive(Clone)]
pub struct SolanaRpcClient {
    url: String,
    client: reqwest::Client,
    next_id: Arc<AtomicU64>,
}

impl SolanaRpcClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_timeout(url, DEFAULT_TIMEOUT_S)
    }

    pub fn with_timeout(url: impl Into<String>, timeout_s: f64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .expect("failed to build reqwest client");
        Self {
            url: url.into(),
            client,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let resp = self.client.post(&self.url).json(&body).send().await?;
        let resp = resp.error_for_status()?;
        let data: Value = resp.json().await?;
        if let Some(err) = data.get("error") {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            return Err(RpcError::JsonRpc { code, message });
        }
        data.get("result")
            .cloned()
            .ok_or_else(|| RpcError::UnexpectedShape("missing result field".to_string()))
    }

    pub async fn get_balance_lamports(&self, address: &str) -> Result<u64, RpcError> {
        let result = self.call("getBalance", json!([address])).await?;
        let parsed: BalanceResult = serde_json::from_value(result)?;
        Ok(parsed.value)
    }

    pub async fn get_balance_sol(&self, address: &str) -> Result<f64, RpcError> {
        let lamports = self.get_balance_lamports(address).await?;
        Ok(lamports as f64 / LAMPORTS_PER_SOL)
    }

    pub async fn get_token_accounts_by_owner(
        &self,
        owner: &str,
        mint: &str,
    ) -> Result<Vec<Value>, RpcError> {
        let params = json!([owner, {"mint": mint}, {"encoding": "jsonParsed"}]);
        let result = self.call("getTokenAccountsByOwner", params).await?;
        let parsed: TokenAccountsResult = serde_json::from_value(result)?;
        Ok(parsed.value)
    }

    pub async fn send_raw_transaction(
        &self,
        tx_b64: &str,
        skip_preflight: bool,
    ) -> Result<String, RpcError> {
        let params = json!([
            tx_b64,
            {
                "encoding": "base64",
                "skipPreflight": skip_preflight,
                "preflightCommitment": "confirmed",
            }
        ]);
        let result = self.call("sendTransaction", params).await?;
        Ok(serde_json::from_value::<String>(result)?)
    }

    pub async fn get_latest_blockhash(&self) -> Result<LatestBlockhash, RpcError> {
        let params = json!([{"commitment": "confirmed"}]);
        let result = self.call("getLatestBlockhash", params).await?;
        let parsed: BlockhashResult = serde_json::from_value(result)?;
        Ok(LatestBlockhash {
            blockhash: parsed.value.blockhash,
            last_valid_block_height: parsed.value.last_valid_block_height,
        })
    }

    pub async fn get_signature_statuses(
        &self,
        signatures: &[String],
    ) -> Result<Vec<Option<SignatureStatus>>, RpcError> {
        let params = json!([signatures, {"searchTransactionHistory": true}]);
        let result = self.call("getSignatureStatuses", params).await?;
        let parsed: SignatureStatusesResult = serde_json::from_value(result)?;
        Ok(parsed.value)
    }

    /// Poll `getSignatureStatuses` until the transaction is confirmed or
    /// finalized, fails on-chain, or `timeout_s` elapses.
    pub async fn confirm_signature(
        &self,
        signature: &str,
        timeout_s: f64,
        poll_interval_s: f64,
    ) -> Result<bool, RpcError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_s);
        loop {
            let statuses = self
                .get_signature_statuses(&[signature.to_string()])
                .await?;
            let entry = statuses.into_iter().next().flatten();
            if let Some(entry) = entry {
                if let Some(err) = entry.err {
                    return Err(RpcError::ConfirmationFailed {
                        signature: signature.to_string(),
                        err: err.to_string(),
                    });
                }
                if matches!(
                    entry.confirmation_status.as_deref(),
                    Some("confirmed") | Some("finalized")
                ) {
                    return Ok(true);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RpcError::ConfirmationTimeout {
                    signature: signature.to_string(),
                    timeout_s,
                });
            }
            tokio::time::sleep(Duration::from_secs_f64(poll_interval_s)).await;
        }
    }
}
