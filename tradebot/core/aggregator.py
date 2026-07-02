from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime

from tradebot.signals.base import MarketContext, Signal, SignalScore, clamp_score


@dataclass(frozen=True)
class AggregatedScore:
    pair: str
    composite: float
    sampled_at: datetime
    scores: list[SignalScore]


class SignalAggregator:
    def __init__(
        self,
        signals: list[Signal],
        timeframe_weights: dict[str, float],
        signal_weights: dict[str, float],
    ) -> None:
        self._signals = signals
        self._tf_weights = timeframe_weights
        self._sig_weights = signal_weights

    async def aggregate(self, ctx: MarketContext) -> AggregatedScore:
        scores: list[SignalScore] = []
        for sig in self._signals:
            scores.append(await sig.score(ctx))

        # Group scores by signal name; compute per-signal tf-weighted average,
        # then weight by signal_weight. Single-TF signals are naturally unpenalised.
        by_signal: dict[str, list[tuple[float, float]]] = {}
        for s in scores:
            tw = self._tf_weights.get(s.timeframe, 0.0)
            by_signal.setdefault(s.signal, []).append((s.score, tw))

        composite = 0.0
        for signal_name, points in by_signal.items():
            total_w = sum(w for _, w in points)
            if total_w <= 0:
                # Treat as equal-weighted if no tf weights match
                avg_score = sum(score for score, _ in points) / max(len(points), 1)
            else:
                avg_score = sum(score * w for score, w in points) / total_w
            composite += avg_score * self._sig_weights.get(signal_name, 0.0)

        return AggregatedScore(
            pair=ctx.pair,
            composite=clamp_score(composite),
            sampled_at=ctx.now,
            scores=scores,
        )
