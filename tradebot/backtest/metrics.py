from __future__ import annotations

import math
from datetime import datetime
from typing import Any


def compute_metrics(
    equity_curve: list[tuple[datetime, float]],
    trades: list[Any],
    starting_cash: float,
    bar_seconds: int = 60,
) -> dict[str, float | int]:
    if not equity_curve:
        return {
            "final_equity": starting_cash,
            "n_wins": 0,
            "n_losses": 0,
            "max_drawdown_pct": 0.0,
            "sharpe": 0.0,
            "total_return_pct": 0.0,
        }
    equities = [e for _, e in equity_curve]
    final_equity = equities[-1]
    total_return = (final_equity - starting_cash) / starting_cash if starting_cash > 0 else 0.0

    # Max drawdown
    peak = equities[0]
    max_dd = 0.0
    for e in equities:
        if e > peak:
            peak = e
        dd = (peak - e) / peak if peak > 0 else 0.0
        if dd > max_dd:
            max_dd = dd

    # Sharpe — per-bar returns annualized
    if len(equities) < 2:
        sharpe = 0.0
    else:
        rets = [(equities[i] / equities[i - 1]) - 1.0 for i in range(1, len(equities))]
        mean = sum(rets) / len(rets)
        var = sum((r - mean) ** 2 for r in rets) / len(rets)
        std = math.sqrt(var)
        bars_per_year = 365 * 24 * 3600 / bar_seconds
        sharpe = (mean / std) * math.sqrt(bars_per_year) if std > 0 else 0.0

    # Win/loss from round trips
    wins = 0
    losses = 0
    last_buy_price = None
    for t in trades:
        side = getattr(t, "side", None) or t["side"]
        price = getattr(t, "price", None) or t["price"]
        if side == "buy":
            last_buy_price = price
        elif side == "sell" and last_buy_price is not None:
            if price > last_buy_price:
                wins += 1
            else:
                losses += 1
            last_buy_price = None

    return {
        "final_equity": final_equity,
        "total_return_pct": total_return,
        "n_wins": wins,
        "n_losses": losses,
        "max_drawdown_pct": max_dd,
        "sharpe": sharpe,
    }
