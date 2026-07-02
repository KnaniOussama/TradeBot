from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class KellyStats:
    n_round_trips: int
    win_rate: float  # 0..1
    avg_win_pct: float  # average winning trade return as fraction
    avg_loss_pct: float  # average losing trade return as fraction (positive number)
    kelly_fraction: float  # raw f* = (p*b - q) / b where b = avg_win/avg_loss


def compute_kelly_stats(round_trip_returns: list[float]) -> KellyStats:
    """Compute Kelly statistics from a list of round-trip returns.

    Each entry is a fractional return: positive for wins, negative for losses.
    Returns KellyStats with n_round_trips=0 if list is empty.
    """
    if not round_trip_returns:
        return KellyStats(0, 0.0, 0.0, 0.0, 0.0)

    wins = [r for r in round_trip_returns if r > 0]
    losses = [-r for r in round_trip_returns if r < 0]
    n = len(round_trip_returns)
    win_rate = len(wins) / n
    avg_win = sum(wins) / max(len(wins), 1)
    avg_loss = sum(losses) / max(len(losses), 1)

    if avg_loss <= 1e-9:
        # No losses recorded, be conservative, don't trust the kelly
        return KellyStats(n, win_rate, avg_win, 0.0, 0.0)

    b = avg_win / avg_loss
    kelly = (win_rate * b - (1 - win_rate)) / b if b > 0 else 0.0
    return KellyStats(n, win_rate, avg_win, avg_loss, max(0.0, kelly))


def kelly_size(
    available_cash: float,
    confidence: float,
    stats: KellyStats,
    cap_fraction: float = 0.5,  # half-Kelly safety cap
    min_fraction: float = 0.05,  # don't waste fees on tiny trades
    max_fraction: float = 0.5,  # absolute ceiling regardless of Kelly output
    min_round_trips: int = 20,  # need this many trades before trusting Kelly
    fallback_min: float = 0.30,
    fallback_max: float = 0.50,
) -> float:
    """Compute trade size in quote units using Kelly fraction.

    Falls back to linear sizing (like legacy RiskManager.size_for) when
    insufficient trade history is available (< min_round_trips).

    Note: partial sells are treated as full closes, documented as approximation.
    """
    if stats.n_round_trips < min_round_trips:
        # Fallback: linear-by-confidence sizing matching legacy RiskManager.size_for
        c = max(0.0, min(1.0, confidence))
        frac = fallback_min + c * (fallback_max - fallback_min)
        return available_cash * frac

    # Half-Kelly, scaled by confidence (weak signals trade smaller)
    raw = stats.kelly_fraction * cap_fraction
    confidence_scaled = raw * max(0.0, min(1.0, confidence))
    frac = max(min_fraction, min(max_fraction, confidence_scaled))
    return available_cash * frac
