/// Errors from the Jupiter aggregator client.
#[derive(Debug, thiserror::Error)]
pub enum JupiterError {
    #[error("jupiter http request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("failed to parse jupiter response: {0}")]
    Json(#[from] serde_json::Error),
}

/// Errors from the Solana JSON-RPC client.
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("rpc http request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("failed to parse rpc response: {0}")]
    Json(#[from] serde_json::Error),

    #[error("rpc error {code}: {message}")]
    JsonRpc { code: i64, message: String },

    #[error("unexpected rpc response shape: {0}")]
    UnexpectedShape(String),

    #[error("transaction {signature} failed: {err}")]
    ConfirmationFailed { signature: String, err: String },

    #[error("transaction {signature} not confirmed within {timeout_s}s")]
    ConfirmationTimeout { signature: String, timeout_s: f64 },
}
