from __future__ import annotations

from datetime import UTC, datetime

import pytest

from tradebot.backtest.executor import SyntheticExecutor
from tradebot.core.portfolio import Portfolio
from tradebot.execution.base import Order


def _now() -> datetime:
    return datetime(2026, 5, 1, 0, 0, 0, tzinfo=UTC)


def _portfolio(cash: float = 100.0) -> Portfolio:
    return Portfolio(mode="backtest", starting_cash=cash)


@pytest.mark.asyncio
async def test_buy_reduces_cash_and_creates_position():
    exec_ = SyntheticExecutor(fee_bps=30, slippage_bps=5)
    port = _portfolio(cash=100.0)
    order = Order(pair="SOL/USDC", side="buy", size_quote=50.0)
    mark = 100.0

    fill = await exec_.execute(order=order, portfolio=port, now=_now(), mark=mark)

    # slippage pushes price up: 100 * (1 + 5/10000) = 100.05
    eff_price = 100.05
    base_amount = 50.0 / eff_price
    fee_quote = 50.0 * (30 / 10_000)

    assert fill.side == "buy"
    assert abs(fill.price - eff_price) < 1e-9
    assert abs(fill.base_amount - base_amount) < 1e-9
    assert abs(fill.fee_quote - fee_quote) < 1e-9
    # cash should be reduced by quote_amount + fee
    assert abs(port.cash - (100.0 - 50.0 - fee_quote)) < 1e-6
    assert port.position_for("SOL/USDC") is not None


@pytest.mark.asyncio
async def test_sell_realizes_pnl():
    exec_ = SyntheticExecutor(fee_bps=30, slippage_bps=5)
    port = _portfolio(cash=100.0)

    # Buy first
    buy_order = Order(pair="SOL/USDC", side="buy", size_quote=50.0)
    buy_fill = await exec_.execute(order=buy_order, portfolio=port, now=_now(), mark=100.0)

    # Sell at higher price -> profit
    sell_order = Order(pair="SOL/USDC", side="sell", size_base=buy_fill.base_amount)
    sell_fill = await exec_.execute(order=sell_order, portfolio=port, now=_now(), mark=110.0)

    assert sell_fill.side == "sell"
    # effective sell price is 110 * (1 - 5/10000) = 109.945
    eff_sell = 110.0 * (1 - 5 / 10_000)
    assert abs(sell_fill.price - eff_sell) < 1e-6
    assert port.position_for("SOL/USDC") is None
    # portfolio has realized pnl
    assert port.realized_pnl_total > 0


@pytest.mark.asyncio
async def test_fee_and_slippage_applied():
    exec_ = SyntheticExecutor(fee_bps=100, slippage_bps=50)
    port = _portfolio(cash=1000.0)
    order = Order(pair="SOL/USDC", side="buy", size_quote=100.0)
    fill = await exec_.execute(order=order, portfolio=port, now=_now(), mark=200.0)

    # slippage 50 bps -> effective price = 200 * 1.005 = 201.0
    assert abs(fill.price - 201.0) < 1e-6
    # fee = 100 * (100/10000) = 1.0
    assert abs(fill.fee_quote - 1.0) < 1e-6
    assert fill.slippage_pct == 50 / 10_000
