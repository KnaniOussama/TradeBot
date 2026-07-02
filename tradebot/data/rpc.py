from __future__ import annotations

import asyncio
from dataclasses import dataclass
from itertools import count
from types import TracebackType
from typing import Any

import httpx

LAMPORTS_PER_SOL = 1_000_000_000


class ConfirmationError(Exception):
    pass


@dataclass(frozen=True)
class LatestBlockhash:
    blockhash: str
    last_valid_block_height: int


class SolanaRpcClient:
    def __init__(self, url: str, timeout_s: float = 10.0) -> None:
        self._url = url
        self._timeout = timeout_s
        self._client: httpx.AsyncClient | None = None
        self._id_counter = count(1)

    async def __aenter__(self) -> SolanaRpcClient:
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

    async def _call(self, method: str, params: list[Any]) -> Any:
        if self._client is None:
            raise RuntimeError("SolanaRpcClient must be used as async context manager")
        body = {
            "jsonrpc": "2.0",
            "id": next(self._id_counter),
            "method": method,
            "params": params,
        }
        resp = await self._client.post(self._url, json=body)
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            err = data["error"]
            raise RuntimeError(f"RPC error {err.get('code')}: {err.get('message')}")
        return data["result"]

    async def get_balance_lamports(self, address: str) -> int:
        result = await self._call("getBalance", [address])
        return int(result["value"])

    async def get_balance_sol(self, address: str) -> float:
        return (await self.get_balance_lamports(address)) / LAMPORTS_PER_SOL

    async def get_token_accounts_by_owner(self, owner: str, mint: str) -> list[dict]:  # type: ignore[type-arg]
        result = await self._call(
            "getTokenAccountsByOwner",
            [owner, {"mint": mint}, {"encoding": "jsonParsed"}],
        )
        return list(result.get("value", []))

    async def send_raw_transaction(self, tx_b64: str, skip_preflight: bool = False) -> str:
        result = await self._call(
            "sendTransaction",
            [
                tx_b64,
                {
                    "encoding": "base64",
                    "skipPreflight": skip_preflight,
                    "preflightCommitment": "confirmed",
                },
            ],
        )
        return str(result)

    async def get_latest_blockhash(self) -> LatestBlockhash:
        result = await self._call("getLatestBlockhash", [{"commitment": "confirmed"}])
        v = result["value"]
        return LatestBlockhash(
            blockhash=str(v["blockhash"]),
            last_valid_block_height=int(v["lastValidBlockHeight"]),
        )

    async def get_signature_statuses(self, signatures: list[str]) -> list[dict | None]:  # type: ignore[type-arg]
        result = await self._call(
            "getSignatureStatuses",
            [signatures, {"searchTransactionHistory": True}],
        )
        return list(result.get("value", []))

    async def confirm_signature(
        self,
        signature: str,
        timeout_s: float = 30.0,
        poll_interval_s: float = 1.0,
    ) -> bool:
        deadline = asyncio.get_event_loop().time() + timeout_s
        while True:
            statuses = await self.get_signature_statuses([signature])
            entry = statuses[0] if statuses else None
            if entry is not None:
                if entry.get("err") is not None:
                    raise ConfirmationError(f"transaction {signature} failed: {entry['err']}")
                conf = entry.get("confirmationStatus")
                if conf in ("confirmed", "finalized"):
                    return True
            if asyncio.get_event_loop().time() >= deadline:
                raise ConfirmationError(
                    f"transaction {signature} not confirmed within {timeout_s}s"
                )
            await asyncio.sleep(poll_interval_s)
