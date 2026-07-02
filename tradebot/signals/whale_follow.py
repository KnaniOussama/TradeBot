from __future__ import annotations

import math
from dataclasses import dataclass, field
from datetime import datetime, timedelta

from tradebot.core.whale_activity import WhaleActivityTracker
from tradebot.data.helius import WhaleSwap
from tradebot.logging_setup import get_logger
from tradebot.signals.base import MarketContext, SignalScore, clamp_score

log = get_logger("signals.whale_follow")


@dataclass
class WhaleFollowSignal:
    """Score a pair by recent activity from a curated list of "smart-money" wallets.

    The shared `WhaleActivityTracker` does the Helius fetch once per cycle; this
    signal just queries the resulting swap list. Logic:
      - For the requested pair, count swaps that touched its base mint:
          * whale BUYS the base (in_mint=quote, out_mint=base) → +1 contribution
          * whale SELLS the base (in_mint=base, out_mint=quote) → -1 contribution
      - Each contribution is weighted by exp(-Δt × ln2 / half_life) so older swaps fade.
      - Score = clamp(sum_contributions / sum_decay_weights, -1, 1).
    """

    pair: str
    base_mint: str
    quote_mint: str
    tracker: WhaleActivityTracker
    lookback_seconds: int = 1800
    decay_half_life_s: float = 600.0
    name: str = "whale_follow"
    timeframe: str = "1m"
    weights: dict[str, float] = field(default_factory=dict)

    def _score_from_swaps(self, swaps: list[WhaleSwap], now: datetime) -> float:
        if not swaps:
            return 0.0
        cutoff = now - timedelta(seconds=self.lookback_seconds)
        relevant = 0.0
        contribution = 0.0
        for s in swaps:
            if s.timestamp < cutoff:
                continue
            dt = (now - s.timestamp).total_seconds()
            decay = math.exp(-dt * math.log(2) / max(1.0, self.decay_half_life_s))
            if s.out_mint == self.base_mint and s.in_mint == self.quote_mint:
                contribution += decay  # buy
                relevant += decay
            elif s.in_mint == self.base_mint and s.out_mint == self.quote_mint:
                contribution -= decay  # sell
                relevant += decay
        if relevant <= 0:
            return 0.0
        return contribution / relevant

    async def score(self, ctx: MarketContext) -> SignalScore:
        if ctx.pair != self.pair:
            return SignalScore(
                signal=self.name,
                pair=ctx.pair,
                timeframe=self.timeframe,
                score=0.0,
                sampled_at=ctx.now,
                components={"reason": 0.0},
            )
        swaps = self.tracker.all_recent()
        raw = self._score_from_swaps(swaps, now=ctx.now)
        score = clamp_score(raw)
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=score,
            sampled_at=ctx.now,
            components={
                "raw": raw,
                "swap_count": float(len(swaps)),
                "wallet_count": float(len(self.tracker.wallets)),
            },
        )
