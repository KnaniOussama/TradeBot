from __future__ import annotations

import re
from datetime import UTC, datetime

import pandas as pd
import pytest

from tradebot.data.helius import HeliusClient, TokenTransfer
from tradebot.signals.base import MarketContext
from tradebot.signals.onchain import (
    OnChainSignal,
    _net_whale_flow,
    _transfer_count_zscore,
)


def _t(ts: int, mint: str, frm: str, to: str, amt: float) -> TokenTransfer:
    return TokenTransfer(  # noqa: E501
        signature=str(ts), timestamp=ts, mint=mint, from_addr=frm, to_addr=to, amount=amt
    )


def test_net_whale_flow_outflow_from_dex_positive():
    transfers = [
        _t(1, "M", "DEX1", "Whale1", 10000.0),  # DEX -> wallet (accumulation)
        _t(2, "M", "DEX1", "Whale2", 5000.0),
        _t(3, "M", "Retail", "DEX1", 50.0),  # tiny retail, ignored
    ]
    score = _net_whale_flow(transfers, mint="M", dex_addresses={"DEX1"}, whale_min=1000.0)
    assert score > 0


def test_net_whale_flow_inflow_to_dex_negative():
    transfers = [
        _t(1, "M", "Whale1", "DEX1", 10000.0),  # selling
    ]
    score = _net_whale_flow(transfers, mint="M", dex_addresses={"DEX1"}, whale_min=1000.0)
    assert score < 0


def test_net_whale_flow_balanced_near_zero():
    transfers = [
        _t(1, "M", "DEX1", "W1", 5000.0),
        _t(2, "M", "W2", "DEX1", 5000.0),
    ]
    score = _net_whale_flow(transfers, mint="M", dex_addresses={"DEX1"}, whale_min=1000.0)
    assert abs(score) < 0.05


def test_transfer_count_zscore_surge_positive():
    # 50 baseline minutes with 1 transfer each, then a recent burst
    bars = []
    for i in range(50):
        bars.append({"timestamp": i * 60, "transfer_count": 1.0})
    bars.append({"timestamp": 50 * 60, "transfer_count": 20.0})
    df = pd.DataFrame(bars)
    score = _transfer_count_zscore(df, window=20)
    assert score > 0.3


@pytest.mark.asyncio
async def test_onchain_signal_bullish_whale_outflow(httpx_mock):
    payload = [
        {
            "signature": "s1",
            "timestamp": 1746288000,
            "tokenTransfers": [
                {
                    "fromUserAccount": "DEX1",
                    "toUserAccount": "W1",
                    "mint": "MintX",
                    "tokenAmount": 10000.0,
                }
            ],
        },
        {
            "signature": "s2",
            "timestamp": 1746288060,
            "tokenTransfers": [
                {
                    "fromUserAccount": "DEX1",
                    "toUserAccount": "W2",
                    "mint": "MintX",
                    "tokenAmount": 8000.0,
                }
            ],
        },
    ]
    httpx_mock.add_response(url=re.compile(r".*helius.*"), json=payload, is_reusable=True)
    async with HeliusClient(api_key="k", base_url="https://api.helius.example") as h:
        sig = OnChainSignal(
            pair="X/USDC",
            timeframe="1m",
            helius=h,
            mint="MintX",
            dex_addresses={"DEX1"},
            whale_min=1000.0,
            lookback_limit=100,
        )
        ctx = MarketContext(pair="X/USDC", now=datetime.now(UTC), ohlcv={})
        score = await sig.score(ctx)
    assert score.signal == "onchain"
    assert score.score > 0
    assert "whale_flow" in score.components


@pytest.mark.asyncio
async def test_onchain_signal_helius_failure_returns_neutral(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*helius.*"), status_code=500, is_reusable=True)
    async with HeliusClient(api_key="k", base_url="https://api.helius.example") as h:
        sig = OnChainSignal(
            pair="X/USDC",
            timeframe="1m",
            helius=h,
            mint="MintX",
            dex_addresses={"DEX1"},
            whale_min=1000.0,
        )
        ctx = MarketContext(pair="X/USDC", now=datetime.now(UTC), ohlcv={})
        score = await sig.score(ctx)
    assert score.score == 0.0
