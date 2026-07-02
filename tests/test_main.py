import json

import pytest
from click.testing import CliRunner


@pytest.fixture
def runner():
    return CliRunner()


def test_init_creates_default_config(runner, tmp_path):
    config_path = tmp_path / "tb.config.json"
    from tradebot.main import cli

    result = runner.invoke(cli, ["init", "--config", str(config_path)])
    assert result.exit_code == 0, result.output
    assert config_path.exists()
    cfg = json.loads(config_path.read_text())
    assert cfg["app"]["starting_capital_usd"] == 50.0


def test_init_refuses_overwrite(runner, tmp_path):
    config_path = tmp_path / "tb.config.json"
    config_path.write_text("{}")
    from tradebot.main import cli

    result = runner.invoke(cli, ["init", "--config", str(config_path)])
    assert result.exit_code != 0
    assert "already exists" in result.output


def test_start_real_without_confirm_aborts(runner, tmp_path, monkeypatch):
    config_path = tmp_path / "tb.config.json"
    from tradebot.config.defaults import default_config
    from tradebot.config.file import save_config

    save_config(config_path, default_config())
    monkeypatch.setenv("TRADEBOT_PASSPHRASE", "x")
    from tradebot.main import cli

    result = runner.invoke(cli, ["start", "--config", str(config_path), "--mode", "real"])
    assert result.exit_code != 0
