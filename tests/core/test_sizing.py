from __future__ import annotations

import pytest

from tradebot.core.sizing import KellyStats, compute_kelly_stats, kelly_size


def test_compute_kelly_stats_basic():
    """6 wins of +5%, 4 losses of -2% → win_rate=0.6, b=2.5, kelly=0.44."""
    returns = [0.05] * 6 + [-0.02] * 4
    s = compute_kelly_stats(returns)
    assert s.n_round_trips == 10
    assert s.win_rate == pytest.approx(0.6)
    # b = 0.05/0.02 = 2.5; kelly = (0.6*2.5 - 0.4)/2.5 = (1.5-0.4)/2.5 = 1.1/2.5 = 0.44
    assert s.kelly_fraction == pytest.approx(0.44, abs=1e-3)


def test_compute_kelly_stats_empty():
    s = compute_kelly_stats([])
    assert s.n_round_trips == 0
    assert s.kelly_fraction == 0.0
    assert s.win_rate == 0.0


def test_compute_kelly_stats_all_wins():
    """No losses → conservative, kelly_fraction=0.0."""
    returns = [0.05] * 5
    s = compute_kelly_stats(returns)
    assert s.n_round_trips == 5
    assert s.win_rate == 1.0
    assert s.kelly_fraction == 0.0  # conservative when no losses


def test_compute_kelly_stats_all_losses():
    """All losses → kelly_fraction=0 (negative kelly clamped)."""
    returns = [-0.02] * 5
    s = compute_kelly_stats(returns)
    assert s.n_round_trips == 5
    assert s.win_rate == 0.0
    assert s.kelly_fraction == 0.0


def test_kelly_size_falls_back_on_insufficient_history():
    """When n_round_trips < min_round_trips, use linear sizing fallback."""
    s = KellyStats(
        n_round_trips=0, win_rate=0.0, avg_win_pct=0.0, avg_loss_pct=0.0, kelly_fraction=0.0
    )
    # confidence=0.6 → frac = 0.30 + 0.6*(0.50-0.30) = 0.30 + 0.12 = 0.42 → 42.0
    size = kelly_size(
        available_cash=100.0, confidence=0.6, stats=s, fallback_min=0.30, fallback_max=0.50
    )
    assert size == pytest.approx(42.0, abs=0.01)


def test_kelly_size_uses_kelly_when_sufficient_history():
    """With 20+ trades and positive kelly, size should be > 0."""
    returns = [0.05] * 15 + [-0.02] * 10  # 25 trades
    s = compute_kelly_stats(returns)
    assert s.n_round_trips == 25
    size = kelly_size(available_cash=1000.0, confidence=1.0, stats=s)
    # kelly_fraction ≈ 0.58, half-kelly = 0.29, capped at max_fraction=0.5 → 290 or capped
    assert size > 0
    assert size <= 1000.0 * 0.5  # never exceeds max_fraction


def test_kelly_size_confidence_zero_returns_min_fraction():
    """confidence=0 → smallest possible kelly-scaled size (min_fraction)."""
    returns = [0.05] * 15 + [-0.02] * 10  # 25 trades, positive kelly
    s = compute_kelly_stats(returns)
    size = kelly_size(
        available_cash=1000.0,
        confidence=0.0,
        stats=s,
        min_fraction=0.05,
        max_fraction=0.5,
    )
    # confidence=0 → confidence_scaled=0, clamped to min_fraction=0.05 → 50
    assert size == pytest.approx(1000.0 * 0.05)


def test_kelly_size_clamps_at_max_fraction():
    """Even a huge kelly fraction stays within max_fraction."""
    # Absurdly good stats: 100% win rate equivalent through large kelly
    s = KellyStats(
        n_round_trips=30,
        win_rate=0.9,
        avg_win_pct=0.1,
        avg_loss_pct=0.01,
        kelly_fraction=8.0,  # absurdly large
    )
    size = kelly_size(available_cash=1000.0, confidence=1.0, stats=s, max_fraction=0.5)
    assert size == pytest.approx(1000.0 * 0.5)
