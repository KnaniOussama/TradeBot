import json
import re
from datetime import UTC, datetime
from pathlib import Path

import pytest

from tradebot.core.portfolio import Portfolio
from tradebot.data.jupiter import JupiterClient
from tradebot.data.rpc import SolanaRpcClient
from tradebot.execution.base import ExecutionError, Order
from tradebot.execution.real import RealExecutor
from tradebot.storage.repo import JsonStorage
from tradebot.wallet.keypair import generate_bot_keypair

QUOTE = json.loads(Path("tests/fixtures/jupiter_quote_sol_usdc.json").read_text())
SWAP = json.loads(Path("tests/fixtures/jupiter_swap_response.json").read_text())
THIN = json.loads(Path("tests/fixtures/jupiter_quote_thin.json").read_text())


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


def _stub_sign_and_send(*, expected_sig: str = "STUBSIG"):
    async def _stub(*, serialized_tx_b64, keypair, rpc):
        return expected_sig

    return _stub


@pytest.mark.asyncio
async def test_real_buy_full_path(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*/quote\?.*"), json=QUOTE, is_reusable=True)
    httpx_mock.add_response(url=re.compile(r".*/swap$"), json=SWAP, is_reusable=True)
    p = Portfolio(mode="real", starting_cash=100.0, starting_sol_balance=1.0)
    kp = generate_bot_keypair()
    confirmed_calls: list[str] = []

    async def fake_confirm(sig, timeout_s=30.0, poll_interval_s=1.0):
        confirmed_calls.append(sig)
        return True

    async with (
        JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup,
        SolanaRpcClient(url="https://example.com") as rpc,
    ):
        rpc.confirm_signature = fake_confirm  # type: ignore[assignment]
        ex = RealExecutor(
            jupiter=jup,
            rpc=rpc,
            storage=storage,
            keypair=kp,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            max_slippage_pct=0.10,
            priority_fee_microlamports=0,
            confirmation_timeout_s=2.0,
            sign_and_send=_stub_sign_and_send(expected_sig="REALSIG"),
        )
        order = Order(pair="SOL/USDC", side="buy", size_quote=10.0)
        fill = await ex.execute(order=order, portfolio=p, now=datetime(2026, 5, 3, tzinfo=UTC))

    assert fill.tx_signature == "REALSIG"
    assert fill.side == "buy"
    assert fill.base_amount > 0
    assert p.cash < 100.0
    assert p.position_for("SOL/USDC") is not None
    assert "REALSIG" in confirmed_calls
    trades = storage.list_trades(mode="real", limit=10)
    assert len(trades) == 1
    assert trades[0].tx_signature == "REALSIG"


@pytest.mark.asyncio
async def test_real_rejects_high_slippage(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*/quote\?.*"), json=THIN, is_reusable=True)
    p = Portfolio(mode="real", starting_cash=100.0, starting_sol_balance=1.0)
    kp = generate_bot_keypair()
    async with (
        JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup,
        SolanaRpcClient(url="https://example.com") as rpc,
    ):
        ex = RealExecutor(
            jupiter=jup,
            rpc=rpc,
            storage=storage,
            keypair=kp,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            max_slippage_pct=0.01,
            priority_fee_microlamports=0,
            confirmation_timeout_s=2.0,
            sign_and_send=_stub_sign_and_send(),
        )
        with pytest.raises(ExecutionError, match="slippage"):
            await ex.execute(
                order=Order(pair="SOL/USDC", side="buy", size_quote=10.0),
                portfolio=p,
                now=datetime.now(UTC),
            )


@pytest.mark.asyncio
async def test_real_rejects_unknown_pair(storage, httpx_mock):
    p = Portfolio(mode="real", starting_cash=100.0, starting_sol_balance=1.0)
    kp = generate_bot_keypair()
    async with (
        JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup,
        SolanaRpcClient(url="https://example.com") as rpc,
    ):
        ex = RealExecutor(
            jupiter=jup,
            rpc=rpc,
            storage=storage,
            keypair=kp,
            base_mints={},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            max_slippage_pct=0.01,
            priority_fee_microlamports=0,
            confirmation_timeout_s=2.0,
            sign_and_send=_stub_sign_and_send(),
        )
        with pytest.raises(ExecutionError, match="unknown pair"):
            await ex.execute(
                order=Order(pair="X/USDC", side="buy", size_quote=10.0),
                portfolio=p,
                now=datetime.now(UTC),
            )
