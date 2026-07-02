import json
from pathlib import Path

import pytest

from tradebot.config.defaults import default_config
from tradebot.config.file import (
    TradeBotConfig,
    load_config,
    save_config,
)


def test_default_config_is_valid():
    cfg = default_config()
    assert isinstance(cfg, TradeBotConfig)
    assert cfg.app.starting_capital_usd == 50.0
    assert cfg.dashboard.port == 8765
    assert any(e.symbol == "SOL" for e in cfg.watchlist.entries)


def test_save_and_load_roundtrip(tmp_path: Path):
    cfg = default_config()
    p = tmp_path / "tradebot.config.json"
    save_config(p, cfg)
    loaded = load_config(p)
    assert loaded.app.starting_capital_usd == cfg.app.starting_capital_usd
    assert loaded.risk.max_concurrent_positions == cfg.risk.max_concurrent_positions


def test_load_missing_file_raises(tmp_path: Path):
    with pytest.raises(FileNotFoundError):
        load_config(tmp_path / "missing.json")


def test_save_writes_pretty_json(tmp_path: Path):
    cfg = default_config()
    p = tmp_path / "x.json"
    save_config(p, cfg)
    s = p.read_text()
    assert "\n" in s  # indented
    parsed = json.loads(s)
    assert parsed["app"]["starting_capital_usd"] == 50.0


def test_load_validates_schema(tmp_path: Path):
    p = tmp_path / "bad.json"
    p.write_text(json.dumps({"app": "not an object"}))
    with pytest.raises(ValueError):
        load_config(p)
