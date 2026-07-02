from __future__ import annotations

import asyncio

from tradebot.dashboard.state import DashboardSnapshot
from tradebot.logging_setup import get_logger

log = get_logger("dashboard.hub")


class DashboardHub:
    def __init__(self, subscriber_queue_size: int = 4) -> None:
        self._latest: DashboardSnapshot | None = None
        self._subscribers: list[asyncio.Queue[DashboardSnapshot]] = []
        self._sub_size = subscriber_queue_size

    def latest(self) -> DashboardSnapshot | None:
        return self._latest

    def subscribe(self) -> asyncio.Queue[DashboardSnapshot]:
        q: asyncio.Queue[DashboardSnapshot] = asyncio.Queue(maxsize=self._sub_size)
        self._subscribers.append(q)
        return q

    def unsubscribe(self, q: asyncio.Queue[DashboardSnapshot]) -> None:
        if q in self._subscribers:
            self._subscribers.remove(q)

    async def publish(self, snapshot: DashboardSnapshot) -> None:
        self._latest = snapshot
        for sub in self._subscribers:
            try:
                sub.put_nowait(snapshot)
            except asyncio.QueueFull:
                # Drop oldest, push newest
                try:
                    sub.get_nowait()
                except asyncio.QueueEmpty:
                    pass
                try:
                    sub.put_nowait(snapshot)
                except asyncio.QueueFull:
                    log.warning("subscriber_drop_after_retry")
