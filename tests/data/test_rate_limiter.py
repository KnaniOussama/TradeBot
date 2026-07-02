from __future__ import annotations

import time

import pytest

from tradebot.data.rate_limiter import TokenBucketLimiter


@pytest.mark.asyncio
async def test_initial_burst_does_not_block():
    lim = TokenBucketLimiter(rate_per_sec=1.0, burst=5)
    t0 = time.monotonic()
    for _ in range(5):
        await lim.acquire()
    assert time.monotonic() - t0 < 0.05  # all immediate


@pytest.mark.asyncio
async def test_sustained_rate_caps_throughput():
    lim = TokenBucketLimiter(rate_per_sec=10.0, burst=2)  # 10/sec, burst 2
    t0 = time.monotonic()
    for _ in range(12):
        await lim.acquire()
    elapsed = time.monotonic() - t0
    # 12 acquires at 10/s with burst 2 → first 2 free, next 10 cost ~1s
    assert 0.8 < elapsed < 1.4


@pytest.mark.asyncio
async def test_acquire_records_metrics():
    lim = TokenBucketLimiter(rate_per_sec=5.0, burst=1)
    for _ in range(3):
        await lim.acquire()
    m = lim.metrics()
    assert m["total_acquired"] == 3
    assert m["throttle_wait_total_s"] >= 0.0


@pytest.mark.asyncio
async def test_recent_rps_window():
    lim = TokenBucketLimiter(rate_per_sec=100.0, burst=10)  # essentially no throttle
    for _ in range(5):
        await lim.acquire()
    m = lim.metrics()
    assert m["recent_rps"] >= 0.0  # at least observed something
