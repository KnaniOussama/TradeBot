//! Cross-language keystore compatibility gate.
//!
//! This is the hard requirement for the wallet port: a keystore encrypted
//! by the Python bot must decrypt in Rust, and a keystore encrypted by Rust
//! must decrypt in Python, given the same passphrase. If this test fails,
//! the crypto parameters or on-disk format have drifted and existing
//! wallets would stop working.
//!
//! Skips (rather than fails) if the project's `.venv` is not present, so
//! the suite still runs in environments without the Python toolchain.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;
use tradebot_wallet::keypair::{generate_bot_keypair, save_bot_keypair, BotKeypair};
use tradebot_wallet::keystore::{decrypt_keystore, load_keystore};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root should exist")
}

fn venv_python() -> Option<PathBuf> {
    let root = repo_root();
    for candidate in [".venv/Scripts/python.exe", ".venv/bin/python"] {
        let path = root.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn run_python(python: &Path, script: &str, args: &[&str]) -> String {
    let root = repo_root();
    let mut cmd = Command::new(python);
    cmd.current_dir(&root).arg("-c").arg(script).args(args);
    let output = cmd.output().expect("failed to spawn python");
    if !output.status.success() {
        panic!(
            "python script failed (status {:?})\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout)
        .expect("python stdout should be utf8")
        .trim()
        .to_string()
}

/// Derives the base58 address from a 32/64-byte secret, the same way
/// [`BotKeypair::signing_key`] does, to confirm the decrypted plaintext is
/// actually a usable keypair (not just "some bytes that decrypted okay").
fn address_from_secret(secret_bytes: Vec<u8>) -> String {
    let kp = BotKeypair {
        address: String::new(),
        secret_bytes,
    };
    let signing_key = kp.signing_key().expect("valid secret length");
    bs58::encode(signing_key.verifying_key().to_bytes()).into_string()
}

#[test]
fn python_encrypted_keystore_decrypts_in_rust() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: .venv not found");
        return;
    };

    let dir = tempdir().unwrap();
    let path = dir.path().join("py_written.keystore.json");
    let passphrase = "cross-lang test passphrase 1";

    let script = r#"
import sys
from pathlib import Path
from tradebot.wallet.keypair import generate_bot_keypair, save_bot_keypair

kp = generate_bot_keypair()
path = Path(sys.argv[1])
passphrase = sys.argv[2]
save_bot_keypair(kp, path=path, passphrase=passphrase)
print(kp.address)
"#;
    let python_address = run_python(&python, script, &[path.to_str().unwrap(), passphrase]);

    let blob = load_keystore(&path).expect("rust should load the python-written keystore");
    assert_eq!(blob.version, 1);
    assert_eq!(blob.kdf, "argon2id");
    assert_eq!(blob.cipher, "aes-256-gcm");
    assert_eq!(blob.address, python_address);

    let secret = decrypt_keystore(&blob, passphrase)
        .expect("rust should decrypt a python-encrypted keystore");
    let recovered_address = address_from_secret(secret);

    assert_eq!(recovered_address, python_address);
}

#[test]
fn rust_encrypted_keystore_decrypts_in_python() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: .venv not found");
        return;
    };

    let dir = tempdir().unwrap();
    let path = dir.path().join("rust_written.keystore.json");
    let passphrase = "cross-lang test passphrase 2";

    let kp = generate_bot_keypair();
    save_bot_keypair(&kp, &path, passphrase).expect("rust should save the keystore");

    let script = r#"
import sys
from pathlib import Path
from tradebot.wallet.keypair import load_bot_keypair

path = Path(sys.argv[1])
passphrase = sys.argv[2]
kp = load_bot_keypair(path=path, passphrase=passphrase)
print(kp.address)
"#;
    let python_address = run_python(&python, script, &[path.to_str().unwrap(), passphrase]);

    assert_eq!(python_address, kp.address);
}

#[test]
fn wrong_passphrase_fails_across_languages() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: .venv not found");
        return;
    };

    let dir = tempdir().unwrap();
    let path = dir.path().join("rust_written_wrong_pp.keystore.json");
    let kp = generate_bot_keypair();
    save_bot_keypair(&kp, &path, "right passphrase").expect("rust should save the keystore");

    let script = r#"
import sys
from pathlib import Path
from tradebot.wallet.keypair import load_bot_keypair
from tradebot.wallet.keystore import KeystoreError

path = Path(sys.argv[1])
try:
    load_bot_keypair(path=path, passphrase="wrong passphrase")
except KeystoreError:
    print("KeystoreError")
else:
    print("NO_ERROR")
"#;
    let result = run_python(&python, script, &[path.to_str().unwrap()]);
    assert_eq!(result, "KeystoreError");
}
