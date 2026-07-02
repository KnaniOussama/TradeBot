from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from solders.keypair import Keypair as SoldersKeypair

from tradebot.wallet.keystore import (
    decrypt_keystore,
    encrypt_secret,
    load_keystore,
    save_keystore,
)


@dataclass
class BotKeypair:
    address: str
    secret_bytes: bytes


def generate_bot_keypair() -> BotKeypair:
    kp = SoldersKeypair()
    return BotKeypair(address=str(kp.pubkey()), secret_bytes=bytes(kp))


def save_bot_keypair(kp: BotKeypair, path: Path, passphrase: str) -> None:
    blob = encrypt_secret(secret=kp.secret_bytes, passphrase=passphrase, address=kp.address)
    save_keystore(path, blob)


def load_bot_keypair(path: Path, passphrase: str) -> BotKeypair:
    blob = load_keystore(path)
    secret = decrypt_keystore(blob, passphrase=passphrase)
    return BotKeypair(address=blob["address"], secret_bytes=secret)


def to_solders(kp: BotKeypair) -> SoldersKeypair:
    if len(kp.secret_bytes) == 64:
        return SoldersKeypair.from_bytes(kp.secret_bytes)
    return SoldersKeypair.from_seed(kp.secret_bytes)
