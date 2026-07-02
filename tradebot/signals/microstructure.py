from __future__ import annotations

from dataclasses import dataclass, field

import pandas as pd

from tradebot.data.jupiter import JupiterClient
from tradebot.logging_setup import get_logger
from tradebot.signals.base import MarketContext, SignalScore, clamp_score, rolling_zscore

log = get_logger("microstructure")


def _vwap_deviation(df: pd.DataFrame, period: int = 20) -> float:
    if len(df) < period:
        return 0.0
    tail = df.tail(period)
    pv = (tail["close"] * tail["volume"]).sum()
    v = tail["volume"].sum()
    if v <= 0:
        return 0.0
    vwap = pv / v
    last = float(df["close"].iloc[-1])
    if vwap <= 0:
        return 0.0
    dev = (last - vwap) / vwap
    # 2% deviation == max signal magnitude; sign inverted (mean-reverting)
    return clamp_score(-dev * 50.0)


def _volume_zscore(df: pd.DataFrame, window: int = 20) -> float:
    if len(df) < window + 1:
        return 0.0
    z = rolling_zscore(df["volume"], window=window)
    last_z = float(z.iloc[-1])
    # Direction tied to last bar's close direction
    last_change = float(df["close"].iloc[-1] - df["close"].iloc[-2])
    direction = 1.0 if last_change > 0 else (-1.0 if last_change < 0 else 0.0)
    return clamp_score((last_z / 3.0) * direction)


@dataclass
class _CacheEntry:
    token: int
    value: float


@dataclass
class MicrostructureSignal:
    pair: str
    timeframe: str
    jupiter: JupiterClient
    base_mint: str
    quote_mint: str
    base_decimals: int
    quote_decimals: int
    probe_size_in_quote: float = 10.0
    name: str = "microstructure"
    weights: dict[str, float] = field(
        default_factory=lambda: {
            "depth_imbalance": 0.35,
            "vwap_dev": 0.35,
            "volume_z": 0.30,
        }
    )
    _cycle_token: int = field(default=0, init=False, repr=False)
    _depth_cache: _CacheEntry | None = field(default=None, init=False, repr=False)

    def set_cycle_token(self, token: int) -> None:
        """Call at the top of each cycle to enable within-cycle depth probe caching."""
        self._cycle_token = token

    async def _depth_imbalance(self) -> float:
        """Probe Jupiter both ways at the same quote notional. Higher slippage on
        a side means that side is thinner. Bias toward the thinner side
        (i.e., if buying is hard / asks are thin, that's bullish).

        Result is cached per cycle token to avoid duplicate HTTP calls when multiple
        timeframe instances exist for the same pair.
        """
        # Check cache hit
        if self._depth_cache is not None and self._depth_cache.token == self._cycle_token:
            return self._depth_cache.value

        try:
            quote_units_in = int(self.probe_size_in_quote * (10**self.quote_decimals))
            buy_q = await self.jupiter.quote(
                input_mint=self.quote_mint,
                output_mint=self.base_mint,
                amount=quote_units_in,
                slippage_bps=200,
            )
            # For symmetric probe on the sell side, use the equivalent base amount
            base_out = buy_q.out_amount
            if base_out == 0:
                score = 0.0
                self._depth_cache = _CacheEntry(token=self._cycle_token, value=score)
                return score
            sell_q = await self.jupiter.quote(
                input_mint=self.base_mint,
                output_mint=self.quote_mint,
                amount=base_out,
                slippage_bps=200,
            )
        except Exception as e:
            log.warning("depth_probe_failed", error=str(e))
            score = 0.0
            self._depth_cache = _CacheEntry(token=self._cycle_token, value=score)
            return score

        # If sell side has more impact than buy side, sellers are scarce → bullish
        diff = sell_q.price_impact_pct - buy_q.price_impact_pct
        # 2% diff in price impact = max magnitude
        score = clamp_score(diff * 50.0)
        self._depth_cache = _CacheEntry(token=self._cycle_token, value=score)
        return score

    async def score(self, ctx: MarketContext) -> SignalScore:
        if ctx.pair != self.pair:
            return SignalScore(
                signal=self.name,
                pair=ctx.pair,
                timeframe=self.timeframe,
                score=0.0,
                sampled_at=ctx.now,
                components={},
            )
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
        components = {
            "depth_imbalance": await self._depth_imbalance(),
            "vwap_dev": _vwap_deviation(df),
            "volume_z": _volume_zscore(df),
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
