//! Port of `tradebot/wallet/keystore.py`: an Argon2id + AES-256-GCM
//! encrypted keystore for the bot's Solana secret key.
//!
//! This module is security-critical and its on-disk format and crypto
//! parameters are ported byte-for-byte so a keystore file written by the
//! Python bot decrypts in Rust (and vice versa) given the same passphrase.
//! Do not change the KDF parameters, cipher, field names, or base64
//! alphabet without also migrating existing keystores.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::KeystoreError;

const KDF_TIME_COST: u32 = 3;
const KDF_MEMORY_COST: u32 = 65536;
const KDF_PARALLELISM: u32 = 4;
const KDF_KEY_LEN: usize = 32;
const AES_NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;

/// `kdf_params` object inside the keystore JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub time_cost: u32,
    pub memory_cost: u32,
    pub parallelism: u32,
    pub salt_b64: String,
}

/// The keystore JSON document (version 1). Field order matches
/// `keystore.py`'s `encrypt_secret` return dict so serialized output reads
/// the same in both implementations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeystoreBlob {
    pub version: u32,
    pub kdf: String,
    pub kdf_params: KdfParams,
    pub cipher: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub address: String,
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KDF_KEY_LEN], KeystoreError> {
    let params = Params::new(
        KDF_MEMORY_COST,
        KDF_TIME_COST,
        KDF_PARALLELISM,
        Some(KDF_KEY_LEN),
    )
    .map_err(|e| KeystoreError::Corrupted(format!("invalid kdf params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KDF_KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| KeystoreError::Corrupted(format!("kdf failed: {e}")))?;
    Ok(key)
}

/// Encrypts `secret` under `passphrase`, binding the ciphertext to `address`
/// via AES-GCM associated data. Returns the JSON-serializable keystore blob;
/// the caller passes it to [`save_keystore`].
pub fn encrypt_secret(
    secret: &[u8],
    passphrase: &str,
    address: &str,
) -> Result<KeystoreBlob, KeystoreError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; AES_NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| KeystoreError::Corrupted(format!("bad key length: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: secret,
                aad: address.as_bytes(),
            },
        )
        .map_err(|_| KeystoreError::Corrupted("encryption failed".to_string()))?;

    Ok(KeystoreBlob {
        version: 1,
        kdf: "argon2id".to_string(),
        kdf_params: KdfParams {
            time_cost: KDF_TIME_COST,
            memory_cost: KDF_MEMORY_COST,
            parallelism: KDF_PARALLELISM,
            salt_b64: BASE64.encode(salt),
        },
        cipher: "aes-256-gcm".to_string(),
        nonce_b64: BASE64.encode(nonce_bytes),
        ciphertext_b64: BASE64.encode(&ciphertext),
        address: address.to_string(),
    })
}

/// Decrypts a keystore blob under `passphrase`, verifying the AES-GCM tag
/// and that the associated data matches `blob.address`.
pub fn decrypt_keystore(blob: &KeystoreBlob, passphrase: &str) -> Result<Vec<u8>, KeystoreError> {
    let salt = BASE64
        .decode(&blob.kdf_params.salt_b64)
        .map_err(|e| KeystoreError::Corrupted(format!("bad salt_b64: {e}")))?;
    let nonce_bytes = BASE64
        .decode(&blob.nonce_b64)
        .map_err(|e| KeystoreError::Corrupted(format!("bad nonce_b64: {e}")))?;
    let ciphertext = BASE64
        .decode(&blob.ciphertext_b64)
        .map_err(|e| KeystoreError::Corrupted(format!("bad ciphertext_b64: {e}")))?;

    let nonce_arr: [u8; AES_NONCE_LEN] = nonce_bytes
        .as_slice()
        .try_into()
        .map_err(|_| KeystoreError::Corrupted("invalid nonce length".to_string()))?;

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| KeystoreError::Corrupted(format!("bad key length: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_arr);

    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &ciphertext,
                aad: blob.address.as_bytes(),
            },
        )
        .map_err(|_| KeystoreError::DecryptionFailed)
}

/// Writes `blob` to `path` as pretty-printed JSON, creating parent
/// directories as needed. Best-effort restricts file permissions to the
/// owner on Unix, matching Python's `os.chmod(path, 0o600)` (also
/// best-effort there via a swallowed `OSError`).
pub fn save_keystore(path: &Path, blob: &KeystoreBlob) -> Result<(), KeystoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(blob)
        .map_err(|e| KeystoreError::Corrupted(format!("failed to serialize keystore: {e}")))?;
    std::fs::write(path, json)?;
    set_owner_only_permissions(path);
    Ok(())
}

/// Reads and parses a keystore JSON file written by [`save_keystore`] (or
/// by the Python `save_keystore`).
pub fn load_keystore(path: &Path) -> Result<KeystoreBlob, KeystoreError> {
    if !path.exists() {
        return Err(KeystoreError::NotFound(path.display().to_string()));
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| KeystoreError::InvalidJson(e.to_string()))
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_encrypt_decrypt() {
        let secret: Vec<u8> = (0..64).map(|_| rand::random::<u8>()).collect();
        let address = "Stub11111111111111111111111111111111111111";
        let blob = encrypt_secret(&secret, "correct horse battery staple", address).unwrap();
        assert_eq!(blob.version, 1);
        assert_eq!(blob.address, address);
        let decrypted = decrypt_keystore(&blob, "correct horse battery staple").unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn wrong_passphrase_raises() {
        let secret = vec![7u8; 64];
        let blob = encrypt_secret(&secret, "right", "X").unwrap();
        let result = decrypt_keystore(&blob, "wrong");
        assert!(matches!(result, Err(KeystoreError::DecryptionFailed)));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let secret = vec![9u8; 64];
        let blob = encrypt_secret(&secret, "pp", "Addr").unwrap();
        let path = dir.path().join("bot.keystore.json");
        save_keystore(&path, &blob).unwrap();
        let loaded = load_keystore(&path).unwrap();
        assert_eq!(decrypt_keystore(&loaded, "pp").unwrap(), secret);
    }

    #[test]
    fn load_missing_file_raises() {
        let dir = tempdir().unwrap();
        let result = load_keystore(&dir.path().join("nope.json"));
        assert!(matches!(result, Err(KeystoreError::NotFound(_))));
    }

    #[test]
    fn corrupted_blob_raises() {
        let mut blob = encrypt_secret(&[b'y'; 32], "pp", "A").unwrap();
        blob.ciphertext_b64 = "###bad###".to_string();
        let result = decrypt_keystore(&blob, "pp");
        assert!(matches!(result, Err(KeystoreError::Corrupted(_))));
    }

    #[test]
    fn blob_does_not_contain_plaintext_secret() {
        let secret = b"SUPER_SECRET_BYTES_FOR_TEST_ONLY";
        let blob = encrypt_secret(secret, "pp", "A").unwrap();
        let s = serde_json::to_string(&blob).unwrap();
        assert!(!s.contains("SUPER_SECRET_BYTES"));
        let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!s.contains(&hex));
    }
}
