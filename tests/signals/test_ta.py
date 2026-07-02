from datetime import UTC, datetime
from pathlib import Path

import pandas as pd
import pytest

from tradebot.signals.base import MarketContext
from tradebot.signals.ta import TASignal, _bb_position, _ema_cross, _macd_score, _rsi_score


def _load(name: str) -> pd.DataFrame:
    df = pd.read_csv(Path("tests/fixtures") / name)
    return df


def test_rsi_score_overbought_negative():
    # Series climbing fast → RSI > 70 → score should be negative (mean-reverting bearish)
    s = pd.Series([100 + i * 1.0 for i in range(50)], dtype=float)
    score = _rsi_score(s, period=14)
    assert score < -0.3


def test_rsi_score_oversold_positive():
    s = pd.Series([100 - i * 1.0 for i in range(50)], dtype=float)
    score = _rsi_score(s, period=14)
    assert score > 0.3


def test_macd_uptrend_positive():
    df = _load("ohlcv_sol_uptrend.csv")
    score = _macd_score(df["close"])
    assert score > 0


def test_macd_downtrend_negative():
    df = _load("ohlcv_sol_downtrend.csv")
    score = _macd_score(df["close"])
    assert score < 0


def test_ema_cross_uptrend_positive():
    df = _load("ohlcv_sol_uptrend.csv")
    score = _ema_cross(df["close"], fast=20, slow=50)
    assert score > 0


def test_bb_position_within_range():
    df = _load("ohlcv_sol_choppy.csv")
    score = _bb_position(df["close"], period=20, stds=2.0)
    assert -1.0 <= score <= 1.0


@pytest.mark.asyncio
async def test_ta_signal_uptrend_bullish():
    df = _load("ohlcv_sol_uptrend.csv")
    sig = TASignal(timeframe="1m")
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime.now(UTC),
        ohlcv={"1m": df},
    )
    score = await sig.score(ctx)
    assert score.signal == "ta"
    assert score.pair == "SOL/USDC"
    assert score.timeframe == "1m"
    assert -1.0 <= score.score <= 1.0
    # Trend signals (macd, ema_cross) should dominate; expect positive composite
    assert score.score > 0
    assert "rsi" in score.components
    assert "macd" in score.components
    assert "ema_cross" in score.components
    assert "bb" in score.components


@pytest.mark.asyncio
async def test_ta_signal_downtrend_bearish():
    df = _load("ohlcv_sol_downtrend.csv")
    sig = TASignal(timeframe="1m")
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime.now(UTC),
        ohlcv={"1m": df},
    )
    score = await sig.score(ctx)
    assert score.score < 0


@pytest.mark.asyncio
async def test_ta_signal_returns_zero_when_insufficient_data():
    df = pd.DataFrame(
        {
            "open": [100.0] * 5,
            "high": [101.0] * 5,
            "low": [99.0] * 5,
            "close": [100.5] * 5,
            "volume": [1000.0] * 5,
        }
    )
    sig = TASignal(timeframe="1m")
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime.now(UTC),
        ohlcv={"1m": df},
    )
    score = await sig.score(ctx)
    assert score.score == 0.0


@pytest.mark.asyncio
async def test_ta_signal_missing_timeframe_returns_zero():
    sig = TASignal(timeframe="1m")
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime.now(UTC),
        ohlcv={},
    )
    score = await sig.score(ctx)
    assert score.score == 0.0
