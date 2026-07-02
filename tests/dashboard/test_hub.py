import asyncio

import pytest

from tradebot.dashboard.hub import DashboardHub
from tradebot.dashboard.state import DashboardSnapshot


def _snap(mode: str = "demo", equity: float = 50.0) -> DashboardSnapshot:
    return DashboardSnapshot(
        mode=mode,
        now="2026-05-03T12:00:00+00:00",
        cash=equity,
        equity=equity,
        equity_high=equity,
        drawdown_pct=0.0,
        realized_pnl_total=0.0,
        sol_balance=0.05,
        sol_gas_paid_total=0.0,
        sol_mark=140.0,
        kill_switch_active=False,
        kill_switch_reason="",
    )


@pytest.mark.asyncio
async def test_hub_starts_with_no_snapshot():
    hub = DashboardHub()
    assert hub.latest() is None


@pytest.mark.asyncio
async def test_publish_updates_latest():
    hub = DashboardHub()
    await hub.publish(_snap())
    assert hub.latest() is not None
    assert hub.latest().mode == "demo"


@pytest.mark.asyncio
async def test_subscriber_receives_published_snapshot():
    hub = DashboardHub()
    sub = hub.subscribe()
    try:
        await hub.publish(_snap(equity=42.0))
        snap = await asyncio.wait_for(sub.get(), timeout=1.0)
        assert snap.equity == 42.0
    finally:
        hub.unsubscribe(sub)


@pytest.mark.asyncio
async def test_multiple_subscribers_each_receive():
    hub = DashboardHub()
    s1 = hub.subscribe()
    s2 = hub.subscribe()
    try:
        await hub.publish(_snap(equity=100.0))
        a = await asyncio.wait_for(s1.get(), timeout=1.0)
        b = await asyncio.wait_for(s2.get(), timeout=1.0)
        assert a.equity == b.equity == 100.0
    finally:
        hub.unsubscribe(s1)
        hub.unsubscribe(s2)


@pytest.mark.asyncio
async def test_full_subscriber_drops_oldest_silently():
    hub = DashboardHub(subscriber_queue_size=1)
    sub = hub.subscribe()
    try:
        await hub.publish(_snap(equity=1.0))
        await hub.publish(_snap(equity=2.0))
        # newest wins
        snap = await asyncio.wait_for(sub.get(), timeout=1.0)
        assert snap.equity == 2.0
    finally:
        hub.unsubscribe(sub)


@pytest.mark.asyncio
async def test_unsubscribe_removes_subscriber():
    hub = DashboardHub()
    sub = hub.subscribe()
    hub.unsubscribe(sub)
    assert sub not in hub._subscribers  # noqa: SLF001
