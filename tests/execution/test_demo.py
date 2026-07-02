import json
import re
from datetime import UTC, datetime
from pathlib import Path

import pytest

from tradebot.core.portfolio import Portfolio
from tradebot.data.jupiter import JupiterClient
from tradebot.execution.base import ExecutionError, Fill, Order
from tradebot.execution.demo import DemoExecutor
from tradebot.storage.repo import JsonStorage

QUOTE = json.loads(Path("tests/fixtures/jupiter_quote_sol_usdc.json").read_text())


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


@pytest.mark.asyncio
async def test_demo_buy_simulates_fill(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
        )
        order = Order(pair="SOL/USDC", side="buy", size_quote=10.0)
        fill = await ex.execute(order=order, portfolio=p, now=datetime(2026, 5, 3, tzinfo=UTC))
    assert isinstance(fill, Fill)
    assert fill.pair == "SOL/USDC"
    assert fill.side == "buy"
    assert fill.base_amount > 0
    assert fill.quote_amount == pytest.approx(10.0)
    # Portfolio updated
    assert p.cash < 100.0
    pos = p.position_for("SOL/USDC")
    assert pos is not None


@pytest.mark.asyncio
async def test_demo_sell_returns_quote_to_cash(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
        )
        order = Order(pair="SOL/USDC", side="sell", size_base=0.1)
        fill = await ex.execute(order=order, portfolio=p, now=datetime(2026, 5, 3, tzinfo=UTC))
    assert fill.side == "sell"
    assert fill.quote_amount > 0
    assert p.position_for("SOL/USDC") is None


@pytest.mark.asyncio
async def test_demo_persists_trade(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
        )
        await ex.execute(
            order=Order(pair="SOL/USDC", side="buy", size_quote=10.0),
            portfolio=p,
            now=datetime(2026, 5, 3, tzinfo=UTC),
        )
    assert len(storage.list_trades(mode="demo", limit=10)) == 1


@pytest.mark.asyncio
async def test_demo_unknown_pair_raises(storage, httpx_mock):
    httpx_mock.add_response(
        url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True, is_optional=True
    )
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
        )
        with pytest.raises(ExecutionError):
            await ex.execute(
                order=Order(pair="UNKNOWN/USDC", side="buy", size_quote=10.0),
                portfolio=p,
                now=datetime.now(UTC),
            )


@pytest.mark.asyncio
async def test_demo_charges_sol_gas_on_fill(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=0.01)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
            priority_fee_microlamports=0,
        )
        await ex.execute(
            order=Order(pair="SOL/USDC", side="buy", size_quote=10.0),
            portfolio=p,
            now=datetime.now(UTC),
        )
    # Base fee 5000 lamports = 5e-6 SOL deducted exactly once
    assert p.sol_balance == pytest.approx(0.01 - 5e-6)
    assert p.sol_gas_paid_total == pytest.approx(5e-6)


@pytest.mark.asyncio
async def test_demo_rejects_when_sol_balance_zero(storage, httpx_mock):
    from tradebot.core.portfolio import PortfolioError

    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=0.0)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            confirm_latency_s=0.0,
        )
        with pytest.raises(PortfolioError, match="insufficient SOL"):
            await ex.execute(
                order=Order(pair="SOL/USDC", side="buy", size_quote=10.0),
                portfolio=p,
                now=datetime.now(UTC),
            )


@pytest.mark.asyncio
async def test_demo_rejects_when_drift_exceeds_max_slippage(storage, httpx_mock):
    """Trigger quote: out=15_050_000. Fill quote: out=14_500_000 (~3.6% worse)."""
    import copy

    drifted = copy.deepcopy(QUOTE)
    drifted["outAmount"] = "14500000"  # >3% drift vs trigger 15050000
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE)        # trigger
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=drifted)      # fill

    p = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=0.01)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            max_slippage_pct=0.01,  # 1% — drift is ~3.6%, should fail
            confirm_latency_s=0.0,
        )
        with pytest.raises(ExecutionError, match="drift"):
            await ex.execute(
                order=Order(pair="SOL/USDC", side="buy", size_quote=10.0),
                portfolio=p,
                now=datetime.now(UTC),
            )
    # No state mutation when fill rejected
    assert p.cash == 100.0
    assert p.sol_balance == 0.01
