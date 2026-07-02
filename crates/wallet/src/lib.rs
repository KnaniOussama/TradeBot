//! Rust port of `tradebot/wallet`: the encrypted bot keystore, keypair,
//! on-chain balance lookups, and portfolio reconciliation against the
//! Solana RPC.
//!
//! Security-critical: this crate holds the encrypted bot keypair. The
//! on-disk keystore format (Argon2id KDF, AES-256-GCM cipher, JSON layout)
//! is ported byte-for-byte from `tradebot/wallet/keystore.py` so a keystore
//! written by either implementation can be decrypted by the other, given
//! the same passphrase. See `keystore.rs` for the parameter values and why
//! they must not change without a migration plan.

pub mod balance;
pub mod error;
pub mod keypair;
pub mod keystore;
pub mod reconcile;

pub use balance::{get_sol_balance, get_token_balance};
pub use error::{KeystoreError, WalletError};
pub use keypair::{generate_bot_keypair, load_bot_keypair, save_bot_keypair, BotKeypair};
pub use keystore::{
    decrypt_keystore, encrypt_secret, load_keystore, save_keystore, KdfParams, KeystoreBlob,
};
pub use reconcile::{
    log_findings, reconcile, FindingKind, ReconcileFinding, DEFAULT_MIN_SOL_FOR_FEES,
};
