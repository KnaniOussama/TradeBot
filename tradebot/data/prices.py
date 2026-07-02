from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import UTC, datetime

from tradebot.data.jupiter import JupiterClient
from tradebot.logging_setup import get_logger

log = get_logger("prices")


@dataclass(frozen=True)
class PriceTick:
    pair: str
    price: float
    sampled_at: datetime
    in_amount_quote: float
    price_impact_pct: float


@dataclass
class _PairSpec:
    symbol: str
    mint: str
    decimals: int


class PriceFeed:
    def __init__(
        self,
        jupiter: JupiterClient,
        quote_mint: str,
        quote_decimals: int,
        poll_interval_s: float = 5.0,
        sample_size_in_quote: float = 10.0,
        slippage_bps: int = 50,
        quote_symbol: str = "USDC",
    ) -> None:
        self._jup = jupiter
        self._quote_mint = quote_mint
        self._quote_decimals = quote_decimals
        self._quote_symbol = quote_symbol
        self._poll = poll_interval_s
        self._sample = sample_size_in_quote
        self._slippage_bps = slippage_bps
        self._pairs: list[_PairSpec] = []
        self._subscribers: list[asyncio.Queue[PriceTick]] = []
        self._stop = asyncio.Event()

    def add_pair(self, symbol: str, mint: str, decimals: int) -> None:
        self._pairs.append(_PairSpec(symbol=symbol, mint=mint, decimals=decimals))

    def subscribe(self) -> asyncio.Queue[PriceTick]:
        q: asyncio.Queue[PriceTick] = asyncio.Queue(maxsize=256)
        self._subscribers.append(q)
        return q

    def stop(self) -> None:
        self._stop.set()

    async def run(self) -> None:
        while not self._stop.is_set():
            for spec in self._pairs:
                try:
                    await self._poll_pair(spec)
                except Exception as e:  # transient: log and continue
                    pair_str = f"{spec.symbol}/{self._quote_symbol}"
                    log.warning("price_poll_failed", pair=pair_str, error=str(e))
            try:
                await asyncio.wait_for(self._stop.wait(), timeout=self._poll)
            except TimeoutError:
                pass

    async def _poll_pair(self, spec: _PairSpec) -> None:
        # Quote: input = quote token (USDC), output = base token. Then invert.
        amount_quote_units = int(self._sample * (10**self._quote_decimals))
        q = await self._jup.quote(
            input_mint=self._quote_mint,
            output_mint=spec.mint,
            amount=amount_quote_units,
            slippage_bps=self._slippage_bps,
        )
        # quote tells us how many base tokens we get per `self._sample` quote.
        # price = quote_in / base_out (per 1 base token)
        out_human = q.out_amount / (10**spec.decimals)
        if out_human == 0:
            return
        price = self._sample / out_human
        tick = PriceTick(
            pair=f"{spec.symbol}/{self._quote_symbol}",
            price=price,
            sampled_at=datetime.now(UTC),
            in_amount_quote=self._sample,
            price_impact_pct=q.price_impact_pct,
        )
        for sub in self._subscribers:
            try:
                sub.put_nowait(tick)
            except asyncio.QueueFull:
                log.warning("subscriber_queue_full", pair=tick.pair)
