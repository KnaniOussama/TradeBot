import json
import os
from pathlib import Path

import pytest

from tradebot.wallet.keystore import (
    KeystoreError,
    decrypt_keystore,
    encrypt_secret,
    load_keystore,
    save_keystore,
)


def test_round_trip_encrypt_decrypt():
    secret = os.urandom(64)
    address = "Stub11111111111111111111111111111111111111"
    blob = encrypt_secret(secret=secret, passphrase="correct horse battery staple", address=address)
    assert blob["version"] == 1
    assert blob["address"] == address
    decrypted = decrypt_keystore(blob, passphrase="correct horse battery staple")
    assert decrypted == secret


def test_wrong_passphrase_raises():
    secret = os.urandom(64)
    blob = encrypt_secret(secret=secret, passphrase="right", address="X")
    with pytest.raises(KeystoreError):
        decrypt_keystore(blob, passphrase="wrong")


def test_save_and_load_roundtrip(tmp_path: Path):
    secret = os.urandom(64)
    blob = encrypt_secret(secret=secret, passphrase="pp", address="Addr")
    p = tmp_path / "bot.keystore.json"
    save_keystore(p, blob)
    loaded = load_keystore(p)
    assert decrypt_keystore(loaded, passphrase="pp") == secret


def test_load_missing_file_raises(tmp_path: Path):
    with pytest.raises(KeystoreError):
        load_keystore(tmp_path / "nope.json")


def test_corrupted_blob_raises():
    blob = encrypt_secret(secret=b"y" * 32, passphrase="pp", address="A")
    blob["ciphertext_b64"] = "###bad###"
    with pytest.raises(KeystoreError):
        decrypt_keystore(blob, passphrase="pp")


def test_blob_does_not_contain_plaintext_secret():
    secret = b"SUPER_SECRET_BYTES_FOR_TEST_ONLY"
    blob = encrypt_secret(secret=secret, passphrase="pp", address="A")
    s = json.dumps(blob)
    assert "SUPER_SECRET_BYTES" not in s
    assert secret.hex() not in s
