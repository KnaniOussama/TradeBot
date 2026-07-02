//! Error types for the wallet crate.

use thiserror::Error;

/// Errors from keystore encryption, decryption, and file I/O. Port of the
/// single `KeystoreError` exception in `tradebot/wallet/keystore.py`, split
/// into variants so callers can distinguish failure modes while still
/// matching the Python error messages.
#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("corrupted keystore: {0}")]
    Corrupted(String),

    #[error("decryption failed (wrong passphrase or corrupted file)")]
    DecryptionFailed,

    #[error("keystore not found: {0}")]
    NotFound(String),

    #[error("keystore not valid JSON: {0}")]
    InvalidJson(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from the wallet crate as a whole.
#[derive(Debug, Error)]
pub enum WalletError {
    #[error(transparent)]
    Keystore(#[from] KeystoreError),

    #[error("invalid secret length: {0} bytes (expected 32 or 64)")]
    InvalidSecretLength(usize),
}
