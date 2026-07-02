from __future__ import annotations

from dataclasses import dataclass, field
from datetime import date, datetime, timedelta
from typing import Literal

from tradebot.config.models import RiskConfig
from tradebot.core.portfolio import Portfolio


@dataclass
class RiskState:
    trades_per_day: dict[date, int] = field(default_factory=dict)
    daily_start_equity: dict[date, float] = field(default_factory=dict)
    weekly_start_equity: dict[date, float] = field(default_factory=dict)
    day_paused_until: date | None = None
    week_paused_until: date | None = None
    kill_switch_active: bool = False
    kill_switch_reason: str = ""


@dataclass(frozen=True)
class RiskDecision:
    allowed: bool
    size_quote: float
    reason: str = ""


def _week_start(d: date) -> date:
    return d - timedelta(days=d.weekday())


class RiskManager:
    def __init__(self, cfg: RiskConfig) -> None:
        self._cfg = cfg

    def size_for(self, confidence: float, available_cash: float) -> float:
        # Confidence in [0, 1] maps linearly up to size_max.
        # At c=0.6 -> 30% (size_min), at c=1.0 -> 50% (size_max).
        # Formula: frac = c * size_max, clamped to [size_min, size_max].
        c = max(0.0, min(1.0, confidence))
        frac = c * self._cfg.per_trade_size_max
        # Clamp to [size_min, size_max] when positive
        if frac > 0:
            frac = max(self._cfg.per_trade_size_min, min(self._cfg.per_trade_size_max, frac))
        return available_cash * frac

    def evaluate_entry(
        self,
        pair: str,
        confidence: float,
        portfolio: Portfolio,
        state: RiskState,
        slippage_pct: float,
        now: datetime,
    ) -> RiskDecision:
        today = now.date()

        if state.kill_switch_active:
            return RiskDecision(False, 0.0, f"kill switch active: {state.kill_switch_reason}")
        if state.day_paused_until is not None and today < state.day_paused_until:
            return RiskDecision(False, 0.0, f"day paused until {state.day_paused_until}")
        if state.week_paused_until is not None and today < state.week_paused_until:
            return RiskDecision(False, 0.0, f"week paused until {state.week_paused_until}")
        if slippage_pct > self._cfg.max_slippage_pct:
            return RiskDecision(
                False, 0.0, f"slippage {slippage_pct:.4f} > max {self._cfg.max_slippage_pct}"
            )
        if state.trades_per_day.get(today, 0) >= self._cfg.max_trades_per_day:
            return RiskDecision(False, 0.0, "max trades per day reached")
        if portfolio.position_for(pair) is not None:
            return RiskDecision(False, 0.0, f"position already open for {pair}")
        if len(portfolio.open_positions()) >= self._cfg.max_concurrent_positions:
            return RiskDecision(False, 0.0, "max concurrent positions reached")

        size = self.size_for(confidence=confidence, available_cash=portfolio.cash)
        if size <= 0:
            return RiskDecision(False, 0.0, "sized to zero (no cash)")

        return RiskDecision(True, size, "ok")

    def update_state(
        self,
        portfolio: Portfolio,
        state: RiskState,
        current_equity: float,
        now: datetime,
    ) -> None:
        today = now.date()
        # Track day-start equity
        state.daily_start_equity.setdefault(today, current_equity)
        week_start = _week_start(today)
        state.weekly_start_equity.setdefault(week_start, current_equity)

        # Update equity high
        portfolio.update_equity_high(current_equity)

        # Drawdown circuit
        dd = portfolio.drawdown_pct(current_equity)
        if dd >= self._cfg.drawdown_circuit_pct:
            state.kill_switch_active = True
            state.kill_switch_reason = f"drawdown {dd:.2%} >= {self._cfg.drawdown_circuit_pct:.2%}"
            return

        # Daily loss limit
        day_start = state.daily_start_equity[today]
        if day_start > 0:
            daily_pnl_pct = (current_equity - day_start) / day_start
            if daily_pnl_pct <= -self._cfg.daily_loss_limit_pct:
                state.day_paused_until = today + timedelta(days=1)

        # Weekly loss limit
        week_start_eq = state.weekly_start_equity[week_start]
        if week_start_eq > 0:
            weekly_pnl_pct = (current_equity - week_start_eq) / week_start_eq
            if weekly_pnl_pct <= -self._cfg.weekly_loss_limit_pct:
                state.week_paused_until = week_start + timedelta(days=7)

    def record_trade(self, state: RiskState, now: datetime) -> None:
        today = now.date()
        state.trades_per_day[today] = state.trades_per_day.get(today, 0) + 1

    def check_per_trade_kill(self, unrealized_loss_pct: float) -> Literal["ok", "kill"]:
        if unrealized_loss_pct >= self._cfg.per_trade_kill_pct:
            return "kill"
        return "ok"
