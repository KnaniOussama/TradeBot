from datetime import UTC, date, datetime

import pytest

from tradebot.config.models import RiskConfig
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import (
    RiskManager,
    RiskState,
)


def _cfg(**overrides) -> RiskConfig:
    return RiskConfig(**overrides)


def test_size_scales_with_confidence():
    rm = RiskManager(_cfg())
    p = Portfolio(mode="demo", starting_cash=100.0)
    low = rm.size_for(confidence=0.6, available_cash=p.cash)
    high = rm.size_for(confidence=1.0, available_cash=p.cash)
    assert low < high
    # min sizing 30% of 100 = 30 at confidence 0.6
    assert low == pytest.approx(30.0, rel=1e-2)
    # max sizing 50% of 100 = 50 at confidence 1.0
    assert high == pytest.approx(50.0, rel=1e-2)


def test_evaluate_buy_accepts_basic():
    rm = RiskManager(_cfg())
    state = RiskState()
    p = Portfolio(mode="demo", starting_cash=100.0)
    decision = rm.evaluate_entry(
        pair="SOL/USDC",
        confidence=0.8,
        portfolio=p,
        state=state,
        slippage_pct=0.005,
        now=datetime(2026, 5, 3, tzinfo=UTC),
    )
    assert decision.allowed is True
    assert decision.size_quote > 0


def test_evaluate_buy_rejects_high_slippage():
    rm = RiskManager(_cfg())
    p = Portfolio(mode="demo", starting_cash=100.0)
    decision = rm.evaluate_entry(
        pair="X",
        confidence=0.9,
        portfolio=p,
        state=RiskState(),
        slippage_pct=0.02,
        now=datetime.now(UTC),
    )
    assert not decision.allowed
    assert "slippage" in decision.reason.lower()


def test_evaluate_buy_rejects_when_daily_trade_cap_hit():
    rm = RiskManager(_cfg(max_trades_per_day=3))
    p = Portfolio(mode="demo", starting_cash=100.0)
    today = date(2026, 5, 3)
    state = RiskState(trades_per_day={today: 3})
    decision = rm.evaluate_entry(
        pair="X",
        confidence=0.9,
        portfolio=p,
        state=state,
        slippage_pct=0.005,
        now=datetime(2026, 5, 3, 12, tzinfo=UTC),
    )
    assert not decision.allowed
    assert "trades per day" in decision.reason.lower()


def test_evaluate_buy_rejects_when_max_concurrent_positions():
    rm = RiskManager(_cfg(max_concurrent_positions=1))
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="A/USDC", side="buy", base_amount=1, quote_amount=10, fee_quote=0)
    decision = rm.evaluate_entry(
        pair="B/USDC",
        confidence=0.9,
        portfolio=p,
        state=RiskState(),
        slippage_pct=0.005,
        now=datetime.now(UTC),
    )
    assert not decision.allowed
    assert "concurrent" in decision.reason.lower()


def test_evaluate_buy_rejects_existing_position_for_same_pair():
    rm = RiskManager(_cfg())
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="A/USDC", side="buy", base_amount=1, quote_amount=10, fee_quote=0)
    decision = rm.evaluate_entry(
        pair="A/USDC",
        confidence=0.9,
        portfolio=p,
        state=RiskState(),
        slippage_pct=0.005,
        now=datetime.now(UTC),
    )
    assert not decision.allowed


def test_drawdown_circuit_trips_kill_switch():
    rm = RiskManager(_cfg(drawdown_circuit_pct=0.15))
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.equity_high = 100.0
    state = RiskState()
    rm.update_state(
        portfolio=p, state=state, current_equity=80.0, now=datetime(2026, 5, 3, tzinfo=UTC)
    )
    assert state.kill_switch_active is True
    assert "drawdown" in state.kill_switch_reason.lower()


def test_daily_loss_limit_pauses_until_next_day():
    rm = RiskManager(_cfg(daily_loss_limit_pct=0.08))
    p = Portfolio(mode="demo", starting_cash=100.0)
    state = RiskState(daily_start_equity={date(2026, 5, 3): 100.0})
    rm.update_state(
        portfolio=p, state=state, current_equity=91.0, now=datetime(2026, 5, 3, 12, tzinfo=UTC)
    )
    assert state.day_paused_until == date(2026, 5, 4)


def test_record_trade_increments_counter():
    rm = RiskManager(_cfg())
    state = RiskState()
    rm.record_trade(state=state, now=datetime(2026, 5, 3, tzinfo=UTC))
    assert state.trades_per_day[date(2026, 5, 3)] == 1


def test_per_trade_kill_returns_close_action():
    rm = RiskManager(_cfg(per_trade_kill_pct=0.03))
    decision = rm.check_per_trade_kill(unrealized_loss_pct=0.04)
    assert decision == "kill"
    decision = rm.check_per_trade_kill(unrealized_loss_pct=0.02)
    assert decision == "ok"
