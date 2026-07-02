from __future__ import annotations

import json
import re
from pathlib import Path

import httpx
import pytest

from tradebot.data.jupiter import JupiterClient, JupiterQuote

FIXTURE = Path("tests/fixtures/jupiter_quote_sol_usdc.json")
_PAYLOAD = json.loads(FIXTURE.read_text())


@pytest.mark.asyncio
async def test_quote_parses_response(httpx_mock):
    payload = _PAYLOAD
    httpx_mock.add_response(
        url=re.compile(r"https://quote-api\.jup\.ag/v6/quote\?.*"),
        json=payload,
    )
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as c:
        q = await c.quote(
            input_mint="So11111111111111111111111111111111111111112",
            output_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            amount=100_000_000,
            slippage_bps=50,
        )
    assert isinstance(q, JupiterQuote)
    assert q.in_amount == 100_000_000
    assert q.out_amount == 15_050_000
    assert q.price_impact_pct == 0.0012
    assert q.slippage_bps == 50
    assert q.route_labels == ["Raydium"]


@pytest.mark.asyncio
async def test_quote_implied_price_helper(httpx_mock):
    payload = _PAYLOAD
    httpx_mock.add_response(url=re.compile(r".*"), json=payload)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as c:
        q = await c.quote(
            input_mint="So11111111111111111111111111111111111111112",
            output_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            amount=100_000_000,
            slippage_bps=50,
        )
    # 0.1 SOL (9 decimals) -> 15.05 USDC (6 decimals) => price 150.5 USDC/SOL
    price = q.implied_price(in_decimals=9, out_decimals=6)
    assert price == pytest.approx(150.5, rel=1e-6)


@pytest.mark.asyncio
async def test_quote_http_error_raises(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*"), status_code=500, json={"error": "boom"})
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as c:
        with pytest.raises(httpx.HTTPStatusError):
            await c.quote(
                input_mint="A" * 43,
                output_mint="B" * 43,
                amount=1,
                slippage_bps=50,
            )


@pytest.mark.asyncio
async def test_quote_retries_on_429_then_succeeds(httpx_mock):
    payload = json.loads(FIXTURE.read_text())
    # First response: 429 with Retry-After: 0
    httpx_mock.add_response(
        url=re.compile(r".*quote.*"),
        status_code=429,
        headers={"Retry-After": "0"},
    )
    # Second response: success
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=payload)
    async with JupiterClient(base_url="https://lite-api.jup.ag/swap/v1", max_429_retries=2) as c:
        q = await c.quote(
            input_mint="A" * 43,
            output_mint="B" * 43,
            amount=1_000_000,
            slippage_bps=50,
        )
    assert q.in_amount == 100_000_000  # came from successful retry


@pytest.mark.asyncio
async def test_quote_gives_up_after_max_retries(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), status_code=429, is_reusable=True)
    async with JupiterClient(base_url="https://lite-api.jup.ag/swap/v1", max_429_retries=2) as c:
        with pytest.raises(httpx.HTTPStatusError):
            await c.quote(
                input_mint="A" * 43,
                output_mint="B" * 43,
                amount=1_000_000,
                slippage_bps=50,
            )


@pytest.mark.asyncio
async def test_quote_with_limiter_acquires_token(httpx_mock):
    payload = json.loads(FIXTURE.read_text())
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=payload)
    from tradebot.data.rate_limiter import TokenBucketLimiter

    lim = TokenBucketLimiter(rate_per_sec=10.0, burst=1)
    async with JupiterClient(base_url="https://lite-api.jup.ag/swap/v1", limiter=lim) as c:
        await c.quote(
            input_mint="A" * 43,
            output_mint="B" * 43,
            amount=1_000_000,
            slippage_bps=50,
        )
    assert lim.metrics()["total_acquired"] == 1
