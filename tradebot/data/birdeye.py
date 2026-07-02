from __future__ import annotations

import asyncio
import random
from types import TracebackType
from typing import TYPE_CHECKING, Any

import httpx

from tradebot.logging_setup import get_logger

if TYPE_CHECKING:
    from tradebot.data.rate_limiter import TokenBucketLimiter

log = get_logger("data.birdeye")


class BirdeyeClient:
    """Async wrapper for Birdeye's public DeFi data API.

    Free-tier endpoint we use:
      GET /defi/multi_price?list_address=mint1,mint2,...
      Headers: X-API-KEY: <key>, x-chain: solana

    Returns price_usd per mint. One call covers up to ~100 tokens, so the entire
    watchlist's marks are a single HTTP round-trip rather than N Jupiter quotes.
    """

    DEFAULT_BASE_URL = "https://public-api.birdeye.so"

    def __init__(
        self,
        api_key: str,
        base_url: str = DEFAULT_BASE_URL,
        chain: str = "solana",
        timeout_s: float = 5.0,
        limiter: TokenBucketLimiter | None = None,
        max_429_retries: int = 2,
    ) -> None:
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._chain = chain
        self._timeout = timeout_s
        self._client: httpx.AsyncClient | None = None
        self._limiter = limiter
        self._max_429_retries = max_429_retries
        # Set to True once /defi/multi_price returns 401/403 — we then use the
        # per-token /defi/price endpoint exclusively (free Standard tier).
        self._multi_price_locked: bool = False

    async def _get_with_limit(self, url: str, params: dict[str, Any]) -> httpx.Response:
        """GET with shared rate limiter + 429 backoff + retry."""
        assert self._client is not None
        attempt = 0
        while True:
            if self._limiter is not None:
                await self._limiter.acquire()
            resp = await self._client.get(url, params=params, headers=self._headers())
            if resp.status_code != 429:
                return resp
            if self._limiter is not None:
                self._limiter.record_429()
            if attempt >= self._max_429_retries:
                return resp
            sleep_s = (1.0 * (2**attempt)) + random.uniform(0, 0.25)
            log.warning(
                "birdeye_429_retry",
                attempt=attempt + 1,
                sleep_s=round(sleep_s, 3),
                url=url.rsplit("/", 1)[-1],
            )
            await asyncio.sleep(sleep_s)
            attempt += 1

    async def __aenter__(self) -> BirdeyeClient:
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

    def _headers(self) -> dict[str, str]:
        return {
            "X-API-KEY": self._api_key,
            "x-chain": self._chain,
            "accept": "application/json",
        }

    async def multi_price(self, mints: list[str]) -> dict[str, float]:
        """Fetch USD price for a batch of token mints. Returns {mint: price_usd}.

        Tries `/defi/multi_price` first (batched, 1 HTTP call — but requires
        paid Starter tier). On 401/403 (free-tier endpoint restriction) falls
        back to per-token `/defi/price`, which is on the free Standard tier.
        Switches mode permanently after the first 401 so we don't keep hitting
        the locked endpoint every cycle.
        """
        if self._client is None:
            raise RuntimeError("BirdeyeClient must be used as async context manager")
        if not mints:
            return {}
        if not self._multi_price_locked:
            batched = await self._try_multi_price(mints)
            if batched is not None:
                return batched
            # _try_multi_price set _multi_price_locked when it hit 401/403.
        return await self._single_prices(mints)

    async def _try_multi_price(self, mints: list[str]) -> dict[str, float] | None:
        """Returns parsed result, or None if endpoint is unavailable on this plan."""
        url = f"{self._base_url}/defi/multi_price"
        params: dict[str, Any] = {"list_address": ",".join(mints)}
        try:
            resp = await self._get_with_limit(url, params)
        except httpx.HTTPError as e:
            log.warning("birdeye_request_failed", endpoint="multi_price", error=str(e))
            return {}
        if resp.status_code in (401, 403):
            log.info(
                "birdeye_multi_price_unavailable_falling_back_to_single",
                status=resp.status_code,
                hint="free Standard tier only includes /defi/price (single)",
            )
            self._multi_price_locked = True
            return None
        if resp.status_code != 200:
            log.warning(
                "birdeye_http_error",
                endpoint="multi_price",
                status=resp.status_code,
                body=resp.text[:200],
            )
            return {}
        try:
            payload = resp.json()
        except ValueError:
            log.warning("birdeye_bad_json", endpoint="multi_price")
            return {}
        if not isinstance(payload, dict) or not payload.get("success"):
            log.warning("birdeye_unsuccessful", body=str(payload)[:200])
            return {}
        data = payload.get("data") or {}
        out: dict[str, float] = {}
        for mint, info in data.items():
            if not isinstance(info, dict):
                continue
            value = info.get("value")
            if value is None:
                continue
            try:
                out[mint] = float(value)
            except (TypeError, ValueError):
                continue
        return out

    async def _single_prices(self, mints: list[str]) -> dict[str, float]:
        """Fan-out to /defi/price for each mint. One rate-limited HTTP call per token."""
        url = f"{self._base_url}/defi/price"
        out: dict[str, float] = {}
        for mint in mints:
            try:
                resp = await self._get_with_limit(url, {"address": mint})
            except httpx.HTTPError as e:
                log.warning("birdeye_single_price_failed", mint=mint, error=str(e))
                continue
            if resp.status_code != 200:
                log.warning(
                    "birdeye_single_price_http_error",
                    mint=mint,
                    status=resp.status_code,
                    body=resp.text[:120],
                )
                continue
            try:
                payload = resp.json()
            except ValueError:
                continue
            if not isinstance(payload, dict) or not payload.get("success"):
                continue
            data = payload.get("data") or {}
            value = data.get("value") if isinstance(data, dict) else None
            if value is None:
                continue
            try:
                out[mint] = float(value)
            except (TypeError, ValueError):
                continue
        return out
