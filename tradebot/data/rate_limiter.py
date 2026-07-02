from __future__ import annotations

import asyncio
import time
from collections import deque
from dataclasses import dataclass, field
from typing import Any


@dataclass
class TokenBucketLimiter:
    """Async token bucket. `rate_per_sec` tokens added per second, capped at `burst`.
    `acquire()` blocks until at least one token is available, then consumes one.
    """

    rate_per_sec: float
    burst: int
    _tokens: float = field(init=False)
    _last_refill: float = field(init=False)
    _lock: asyncio.Lock = field(init=False, default_factory=asyncio.Lock)
    _total_acquired: int = field(init=False, default=0)
    _throttle_wait_total: float = field(init=False, default=0.0)
    _recent_acquires: deque[float] = field(init=False)
    _last_429_at: float | None = field(init=False, default=None)
    _total_429s: int = field(init=False, default=0)

    def __post_init__(self) -> None:
        self._tokens = float(self.burst)
        self._last_refill = time.monotonic()
        self._recent_acquires: deque[float] = deque(maxlen=200)

    def _refill(self, now: float) -> None:
        elapsed = now - self._last_refill
        if elapsed > 0:
            self._tokens = min(float(self.burst), self._tokens + elapsed * self.rate_per_sec)
            self._last_refill = now

    async def acquire(self) -> None:
        wait_started = time.monotonic()
        async with self._lock:
            while True:
                now = time.monotonic()
                self._refill(now)
                if self._tokens >= 1.0:
                    self._tokens -= 1.0
                    self._total_acquired += 1
                    self._recent_acquires.append(now)
                    self._throttle_wait_total += now - wait_started
                    return
                # Time until at least one token: (1 - tokens) / rate
                deficit = 1.0 - self._tokens
                wait_s = max(deficit / self.rate_per_sec, 0.005)
                await asyncio.sleep(wait_s)

    def record_429(self) -> None:
        self._last_429_at = time.monotonic()
        self._total_429s += 1

    def metrics(self) -> dict[str, Any]:
        now = time.monotonic()
        # rps over last 10s
        cutoff = now - 10.0
        recent = sum(1 for ts in self._recent_acquires if ts >= cutoff)
        return {
            "rate_limit_rps": self.rate_per_sec,
            "burst": self.burst,
            "current_tokens": round(self._tokens, 3),
            "total_acquired": self._total_acquired,
            "throttle_wait_total_s": round(self._throttle_wait_total, 3),
            "recent_rps": round(recent / 10.0, 3),
            "total_429s": self._total_429s,
            "last_429_at": self._last_429_at,
        }
