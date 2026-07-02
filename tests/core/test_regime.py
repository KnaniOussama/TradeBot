from __future__ import annotations

import numpy as np
import pandas as pd
import pytest

from tradebot.core.regime import Regime, classify_regime


def _make_uptrend_df(n: int) -> pd.DataFrame:
    """Rising price with enough volatility for ADX to detect a trend."""
    rng = np.random.default_rng(42)
    prices = 100.0 + np.arange(n) * 0.5 + rng.normal(0, 0.1, n)
    # Build OHLC around close prices
    high = prices + rng.uniform(0.5, 1.0, n)
    low = prices - rng.uniform(0.5, 1.0, n)
    return pd.DataFrame(
        {
            "open": prices - 0.1,
            "high": high,
            "low": low,
            "close": prices,
            "volume": np.ones(n) * 1000,
        }
    )


def _make_choppy_df(n: int) -> pd.DataFrame:
    """Oscillating price that generates very low ADX."""
    rng = np.random.default_rng(7)
    # Tight sine-wave chop around 100
    t = np.linspace(0, 4 * np.pi, n)
    prices = 100.0 + np.sin(t) * 0.3 + rng.normal(0, 0.05, n)
    high = prices + 0.1
    low = prices - 0.1
    return pd.DataFrame(
        {
            "open": prices - 0.05,
            "high": high,
            "low": low,
            "close": prices,
            "volume": np.ones(n) * 1000,
        }
    )


def test_classify_trending_up():
    df = _make_uptrend_df(60)
    r = classify_regime(df, adx_period=14, ema_fast=20, ema_slow=50)
    assert r.label == "trending_up"


def test_classify_choppy():
    df = _make_choppy_df(60)
    r = classify_regime(df, adx_period=14, ema_fast=20, ema_slow=50)
    assert r.label == "chop"


def test_classify_returns_neutral_on_short_data():
    df = _make_uptrend_df(10)
    r = classify_regime(df)
    assert r.label == "neutral"


def test_regime_is_frozen_dataclass():
    r = Regime(label="neutral", adx=0.0, ema_fast_above_slow=False)
    with pytest.raises((AttributeError, TypeError)):
        r.label = "chop"  # type: ignore[misc]


def test_regime_adx_field_populated():
    df = _make_uptrend_df(60)
    r = classify_regime(df, adx_period=14, ema_fast=20, ema_slow=50)
    assert r.adx > 0.0


def test_regime_ema_fast_above_slow_on_uptrend():
    df = _make_uptrend_df(60)
    r = classify_regime(df, adx_period=14, ema_fast=20, ema_slow=50)
    assert r.ema_fast_above_slow is True
