import asyncio
from datetime import UTC, datetime
from pathlib import Path

import pytest

from tradebot.data.ohlcv import OHLCVAggregator
from tradebot.data.prices import PriceTick
from tradebot.storage.repo import JsonStorage


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


def _tick(pair: str, price: float, ts: datetime) -> PriceTick:
    return PriceTick(
        pair=pair,
        price=price,
        sampled_at=ts,
        in_amount_quote=10.0,
        price_impact_pct=0.0,
    )


@pytest.mark.asyncio
async def test_aggregator_writes_one_candle_per_bucket(storage):
    queue: asyncio.Queue[PriceTick] = asyncio.Queue()
    agg = OHLCVAggregator(storage=storage, source=queue, timeframe="1m")
    base = datetime(2026, 5, 3, 12, 0, 30, tzinfo=UTC)
    # three ticks within same minute
    await queue.put(_tick("SOL/USDC", 100.0, base))
    await queue.put(_tick("SOL/USDC", 102.0, base.replace(second=45)))
    await queue.put(_tick("SOL/USDC", 99.0, base.replace(second=55)))
    # one tick in next minute -> flushes the previous bucket
    await queue.put(_tick("SOL/USDC", 101.0, base.replace(minute=1, second=5)))
    # sentinel to stop
    agg.stop_after_drain()
    await agg.run()

    df = storage.load_ohlcv(pair="SOL/USDC", timeframe="1m", limit=10)
    assert len(df) == 1
    row = df.iloc[0]
    assert row["open"] == 100.0
    assert row["high"] == 102.0
    assert row["low"] == 99.0
    assert row["close"] == 99.0


@pytest.mark.asyncio
async def test_aggregator_separates_pairs(storage):
    queue: asyncio.Queue[PriceTick] = asyncio.Queue()
    agg = OHLCVAggregator(storage=storage, source=queue, timeframe="1m")
    base = datetime(2026, 5, 3, 12, 0, 0, tzinfo=UTC)
    await queue.put(_tick("SOL/USDC", 100.0, base))
    await queue.put(_tick("JUP/USDC", 1.5, base))
    await queue.put(_tick("SOL/USDC", 110.0, base.replace(minute=1)))
    await queue.put(_tick("JUP/USDC", 1.6, base.replace(minute=1)))
    agg.stop_after_drain()
    await agg.run()

    sol_df = storage.load_ohlcv(pair="SOL/USDC", timeframe="1m", limit=10)
    jup_df = storage.load_ohlcv(pair="JUP/USDC", timeframe="1m", limit=10)
    assert len(sol_df) == 1
    assert len(jup_df) == 1
