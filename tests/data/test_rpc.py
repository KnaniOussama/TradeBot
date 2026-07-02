import json
from pathlib import Path

import pytest

from tradebot.data.rpc import SolanaRpcClient

FIXTURE = Path("tests/fixtures/rpc_get_balance.json")
_PAYLOAD = json.loads(FIXTURE.read_text())


@pytest.mark.asyncio
async def test_get_balance_lamports(httpx_mock):
    httpx_mock.add_response(json=_PAYLOAD)
    async with SolanaRpcClient(url="https://example.com/rpc") as c:
        lamports = await c.get_balance_lamports("11111111111111111111111111111111")
    assert lamports == 1234567890


@pytest.mark.asyncio
async def test_get_balance_sol(httpx_mock):
    httpx_mock.add_response(json=_PAYLOAD)
    async with SolanaRpcClient(url="https://example.com/rpc") as c:
        sol = await c.get_balance_sol("11111111111111111111111111111111")
    assert sol == pytest.approx(1.23456789, rel=1e-9)


@pytest.mark.asyncio
async def test_rpc_error_raises(httpx_mock):
    httpx_mock.add_response(
        json={
            "jsonrpc": "2.0",
            "error": {"code": -32601, "message": "Method not found"},
            "id": 1,
        }
    )
    async with SolanaRpcClient(url="https://example.com/rpc") as c:
        with pytest.raises(RuntimeError, match="Method not found"):
            await c.get_balance_lamports("11111111111111111111111111111111")
