import json
import re
from pathlib import Path

import pytest

from tradebot.data.helius import HeliusClient, TokenTransfer

FIXTURE = json.loads(Path("tests/fixtures/helius_enhanced_txs.json").read_text())


@pytest.mark.asyncio
async def test_get_recent_token_transfers_parses(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*helius.*"), json=FIXTURE)
    async with HeliusClient(api_key="test", base_url="https://api.helius.example") as c:
        transfers = await c.recent_token_transfers(
            address="DEX1",
            limit=100,
        )
    assert len(transfers) == 3
    t0 = transfers[0]
    assert isinstance(t0, TokenTransfer)
    assert t0.amount == 5000.0
    assert t0.mint == "So11111111111111111111111111111111111111112"


@pytest.mark.asyncio
async def test_helius_http_error(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*helius.*"), status_code=500)
    async with HeliusClient(api_key="test", base_url="https://api.helius.example") as c:
        with pytest.raises(Exception):  # noqa: B017
            await c.recent_token_transfers(address="X", limit=10)


@pytest.mark.asyncio
async def test_filter_transfers_for_mint():
    transfers = [
        TokenTransfer(
            signature="a", timestamp=1, mint="MINT_A", from_addr="X", to_addr="Y", amount=10.0
        ),  # noqa: E501
        TokenTransfer(
            signature="b", timestamp=2, mint="MINT_B", from_addr="X", to_addr="Y", amount=20.0
        ),  # noqa: E501
    ]
    from tradebot.data.helius import filter_for_mint

    only_a = filter_for_mint(transfers, "MINT_A")
    assert len(only_a) == 1
    assert only_a[0].mint == "MINT_A"
