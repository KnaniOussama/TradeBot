from __future__ import annotations

import math
from dataclasses import dataclass, field

import pandas as pd

from tradebot.data.helius import HeliusClient, TokenTransfer, filter_for_mint
from tradebot.logging_setup import get_logger
from tradebot.signals.base import MarketContext, SignalScore, clamp_score, rolling_zscore

log = get_logger("onchain")


def _net_whale_flow(
    transfers: list[TokenTransfer],
    mint: str,
    dex_addresses: set[str],
    whale_min: float,
) -> float:
    """Positive = net DEX -> wallet (accumulation). Negative = net wallet -> DEX (distribution)."""
    relevant = [t for t in filter_for_mint(transfers, mint) if t.amount >= whale_min]
    if not relevant:
        return 0.0
    out_of_dex = sum(
        t.amount
        for t in relevant
        if t.from_addr in dex_addresses and t.to_addr not in dex_addresses
    )
    into_dex = sum(
        t.amount
        for t in relevant
        if t.to_addr in dex_addresses and t.from_addr not in dex_addresses
    )
    total = out_of_dex + into_dex
    if total <= 0:
        return 0.0
    net = (out_of_dex - into_dex) / total
    # Already in [-1, 1] by construction, but clamp defensively
    return clamp_score(net)


def _transfer_count_zscore(df: pd.DataFrame, window: int = 20) -> float:
    """Z-score of recent transfer count vs rolling baseline.

    df expected to have a `transfer_count` column (one row per minute bucket).
    """
    if "transfer_count" not in df.columns or len(df) < window + 1:
        return 0.0
    z = rolling_zscore(df["transfer_count"], window=window)
    last = float(z.iloc[-1])
    if not math.isfinite(last):
        return 0.0
    return clamp_score(last / 3.0)


@dataclass
class OnChainSignal:
    pair: str
    timeframe: str
    helius: HeliusClient
    mint: str
    dex_addresses: set[str]
    whale_min: float = 1000.0
    lookback_limit: int = 100
    name: str = "onchain"
    weights: dict[str, float] = field(
        default_factory=lambda: {
            "whale_flow": 0.70,
            "transfer_z": 0.30,
        }
    )

    async def _fetch_recent_transfers(self) -> list[TokenTransfer]:
        # Query the first DEX address for recent activity. v1 simplification.
        if not self.dex_addresses:
            return []
        primary = next(iter(self.dex_addresses))
        try:
            return await self.helius.recent_token_transfers(
                address=primary, limit=self.lookback_limit
            )
        except Exception as e:
            log.warning("helius_fetch_failed", error=str(e))
            return []

    async def score(self, ctx: MarketContext) -> SignalScore:
        if ctx.pair != self.pair:
            return SignalScore(
                signal=self.name,
                pair=ctx.pair,
                timeframe=self.timeframe,
                score=0.0,
                sampled_at=ctx.now,
                components={},
            )
        transfers = await self._fetch_recent_transfers()
        whale = _net_whale_flow(
            transfers,
            mint=self.mint,
            dex_addresses=self.dex_addresses,
            whale_min=self.whale_min,
        )
        # Transfer count z-score: derive a simple per-minute count from transfers
        transfer_z = 0.0
        if transfers:
            ts_list = [t.timestamp for t in transfers if t.mint == self.mint]
            df = pd.DataFrame({"timestamp": ts_list})
            if not df.empty:
                df["bucket"] = (df["timestamp"] // 60) * 60
                grouped = (
                    df.groupby("bucket")
                    .size()
                    .rename("transfer_count")
                    .reset_index()
                    .sort_values("bucket")
                )
                transfer_z = _transfer_count_zscore(grouped)
        components = {"whale_flow": whale, "transfer_z": transfer_z}
        composite = sum(components[k] * self.weights[k] for k in components)
        return SignalScore(
            signal=self.name,
            pair=ctx.pair,
            timeframe=self.timeframe,
            score=clamp_score(composite),
            sampled_at=ctx.now,
            components=components,
        )
