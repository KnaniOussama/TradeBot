import asyncio
import json
from pathlib import Path

import pytest

from tradebot.data.jupiter import JupiterClient
from tradebot.data.prices import PriceFeed, PriceTick

FIXTURE = Path("tests/fixtures/jupiter_quote_sol_usdc.json")
_PAYLOAD = json.loads(FIXTURE.read_text())


@pytest.mark.asyncio
async def test_price_feed_emits_tick(httpx_mock):
    httpx_mock.add_response(json=_PAYLOAD)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        feed = PriceFeed(
            jupiter=jup,
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            poll_interval_s=0.01,
            sample_size_in_quote=1.0,
        )
        feed.add_pair(symbol="SOL", mint="So11111111111111111111111111111111111111112", decimals=9)
        sub = feed.subscribe()
        task = asyncio.create_task(feed.run())
        try:
            tick = await asyncio.wait_for(sub.get(), timeout=2.0)
        finally:
            feed.stop()
            await task
        assert isinstance(tick, PriceTick)
        assert tick.pair == "SOL/USDC"
        assert tick.price > 0


@pytest.mark.asyncio
async def test_price_feed_multiple_subscribers(httpx_mock):
    httpx_mock.add_response(json=_PAYLOAD, is_reusable=True)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        feed = PriceFeed(
            jupiter=jup,
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            poll_interval_s=0.01,
            sample_size_in_quote=1.0,
        )
        feed.add_pair(symbol="SOL", mint="So11111111111111111111111111111111111111112", decimals=9)
        s1 = feed.subscribe()
        s2 = feed.subscribe()
        task = asyncio.create_task(feed.run())
        try:
            t1 = await asyncio.wait_for(s1.get(), timeout=2.0)
            t2 = await asyncio.wait_for(s2.get(), timeout=2.0)
        finally:
            feed.stop()
            await task
        assert t1.pair == t2.pair == "SOL/USDC"


@pytest.mark.asyncio
async def test_price_feed_stop_terminates_run(httpx_mock):
    httpx_mock.add_response(json=_PAYLOAD, is_reusable=True)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        feed = PriceFeed(
            jupiter=jup,
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            poll_interval_s=0.01,
            sample_size_in_quote=1.0,
        )
        feed.add_pair(symbol="SOL", mint="So11111111111111111111111111111111111111112", decimals=9)
        task = asyncio.create_task(feed.run())
        await asyncio.sleep(0.05)
        feed.stop()
        await asyncio.wait_for(task, timeout=2.0)
