from pathlib import Path

import pytest

from tradebot.wallet.keypair import (
    BotKeypair,
    generate_bot_keypair,
    load_bot_keypair,
    save_bot_keypair,
    to_solders,
)
from tradebot.wallet.keystore import KeystoreError


def test_generate_returns_address_and_secret():
    kp = generate_bot_keypair()
    assert isinstance(kp, BotKeypair)
    assert isinstance(kp.address, str)
    assert len(kp.address) >= 32
    assert isinstance(kp.secret_bytes, (bytes, bytearray))
    assert len(kp.secret_bytes) in (32, 64)


def test_save_and_load_roundtrip(tmp_path: Path):
    kp = generate_bot_keypair()
    path = tmp_path / "bot.keystore.json"
    save_bot_keypair(kp, path=path, passphrase="pp")
    loaded = load_bot_keypair(path=path, passphrase="pp")
    assert loaded.address == kp.address
    assert loaded.secret_bytes == kp.secret_bytes


def test_load_with_wrong_passphrase(tmp_path: Path):
    kp = generate_bot_keypair()
    path = tmp_path / "k.json"
    save_bot_keypair(kp, path=path, passphrase="right")
    with pytest.raises(KeystoreError):
        load_bot_keypair(path=path, passphrase="wrong")


def test_to_solders_roundtrip():
    kp = generate_bot_keypair()
    s = to_solders(kp)
    assert str(s.pubkey()) == kp.address
