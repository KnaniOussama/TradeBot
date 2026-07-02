import re

import pytest

from tradebot.data.birdeye import BirdeyeClient

SOL = "So11111111111111111111111111111111111111112"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"


@pytest.mark.asyncio
async def test_multi_price_returns_dict(httpx_mock):
    httpx_mock.add_response(
        url=re.compile(r".*/defi/multi_price.*"),
        json={
            "success": True,
            "data": {
                SOL: {"value": 145.32, "updateUnixTime": 1714742400},
                USDC: {"value": 0.9998},
            },
        },
    )
    async with BirdeyeClient(api_key="test-key") as c:
        prices = await c.multi_price([SOL, USDC])
    assert prices[SOL] == pytest.approx(145.32)
    assert prices[USDC] == pytest.approx(0.9998)


@pytest.mark.asyncio
async def test_multi_price_skips_entries_without_value(httpx_mock):
    httpx_mock.add_response(
        url=re.compile(r".*/defi/multi_price.*"),
        json={
            "success": True,
            "data": {
                SOL: {"value": 145.0},
                "BadMint11111111111111111111111111111111111": {},
            },
        },
    )
    async with BirdeyeClient(api_key="test-key") as c:
        prices = await c.multi_price([SOL, "BadMint11111111111111111111111111111111111"])
    assert SOL in prices
    assert "BadMint11111111111111111111111111111111111" not in prices


@pytest.mark.asyncio
async def test_multi_price_returns_empty_on_http_error(httpx_mock):
    # Persistent 429 after retries → returns {} from _try_multi_price (no plan-lock,
    # so the bot will try multi_price again next cycle rather than fanning out).
    httpx_mock.add_response(
        url=re.compile(r".*/defi/multi_price.*"), status_code=429, is_reusable=True
    )
    async with BirdeyeClient(api_key="test-key", max_429_retries=0) as c:
        prices = await c.multi_price([SOL])
    assert prices == {}


@pytest.mark.asyncio
async def test_multi_price_returns_empty_when_unsuccessful(httpx_mock):
    httpx_mock.add_response(
        url=re.compile(r".*/defi/multi_price.*"),
        json={"success": False, "message": "rate limit"},
    )
    async with BirdeyeClient(api_key="test-key") as c:
        prices = await c.multi_price([SOL])
    assert prices == {}


@pytest.mark.asyncio
async def test_multi_price_empty_input_short_circuits():
    # No mock: should never make a network call.
    async with BirdeyeClient(api_key="test-key") as c:
        prices = await c.multi_price([])
    assert prices == {}


@pytest.mark.asyncio
async def test_falls_back_to_single_price_on_401(httpx_mock):
    # First call: multi_price returns 401 (free tier doesn't include it).
    httpx_mock.add_response(
        url=re.compile(r".*/defi/multi_price.*"), status_code=401
    )
    # Subsequent calls: per-token /defi/price succeeds.
    httpx_mock.add_response(
        url=re.compile(r".*/defi/price\?address=" + SOL),
        json={"success": True, "data": {"value": 145.0}},
    )
    httpx_mock.add_response(
        url=re.compile(r".*/defi/price\?address=" + USDC),
        json={"success": True, "data": {"value": 1.0}},
    )
    async with BirdeyeClient(api_key="test-key") as c:
        prices = await c.multi_price([SOL, USDC])
    assert prices[SOL] == pytest.approx(145.0)
    assert prices[USDC] == pytest.approx(1.0)
    # Subsequent calls skip multi_price entirely.
    httpx_mock.add_response(
        url=re.compile(r".*/defi/price\?address=" + SOL),
        json={"success": True, "data": {"value": 146.0}},
    )
    async with BirdeyeClient(api_key="test-key") as c2:
        # Use the SAME instance with the lock set
        c2._multi_price_locked = True
        prices2 = await c2.multi_price([SOL])
    assert prices2[SOL] == pytest.approx(146.0)
