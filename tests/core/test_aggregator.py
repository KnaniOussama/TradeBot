from __future__ import annotations

from datetime import UTC, datetime

import pytest

from tradebot.core.aggregator import AggregatedScore, SignalAggregator
from tradebot.signals.base import MarketContext, SignalScore


def _ctx() -> MarketContext:
    return MarketContext(pair="SOL/USDC", now=datetime.now(UTC), ohlcv={})


class _StubSignal:
    def __init__(self, name: str, timeframe: str, fixed_score: float):
        self.name = name
        self.timeframe = timeframe
        self._fixed = fixed_score

    async def score(self, ctx: MarketContext) -> SignalScore:
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=self._fixed,
            sampled_at=ctx.now,
            components={},
        )


@pytest.mark.asyncio
async def test_aggregator_combines_two_signals_one_timeframe():
    agg = SignalAggregator(
        signals=[
            _StubSignal("ta", "1m", 0.6),
            _StubSignal("microstructure", "1m", 0.4),
        ],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 0.5, "microstructure": 0.5},
    )
    ctx = MarketContext(pair="SOL/USDC", now=datetime.now(UTC), ohlcv={})
    out = await agg.aggregate(ctx)
    assert isinstance(out, AggregatedScore)
    assert out.pair == "SOL/USDC"
    # New math: ta_avg=0.6*1/1=0.6 * 0.5 = 0.3; micro_avg=0.4*1/1=0.4 * 0.5 = 0.2; total=0.5
    assert out.composite == pytest.approx(0.5)
    assert len(out.scores) == 2


@pytest.mark.asyncio
async def test_aggregator_combines_two_timeframes():
    agg = SignalAggregator(
        signals=[
            _StubSignal("ta", "1m", 0.2),
            _StubSignal("ta", "1h", 0.8),
        ],
        timeframe_weights={"1m": 0.3, "1h": 0.7},
        signal_weights={"ta": 1.0},
    )
    ctx = MarketContext(pair="SOL/USDC", now=datetime.now(UTC), ohlcv={})
    out = await agg.aggregate(ctx)
    # New math: ta_avg = (0.2*0.3 + 0.8*0.7) / (0.3+0.7) = 0.62; composite = 0.62*1.0 = 0.62
    assert out.composite == pytest.approx(0.62)


@pytest.mark.asyncio
async def test_aggregator_clamps_to_unit_range():
    agg = SignalAggregator(
        signals=[_StubSignal("ta", "1m", 1.0), _StubSignal("micro", "1m", 1.0)],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 0.6, "micro": 0.6},  # sum > 1 deliberately
    )
    ctx = MarketContext(pair="X", now=datetime.now(UTC), ohlcv={})
    out = await agg.aggregate(ctx)
    assert -1.0 <= out.composite <= 1.0


@pytest.mark.asyncio
async def test_aggregator_skips_unknown_signal_weight():
    agg = SignalAggregator(
        signals=[_StubSignal("ta", "1m", 0.5), _StubSignal("ghost", "1m", 1.0)],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},  # ghost not weighted
    )
    ctx = MarketContext(pair="X", now=datetime.now(UTC), ohlcv={})
    out = await agg.aggregate(ctx)
    # New math: ta_avg=0.5 * 1.0 = 0.5; ghost has no signal_weight so contributes 0
    assert out.composite == pytest.approx(0.5)


@pytest.mark.asyncio
async def test_aggregator_records_per_signal_scores():
    agg = SignalAggregator(
        signals=[_StubSignal("ta", "1m", 0.4)],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
    )
    ctx = MarketContext(pair="X", now=datetime.now(UTC), ohlcv={})
    out = await agg.aggregate(ctx)
    assert out.scores[0].signal == "ta"
    assert out.scores[0].score == 0.4


@pytest.mark.asyncio
async def test_single_timeframe_signal_uses_full_signal_weight():
    # microstructure only at 1m, weighted ta:0.5, micro:0.5; tf weights 1m:0.2, 1h:0.8
    # TA gets 0 (no instances), micro at 1m → micro contribution = score * 0.5
    agg = SignalAggregator(
        signals=[_StubSignal("microstructure", "1m", 0.6)],
        timeframe_weights={"1m": 0.2, "1h": 0.8},
        signal_weights={"ta": 0.5, "microstructure": 0.5},
    )
    out = await agg.aggregate(_ctx())
    # micro avg = 0.6 (only point, tf_weight=0.2 → normalized avg = 0.6), weighted by 0.5 → 0.30
    assert out.composite == pytest.approx(0.30)


@pytest.mark.asyncio
async def test_multi_timeframe_signal_averages_by_tf_weight():
    # ta at 1m (score 0.4) and 1h (score 0.8), tf weights 1m:0.25, 1h:0.75
    # ta avg = (0.4*0.25 + 0.8*0.75) / (0.25+0.75) = (0.1+0.6)/1.0 = 0.7
    # weighted by signal_weight 1.0 → 0.7
    agg = SignalAggregator(
        signals=[_StubSignal("ta", "1m", 0.4), _StubSignal("ta", "1h", 0.8)],
        timeframe_weights={"1m": 0.25, "1h": 0.75},
        signal_weights={"ta": 1.0},
    )
    out = await agg.aggregate(_ctx())
    assert out.composite == pytest.approx(0.7)


@pytest.mark.asyncio
async def test_mixed_signals_combine_correctly():
    # ta at 1m (0.4) + 1h (0.8); micro at 1m (1.0); weights ta:0.6, micro:0.4
    # ta_avg = (0.4*0.25 + 0.8*0.75) / 1.0 = 0.7
    # micro_avg = 1.0 (only point)
    # composite = 0.7*0.6 + 1.0*0.4 = 0.42 + 0.40 = 0.82
    agg = SignalAggregator(
        signals=[
            _StubSignal("ta", "1m", 0.4),
            _StubSignal("ta", "1h", 0.8),
            _StubSignal("microstructure", "1m", 1.0),
        ],
        timeframe_weights={"1m": 0.25, "1h": 0.75},
        signal_weights={"ta": 0.6, "microstructure": 0.4},
    )
    out = await agg.aggregate(_ctx())
    assert out.composite == pytest.approx(0.82)
