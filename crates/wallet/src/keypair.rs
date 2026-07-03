//! Port of `tradebot/wallet/keypair.py`: the bot's Solana ed25519 keypair,
//! encrypted at rest via [`crate::keystore`].
//!
//! `secret_bytes` mirrors the byte layout solders' `Keypair` produces via
//! `bytes(kp)`: a 64-byte `seed(32) || pubkey(32)` array, the same layout
//! Solana CLI `id.json` files and `solana-sdk::Keypair::to_bytes()` use.
//! Storing this exact layout (rather than just the 32-byte seed) is what
//! keeps keystores interchangeable between the Python and Rust bots.

use std::path::Path;

use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;

use crate::error::WalletError;
use crate::keystore::{decrypt_keystore, encrypt_secret, load_keystore, save_keystore};

/// The bot's Solana keypair: a base58 address and the raw secret bytes as
/// stored in the keystore (32-byte seed, or 64-byte seed||pubkey).
#[derive(Clone, PartialEq, Eq)]
pub struct BotKeypair {
    pub address: String,
    pub secret_bytes: Vec<u8>,
}

// Manual Debug that never prints the private key. Deriving Debug would leak
// `secret_bytes` (the raw seed) into any `{:?}` / tracing debug output.
impl std::fmt::Debug for BotKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotKeypair")
            .field("address", &self.address)
            .field("secret_bytes", &"<redacted>")
            .finish()
    }
}

// Wipe the secret seed from memory when the keypair is dropped so it does not
// linger in freed heap.
impl Drop for BotKeypair {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.secret_bytes.zeroize();
    }
}

impl BotKeypair {
    /// Builds the ed25519 signing key from `secret_bytes`. Accepts either a
    /// 32-byte seed or a 64-byte `seed || pubkey` array; both encode the
    /// same seed in their first 32 bytes, matching Python's `to_solders`
    /// (`from_bytes` for 64, `from_seed` for 32).
    pub fn signing_key(&self) -> Result<SigningKey, WalletError> {
        let seed: [u8; 32] = match self.secret_bytes.len() {
            32 | 64 => self.secret_bytes[..32]
                .try_into()
                .expect("slice of exactly 32 bytes"),
            n => return Err(WalletError::InvalidSecretLength(n)),
        };
        Ok(SigningKey::from_bytes(&seed))
    }

    /// Signs `message`, returning the 64-byte ed25519 signature.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], WalletError> {
        let signing_key = self.signing_key()?;
        Ok(signing_key.sign(message).to_bytes())
    }
}

/// Generates a fresh random keypair.
pub fn generate_bot_keypair() -> BotKeypair {
    let signing_key = SigningKey::generate(&mut OsRng);
    let seed = signing_key.to_bytes();
    let pubkey_bytes = signing_key.verifying_key().to_bytes();

    let mut secret_bytes = Vec::with_capacity(64);
    secret_bytes.extend_from_slice(&seed);
    secret_bytes.extend_from_slice(&pubkey_bytes);

    let address = bs58::encode(pubkey_bytes).into_string();
    BotKeypair {
        address,
        secret_bytes,
    }
}

/// Encrypts and saves `kp` to `path` under `passphrase`.
pub fn save_bot_keypair(kp: &BotKeypair, path: &Path, passphrase: &str) -> Result<(), WalletError> {
    let blob = encrypt_secret(&kp.secret_bytes, passphrase, &kp.address)?;
    save_keystore(path, &blob)?;
    Ok(())
}

/// Loads and decrypts a keypair from `path` under `passphrase`.
pub fn load_bot_keypair(path: &Path, passphrase: &str) -> Result<BotKeypair, WalletError> {
    let blob = load_keystore(path)?;
    let secret = decrypt_keystore(&blob, passphrase)?;
    Ok(BotKeypair {
        address: blob.address,
        secret_bytes: secret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generate_returns_address_and_secret() {
        let kp = generate_bot_keypair();
        assert!(kp.address.len() >= 32);
        assert!(matches!(kp.secret_bytes.len(), 32 | 64));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let kp = generate_bot_keypair();
        let path = dir.path().join("bot.keystore.json");
        save_bot_keypair(&kp, &path, "pp").unwrap();
        let loaded = load_bot_keypair(&path, "pp").unwrap();
        assert_eq!(loaded.address, kp.address);
        assert_eq!(loaded.secret_bytes, kp.secret_bytes);
    }

    #[test]
    fn load_with_wrong_passphrase_fails() {
        let dir = tempdir().unwrap();
        let kp = generate_bot_keypair();
        let path = dir.path().join("k.json");
        save_bot_keypair(&kp, &path, "right").unwrap();
        let result = load_bot_keypair(&path, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn signing_key_pubkey_matches_address() {
        let kp = generate_bot_keypair();
        let signing_key = kp.signing_key().unwrap();
        let derived_address = bs58::encode(signing_key.verifying_key().to_bytes()).into_string();
        assert_eq!(derived_address, kp.address);
    }

    #[test]
    fn sign_produces_valid_signature() {
        use ed25519_dalek::Verifier;
        let kp = generate_bot_keypair();
        let msg = b"hello wallet";
        let sig_bytes = kp.sign(msg).unwrap();
        let signing_key = kp.signing_key().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(signing_key.verifying_key().verify(msg, &sig).is_ok());
    }
}
