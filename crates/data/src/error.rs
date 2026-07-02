/// Errors from the Jupiter aggregator client.
#[derive(Debug, thiserror::Error)]
pub enum JupiterError {
    #[error("jupiter http request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("failed to parse jupiter response: {0}")]
    Json(#[from] serde_json::Error),
}
