from __future__ import annotations

from datetime import UTC, datetime

import pandas as pd
import pytest

from tradebot.backtest.runner import BacktestParams, BacktestResult, run_backtest
from tradebot.config.models import RiskConfig
from tradebot.signals.base import MarketContext, SignalScore


def _make_ohlcv(n: int, start_price: float = 100.0, trend: float = 0.5) -> pd.DataFrame:
    """Generate n bars of synthetic OHLCV with optional uptrend."""
    from datetime import timedelta

    rows = []
    price = start_price
    base_ts = datetime(2026, 5, 1, 0, 0, 0, tzinfo=UTC)
    for i in range(n):
        ts = base_ts + timedelta(minutes=i)
        close = price + i * trend
        open_ = close - 0.1
        high = close + 0.5
        low = close - 0.5
        rows.append(
            {
                "timestamp": ts,
                "open": open_,
                "high": high,
                "low": low,
                "close": close,
                "volume": 1000.0,
            }
        )
    return pd.DataFrame(rows)


class AlwaysBullSignal:
    """Stub signal that always returns +1.0 (strong buy)."""

    name = "ta"
    timeframe = "1m"

    async def score(self, ctx: MarketContext) -> SignalScore:
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=1.0,
            sampled_at=ctx.now,
            components={},
        )


@pytest.mark.asyncio
async def test_result_has_equity_curve_and_trades():
    ohlcv = _make_ohlcv(n=80)
    params = BacktestParams(
        pair="SOL/USDC",
        timeframe="1m",
        starting_cash=100.0,
        warmup_bars=50,
        entry_threshold=0.6,
        exit_flip_threshold=-0.3,
    )
    result = await run_backtest(
        ohlcv=ohlcv,
        params=params,
        signals_factory=lambda: [AlwaysBullSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
        risk=RiskConfig(),
        backtest_id="test001",
    )
    assert isinstance(result, BacktestResult)
    assert result.bars_processed == 30  # 80 - 50 warmup
    assert len(result.equity_curve) == 30
    assert result.id == "test001"
    assert result.pair == "SOL/USDC"
    assert result.starting_cash == 100.0


@pytest.mark.asyncio
async def test_warmup_bars_skipped():
    ohlcv = _make_ohlcv(n=100)
    params = BacktestParams(pair="SOL/USDC", warmup_bars=70, starting_cash=100.0)
    result = await run_backtest(
        ohlcv=ohlcv,
        params=params,
        signals_factory=lambda: [AlwaysBullSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
        risk=RiskConfig(),
        backtest_id="warmup_test",
    )
    assert result.bars_processed == 30  # 100 - 70 warmup


@pytest.mark.asyncio
async def test_strong_bull_signal_triggers_entry():
    ohlcv = _make_ohlcv(n=80, start_price=100.0, trend=0.01)
    params = BacktestParams(
        pair="SOL/USDC",
        starting_cash=100.0,
        warmup_bars=50,
        entry_threshold=0.6,
    )
    result = await run_backtest(
        ohlcv=ohlcv,
        params=params,
        signals_factory=lambda: [AlwaysBullSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
        risk=RiskConfig(),
        backtest_id="bull_test",
    )
    # With always-bull signal the engine should have entered at least once
    assert result.n_trades >= 1


@pytest.mark.asyncio
async def test_final_equity_reflects_trades():
    ohlcv = _make_ohlcv(n=80, start_price=100.0, trend=1.0)
    params = BacktestParams(
        pair="SOL/USDC",
        starting_cash=100.0,
        warmup_bars=50,
    )
    result = await run_backtest(
        ohlcv=ohlcv,
        params=params,
        signals_factory=lambda: [AlwaysBullSignal()],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
        risk=RiskConfig(),
        backtest_id="equity_test",
    )
    assert result.final_equity > 0
    assert result.completed_at != ""
