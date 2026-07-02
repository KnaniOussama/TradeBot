import json
from pathlib import Path

import pytest

from tradebot.data.rpc import SolanaRpcClient
from tradebot.wallet.balance import get_sol_balance, get_token_balance

GET_BALANCE = json.loads(Path("tests/fixtures/rpc_get_balance.json").read_text())
GET_TOKEN_ACCOUNTS = json.loads(
    Path("tests/fixtures/rpc_get_token_accounts_by_owner.json").read_text()
)


@pytest.mark.asyncio
async def test_get_sol_balance(httpx_mock):
    httpx_mock.add_response(json=GET_BALANCE)
    async with SolanaRpcClient(url="https://example.com") as c:
        sol = await get_sol_balance(c, address="11111111111111111111111111111111")
    assert sol == pytest.approx(1.23456789, rel=1e-9)


@pytest.mark.asyncio
async def test_get_token_balance_existing_account(httpx_mock):
    httpx_mock.add_response(json=GET_TOKEN_ACCOUNTS)
    async with SolanaRpcClient(url="https://example.com") as c:
        bal = await get_token_balance(
            c,
            owner="11111111111111111111111111111111",
            mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        )
    assert bal == pytest.approx(1.5)


@pytest.mark.asyncio
async def test_get_token_balance_no_account_returns_zero(httpx_mock):
    empty = {"jsonrpc": "2.0", "result": {"context": {"slot": 0}, "value": []}, "id": 1}
    httpx_mock.add_response(json=empty)
    async with SolanaRpcClient(url="https://example.com") as c:
        bal = await get_token_balance(
            c,
            owner="11111111111111111111111111111111",
            mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        )
    assert bal == 0.0
