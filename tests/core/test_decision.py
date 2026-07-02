from __future__ import annotations

from datetime import UTC, datetime

import pytest

from tradebot.config.models import RiskConfig
from tradebot.core.aggregator import AggregatedScore
from tradebot.core.decision import DecisionEngine, ExitReason, Observation
from tradebot.core.portfolio import Portfolio
from tradebot.core.regime import Regime
from tradebot.core.risk import RiskManager, RiskState
from tradebot.core.sizing import KellyStats


def _agg(pair: str, composite: float) -> AggregatedScore:
    return AggregatedScore(
        pair=pair,
        composite=composite,
        sampled_at=datetime.now(UTC),
        scores=[],
    )


def test_strong_signal_no_position_emits_enter():
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.7)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    assert len(actions) == 1
    assert actions[0].kind == "enter"
    assert actions[0].pair == "SOL/USDC"
    assert actions[0].size_quote > 0


def test_weak_signal_no_action():
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()), entry_threshold=0.6, exit_flip_threshold=-0.3
    )
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.4)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    assert all(a.kind != "enter" for a in actions)


def test_signal_flip_in_profit_emits_exit():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        # Disable TP ladder so signal-flip logic is exercised in isolation
        risk=RiskManager(RiskConfig(tp_ladder_fraction=0.0)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    # In profit: bought at 100, current 150
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", -0.5)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    exits = [a for a in actions if a.kind == "exit"]
    assert len(exits) == 1
    assert exits[0].reason == ExitReason.SIGNAL_FLIP


def test_trailing_stop_emits_exit():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        # Disable TP ladder so trailing-stop logic is exercised in isolation
        risk=RiskManager(RiskConfig(trailing_stop_pct=0.02, tp_ladder_fraction=0.0)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    state = RiskState()
    # Track a peak via marks
    engine.update_position_peaks(marks={"SOL/USDC": 110.0}, portfolio=p)
    # Now drops more than 2% from peak
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.4)],
        marks={"SOL/USDC": 107.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=state,
        now=datetime.now(UTC),
    )
    exits = [a for a in actions if a.kind == "exit"]
    assert len(exits) == 1
    assert exits[0].reason == ExitReason.TRAILING_STOP


def test_per_trade_kill_emits_exit():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(per_trade_kill_pct=0.03)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.4)],
        marks={"SOL/USDC": 95.0},  # 5% loss vs 100 entry
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    exits = [a for a in actions if a.kind == "exit" and a.reason == ExitReason.PER_TRADE_KILL]
    assert len(exits) == 1


def test_kill_switch_blocks_entries_but_allows_exits():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(per_trade_kill_pct=0.03)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    state = RiskState(kill_switch_active=True, kill_switch_reason="drawdown")
    actions, observations = engine.decide(
        scores=[_agg("OTHER/USDC", 0.9), _agg("SOL/USDC", 0.7)],
        marks={"SOL/USDC": 95.0, "OTHER/USDC": 1.0},
        slippages={"SOL/USDC": 0.005, "OTHER/USDC": 0.005},
        portfolio=p,
        state=state,
        now=datetime.now(UTC),
    )
    # Per-trade kill should still close the loser
    assert any(a.kind == "exit" for a in actions)
    assert all(a.kind != "enter" for a in actions)


def test_take_profit_ladder_triggers_partial_exit():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.2, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(tp_ladder_pct=0.02, tp_ladder_fraction=0.5)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.5)],
        marks={"SOL/USDC": 51.0},  # entry=50, +2%
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    tp_exits = [
        a for a in actions if a.kind == "exit" and a.reason == ExitReason.TAKE_PROFIT_LADDER
    ]
    assert len(tp_exits) == 1
    assert tp_exits[0].size_base == pytest.approx(0.1)  # 50% of 0.2


def test_take_profit_ladder_only_fires_once_per_position():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.2, quote_amount=10.0, fee_quote=0.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(tp_ladder_pct=0.02, tp_ladder_fraction=0.5)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    # First decide: TP fires
    engine.decide(
        scores=[_agg("SOL/USDC", 0.5)],
        marks={"SOL/USDC": 51.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    # Apply the partial exit ourselves (in-loop the executor would do this)
    p.apply_fill(pair="SOL/USDC", side="sell", base_amount=0.1, quote_amount=5.1, fee_quote=0.0)
    # Second decide at higher price: no second TP fires (already laddered out)
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.5)],
        marks={"SOL/USDC": 52.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    tp_exits = [a for a in actions if a.reason == ExitReason.TAKE_PROFIT_LADDER]
    assert len(tp_exits) == 0


# ---------------------------------------------------------------------------
# Regime filter tests (Phase 10)
# ---------------------------------------------------------------------------


def test_chop_regime_blocks_entry():
    """Entry must be skipped when regime is 'chop' and regime_block_chop=True."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(regime_filter_enabled=True, regime_block_chop=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    chop = Regime(label="chop", adx=10.0, ema_fast_above_slow=False)
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        regimes={"SOL/USDC": chop},
    )
    assert all(a.kind != "enter" for a in actions), "Entry should be blocked in chop regime"


def test_trending_up_regime_allows_entry():
    """Entry must NOT be blocked when regime is 'trending_up'."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(regime_filter_enabled=True, regime_block_chop=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    trend = Regime(label="trending_up", adx=30.0, ema_fast_above_slow=True)
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        regimes={"SOL/USDC": trend},
    )
    enters = [a for a in actions if a.kind == "enter"]
    assert len(enters) == 1


def test_regime_filter_disabled_allows_entry_in_chop():
    """When regime_filter_enabled=False, entries are not blocked even in chop."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(regime_filter_enabled=False, regime_block_chop=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    chop = Regime(label="chop", adx=10.0, ema_fast_above_slow=False)
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        regimes={"SOL/USDC": chop},
    )
    enters = [a for a in actions if a.kind == "enter"]
    assert len(enters) == 1


def test_missing_regime_allows_entry():
    """If regime is not available for a pair, entry should still be allowed."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(regime_filter_enabled=True, regime_block_chop=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    # No regime passed for this pair
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        regimes={},  # empty — no regime data
    )
    enters = [a for a in actions if a.kind == "enter"]
    assert len(enters) == 1


# ---------------------------------------------------------------------------
# Kelly sizing tests (Phase 10)
# ---------------------------------------------------------------------------


def test_kelly_sizing_uses_stats_when_sufficient_history():
    """With 25 round-trip stats, Kelly path should produce non-zero sized entry."""
    p = Portfolio(mode="demo", starting_cash=1000.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(use_kelly_sizing=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    # 15 wins of +5%, 10 losses of -2% → 25 total round trips
    returns = [0.05] * 15 + [-0.02] * 10
    from tradebot.core.sizing import compute_kelly_stats

    stats = compute_kelly_stats(returns)
    assert stats.n_round_trips == 25

    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        kelly_stats={"SOL/USDC": stats},
    )
    enters = [a for a in actions if a.kind == "enter"]
    assert len(enters) == 1
    assert enters[0].size_quote > 0


def test_kelly_sizing_fallback_on_no_history():
    """With no history (KellyStats.n=0), fallback to legacy linear sizing."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig(use_kelly_sizing=True)),
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    empty_stats = KellyStats(
        n_round_trips=0, win_rate=0.0, avg_win_pct=0.0, avg_loss_pct=0.0, kelly_fraction=0.0
    )
    actions, observations = engine.decide(
        scores=[_agg("SOL/USDC", 0.9)],
        marks={"SOL/USDC": 150.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        kelly_stats={"SOL/USDC": empty_stats},
    )
    enters = [a for a in actions if a.kind == "enter"]
    assert len(enters) == 1
    # Fallback linear: 0.30..0.50 * 100 cash
    assert 25.0 <= enters[0].size_quote <= 55.0


# ---------------------------------------------------------------------------
# Observation tests (Phase 10b)
# ---------------------------------------------------------------------------


def test_observation_emitted_for_blocked_chop():
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()), entry_threshold=0.6, exit_flip_threshold=-0.3
    )
    actions, obs = engine.decide(
        scores=[_agg("SOL/USDC", 0.8)],
        marks={"SOL/USDC": 100.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
        regimes={"SOL/USDC": Regime("chop", 15.0, False)},
    )
    assert actions == []
    assert len(obs) == 1
    assert obs[0].decision == "hold"
    assert "chop" in obs[0].reason.lower()


def test_observation_emitted_for_low_composite():
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()), entry_threshold=0.6, exit_flip_threshold=-0.3
    )
    actions, obs = engine.decide(
        scores=[_agg("SOL/USDC", 0.3)],
        marks={"SOL/USDC": 100.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    assert actions == []
    assert len(obs) == 1
    assert obs[0].decision == "hold"
    assert "composite" in obs[0].reason.lower() or "threshold" in obs[0].reason.lower()


def test_observation_emitted_for_buy():
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()), entry_threshold=0.6, exit_flip_threshold=-0.3
    )
    actions, obs = engine.decide(
        scores=[_agg("SOL/USDC", 0.8)],
        marks={"SOL/USDC": 100.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    assert len(actions) == 1
    assert len(obs) == 1
    assert obs[0].decision == "enter"
    assert obs[0].size_quote > 0


def test_observation_has_correct_fields():
    """Observation dataclass must have all required fields."""
    p = Portfolio(mode="demo", starting_cash=100.0)
    engine = DecisionEngine(
        risk=RiskManager(RiskConfig()), entry_threshold=0.6, exit_flip_threshold=-0.3
    )
    actions, obs = engine.decide(
        scores=[_agg("SOL/USDC", 0.8)],
        marks={"SOL/USDC": 100.0},
        slippages={"SOL/USDC": 0.005},
        portfolio=p,
        state=RiskState(),
        now=datetime.now(UTC),
    )
    assert len(obs) >= 1
    o = obs[0]
    assert isinstance(o, Observation)
    assert o.pair == "SOL/USDC"
    assert isinstance(o.timestamp, str)
    assert isinstance(o.composite, float)
    assert isinstance(o.mark, float)
    assert isinstance(o.decision, str)
    assert isinstance(o.reason, str)
    assert isinstance(o.size_quote, float)
    assert isinstance(o.size_base, float)
