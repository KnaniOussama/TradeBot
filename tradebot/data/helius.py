from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from types import TracebackType
from typing import Any

import httpx

from tradebot.logging_setup import get_logger

log = get_logger("data.helius")


@dataclass(frozen=True)
class TokenTransfer:
    signature: str
    timestamp: int
    mint: str
    from_addr: str
    to_addr: str
    amount: float


@dataclass(frozen=True)
class WhaleSwap:
    """One parsed swap event for a watched wallet (whale-tracker view).

    `in_mint` / `in_amount_raw` = what the whale spent.
    `out_mint` / `out_amount_raw` = what the whale received.
    """

    wallet: str
    timestamp: datetime
    signature: str
    in_mint: str
    out_mint: str
    in_amount_raw: int
    out_amount_raw: int


class HeliusClient:
    def __init__(
        self, api_key: str, base_url: str = "https://api.helius.xyz", timeout_s: float = 10.0
    ) -> None:
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout_s
        self._client: httpx.AsyncClient | None = None

    async def __aenter__(self) -> HeliusClient:
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

    async def recent_token_transfers(self, address: str, limit: int = 100) -> list[TokenTransfer]:
        if self._client is None:
            raise RuntimeError("HeliusClient must be used as async context manager")
        url = f"{self._base_url}/v0/addresses/{address}/transactions"
        params: dict[str, str | int] = {"api-key": self._api_key, "limit": limit}
        resp = await self._client.get(url, params=params)
        resp.raise_for_status()
        data = resp.json()
        out: list[TokenTransfer] = []
        for tx in data:
            sig = tx.get("signature", "")
            ts = int(tx.get("timestamp", 0))
            for tt in tx.get("tokenTransfers", []) or []:
                out.append(
                    TokenTransfer(
                        signature=sig,
                        timestamp=ts,
                        mint=tt.get("mint", ""),
                        from_addr=tt.get("fromUserAccount", "") or "",
                        to_addr=tt.get("toUserAccount", "") or "",
                        amount=float(tt.get("tokenAmount", 0.0) or 0.0),
                    )
                )
        return out


def filter_for_mint(transfers: list[TokenTransfer], mint: str) -> list[TokenTransfer]:
    return [t for t in transfers if t.mint == mint]


def _amount_to_raw(transfer: dict[str, Any]) -> int:
    """Best-effort SPL amount → smallest-units int.

    Prefers `rawTokenAmount.tokenAmount` when present (lossless). Falls back to
    `tokenAmount × 1e6` (works for USDC and most 6-dec SPL tokens; over/underestimates
    other decimals; used only for relative ranking, not accounting).
    """
    raw = transfer.get("rawTokenAmount")
    if isinstance(raw, dict):
        amt = raw.get("tokenAmount")
        if amt is not None:
            try:
                return int(amt)
            except (TypeError, ValueError):
                pass
    ta = transfer.get("tokenAmount")
    if isinstance(ta, (int, float)):
        return int(float(ta) * 1_000_000)
    return 0


def _parse_swap_tx(wallet: str, tx: dict[str, Any]) -> WhaleSwap | None:
    """Convert a Helius enhanced-tx JSON entry (type=SWAP) into a WhaleSwap.

    Wallet-relative legs:
      - outgoing transfer (fromUserAccount=wallet) = what the whale spent
      - incoming transfer (toUserAccount=wallet)   = what the whale received
    Returns None if either leg is missing or amounts are zero.
    """
    transfers = tx.get("tokenTransfers") or []
    if not transfers:
        return None
    out_leg = next((t for t in transfers if t.get("fromUserAccount") == wallet), None)
    in_leg = next((t for t in transfers if t.get("toUserAccount") == wallet), None)
    if out_leg is None or in_leg is None:
        return None
    spent_mint = str(out_leg.get("mint") or "")
    received_mint = str(in_leg.get("mint") or "")
    if not spent_mint or not received_mint:
        return None
    try:
        ts = datetime.fromtimestamp(int(tx.get("timestamp", 0)), tz=UTC)
    except (TypeError, ValueError, OSError):
        return None
    spent_raw = _amount_to_raw(out_leg)
    received_raw = _amount_to_raw(in_leg)
    if spent_raw <= 0 or received_raw <= 0:
        return None
    return WhaleSwap(
        wallet=wallet,
        timestamp=ts,
        signature=str(tx.get("signature", "")),
        in_mint=spent_mint,
        out_mint=received_mint,
        in_amount_raw=spent_raw,
        out_amount_raw=received_raw,
    )


async def get_recent_swaps_for_wallet(
    client: HeliusClient, *, address: str, limit: int = 20
) -> list[WhaleSwap]:
    """Fetch the most recent SWAP-type transactions for one wallet."""
    if client._client is None:  # noqa: SLF001
        raise RuntimeError("HeliusClient must be used as async context manager")
    url = f"{client._base_url}/v0/addresses/{address}/transactions"  # noqa: SLF001
    params: dict[str, Any] = {
        "api-key": client._api_key,  # noqa: SLF001
        "type": "SWAP",
        "limit": str(limit),
    }
    resp = await client._client.get(url, params=params)  # noqa: SLF001
    if resp.status_code != 200:
        log.warning(
            "helius_swaps_http_error",
            address=address,
            status=resp.status_code,
            body=resp.text[:200],
        )
        return []
    try:
        payload = resp.json()
    except ValueError:
        log.warning("helius_swaps_bad_json", address=address)
        return []
    if not isinstance(payload, list):
        return []
    out: list[WhaleSwap] = []
    for tx in payload:
        if not isinstance(tx, dict):
            continue
        parsed = _parse_swap_tx(address, tx)
        if parsed is not None:
            out.append(parsed)
    return out
