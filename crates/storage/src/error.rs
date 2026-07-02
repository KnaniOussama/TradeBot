use std::path::PathBuf;

/// Errors from the storage layer: atomic file writes, JSON (de)serialization.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize JSON for {path}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}
