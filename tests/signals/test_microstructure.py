from __future__ import annotations

import json
import re
from datetime import UTC, datetime
from pathlib import Path

import pandas as pd
import pytest

from tradebot.data.jupiter import JupiterClient
from tradebot.signals.base import MarketContext
from tradebot.signals.microstructure import MicrostructureSignal, _volume_zscore, _vwap_deviation

THIN = json.loads(Path("tests/fixtures/jupiter_quote_thin.json").read_text())
NORMAL = json.loads(Path("tests/fixtures/jupiter_quote_sol_usdc.json").read_text())


def test_vwap_deviation_above_vwap_negative():
    df = pd.DataFrame(
        {
            "open": [100.0] * 30,
            "high": [100.0] * 30,
            "low": [100.0] * 30,
            "close": [100.0] * 29 + [110.0],
            "volume": [1000.0] * 30,
        }
    )
    score = _vwap_deviation(df, period=20)
    assert score < -0.3  # sharply above VWAP → mean-reverting bearish


def test_vwap_deviation_at_vwap_zero():
    df = pd.DataFrame(
        {
            "open": [100.0] * 30,
            "high": [100.0] * 30,
            "low": [100.0] * 30,
            "close": [100.0] * 30,
            "volume": [1000.0] * 30,
        }
    )
    score = _vwap_deviation(df, period=20)
    assert abs(score) < 0.05


def test_volume_zscore_surge_with_up_close_positive():
    df = pd.DataFrame(
        {
            "open": [100.0] * 50,
            "high": [100.0] * 50,
            "low": [100.0] * 50,
            "close": [100.0] * 49 + [101.0],
            "volume": [1000.0] * 49 + [10000.0],
        }
    )
    score = _volume_zscore(df, window=20)
    assert score > 0.3


def test_volume_zscore_surge_with_down_close_negative():
    df = pd.DataFrame(
        {
            "open": [100.0] * 50,
            "high": [100.0] * 50,
            "low": [100.0] * 50,
            "close": [100.0] * 49 + [99.0],
            "volume": [1000.0] * 49 + [10000.0],
        }
    )
    score = _volume_zscore(df, window=20)
    assert score < -0.3


@pytest.mark.asyncio
async def test_microstructure_signal_thin_market_emits_score(httpx_mock):
    # Same payload returned for both quote requests (buy and sell)
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=THIN, is_reusable=True)
    df = pd.DataFrame(
        {
            "open": [100.0] * 30,
            "high": [100.0] * 30,
            "low": [100.0] * 30,
            "close": [100.0] * 30,
            "volume": [1000.0] * 30,
        }
    )
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        sig = MicrostructureSignal(
            pair="SOL/USDC",
            timeframe="1m",
            jupiter=jup,
            base_mint="So11111111111111111111111111111111111111112",
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            base_decimals=9,
            quote_decimals=6,
            probe_size_in_quote=10.0,
        )
        ctx = MarketContext(
            pair="SOL/USDC",
            now=datetime.now(UTC),
            ohlcv={"1m": df},
        )
        score = await sig.score(ctx)
    assert score.signal == "microstructure"
    assert -1.0 <= score.score <= 1.0
    assert "depth_imbalance" in score.components
    assert "vwap_dev" in score.components
    assert "volume_z" in score.components


@pytest.mark.asyncio
async def test_microstructure_signal_no_quote_returns_partial(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), status_code=500, is_reusable=True)
    df = pd.DataFrame(
        {
            "open": [100.0] * 30,
            "high": [100.0] * 30,
            "low": [100.0] * 30,
            "close": [100.0] * 30,
            "volume": [1000.0] * 30,
        }
    )
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        sig = MicrostructureSignal(
            pair="X/Y",
            timeframe="1m",
            jupiter=jup,
            base_mint="A" * 43,
            quote_mint="B" * 43,
            base_decimals=9,
            quote_decimals=6,
            probe_size_in_quote=10.0,
        )
        ctx = MarketContext(
            pair="X/Y",
            now=datetime.now(UTC),
            ohlcv={"1m": df},
        )
        score = await sig.score(ctx)
    # When depth probe fails, depth_imbalance should be 0 but other components still computed
    assert score.components["depth_imbalance"] == 0.0


@pytest.mark.asyncio
async def test_microstructure_caches_depth_within_cycle(httpx_mock):
    # Add exactly 2 responses for the first score() call (buy + sell depth probe).
    # If cache doesn't work, the second score() call would attempt 2 more HTTP calls
    # and raise an error (no more responses available).
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=NORMAL, is_optional=False)
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=NORMAL, is_optional=False)
    df = pd.DataFrame(
        {
            "open": [100.0] * 30,
            "high": [100.0] * 30,
            "low": [100.0] * 30,
            "close": [100.0] * 30,
            "volume": [1000.0] * 30,
        }
    )
    async with JupiterClient(base_url="https://lite-api.jup.ag/swap/v1") as jup:
        sig = MicrostructureSignal(
            pair="X/Y",
            timeframe="1m",
            jupiter=jup,
            base_mint="A" * 43,
            quote_mint="B" * 43,
            base_decimals=9,
            quote_decimals=6,
            probe_size_in_quote=10.0,
        )
        sig.set_cycle_token(123)
        ctx = MarketContext(pair="X/Y", now=datetime.now(UTC), ohlcv={"1m": df})
        await sig.score(ctx)
        await sig.score(ctx)  # second call within same cycle: should not re-query Jupiter
