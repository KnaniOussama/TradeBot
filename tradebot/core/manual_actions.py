from __future__ import annotations

from collections import deque
from dataclasses import dataclass
from datetime import UTC, datetime


@dataclass(frozen=True)
class ManualExitRequest:
    pair: str
    reason: str
    requested_at: datetime


class ManualActionQueue:
    """Single-process queue of user-initiated trade actions.

    The dashboard server enqueues from HTTP request handlers; the trading loop
    drains at the top of each cycle and converts entries into normal Actions
    that run through the same executor (so slippage gates, gas accounting,
    and trade logging all apply consistently).
    """

    def __init__(self) -> None:
        self._pending: deque[ManualExitRequest] = deque()

    def request_exit(self, *, pair: str, reason: str = "") -> ManualExitRequest:
        req = ManualExitRequest(
            pair=pair,
            reason=reason or "manual sell",
            requested_at=datetime.now(UTC),
        )
        self._pending.append(req)
        return req

    def drain(self) -> list[ManualExitRequest]:
        out = list(self._pending)
        self._pending.clear()
        return out

    def __len__(self) -> int:
        return len(self._pending)
