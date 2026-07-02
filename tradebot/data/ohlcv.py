from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import UTC, datetime

from tradebot.data.prices import PriceTick
from tradebot.storage.repo import JsonStorage, OHLCVCandle

TIMEFRAME_SECONDS: dict[str, int] = {
    "5s": 5,
    "1m": 60,
    "15m": 900,
    "1h": 3600,
}


def bucket_for(ts: datetime, timeframe: str) -> datetime:
    secs = TIMEFRAME_SECONDS[timeframe]
    epoch = int(ts.replace(tzinfo=UTC).timestamp())
    floored = epoch - (epoch % secs)
    return datetime.fromtimestamp(floored, tz=UTC)


@dataclass
class _BucketState:
    bucket_start: datetime
    open: float
    high: float
    low: float
    close: float


class OHLCVAggregator:
    def __init__(
        self, storage: JsonStorage, source: asyncio.Queue[PriceTick], timeframe: str
    ) -> None:
        self._storage = storage
        self._source = source
        self._timeframe = timeframe
        self._state: dict[str, _BucketState] = {}
        self._stop_after_drain = False

    def stop_after_drain(self) -> None:
        self._stop_after_drain = True

    async def run(self) -> None:
        while True:
            try:
                tick = await asyncio.wait_for(self._source.get(), timeout=0.05)
            except TimeoutError:
                if self._stop_after_drain:
                    return
                continue
            await self._handle(tick)

    async def _handle(self, tick: PriceTick) -> None:
        bucket = bucket_for(tick.sampled_at, self._timeframe)
        cur = self._state.get(tick.pair)
        if cur is None:
            state = _BucketState(bucket, tick.price, tick.price, tick.price, tick.price)
            self._state[tick.pair] = state
            return
        if bucket > cur.bucket_start:
            self._write(tick.pair, cur)
            state = _BucketState(bucket, tick.price, tick.price, tick.price, tick.price)
            self._state[tick.pair] = state
            return
        cur.high = max(cur.high, tick.price)
        cur.low = min(cur.low, tick.price)
        cur.close = tick.price

    def _write(self, pair: str, state: _BucketState) -> None:
        self._storage.upsert_ohlcv(
            OHLCVCandle(
                pair=pair,
                timeframe=self._timeframe,
                bucket_start=state.bucket_start,
                open=state.open,
                high=state.high,
                low=state.low,
                close=state.close,
                volume_quote=0.0,  # populated in a later phase when fills exist
            ),
        )

    def _flush_all(self) -> None:
        for pair, state in self._state.items():
            self._write(pair, state)
        self._state.clear()
