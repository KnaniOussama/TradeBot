from __future__ import annotations

import math
from dataclasses import dataclass, field
from datetime import datetime
from typing import Protocol

import pandas as pd


@dataclass(frozen=True)
class MarketContext:
    """Everything a signal needs to score a single pair at a single instant."""

    pair: str
    now: datetime
    ohlcv: dict[str, pd.DataFrame]  # timeframe -> DataFrame[open,high,low,close,volume]
    extras: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class SignalScore:
    signal: str
    pair: str
    timeframe: str
    score: float  # in [-1, +1]
    sampled_at: datetime
    components: dict[str, float]  # named sub-scores for transparency

    def __post_init__(self) -> None:
        if not -1.0 <= self.score <= 1.0:
            raise ValueError(f"score out of range [-1, 1]: {self.score}")


class Signal(Protocol):
    """A signal source. Stateless beyond internal config; emits a SignalScore.

    Implementations live in tradebot/signals/{ta,microstructure,onchain}.py.
    """

    name: str
    timeframe: str

    async def score(self, ctx: MarketContext) -> SignalScore: ...


def clamp_score(value: float) -> float:
    """Clip to [-1, 1]; treat NaN/inf as neutral 0.0."""
    if value is None or not math.isfinite(value):
        return 0.0
    return max(-1.0, min(1.0, value))


def rolling_zscore(series: pd.Series, window: int) -> pd.Series:
    """Z-score against rolling window. Returns NaN for the first window-1 entries.

    Zero-variance windows return 0 (instead of inf) so callers don't get garbage.
    """
    rolling = series.rolling(window=window)
    mean = rolling.mean()
    std = rolling.std(ddof=0)
    z = (series - mean) / std
    # Only replace zero-variance (std near 0 but not NaN) with 0; keep NaN for warm-up period
    zero_var = std.notna() & (std <= 1e-12)
    z = z.where(~zero_var, other=0.0)
    return z
