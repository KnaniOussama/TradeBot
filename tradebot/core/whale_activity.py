from __future__ import annotations

from collections import deque
from dataclasses import dataclass
from datetime import datetime

from tradebot.data.helius import (
    HeliusClient,
    WhaleSwap,
    get_recent_swaps_for_wallet,
)
from tradebot.logging_setup import get_logger

log = get_logger("core.whale_activity")


@dataclass
class WhaleActivityTracker:
    """Single source of truth for recent whale swap activity.

    The trading loop calls `fetch_all()` once per cycle. Multiple `WhaleFollowSignal`
    instances (one per pair) read from this tracker so we don't repeat Helius calls
    per pair. The dashboard reads `unmatched_swaps()` to surface activity in tokens
    the bot doesn't currently watch.
    """

    helius: HeliusClient
    wallets: list[str]
    per_wallet_limit: int = 20
    history_max: int = 200

    def __post_init__(self) -> None:
        self._latest_per_wallet: dict[str, list[WhaleSwap]] = {}
        self._history: deque[WhaleSwap] = deque(maxlen=self.history_max)
        self._last_fetched_at: datetime | None = None

    async def fetch_all(self) -> None:
        """Pull the latest swaps for every watched wallet. Best-effort: per-wallet
        failures are logged and skipped so one bad wallet doesn't kill the whole
        cycle."""
        if not self.wallets:
            return
        for wallet in self.wallets:
            try:
                swaps = await get_recent_swaps_for_wallet(
                    self.helius, address=wallet, limit=self.per_wallet_limit
                )
            except Exception as e:  # noqa: BLE001
                log.warning("whale_fetch_failed", wallet=wallet, error=str(e))
                continue
            # Dedupe against history by signature so the rolling buffer doesn't
            # accumulate duplicates across cycles.
            seen_sigs = {s.signature for s in self._history}
            new_swaps = [s for s in swaps if s.signature not in seen_sigs]
            self._latest_per_wallet[wallet] = swaps
            for s in new_swaps:
                self._history.append(s)

    def all_recent(self) -> list[WhaleSwap]:
        """Combined swap stream across all wallets, newest first."""
        return sorted(self._history, key=lambda s: s.timestamp, reverse=True)

    def unmatched_swaps(
        self, watched_mints: set[str], limit: int = 50
    ) -> list[WhaleSwap]:
        """Swaps where neither leg's mint is in our watchlist (off-radar activity)."""
        out: list[WhaleSwap] = []
        for s in self.all_recent():
            if s.in_mint in watched_mints or s.out_mint in watched_mints:
                continue
            out.append(s)
            if len(out) >= limit:
                break
        return out
