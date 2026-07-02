from datetime import UTC, datetime

import pandas as pd
import pytest

from tradebot.signals.base import (
    MarketContext,
    SignalScore,
    clamp_score,
    rolling_zscore,
)


def test_clamp_score_clips_to_range():
    assert clamp_score(0.5) == 0.5
    assert clamp_score(2.0) == 1.0
    assert clamp_score(-3.0) == -1.0
    assert clamp_score(0.0) == 0.0


def test_clamp_score_handles_nan():
    import math

    assert clamp_score(math.nan) == 0.0


def test_signal_score_dataclass():
    s = SignalScore(
        signal="ta",
        pair="SOL/USDC",
        timeframe="1m",
        score=0.5,
        sampled_at=datetime(2026, 5, 3, tzinfo=UTC),
        components={"rsi": 0.3, "macd": 0.7},
    )
    assert s.score == 0.5
    assert s.components["rsi"] == 0.3


def test_signal_score_rejects_out_of_range():
    with pytest.raises(ValueError):
        SignalScore(
            signal="ta",
            pair="SOL/USDC",
            timeframe="1m",
            score=1.5,
            sampled_at=datetime.now(UTC),
            components={},
        )


def test_rolling_zscore_normal_case():
    series = pd.Series([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], dtype=float)
    z = rolling_zscore(series, window=5)
    # Last value should be a clear positive z-score (above mean of last 5)
    assert z.iloc[-1] > 0
    # Insufficient-window leading values should be NaN
    assert pd.isna(z.iloc[0])


def test_rolling_zscore_zero_variance():
    series = pd.Series([5.0] * 20)
    z = rolling_zscore(series, window=5)
    # All zeros (or NaN safely handled) — never inf
    assert not z.iloc[-1] != z.iloc[-1] * 1  # not inf
    assert abs(z.iloc[-1]) < 1e9


def test_market_context_minimal():
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime(2026, 5, 3, tzinfo=UTC),
        ohlcv={
            "1m": pd.DataFrame({"open": [], "high": [], "low": [], "close": [], "volume": []}),
        },
    )
    assert ctx.pair == "SOL/USDC"
    assert "1m" in ctx.ohlcv
