from __future__ import annotations

import base64
from collections.abc import Awaitable, Callable
from datetime import datetime

from solders.keypair import Keypair as SoldersKeypair
from solders.transaction import VersionedTransaction

from tradebot.core.portfolio import Portfolio
from tradebot.data.jupiter import JupiterClient
from tradebot.data.rpc import SolanaRpcClient
from tradebot.execution.base import ExecutionError, Fill, Order
from tradebot.execution.gas import gas_cost_sol
from tradebot.logging_setup import get_logger
from tradebot.storage.repo import JsonStorage, Trade
from tradebot.wallet.keypair import BotKeypair, to_solders

log = get_logger("execution.real")

SignAndSend = Callable[..., Awaitable[str]]


async def _default_sign_and_send(
    *,
    serialized_tx_b64: str,
    keypair: BotKeypair,
    rpc: SolanaRpcClient,
) -> str:
    raw = base64.b64decode(serialized_tx_b64)
    unsigned_tx = VersionedTransaction.from_bytes(raw)
    signer: SoldersKeypair = to_solders(keypair)
    signed_tx = VersionedTransaction(unsigned_tx.message, [signer])
    signed_b64 = base64.b64encode(bytes(signed_tx)).decode("ascii")
    return await rpc.send_raw_transaction(signed_b64, skip_preflight=False)


class RealExecutor:
    def __init__(
        self,
        jupiter: JupiterClient,
        rpc: SolanaRpcClient,
        storage: JsonStorage,
        keypair: BotKeypair,
        base_mints: dict[str, tuple[str, int]],
        quote_mint: str,
        quote_decimals: int,
        max_slippage_pct: float,
        priority_fee_microlamports: int,
        confirmation_timeout_s: float = 30.0,
        sign_and_send: SignAndSend | None = None,
    ) -> None:
        self._jup = jupiter
        self._rpc = rpc
        self._storage = storage
        self._keypair = keypair
        self._base_mints = base_mints
        self._quote_mint = quote_mint
        self._quote_decimals = quote_decimals
        self._max_slippage = max_slippage_pct
        self._priority_fee = priority_fee_microlamports
        self._confirm_timeout = confirmation_timeout_s
        self._sign_and_send: SignAndSend = sign_and_send or _default_sign_and_send

    def _resolve(self, pair: str) -> tuple[str, int]:
        if pair not in self._base_mints:
            raise ExecutionError(f"unknown pair: {pair}")
        return self._base_mints[pair]

    async def execute(self, order: Order, portfolio: Portfolio, now: datetime) -> Fill:
        base_mint, base_decimals = self._resolve(order.pair)

        if order.side == "buy":
            in_mint = self._quote_mint
            out_mint = base_mint
            in_units = int(order.size_quote * (10**self._quote_decimals))
            if in_units <= 0:
                raise ExecutionError("buy requires size_quote > 0")
        elif order.side == "sell":
            in_mint = base_mint
            out_mint = self._quote_mint
            in_units = int(order.size_base * (10**base_decimals))
            if in_units <= 0:
                raise ExecutionError("sell requires size_base > 0")
        else:
            raise ExecutionError(f"unknown side: {order.side}")

        slippage_bps = max(1, int(self._max_slippage * 10_000))
        quote = await self._jup.quote(
            input_mint=in_mint,
            output_mint=out_mint,
            amount=in_units,
            slippage_bps=slippage_bps,
        )
        if quote.price_impact_pct > self._max_slippage:
            raise ExecutionError(
                f"slippage {quote.price_impact_pct:.4f} > max {self._max_slippage}"
            )

        swap = await self._jup.build_swap(
            quote=quote,
            user_pubkey=self._keypair.address,
            priority_fee_microlamports=self._priority_fee,
            wrap_and_unwrap_sol=True,
        )
        signature = await self._sign_and_send(
            serialized_tx_b64=swap.serialized_tx_b64,
            keypair=self._keypair,
            rpc=self._rpc,
        )
        await self._rpc.confirm_signature(
            signature,
            timeout_s=self._confirm_timeout,
            poll_interval_s=1.0,
        )

        if order.side == "buy":
            base_amount = quote.out_amount / (10**base_decimals)
            quote_amount = order.size_quote
        else:
            base_amount = order.size_base
            quote_amount = quote.out_amount / (10**self._quote_decimals)

        fee_quote = 0.0  # LP fees already inside out_amount; SOL gas charged below.
        price = (quote_amount / base_amount) if base_amount > 0 else 0.0

        # Charge SOL gas (base + priority). On-chain tx already burned this; we
        # mirror it in the portfolio so equity reflects reality.
        sol_gas = gas_cost_sol(self._priority_fee)
        portfolio.charge_gas(sol_gas)

        portfolio.apply_fill(
            pair=order.pair,
            side=order.side,
            base_amount=base_amount,
            quote_amount=quote_amount,
            fee_quote=fee_quote,
        )

        fill = Fill(
            pair=order.pair,
            side=order.side,
            base_amount=base_amount,
            quote_amount=quote_amount,
            price=price,
            fee_quote=fee_quote,
            slippage_pct=quote.price_impact_pct,
            tx_signature=signature,
            filled_at=now,
        )

        self._storage.append_trade(
            Trade(
                mode="real",
                pair=order.pair,
                side=order.side,
                base_amount=base_amount,
                quote_amount=quote_amount,
                price=price,
                fee_quote=fee_quote,
                slippage_pct=quote.price_impact_pct,
                tx_signature=signature,
                opened_at=now,
                confidence=None,
                notes=None,
            ),
        )

        log.info(
            "real_fill",
            pair=order.pair,
            side=order.side,
            signature=signature,
            base=base_amount,
            quote=quote_amount,
            slippage=quote.price_impact_pct,
        )
        return fill
