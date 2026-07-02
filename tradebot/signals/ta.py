from __future__ import annotations

from dataclasses import dataclass, field

import pandas as pd

from tradebot.signals.base import MarketContext, SignalScore, clamp_score


def _ema(series: pd.Series, period: int) -> pd.Series:
    return series.ewm(span=period, adjust=False).mean()


def _rsi(series: pd.Series, period: int = 14) -> pd.Series:
    delta = series.diff()
    gain = delta.clip(lower=0.0)
    loss = -delta.clip(upper=0.0)
    avg_gain = gain.ewm(alpha=1 / period, adjust=False).mean()
    avg_loss = loss.ewm(alpha=1 / period, adjust=False).mean()
    # Use a floor on avg_loss to avoid div-by-zero; zero avg_loss means RSI→100
    rsi = 100.0 - (100.0 / (1.0 + avg_gain / avg_loss.where(avg_loss > 1e-12, other=1e-12)))
    return rsi


def _rsi_score(close: pd.Series, period: int = 14) -> float:
    if len(close) < period + 1:
        return 0.0
    rsi = _rsi(close, period)
    last = float(rsi.iloc[-1])
    # Map RSI: 30 -> +1 (oversold buy), 70 -> -1 (overbought sell), 50 -> 0
    return clamp_score((50.0 - last) / 20.0)


def _macd_score(close: pd.Series, fast: int = 12, slow: int = 26, signal: int = 9) -> float:
    if len(close) < slow + signal:
        return 0.0
    macd_line = _ema(close, fast) - _ema(close, slow)
    # Use MACD line vs zero (normalized by its own stdev) for reliable trend direction.
    # The histogram (macd_line - signal) can diverge at end of long trends due to EMA lag.
    std = float(macd_line.tail(50).std(ddof=0))
    if std < 1e-12:
        return 0.0
    return clamp_score(float(macd_line.iloc[-1]) / (2.0 * std))


def _ema_cross(close: pd.Series, fast: int = 20, slow: int = 50) -> float:
    if len(close) < slow + 5:
        return 0.0
    fast_ema = _ema(close, fast)
    slow_ema = _ema(close, slow)
    diff = (fast_ema - slow_ema) / slow_ema
    return clamp_score(float(diff.iloc[-1]) * 50.0)  # scale: 2% diff -> ±1


def _bb_position(close: pd.Series, period: int = 20, stds: float = 2.0) -> float:
    if len(close) < period:
        return 0.0
    mid = close.rolling(period).mean()
    std = close.rolling(period).std(ddof=0)
    upper = mid + stds * std
    lower = mid - stds * std
    last = float(close.iloc[-1])
    u = float(upper.iloc[-1])
    lo = float(lower.iloc[-1])
    if u - lo < 1e-12:
        return 0.0
    # Position: 0 at midline, +1 at upper, -1 at lower → invert (mean-reverting)
    pos = 2 * (last - lo) / (u - lo) - 1
    return clamp_score(-pos)


def _atr_momentum(df: pd.DataFrame, period: int = 14) -> float:
    if len(df) < period + 5:
        return 0.0
    tr = pd.concat(
        [
            (df["high"] - df["low"]),
            (df["high"] - df["close"].shift()).abs(),
            (df["low"] - df["close"].shift()).abs(),
        ],
        axis=1,
    ).max(axis=1)
    atr = tr.ewm(alpha=1 / period, adjust=False).mean()
    last_atr = float(atr.iloc[-1])
    if last_atr < 1e-12:
        return 0.0
    momentum = (float(df["close"].iloc[-1]) - float(df["close"].iloc[-period])) / last_atr
    return clamp_score(momentum / 5.0)  # ~5 ATRs of move = max signal


@dataclass
class TASignal:
    timeframe: str
    name: str = "ta"
    weights: dict[str, float] = field(
        default_factory=lambda: {
            "rsi": 0.15,
            "macd": 0.30,
            "ema_cross": 0.30,
            "bb": 0.10,
            "atr_mom": 0.15,
        }
    )

    async def score(self, ctx: MarketContext) -> SignalScore:
        df = ctx.ohlcv.get(self.timeframe)
        if df is None or len(df) == 0:
            return SignalScore(
                signal=self.name,
                pair=ctx.pair,
                timeframe=self.timeframe,
                score=0.0,
                sampled_at=ctx.now,
                components={},
            )
        close = df["close"]
        components = {
            "rsi": _rsi_score(close),
            "macd": _macd_score(close),
            "ema_cross": _ema_cross(close),
            "bb": _bb_position(close),
            "atr_mom": _atr_momentum(df),
        }
        composite = sum(components[k] * self.weights[k] for k in components)
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=clamp_score(composite),
            sampled_at=ctx.now,
            components=components,
        )
