from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

import numpy as np
import pandas as pd

RegimeLabel = Literal["trending_up", "trending_down", "chop", "neutral"]


@dataclass(frozen=True)
class Regime:
    label: RegimeLabel
    adx: float
    ema_fast_above_slow: bool


def _adx(df: pd.DataFrame, period: int = 14) -> pd.Series:
    high, low, close = df["high"], df["low"], df["close"]
    plus_dm = high.diff().clip(lower=0)
    minus_dm = -low.diff().clip(upper=0)
    tr = pd.concat(
        [(high - low), (high - close.shift()).abs(), (low - close.shift()).abs()],
        axis=1,
    ).max(axis=1)
    atr = tr.ewm(alpha=1 / period, adjust=False).mean()
    plus_di = 100 * (plus_dm.ewm(alpha=1 / period, adjust=False).mean() / atr.replace(0, np.nan))
    minus_di = 100 * (minus_dm.ewm(alpha=1 / period, adjust=False).mean() / atr.replace(0, np.nan))
    dx = 100 * (plus_di - minus_di).abs() / (plus_di + minus_di).replace(0, np.nan)
    adx = dx.ewm(alpha=1 / period, adjust=False).mean()
    return adx.fillna(0.0)


def classify_regime(
    df: pd.DataFrame,
    adx_period: int = 14,
    ema_fast: int = 20,
    ema_slow: int = 50,
    trending_threshold: float = 25.0,
    chop_threshold: float = 20.0,
) -> Regime:
    """Classify the current market regime from OHLCV data.

    Returns Regime("neutral", 0.0, False) if insufficient data for warmup.
    """
    if len(df) < max(adx_period, ema_slow) + 5:
        return Regime("neutral", 0.0, False)

    adx_val = float(_adx(df, period=adx_period).iloc[-1])
    ema_fast_v = float(df["close"].ewm(span=ema_fast, adjust=False).mean().iloc[-1])
    ema_slow_v = float(df["close"].ewm(span=ema_slow, adjust=False).mean().iloc[-1])
    fast_above = bool(ema_fast_v > ema_slow_v)

    if adx_val >= trending_threshold:
        label: RegimeLabel = "trending_up" if fast_above else "trending_down"
    elif adx_val <= chop_threshold:
        label = "chop"
    else:
        label = "neutral"

    return Regime(label=label, adx=adx_val, ema_fast_above_slow=fast_above)
