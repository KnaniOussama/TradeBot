from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Literal, Protocol

from tradebot.core.portfolio import Portfolio


class ExecutionError(Exception):
    pass


@dataclass(frozen=True)
class Order:
    pair: str
    side: Literal["buy", "sell"]
    size_quote: float = 0.0  # for buys
    size_base: float = 0.0  # for sells


@dataclass(frozen=True)
class Fill:
    pair: str
    side: Literal["buy", "sell"]
    base_amount: float
    quote_amount: float
    price: float
    fee_quote: float
    slippage_pct: float
    tx_signature: str | None
    filled_at: datetime


class Executor(Protocol):
    async def execute(self, order: Order, portfolio: Portfolio, now: datetime) -> Fill: ...
