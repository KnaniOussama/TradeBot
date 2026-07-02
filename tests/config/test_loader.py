from pathlib import Path

import pytest

from tradebot.config.loader import (
    ConfigBundle,
    load_app_config,
    load_bundle,
    load_risk_config,
    load_watchlist,
    load_weights,
)


def _write(tmp_path: Path, name: str, content: str) -> Path:
    p = tmp_path / name
    p.write_text(content)
    return p


WATCHLIST_YAML = """\
quote_symbol: USDC
quote_mint: EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
entries:
  - symbol: SOL
    mint: So11111111111111111111111111111111111111112
    decimals: 9
"""

RISK_YAML = """\
max_concurrent_positions: 3
per_trade_size_min: 0.30
per_trade_size_max: 0.50
"""

WEIGHTS_YAML = """\
timeframes:
  5s: 0.10
  1m: 0.20
  15m: 0.30
  1h: 0.40
signals:
  ta: 0.40
  microstructure: 0.30
  onchain: 0.30
"""

APP_YAML = """\
rpc_url: https://api.devnet.solana.com
helius_api_key_env: HELIUS_API_KEY
starting_capital_usd: 50.0
jupiter_base_url: https://quote-api.jup.ag/v6
"""


def test_load_watchlist_example(tmp_path):
    p = _write(tmp_path, "watchlist.yaml", WATCHLIST_YAML)
    w = load_watchlist(p)
    assert w.quote_symbol == "USDC"
    assert any(e.symbol == "SOL" for e in w.entries)


def test_load_risk_example(tmp_path):
    p = _write(tmp_path, "risk.yaml", RISK_YAML)
    r = load_risk_config(p)
    assert r.max_concurrent_positions == 3


def test_load_weights_example(tmp_path):
    p = _write(tmp_path, "weights.yaml", WEIGHTS_YAML)
    w = load_weights(p)
    assert sum(w.timeframes.values()) == pytest.approx(1.0)


def test_load_app_example(tmp_path):
    p = _write(tmp_path, "app.yaml", APP_YAML)
    a = load_app_config(p)
    assert a.starting_capital_usd == 50.0


def test_load_bundle(tmp_path):
    bundle = load_bundle(
        app_path=_write(tmp_path, "app.yaml", APP_YAML),
        risk_path=_write(tmp_path, "risk.yaml", RISK_YAML),
        weights_path=_write(tmp_path, "weights.yaml", WEIGHTS_YAML),
        watchlist_path=_write(tmp_path, "watchlist.yaml", WATCHLIST_YAML),
    )
    assert isinstance(bundle, ConfigBundle)
    assert bundle.app.starting_capital_usd == 50.0
    assert len(bundle.watchlist.entries) > 0


def test_load_missing_file_raises(tmp_path):
    with pytest.raises(FileNotFoundError):
        load_app_config(tmp_path / "nope.yaml")


def test_load_invalid_yaml_raises(tmp_path):
    bad = tmp_path / "bad.yaml"
    bad.write_text("rpc_url: [unterminated")
    with pytest.raises(ValueError):
        load_app_config(bad)
