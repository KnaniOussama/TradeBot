from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class PortfolioError(Exception):
    pass


@dataclass
class _Position:
    pair: str
    base_amount: float
    avg_entry_price: float
    fees_paid_quote: float = 0.0


SOL_PAIR = "SOL/USDC"
DEFAULT_SOL_FALLBACK_PRICE = 140.0


@dataclass
class Portfolio:
    mode: str
    starting_cash: float
    starting_sol_balance: float = 0.0
    cash: float = field(init=False)
    sol_balance: float = field(init=False)
    sol_gas_paid_total: float = 0.0  # cumulative SOL spent on gas
    realized_pnl_total: float = 0.0
    equity_high: float = field(init=False)
    _positions: dict[str, _Position] = field(default_factory=dict, init=False)

    def __post_init__(self) -> None:
        self.cash = self.starting_cash
        self.sol_balance = self.starting_sol_balance
        # Equity_high seeded from starting_cash only: SOL contribution depends on a mark
        # we don't have at construction time. update_equity_high() catches up on first cycle.
        self.equity_high = self.starting_cash

    def position_for(self, pair: str) -> _Position | None:
        return self._positions.get(pair)

    def open_positions(self) -> list[_Position]:
        return list(self._positions.values())

    def charge_gas(self, sol_amount: float) -> None:
        """Deduct SOL spent on transaction fees. Raises if insufficient SOL."""
        if sol_amount < 0:
            raise PortfolioError(f"negative gas charge: {sol_amount}")
        if sol_amount == 0:
            return
        if sol_amount > self.sol_balance + 1e-12:
            raise PortfolioError(
                f"insufficient SOL for gas: need {sol_amount}, have {self.sol_balance}"
            )
        self.sol_balance -= sol_amount
        self.sol_gas_paid_total += sol_amount

    def apply_fill(
        self,
        pair: str,
        side: str,
        base_amount: float,
        quote_amount: float,
        fee_quote: float,
    ) -> float:
        """Apply a fill. Returns realized P&L for this fill (0 for buys, gain/loss for sells)."""
        if base_amount <= 0 or quote_amount <= 0:
            raise PortfolioError(f"non-positive amounts: base={base_amount}, quote={quote_amount}")
        price = quote_amount / base_amount

        if side == "buy":
            total_out = quote_amount + fee_quote
            if total_out > self.cash + 1e-9:
                raise PortfolioError(f"insufficient cash: need {total_out}, have {self.cash}")
            self.cash -= total_out
            existing = self._positions.get(pair)
            if existing is None:
                self._positions[pair] = _Position(
                    pair=pair,
                    base_amount=base_amount,
                    avg_entry_price=price,
                    fees_paid_quote=fee_quote,
                )
            else:
                new_base = existing.base_amount + base_amount
                existing.avg_entry_price = (
                    existing.avg_entry_price * existing.base_amount + price * base_amount
                ) / new_base
                existing.base_amount = new_base
                existing.fees_paid_quote += fee_quote
            return 0.0

        if side == "sell":
            existing = self._positions.get(pair)
            if existing is None:
                raise PortfolioError(f"no position to sell: {pair}")
            if base_amount > existing.base_amount + 1e-9:
                raise PortfolioError(f"oversell: trying {base_amount}, have {existing.base_amount}")
            cost_basis = existing.avg_entry_price * base_amount
            realized = quote_amount - cost_basis - fee_quote
            self.cash += quote_amount - fee_quote
            self.realized_pnl_total += realized
            existing.base_amount -= base_amount
            if existing.base_amount <= 1e-9:
                del self._positions[pair]
            return realized

        raise PortfolioError(f"unknown side: {side}")

    def equity(self, marks: dict[str, float]) -> float:
        total = self.cash
        for pos in self._positions.values():
            mark = marks.get(pos.pair, pos.avg_entry_price)
            total += pos.base_amount * mark
        if self.sol_balance > 0:
            sol_mark = marks.get(SOL_PAIR, DEFAULT_SOL_FALLBACK_PRICE)
            total += self.sol_balance * sol_mark
        return total

    def update_equity_high(self, current_equity: float) -> None:
        if current_equity > self.equity_high:
            self.equity_high = current_equity

    def drawdown_pct(self, current_equity: float) -> float:
        if self.equity_high <= 0:
            return 0.0
        return max(0.0, (self.equity_high - current_equity) / self.equity_high)

    @classmethod
    def from_state(cls, state: Any) -> Portfolio:
        # state: PortfolioState (late import in caller to avoid cycle)
        p = cls(
            mode=state.mode,
            starting_cash=state.cash,
            starting_sol_balance=getattr(state, "sol_balance", 0.0),
        )
        p.realized_pnl_total = state.realized_pnl_total
        p.equity_high = state.equity_high
        p.sol_gas_paid_total = getattr(state, "sol_gas_paid_total", 0.0)
        for pr in state.positions:
            p._positions[pr.pair] = _Position(  # noqa: SLF001
                pair=pr.pair,
                base_amount=pr.base_amount,
                avg_entry_price=pr.avg_entry_price,
                fees_paid_quote=pr.fees_paid_quote,
            )
        return p
