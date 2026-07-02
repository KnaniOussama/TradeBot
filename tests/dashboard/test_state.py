from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from tradebot.core.aggregator import AggregatedScore
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskState
from tradebot.dashboard.state import DashboardSnapshot, build_snapshot
from tradebot.storage.repo import JsonStorage, Trade


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


@pytest.mark.asyncio
async def test_build_snapshot_minimal(storage):
    p = Portfolio(mode="demo", starting_cash=50.0)
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
        equity_history_limit=50,
    )
    assert isinstance(snap, DashboardSnapshot)
    assert snap.mode == "demo"
    assert snap.cash == 50.0
    assert snap.equity == 50.0
    assert snap.positions == []
    assert snap.recent_trades == []
    assert snap.equity_history == []


@pytest.mark.asyncio
async def test_build_snapshot_includes_positions(storage):
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={"SOL/USDC": 120.0},
        scores=[],
        now=datetime.now(UTC),
        equity_history_limit=50,
    )
    assert len(snap.positions) == 1
    pos = snap.positions[0]
    assert pos["pair"] == "SOL/USDC"
    assert pos["base_amount"] == pytest.approx(0.1)
    assert pos["avg_entry_price"] == pytest.approx(100.0)
    assert pos["mark_price"] == 120.0
    assert pos["unrealized_pnl_quote"] == pytest.approx(2.0)
    assert pos["unrealized_pnl_pct"] == pytest.approx(0.20)


@pytest.mark.asyncio
async def test_build_snapshot_includes_recent_trades(storage):
    p = Portfolio(mode="demo", starting_cash=100.0)
    storage.append_trade(
        Trade(
            mode="demo",
            pair="SOL/USDC",
            side="buy",
            base_amount=0.1,
            quote_amount=10.0,
            price=100.0,
            fee_quote=0.01,
            slippage_pct=0.001,
            tx_signature=None,
            opened_at=datetime(2026, 5, 3, 12, tzinfo=UTC),
            confidence=0.7,
            notes=None,
        ),
    )
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime.now(UTC),
        equity_history_limit=50,
    )
    assert len(snap.recent_trades) == 1
    t = snap.recent_trades[0]
    assert t["pair"] == "SOL/USDC"
    assert t["side"] == "buy"
    assert t["price"] == 100.0


@pytest.mark.asyncio
async def test_build_snapshot_equity_history(storage):
    p = Portfolio(mode="demo", starting_cash=50.0)
    base = datetime(2026, 5, 3, 12, tzinfo=UTC)
    for i in range(5):
        storage.append_equity_snapshot(
            mode="demo",
            snapshot_at=base + timedelta(minutes=i),
            equity=50.0 + i,
            cash=50.0 + i,
            positions_value=0.0,
        )
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime.now(UTC),
        equity_history_limit=10,
    )
    assert len(snap.equity_history) == 5
    assert snap.equity_history[0]["equity"] == 50.0
    assert snap.equity_history[-1]["equity"] == 54.0


@pytest.mark.asyncio
async def test_build_snapshot_signal_scores(storage):
    p = Portfolio(mode="demo", starting_cash=50.0)
    scores = [
        AggregatedScore(
            pair="SOL/USDC",
            composite=0.65,
            sampled_at=datetime.now(UTC),
            scores=[],
        ),
    ]
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={"SOL/USDC": 150.0},
        scores=scores,
        now=datetime.now(UTC),
        equity_history_limit=50,
    )
    assert len(snap.signals) == 1
    assert snap.signals[0]["pair"] == "SOL/USDC"
    assert snap.signals[0]["composite"] == 0.65


def test_snapshot_to_dict_roundtrips_via_json():
    import json

    snap = DashboardSnapshot(
        mode="demo",
        now="2026-05-03T12:00:00+00:00",
        cash=50.0,
        equity=50.0,
        equity_high=50.0,
        drawdown_pct=0.0,
        realized_pnl_total=0.0,
        sol_balance=0.05,
        sol_gas_paid_total=0.0,
        sol_mark=140.0,
        kill_switch_active=False,
        kill_switch_reason="",
        positions=[],
        recent_trades=[],
        equity_history=[],
        signals=[],
    )
    payload = snap.to_dict()
    s = json.dumps(payload)
    assert "demo" in s


# ---------------------------------------------------------------------------
# Phase 10b: decisions field in snapshot
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_build_snapshot_includes_decisions(storage):
    """When observations are passed, snapshot.decisions is populated newest-first."""
    from tradebot.core.decision import Observation

    p = Portfolio(mode="demo", starting_cash=50.0)
    obs = [
        Observation(
            timestamp="2026-05-03T12:00:00+00:00",
            pair="SOL/USDC",
            composite=0.8,
            mark=150.0,
            regime="trending_up",
            decision="enter",
            reason="composite 0.800 >= threshold 0.600",
            size_quote=30.0,
        ),
        Observation(
            timestamp="2026-05-03T12:01:00+00:00",
            pair="SOL/USDC",
            composite=0.3,
            mark=149.0,
            regime="chop",
            decision="hold",
            reason="composite 0.300 below threshold 0.600",
        ),
    ]
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
        observations=obs,
    )
    assert hasattr(snap, "decisions")
    assert len(snap.decisions) == 2
    # newest first — second obs should be first in decisions list
    assert snap.decisions[0]["timestamp"] == "2026-05-03T12:01:00+00:00"
    assert snap.decisions[1]["timestamp"] == "2026-05-03T12:00:00+00:00"


@pytest.mark.asyncio
async def test_build_snapshot_decisions_empty_by_default(storage):
    """When no observations are passed, snapshot.decisions defaults to []."""
    p = Portfolio(mode="demo", starting_cash=50.0)
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
    )
    assert snap.decisions == []


@pytest.mark.asyncio
async def test_build_snapshot_decisions_capped_at_100(storage):
    """Decisions list in snapshot is capped at most recent 100 entries."""
    from tradebot.core.decision import Observation

    p = Portfolio(mode="demo", starting_cash=50.0)
    obs = [
        Observation(
            timestamp=f"2026-05-03T12:{i:02d}:00+00:00",
            pair="SOL/USDC",
            composite=0.3,
            mark=100.0,
            regime=None,
            decision="hold",
            reason="below threshold",
        )
        for i in range(150)
    ]
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
        observations=obs,
    )
    # Only the last 100 observations should appear (newest first)
    assert len(snap.decisions) == 100
    # The last item in obs list (index 149) should be first in decisions (newest first)
    assert snap.decisions[0]["timestamp"] == "2026-05-03T12:149:00+00:00"


@pytest.mark.asyncio
async def test_build_snapshot_limiter_metrics_populated(storage):
    """When limiter_metrics is passed, snap.limiter is set."""
    p = Portfolio(mode="demo", starting_cash=50.0)
    metrics = {
        "rate_limit_rps": 0.9,
        "burst": 5,
        "current_tokens": 4.5,
        "total_acquired": 10,
        "throttle_wait_total_s": 0.05,
        "recent_rps": 0.3,
        "total_429s": 0,
        "last_429_at": None,
    }
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
        limiter_metrics=metrics,
    )
    assert snap.limiter is not None
    assert snap.limiter["rate_limit_rps"] == 0.9
    assert snap.limiter["total_acquired"] == 10


@pytest.mark.asyncio
async def test_build_snapshot_limiter_none_by_default(storage):
    """When limiter_metrics is not passed, snap.limiter is None."""
    p = Portfolio(mode="demo", starting_cash=50.0)
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
    )
    assert snap.limiter is None


@pytest.mark.asyncio
async def test_build_snapshot_decisions_dict_has_all_fields(storage):
    """Each decision dict has all required keys."""
    from tradebot.core.decision import Observation

    p = Portfolio(mode="demo", starting_cash=50.0)
    obs = [
        Observation(
            timestamp="2026-05-03T12:00:00+00:00",
            pair="SOL/USDC",
            composite=0.7,
            mark=100.0,
            regime="neutral",
            decision="enter",
            reason="entry triggered",
            size_quote=25.0,
            size_base=0.25,
        )
    ]
    snap = await build_snapshot(
        storage=storage,
        portfolio=p,
        risk_state=RiskState(),
        marks={},
        scores=[],
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
        observations=obs,
    )
    d = snap.decisions[0]
    required_keys = (
        "timestamp",
        "pair",
        "composite",
        "mark",
        "regime",
        "decision",
        "reason",
        "size_quote",
        "size_base",
    )
    for key in required_keys:
        assert key in d, f"Missing key: {key}"
    assert d["pair"] == "SOL/USDC"
    assert d["decision"] == "enter"
    assert d["size_quote"] == 25.0


# --- trade lineage / badge annotation ---


def test_annotate_open_then_full_close_tags_with_close_badge_and_pnl():
    from datetime import UTC, datetime

    from tradebot.dashboard.state import _annotate_trades
    from tradebot.storage.repo import Trade

    t1 = Trade(mode="demo", pair="SOL/USDC", side="buy",
               base_amount=1.0, quote_amount=100.0, price=100.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 0, tzinfo=UTC),
               confidence=None, notes=None)
    t2 = Trade(mode="demo", pair="SOL/USDC", side="sell",
               base_amount=1.0, quote_amount=110.0, price=110.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 5, tzinfo=UTC),
               confidence=None, notes=None)
    annotated, open_lineages = _annotate_trades([t1, t2])
    assert annotated[0]["badge"] == "OPEN"
    assert annotated[0]["realized_pnl"] is None
    assert annotated[1]["badge"].startswith("CLOSE +$10")
    assert annotated[1]["realized_pnl"] == 10.0
    # Position fully closed → no open lineage.
    assert "SOL/USDC" not in open_lineages


def test_annotate_open_then_partial_sell_tags_trim_and_keeps_lineage():
    from datetime import UTC, datetime

    from tradebot.dashboard.state import _annotate_trades
    from tradebot.storage.repo import Trade

    t1 = Trade(mode="demo", pair="SOL/USDC", side="buy",
               base_amount=2.0, quote_amount=200.0, price=100.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 0, tzinfo=UTC),
               confidence=None, notes=None)
    t2 = Trade(mode="demo", pair="SOL/USDC", side="sell",
               base_amount=1.0, quote_amount=110.0, price=110.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 5, tzinfo=UTC),
               confidence=None, notes=None)
    annotated, open_lineages = _annotate_trades([t1, t2])
    assert annotated[1]["badge"] == "TRIM 50%"
    assert annotated[1]["realized_pnl"] == 10.0  # 110 received - 100 cost (1 base * 100/unit)
    # Position still has 1.0 base open with full lineage attached.
    assert "SOL/USDC" in open_lineages
    assert len(open_lineages["SOL/USDC"]) == 2


def test_annotate_dca_buy_after_open_tags_add():
    from datetime import UTC, datetime

    from tradebot.dashboard.state import _annotate_trades
    from tradebot.storage.repo import Trade

    t1 = Trade(mode="demo", pair="SOL/USDC", side="buy",
               base_amount=1.0, quote_amount=100.0, price=100.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 0, tzinfo=UTC),
               confidence=None, notes=None)
    t2 = Trade(mode="demo", pair="SOL/USDC", side="buy",
               base_amount=1.0, quote_amount=120.0, price=120.0,
               fee_quote=0.0, slippage_pct=0.0, tx_signature=None,
               opened_at=datetime(2026, 5, 3, 10, 5, tzinfo=UTC),
               confidence=None, notes=None)
    annotated, _ = _annotate_trades([t1, t2])
    assert annotated[0]["badge"] == "OPEN"
    assert annotated[1]["badge"] == "ADD"
