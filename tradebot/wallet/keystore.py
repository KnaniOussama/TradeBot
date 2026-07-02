from __future__ import annotations

import base64
import json
import os
from pathlib import Path
from typing import Any, cast

from argon2.low_level import Type, hash_secret_raw
from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM


class KeystoreError(Exception):
    pass


_KDF_TIME_COST = 3
_KDF_MEMORY_COST = 65536
_KDF_PARALLELISM = 4
_KDF_KEY_LEN = 32
_AES_NONCE_LEN = 12


def _derive_key(passphrase: str, salt: bytes) -> bytes:
    return hash_secret_raw(
        secret=passphrase.encode("utf-8"),
        salt=salt,
        time_cost=_KDF_TIME_COST,
        memory_cost=_KDF_MEMORY_COST,
        parallelism=_KDF_PARALLELISM,
        hash_len=_KDF_KEY_LEN,
        type=Type.ID,
    )


def encrypt_secret(*, secret: bytes, passphrase: str, address: str) -> dict[str, Any]:
    salt = os.urandom(16)
    nonce = os.urandom(_AES_NONCE_LEN)
    key = _derive_key(passphrase, salt)
    aes = AESGCM(key)
    ciphertext = aes.encrypt(nonce, secret, associated_data=address.encode("utf-8"))
    return {
        "version": 1,
        "kdf": "argon2id",
        "kdf_params": {
            "time_cost": _KDF_TIME_COST,
            "memory_cost": _KDF_MEMORY_COST,
            "parallelism": _KDF_PARALLELISM,
            "salt_b64": base64.b64encode(salt).decode("ascii"),
        },
        "cipher": "aes-256-gcm",
        "nonce_b64": base64.b64encode(nonce).decode("ascii"),
        "ciphertext_b64": base64.b64encode(ciphertext).decode("ascii"),
        "address": address,
    }


def decrypt_keystore(blob: dict[str, Any], passphrase: str) -> bytes:
    try:
        salt = base64.b64decode(blob["kdf_params"]["salt_b64"])
        nonce = base64.b64decode(blob["nonce_b64"])
        ct = base64.b64decode(blob["ciphertext_b64"])
        address = blob["address"]
    except (KeyError, ValueError, TypeError) as e:
        raise KeystoreError(f"corrupted keystore: {e}") from e
    key = _derive_key(passphrase, salt)
    try:
        return AESGCM(key).decrypt(nonce, ct, associated_data=address.encode("utf-8"))
    except InvalidTag as e:
        raise KeystoreError("decryption failed (wrong passphrase or corrupted file)") from e


def save_keystore(path: Path, blob: dict[str, Any]) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(blob, indent=2), encoding="utf-8")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def load_keystore(path: Path) -> dict[str, Any]:
    path = Path(path)
    if not path.exists():
        raise KeystoreError(f"keystore not found: {path}")
    try:
        return cast(dict[str, Any], json.loads(path.read_text(encoding="utf-8")))
    except json.JSONDecodeError as e:
        raise KeystoreError(f"keystore not valid JSON: {e}") from e
