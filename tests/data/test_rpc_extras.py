import json
from pathlib import Path

import pytest

from tradebot.data.rpc import ConfirmationError, SolanaRpcClient

SEND = json.loads(Path("tests/fixtures/rpc_send_tx.json").read_text())
SIG = json.loads(Path("tests/fixtures/rpc_get_signature_statuses.json").read_text())
BLOCKHASH = json.loads(Path("tests/fixtures/rpc_get_latest_blockhash.json").read_text())


@pytest.mark.asyncio
async def test_send_raw_transaction_returns_signature(httpx_mock):
    httpx_mock.add_response(json=SEND)
    async with SolanaRpcClient(url="https://example.com") as c:
        sig = await c.send_raw_transaction("AQABAgM=", skip_preflight=False)
    assert sig.startswith("5J7q9X3z2Y8w4")


@pytest.mark.asyncio
async def test_get_latest_blockhash(httpx_mock):
    httpx_mock.add_response(json=BLOCKHASH)
    async with SolanaRpcClient(url="https://example.com") as c:
        bh = await c.get_latest_blockhash()
    assert bh.blockhash.startswith("5J7q9X3z2Y8w4")
    assert bh.last_valid_block_height == 350001010


@pytest.mark.asyncio
async def test_confirm_signature_success(httpx_mock):
    httpx_mock.add_response(json=SIG, is_reusable=True)
    async with SolanaRpcClient(url="https://example.com") as c:
        ok = await c.confirm_signature("sig123", timeout_s=2.0, poll_interval_s=0.05)
    assert ok is True


@pytest.mark.asyncio
async def test_confirm_signature_fails_on_err(httpx_mock):
    fail_payload = json.loads(json.dumps(SIG))
    fail_payload["result"]["value"][0]["err"] = {"InstructionError": [0, "Custom"]}
    httpx_mock.add_response(json=fail_payload, is_reusable=True)
    async with SolanaRpcClient(url="https://example.com") as c:
        with pytest.raises(ConfirmationError):
            await c.confirm_signature("sig123", timeout_s=2.0, poll_interval_s=0.05)


@pytest.mark.asyncio
async def test_confirm_signature_times_out(httpx_mock):
    pending = {"jsonrpc": "2.0", "result": {"context": {"slot": 1}, "value": [None]}, "id": 1}
    httpx_mock.add_response(json=pending, is_reusable=True)
    async with SolanaRpcClient(url="https://example.com") as c:
        with pytest.raises(ConfirmationError):
            await c.confirm_signature("sig123", timeout_s=0.2, poll_interval_s=0.05)
