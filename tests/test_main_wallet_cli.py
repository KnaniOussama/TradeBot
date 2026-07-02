from pathlib import Path

import pytest
from click.testing import CliRunner


@pytest.fixture
def runner():
    return CliRunner()


def test_wallet_generate_creates_keystore(runner, tmp_path: Path, monkeypatch):
    monkeypatch.setenv("TRADEBOT_PASSPHRASE", "test-pp")
    keystore = tmp_path / "k.json"
    from tradebot.main import cli

    result = runner.invoke(cli, ["wallet", "generate", "--keystore", str(keystore)])
    assert result.exit_code == 0, result.output
    assert keystore.exists()
    assert "Bot address:" in result.output


def test_wallet_generate_refuses_overwrite(runner, tmp_path: Path, monkeypatch):
    monkeypatch.setenv("TRADEBOT_PASSPHRASE", "test-pp")
    keystore = tmp_path / "k.json"
    keystore.write_text("{}")
    from tradebot.main import cli

    result = runner.invoke(cli, ["wallet", "generate", "--keystore", str(keystore)])
    assert result.exit_code != 0
    assert "refusing to overwrite" in result.output


def test_wallet_show_prints_address(runner, tmp_path: Path, monkeypatch):
    monkeypatch.setenv("TRADEBOT_PASSPHRASE", "test-pp")
    keystore = tmp_path / "k.json"
    from tradebot.main import cli

    runner.invoke(cli, ["wallet", "generate", "--keystore", str(keystore)])
    result = runner.invoke(cli, ["wallet", "show", "--keystore", str(keystore)])
    assert result.exit_code == 0
    assert "Bot address:" in result.output


def test_wallet_no_passphrase_aborts(runner, tmp_path: Path, monkeypatch):
    monkeypatch.delenv("TRADEBOT_PASSPHRASE", raising=False)
    from tradebot.main import cli

    result = runner.invoke(cli, ["wallet", "generate", "--keystore", str(tmp_path / "k.json")])
    assert result.exit_code != 0
    assert "TRADEBOT_PASSPHRASE" in result.output
