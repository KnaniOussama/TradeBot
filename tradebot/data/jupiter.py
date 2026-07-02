from __future__ import annotations

import asyncio
import random
from dataclasses import dataclass
from types import TracebackType
from typing import TYPE_CHECKING, Any

import httpx

from tradebot.logging_setup import get_logger

if TYPE_CHECKING:
    from tradebot.data.rate_limiter import TokenBucketLimiter

log = get_logger("data.jupiter")


@dataclass
class JupiterSwap:
    serialized_tx_b64: str
    last_valid_block_height: int
    prioritization_fee_lamports: int
    raw: dict[str, Any]


@dataclass
class JupiterQuote:
    input_mint: str
    output_mint: str
    in_amount: int
    out_amount: int
    other_amount_threshold: int
    slippage_bps: int
    price_impact_pct: float
    route_labels: list[str]
    raw: dict[str, Any]

    def implied_price(self, in_decimals: int, out_decimals: int) -> float:
        in_human = self.in_amount / (10**in_decimals)
        out_human = self.out_amount / (10**out_decimals)
        if in_human == 0:
            return 0.0
        return float(out_human / in_human)


class JupiterClient:
    def __init__(
        self,
        base_url: str,
        timeout_s: float = 5.0,
        limiter: TokenBucketLimiter | None = None,
        max_429_retries: int = 3,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout_s
        self._client: httpx.AsyncClient | None = None
        self._limiter = limiter
        self._max_429_retries = max_429_retries

    async def __aenter__(self) -> JupiterClient:
        self._client = httpx.AsyncClient(timeout=self._timeout)
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    async def _request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        attempt = 0
        while True:
            if self._limiter is not None:
                await self._limiter.acquire()
            assert self._client is not None
            fn = getattr(self._client, method)
            resp: httpx.Response = await fn(url, **kwargs)
            if resp.status_code != 429:
                return resp
            # 429: record and decide retry
            if self._limiter is not None:
                self._limiter.record_429()
            if attempt >= self._max_429_retries:
                resp.raise_for_status()  # let the caller see the 429
                return resp
            retry_after = resp.headers.get("Retry-After")
            if retry_after:
                try:
                    sleep_s = float(retry_after)
                except ValueError:
                    sleep_s = 1.0 * (2**attempt)
            else:
                sleep_s = (1.0 * (2**attempt)) + random.uniform(0, 0.25)
            log.warning("jupiter_429_retry", attempt=attempt + 1, sleep_s=round(sleep_s, 3))
            await asyncio.sleep(sleep_s)
            attempt += 1

    async def build_swap(
        self,
        *,
        quote: JupiterQuote,
        user_pubkey: str,
        priority_fee_microlamports: int = 0,
        wrap_and_unwrap_sol: bool = True,
    ) -> JupiterSwap:
        if self._client is None:
            raise RuntimeError("JupiterClient must be used as async context manager")
        body = {
            "quoteResponse": quote.raw,
            "userPublicKey": user_pubkey,
            "wrapAndUnwrapSol": wrap_and_unwrap_sol,
            "computeUnitPriceMicroLamports": priority_fee_microlamports,
            "asLegacyTransaction": False,
        }
        url = f"{self._base_url}/swap"
        resp = await self._request("post", url, json=body)
        resp.raise_for_status()
        data = resp.json()
        return JupiterSwap(
            serialized_tx_b64=data["swapTransaction"],
            last_valid_block_height=int(data.get("lastValidBlockHeight", 0)),
            prioritization_fee_lamports=int(data.get("prioritizationFeeLamports", 0)),
            raw=data,
        )

    async def quote(
        self,
        *,
        input_mint: str,
        output_mint: str,
        amount: int,
        slippage_bps: int,
    ) -> JupiterQuote:
        if self._client is None:
            raise RuntimeError("JupiterClient must be used as async context manager")
        params = {
            "inputMint": input_mint,
            "outputMint": output_mint,
            "amount": str(amount),
            "slippageBps": str(slippage_bps),
        }
        url = f"{self._base_url}/quote"
        resp = await self._request("get", url, params=params)
        resp.raise_for_status()
        data = resp.json()
        return JupiterQuote(
            input_mint=data["inputMint"],
            output_mint=data["outputMint"],
            in_amount=int(data["inAmount"]),
            out_amount=int(data["outAmount"]),
            other_amount_threshold=int(data["otherAmountThreshold"]),
            slippage_bps=int(data["slippageBps"]),
            price_impact_pct=float(data["priceImpactPct"]),
            route_labels=[step["swapInfo"]["label"] for step in data.get("routePlan", [])],
            raw=data,
        )
