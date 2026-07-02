from __future__ import annotations

from dataclasses import asdict, dataclass, field
from datetime import datetime
from typing import TYPE_CHECKING, Any, cast

from tradebot.core.aggregator import AggregatedScore
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskState
from tradebot.storage.repo import JsonStorage, Mode, Trade

if TYPE_CHECKING:
    from tradebot.core.decision import Observation


def trade_to_dict(t: Trade) -> dict[str, Any]:
    return {
        "pair": t.pair,
        "side": t.side,
        "base_amount": t.base_amount,
        "quote_amount": t.quote_amount,
        "price": t.price,
        "fee_quote": t.fee_quote,
        "slippage_pct": t.slippage_pct,
        "opened_at": t.opened_at.isoformat(),
        "confidence": t.confidence,
    }


def _annotate_trades(
    trades_chrono: list[Trade],
) -> tuple[list[dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    """Walk trades chronologically per pair to assign badge + realized_pnl + lineage.

    Returns:
        annotated_chrono: per-trade dicts with `badge` and `realized_pnl` set.
        open_lineages: pair -> list of leg dicts for pairs whose position is currently open.
    """
    state: dict[str, dict[str, Any]] = {}  # pair -> {base, cost_per_unit, lineage}
    annotated: list[dict[str, Any]] = []

    for t in trades_chrono:
        d = trade_to_dict(t)
        ps = state.setdefault(t.pair, {"base": 0.0, "cost_per_unit": 0.0, "lineage": []})
        if t.side == "buy":
            badge = "OPEN" if ps["base"] <= 1e-9 else "ADD"
            if ps["base"] <= 1e-9:
                ps["lineage"] = []  # fresh position starts a new lineage
            new_base = ps["base"] + t.base_amount
            ps["cost_per_unit"] = (
                ps["cost_per_unit"] * ps["base"] + t.price * t.base_amount
            ) / new_base
            ps["base"] = new_base
            d["badge"] = badge
            d["realized_pnl"] = None
        else:  # sell
            base_before = ps["base"]
            if base_before <= 1e-9:
                # Selling without a tracked position (shouldn't normally happen).
                d["badge"] = "CLOSE"
                d["realized_pnl"] = 0.0
            else:
                frac = min(1.0, t.base_amount / base_before)
                cost = ps["cost_per_unit"] * t.base_amount
                realized = t.quote_amount - cost
                ps["base"] = max(0.0, base_before - t.base_amount)
                if ps["base"] <= 1e-9:
                    sign = "+" if realized >= 0 else ""
                    d["badge"] = f"CLOSE {sign}${realized:.2f}"
                else:
                    d["badge"] = f"TRIM {round(frac * 100)}%"
                d["realized_pnl"] = realized

        ps["lineage"].append(
            {
                "ts": t.opened_at.isoformat(),
                "side": t.side,
                "base": t.base_amount,
                "quote": t.quote_amount,
                "price": t.price,
                "realized_pnl": d.get("realized_pnl"),
                "badge": d["badge"],
            }
        )
        annotated.append(d)
        if t.side == "sell" and ps["base"] <= 1e-9:
            ps["lineage"] = []  # position fully closed; lineage reserved for next OPEN

    open_lineages = {
        pair: ps["lineage"] for pair, ps in state.items() if ps["base"] > 1e-9
    }
    return annotated, open_lineages


@dataclass
class DashboardSnapshot:
    mode: str
    now: str
    cash: float
    equity: float
    equity_high: float
    drawdown_pct: float
    realized_pnl_total: float
    sol_balance: float
    sol_gas_paid_total: float
    sol_mark: float
    kill_switch_active: bool
    kill_switch_reason: str
    positions: list[dict[str, Any]] = field(default_factory=list)
    recent_trades: list[dict[str, Any]] = field(default_factory=list)
    equity_history: list[dict[str, Any]] = field(default_factory=list)
    signals: list[dict[str, Any]] = field(default_factory=list)
    pair_charts: list[dict[str, Any]] = field(default_factory=list)
    decisions: list[dict[str, Any]] = field(default_factory=list)
    whale_activity: list[dict[str, Any]] = field(default_factory=list)
    limiter: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


async def build_snapshot(
    storage: JsonStorage,
    portfolio: Portfolio,
    risk_state: RiskState,
    marks: dict[str, float],
    scores: list[AggregatedScore],
    now: datetime,
    equity_history_limit: int = 200,
    recent_trades_limit: int = 50,
    mark_history: dict[str, list[tuple[datetime, float]]] | None = None,
    observations: list[Observation] | None = None,
    limiter_metrics: dict[str, Any] | None = None,
    whale_activity: list[dict[str, Any]] | None = None,
) -> DashboardSnapshot:
    equity = portfolio.equity(marks)
    portfolio.update_equity_high(equity)
    drawdown = portfolio.drawdown_pct(equity)

    positions = []
    for pos in portfolio.open_positions():
        mark = marks.get(pos.pair, pos.avg_entry_price)
        unrealized = (mark - pos.avg_entry_price) * pos.base_amount
        unrealized_pct = (
            (mark - pos.avg_entry_price) / pos.avg_entry_price if pos.avg_entry_price > 0 else 0.0
        )
        positions.append(
            {
                "pair": pos.pair,
                "base_amount": pos.base_amount,
                "avg_entry_price": pos.avg_entry_price,
                "mark_price": mark,
                "unrealized_pnl_quote": unrealized,
                "unrealized_pnl_pct": unrealized_pct,
            }
        )

    signals = [
        {
            "pair": s.pair,
            "composite": s.composite,
            "sampled_at": s.sampled_at.isoformat(),
            "components": [
                {
                    "signal": cs.signal,
                    "timeframe": cs.timeframe,
                    "score": cs.score,
                    "components": cs.components,
                }
                for cs in s.scores
            ],
        }
        for s in scores
    ]

    mode = cast("Mode", portfolio.mode)
    # Pull all trades for lineage walk (newest-first), then reverse to chronological.
    all_trades_newest_first = storage.list_trades(mode=mode, limit=10_000)
    trades_chrono = list(reversed(all_trades_newest_first))
    annotated_chrono, open_lineages = _annotate_trades(trades_chrono)
    # recent_trades wants newest-first, capped.
    recent_trades = list(reversed(annotated_chrono))[:recent_trades_limit]
    # Attach lineage to each open position dict.
    for pos_dict in positions:
        pos_dict["lineage"] = open_lineages.get(cast("str", pos_dict["pair"]), [])
    equity_history = storage.list_equity_snapshots(mode=mode, limit=equity_history_limit)

    pair_charts: list[dict[str, Any]] = []
    if mark_history:
        for pair, points in mark_history.items():
            if not points:
                continue
            series = [{"t": ts.isoformat(), "p": float(price)} for ts, price in points]
            first: float = series[0]["p"]  # type: ignore[assignment]
            last: float = series[-1]["p"]  # type: ignore[assignment]
            change_pct = ((last - first) / first) if first > 0 else 0.0
            high = max(p["p"] for p in series)  # type: ignore[type-var]
            low = min(p["p"] for p in series)  # type: ignore[type-var]
            pair_charts.append(
                {
                    "pair": pair,
                    "last": last,
                    "change_pct": change_pct,
                    "high": high,
                    "low": low,
                    "points": series,
                }
            )

    decisions: list[dict[str, Any]] = []
    if observations:
        for o in reversed(observations[-100:]):  # newest first, cap at 100 in snapshot
            decisions.append(
                {
                    "timestamp": o.timestamp,
                    "pair": o.pair,
                    "composite": o.composite,
                    "mark": o.mark,
                    "regime": o.regime,
                    "decision": o.decision,
                    "reason": o.reason,
                    "size_quote": o.size_quote,
                    "size_base": o.size_base,
                }
            )

    from tradebot.core.portfolio import DEFAULT_SOL_FALLBACK_PRICE, SOL_PAIR

    sol_mark = marks.get(SOL_PAIR, DEFAULT_SOL_FALLBACK_PRICE)

    snap = DashboardSnapshot(
        mode=portfolio.mode,
        now=now.isoformat(),
        cash=portfolio.cash,
        equity=equity,
        equity_high=portfolio.equity_high,
        drawdown_pct=drawdown,
        realized_pnl_total=portfolio.realized_pnl_total,
        sol_balance=portfolio.sol_balance,
        sol_gas_paid_total=portfolio.sol_gas_paid_total,
        sol_mark=sol_mark,
        kill_switch_active=risk_state.kill_switch_active,
        kill_switch_reason=risk_state.kill_switch_reason,
        positions=positions,
        recent_trades=recent_trades,
        equity_history=equity_history,
        signals=signals,
        pair_charts=pair_charts,
        decisions=decisions,
    )
    if limiter_metrics is not None:
        snap.limiter = limiter_metrics
    if whale_activity:
        snap.whale_activity = whale_activity
    return snap
