import pytest
from pydantic import ValidationError

from tradebot.config.models import (
    AppConfig,
    RiskConfig,
    WatchlistConfig,
    WatchlistEntry,
    WeightsConfig,
)


def test_watchlist_entry_requires_address_and_symbol():
    e = WatchlistEntry(symbol="SOL", mint="So11111111111111111111111111111111111111112", decimals=9)
    assert e.symbol == "SOL"
    assert e.decimals == 9


def test_watchlist_rejects_duplicate_symbols():
    with pytest.raises(ValidationError):
        WatchlistConfig(
            quote_symbol="USDC",
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            entries=[
                WatchlistEntry(symbol="SOL", mint="A" * 43, decimals=9),
                WatchlistEntry(symbol="SOL", mint="B" * 43, decimals=9),
            ],
        )


def test_risk_config_defaults_to_moderate_profile():
    r = RiskConfig()
    assert r.max_concurrent_positions == 3
    assert r.per_trade_size_min == 0.30
    assert r.per_trade_size_max == 0.50
    assert r.trailing_stop_pct == 0.02
    assert r.daily_loss_limit_pct == 0.08
    assert r.weekly_loss_limit_pct == 0.10
    assert r.per_trade_kill_pct == 0.03
    assert r.drawdown_circuit_pct == 0.15
    assert r.max_slippage_pct == 0.01
    assert r.max_trades_per_day == 10


def test_risk_config_rejects_inverted_size_range():
    with pytest.raises(ValidationError):
        RiskConfig(per_trade_size_min=0.6, per_trade_size_max=0.4)


def test_weights_config_normalizes_to_one():
    w = WeightsConfig(
        timeframes={"5s": 0.1, "1m": 0.2, "15m": 0.3, "1h": 0.4},
        signals={"ta": 0.4, "microstructure": 0.3, "onchain": 0.3},
    )
    assert sum(w.timeframes.values()) == pytest.approx(1.0)
    assert sum(w.signals.values()) == pytest.approx(1.0)


def test_weights_config_rejects_unnormalized():
    with pytest.raises(ValidationError):
        WeightsConfig(
            timeframes={"5s": 0.5, "1m": 0.5, "15m": 0.5, "1h": 0.5},
            signals={"ta": 1.0, "microstructure": 0.0, "onchain": 0.0},
        )


def test_app_config_minimal():
    a = AppConfig(
        rpc_url="https://example.com/rpc",
        helius_api_key_env="HELIUS_API_KEY",
        log_level="INFO",
        starting_capital_usd=50.0,
    )
    assert a.starting_capital_usd == 50.0
