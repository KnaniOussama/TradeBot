from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime

from tradebot.core.portfolio import Portfolio
from tradebot.execution.base import Fill, Order


@dataclass
class SyntheticExecutor:
    fee_bps: int = 30
    slippage_bps: int = 5

    async def execute(
        self,
        order: Order,
        portfolio: Portfolio,
        now: datetime,
        mark: float,
    ) -> Fill:
        if order.side == "buy":
            eff_price = mark * (1 + self.slippage_bps / 10_000)
            quote_amount = order.size_quote
            base_amount = quote_amount / eff_price if eff_price > 0 else 0.0
        else:
            eff_price = mark * (1 - self.slippage_bps / 10_000)
            base_amount = order.size_base
            quote_amount = base_amount * eff_price
        fee_quote = quote_amount * (self.fee_bps / 10_000)
        portfolio.apply_fill(
            pair=order.pair,
            side=order.side,
            base_amount=base_amount,
            quote_amount=quote_amount,
            fee_quote=fee_quote,
        )
        return Fill(
            pair=order.pair,
            side=order.side,
            base_amount=base_amount,
            quote_amount=quote_amount,
            price=eff_price,
            fee_quote=fee_quote,
            slippage_pct=self.slippage_bps / 10_000,
            tx_signature=None,
            filled_at=now,
        )
