import json
import re
from datetime import UTC, datetime
from pathlib import Path

import pytest

from tradebot.config.models import RiskConfig
from tradebot.core.aggregator import SignalAggregator
from tradebot.core.decision import DecisionEngine
from tradebot.core.loop import TradingLoop
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskManager, RiskState
from tradebot.data.jupiter import JupiterClient
from tradebot.execution.demo import DemoExecutor
from tradebot.signals.base import MarketContext, SignalScore
from tradebot.storage.repo import JsonStorage

QUOTE = json.loads(Path("tests/fixtures/jupiter_quote_sol_usdc.json").read_text())


class _BullSignal:
    name = "ta"
    timeframe = "1m"

    async def score(self, ctx: MarketContext) -> SignalScore:
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=0.9,
            sampled_at=ctx.now,
            components={},
        )


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


@pytest.mark.asyncio
async def test_one_cycle_enters_position(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(
        signals=[_BullSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
    )
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))
    assert portfolio.position_for("SOL/USDC") is not None
    assert len(storage.list_trades(mode="demo", limit=100)) == 1


@pytest.mark.asyncio
async def test_one_cycle_writes_equity_snapshot(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(signals=[], timeframe_weights={"1m": 1.0}, signal_weights={})
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))
    assert len(storage.list_equity_snapshots(mode="demo", limit=10)) == 1


@pytest.mark.asyncio
async def test_one_cycle_publishes_to_hub(storage, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    from tradebot.dashboard.hub import DashboardHub

    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(signals=[], timeframe_weights={"1m": 1.0}, signal_weights={})
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    hub = DashboardHub()
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            hub=hub,
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))
    snap = hub.latest()
    assert snap is not None
    assert snap.mode == "demo"
    assert snap.equity > 0


@pytest.mark.asyncio
async def test_one_cycle_persists_portfolio_risk_and_mark_history(tmp_path, httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    from tradebot.storage.repo import JsonStorage

    storage = JsonStorage(root=tmp_path / "data")
    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(signals=[], timeframe_weights={"1m": 1.0}, signal_weights={})
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    async with JupiterClient(base_url="https://lite-api.jup.ag/swap/v1") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))

    saved_portfolio = storage.load_portfolio_state(mode="demo")
    assert saved_portfolio is not None
    assert saved_portfolio.cash == pytest.approx(100.0)
    saved_history = storage.load_mark_history(mode="demo")
    assert "SOL/USDC" in saved_history
    assert len(saved_history["SOL/USDC"]) == 1


# ---------------------------------------------------------------------------
# Phase 10b: observations buffered + published in snapshot
# ---------------------------------------------------------------------------


class _StubSignal:
    """Simple stub that always returns a high score so engine emits an enter obs."""

    name = "ta"
    timeframe = "1m"

    async def score(self, ctx: MarketContext) -> SignalScore:
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=0.9,
            sampled_at=ctx.now,
            components={},
        )


@pytest.mark.asyncio
async def test_one_cycle_buffers_observations(tmp_path, httpx_mock):
    """After run_one_cycle, loop._observations has at least one entry for the pair."""
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    storage = JsonStorage(root=tmp_path / "data")
    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(
        signals=[_StubSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
    )
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))
    assert len(loop._observations) >= 1


@pytest.mark.asyncio
async def test_one_cycle_publishes_decisions_in_snapshot(tmp_path, httpx_mock):
    """After run_one_cycle with hub, snap.decisions is non-empty."""
    httpx_mock.add_response(url=re.compile(r".*quote.*"), json=QUOTE, is_reusable=True)
    from tradebot.dashboard.hub import DashboardHub

    storage = JsonStorage(root=tmp_path / "data2")
    portfolio = Portfolio(mode="demo", starting_cash=100.0, starting_sol_balance=1.0)
    risk = RiskManager(RiskConfig())
    state = RiskState()
    aggregator = SignalAggregator(
        signals=[_StubSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
    )
    engine = DecisionEngine(risk=risk, entry_threshold=0.6, exit_flip_threshold=-0.3)
    hub = DashboardHub()
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as jup:
        ex = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            simulated_fee_bps=10,
        )
        loop = TradingLoop(
            storage=storage,
            portfolio=portfolio,
            aggregator=aggregator,
            engine=engine,
            risk=risk,
            state=state,
            executor=ex,
            jupiter=jup,
            pairs=["SOL/USDC"],
            timeframes=["1m"],
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            quote_decimals=6,
            base_mints={"SOL/USDC": ("So11111111111111111111111111111111111111112", 9)},
            hub=hub,
        )
        await loop.run_one_cycle(now=datetime(2026, 5, 3, 12, tzinfo=UTC))
    snap = hub.latest()
    assert snap is not None
    assert hasattr(snap, "decisions")
    assert len(snap.decisions) >= 1
