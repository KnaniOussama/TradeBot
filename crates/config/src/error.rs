use std::path::PathBuf;

/// Errors from loading, parsing, or validating a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config not found: {0}")]
    NotFound(PathBuf),
    #[error("invalid JSON in {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("config validation failed: {0}")]
    Validation(String),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
