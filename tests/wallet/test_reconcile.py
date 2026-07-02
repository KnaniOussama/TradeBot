from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from tradebot.core.portfolio import Portfolio
from tradebot.wallet.reconcile import reconcile


@pytest.mark.asyncio
async def test_reconcile_matches_returns_no_findings():
    p = Portfolio(mode="real", starting_cash=0.0)
    # Wallet has 0.1 SOL, portfolio expects nothing — that's fine, unallocated funds OK
    rpc = AsyncMock()
    rpc.get_balance_sol = AsyncMock(return_value=0.05)
    findings = await reconcile(
        portfolio=p,
        rpc=rpc,
        bot_address="X",
        token_balance_fn=AsyncMock(return_value=0.0),
        base_mints={"SOL/USDC": ("So111", 9)},
    )
    assert findings == []


@pytest.mark.asyncio
async def test_reconcile_flags_missing_position():
    from tradebot.core.portfolio import _Position

    p = Portfolio(mode="real", starting_cash=10.0)
    p._positions["SOL/USDC"] = _Position(pair="SOL/USDC", base_amount=0.5, avg_entry_price=150.0)  # noqa: SLF001
    rpc = AsyncMock()
    rpc.get_balance_sol = AsyncMock(return_value=0.05)
    # Wallet actually only has 0.1 SOL of the 0.5 the portfolio expects
    findings = await reconcile(
        portfolio=p,
        rpc=rpc,
        bot_address="X",
        token_balance_fn=AsyncMock(return_value=0.1),
        base_mints={"SOL/USDC": ("So111", 9)},
    )
    assert len(findings) == 1
    f = findings[0]
    assert f.kind == "position_mismatch"
    assert "SOL/USDC" in f.message


@pytest.mark.asyncio
async def test_reconcile_flags_unexpected_balance():
    p = Portfolio(mode="real", starting_cash=10.0)  # no positions expected
    rpc = AsyncMock()
    rpc.get_balance_sol = AsyncMock(return_value=0.05)
    # Wallet has 1.0 SOL but portfolio thinks it has none
    findings = await reconcile(
        portfolio=p,
        rpc=rpc,
        bot_address="X",
        token_balance_fn=AsyncMock(return_value=1.0),
        base_mints={"SOL/USDC": ("So111", 9)},
    )
    assert any(f.kind == "unexpected_balance" for f in findings)
