from __future__ import annotations

import asyncio
from datetime import datetime

from tradebot.core.portfolio import Portfolio
from tradebot.data.jupiter import JupiterClient, JupiterQuote
from tradebot.execution.base import ExecutionError, Fill, Order
from tradebot.execution.gas import gas_cost_sol
from tradebot.logging_setup import get_logger
from tradebot.storage.repo import JsonStorage, Trade

log = get_logger("execution.demo")


class DemoExecutor:
    """Paper-trading executor that mirrors RealExecutor as faithfully as possible.

    Cost model:
      - Jupiter's `out_amount` is already net of LP fees + price impact, so
        cash flow is `gross_in -> net_out` with no extra fee subtraction.
      - Each fill burns SOL gas (base 5000 lamports + priority fee), deducted
        from a tracked SOL balance just like real mode.
      - The fill is re-quoted after `confirm_latency_s` to capture the
        between-quote price drift real mode experiences during on-chain
        confirmation. The decision uses the trigger quote; the fill uses the
        later quote. If drift exceeds `max_slippage_pct`, the fill is rejected
        (mirroring real-mode mid-flight slippage failure).
    """

    def __init__(
        self,
        jupiter: JupiterClient,
        storage: JsonStorage,
        base_mints: dict[str, tuple[str, int]],  # pair -> (mint, decimals)
        quote_mint: str,
        quote_decimals: int,
        max_slippage_pct: float = 1.0,
        priority_fee_microlamports: int = 0,
        confirm_latency_s: float = 1.0,
        # Accepted for backward-compat with old configs/tests, but ignored:
        # demo no longer simulates an extra fee on top of Jupiter's net out_amount.
        simulated_fee_bps: int | None = None,  # noqa: ARG002
    ) -> None:
        self._jup = jupiter
        self._storage = storage
        self._base_mints = base_mints
        self._quote_mint = quote_mint
        self._quote_decimals = quote_decimals
        self._max_slippage = max_slippage_pct
        self._priority_fee = priority_fee_microlamports
        self._confirm_latency = max(0.0, confirm_latency_s)

    def _resolve(self, pair: str) -> tuple[str, int]:
        if pair not in self._base_mints:
            raise ExecutionError(f"unknown pair: {pair}")
        return self._base_mints[pair]

    async def _quote(self, *, in_mint: str, out_mint: str, in_units: int) -> JupiterQuote:
        return await self._jup.quote(
            input_mint=in_mint,
            output_mint=out_mint,
            amount=in_units,
            slippage_bps=max(1, int(self._max_slippage * 10_000)),
        )

    async def execute(self, order: Order, portfolio: Portfolio, now: datetime) -> Fill:
        base_mint, base_decimals = self._resolve(order.pair)

        if order.side == "buy":
            if order.size_quote <= 0:
                raise ExecutionError("buy requires size_quote > 0")
            in_mint, out_mint = self._quote_mint, base_mint
            in_units = int(order.size_quote * (10**self._quote_decimals))
        elif order.side == "sell":
            if order.size_base <= 0:
                raise ExecutionError("sell requires size_base > 0")
            in_mint, out_mint = base_mint, self._quote_mint
            in_units = int(order.size_base * (10**base_decimals))
        else:
            raise ExecutionError(f"unknown side: {order.side}")

        # 1. Trigger quote — gates the decision (same role as RealExecutor's pre-check).
        trigger_q = await self._quote(in_mint=in_mint, out_mint=out_mint, in_units=in_units)
        if trigger_q.price_impact_pct > self._max_slippage:
            raise ExecutionError(
                f"slippage {trigger_q.price_impact_pct:.4f} > max {self._max_slippage}"
            )

        # 2. Simulate confirmation latency, then re-quote — the FILL uses this number.
        if self._confirm_latency > 0:
            await asyncio.sleep(self._confirm_latency)
        fill_q = await self._quote(in_mint=in_mint, out_mint=out_mint, in_units=in_units)

        # 3. Mid-flight drift gate. If the pool moved more than max_slippage between
        #    trigger and fill, real mode's tx would have been rejected on-chain.
        if trigger_q.out_amount <= 0:
            raise ExecutionError("trigger quote returned zero out_amount")
        drift_pct = abs(fill_q.out_amount - trigger_q.out_amount) / trigger_q.out_amount
        if drift_pct > self._max_slippage:
            raise ExecutionError(
                f"mid-flight drift {drift_pct:.4f} > max {self._max_slippage} "
                f"(trigger out={trigger_q.out_amount}, fill out={fill_q.out_amount})"
            )

        # 4. Compute realized amounts from the FILL quote.
        if order.side == "buy":
            base_amount = fill_q.out_amount / (10**base_decimals)
            quote_amount = order.size_quote
        else:
            base_amount = order.size_base
            quote_amount = fill_q.out_amount / (10**self._quote_decimals)

        if base_amount <= 0 or quote_amount <= 0:
            raise ExecutionError(
                f"non-positive realized amounts: base={base_amount}, quote={quote_amount}"
            )

        slippage_pct = fill_q.price_impact_pct
        fee_quote = 0.0
        price = quote_amount / base_amount

        # 5. Charge gas FIRST so a SOL-bankrupt wallet fails before mutating positions.
        sol_gas = gas_cost_sol(self._priority_fee)
        portfolio.charge_gas(sol_gas)

        # 6. Apply the fill.
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
            slippage_pct=slippage_pct,
            tx_signature=None,
            filled_at=now,
        )
        self._storage.append_trade(
            Trade(
                mode="demo",
                pair=order.pair,
                side=order.side,
                base_amount=base_amount,
                quote_amount=quote_amount,
                price=price,
                fee_quote=fee_quote,
                slippage_pct=slippage_pct,
                tx_signature=None,
                opened_at=now,
                confidence=None,
                notes=None,
            ),
        )
        log.info(
            "demo_fill",
            pair=order.pair,
            side=order.side,
            base=base_amount,
            quote=quote_amount,
            price=price,
            slippage=slippage_pct,
            drift=drift_pct,
            sol_gas=sol_gas,
            sol_balance=portfolio.sol_balance,
        )
        return fill
