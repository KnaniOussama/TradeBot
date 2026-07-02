from __future__ import annotations

from dataclasses import asdict, dataclass, field
from datetime import date as _date
from datetime import datetime
from pathlib import Path
from typing import Any, Literal

import pandas as pd

from tradebot.storage.atomic import atomic_write_json, read_json_or_default

Mode = Literal["demo", "real", "backtest"]
Side = Literal["buy", "sell"]


@dataclass
class Trade:
    mode: Mode
    pair: str
    side: Side
    base_amount: float
    quote_amount: float
    price: float
    fee_quote: float
    slippage_pct: float
    tx_signature: str | None
    opened_at: datetime
    confidence: float | None
    notes: str | None


@dataclass
class PositionRecord:
    pair: str
    base_amount: float
    avg_entry_price: float
    fees_paid_quote: float = 0.0


@dataclass
class PortfolioState:
    mode: Mode
    cash: float
    realized_pnl_total: float
    equity_high: float
    positions: list[PositionRecord] = field(default_factory=list)
    sol_balance: float = 0.0
    sol_gas_paid_total: float = 0.0


@dataclass
class OHLCVCandle:
    pair: str
    timeframe: str
    bucket_start: datetime
    open: float
    high: float
    low: float
    close: float
    volume_quote: float = 0.0


def _safe_pair(pair: str) -> str:
    return pair.replace("/", "--")


class JsonStorage:
    def __init__(self, root: Path) -> None:
        self._root = Path(root)
        self._root.mkdir(parents=True, exist_ok=True)

    # --- trades ---
    def _trades_path(self) -> Path:
        return self._root / "trades.json"

    def append_trade(self, trade: Trade) -> None:
        path = self._trades_path()
        existing = read_json_or_default(path, default=[])
        d = asdict(trade)
        d["opened_at"] = trade.opened_at.isoformat()
        existing.append(d)
        atomic_write_json(path, existing)

    def round_trip_returns(
        self, mode: Mode, pair: str | None = None, limit: int = 200
    ) -> list[float]:
        """Compute per-trade round-trip returns from trade history.

        Walks trades chronologically (storage is newest-first; we reverse).
        For each buy→sell pair:
            return = (sell_quote_net - buy_quote_cost) / buy_quote_cost
        where buy_quote_cost = quote_amount + fee_quote
        and   sell_quote_net = quote_amount - fee_quote.
        Partial sells are treated as full closes, documented approximation.
        """
        trades = self.list_trades(mode=mode, limit=limit * 2)
        if pair:
            trades = [t for t in trades if t.pair == pair]
        trades.reverse()  # chronological order
        returns: list[float] = []
        last_buy_quote: float | None = None
        last_buy_base: float | None = None
        for t in trades:
            if t.side == "buy":
                last_buy_quote = t.quote_amount + t.fee_quote
                last_buy_base = t.base_amount
            elif t.side == "sell" and last_buy_quote is not None and last_buy_base is not None:
                sell_quote = t.quote_amount - t.fee_quote
                ret = (sell_quote - last_buy_quote) / last_buy_quote
                returns.append(ret)
                last_buy_quote = None
                last_buy_base = None
        return returns

    def list_trades(self, mode: Mode, limit: int = 50) -> list[Trade]:
        rows = read_json_or_default(self._trades_path(), default=[])
        filtered = [r for r in rows if r.get("mode") == mode]
        filtered.reverse()  # newest first
        out: list[Trade] = []
        for r in filtered[:limit]:
            out.append(
                Trade(
                    mode=r["mode"],
                    pair=r["pair"],
                    side=r["side"],
                    base_amount=r["base_amount"],
                    quote_amount=r["quote_amount"],
                    price=r["price"],
                    fee_quote=r["fee_quote"],
                    slippage_pct=r["slippage_pct"],
                    tx_signature=r.get("tx_signature"),
                    opened_at=datetime.fromisoformat(r["opened_at"]),
                    confidence=r.get("confidence"),
                    notes=r.get("notes"),
                )
            )
        return out

    # --- portfolio state ---
    def _portfolio_path(self, mode: Mode) -> Path:
        return self._root / f"portfolio.{mode}.json"

    def save_portfolio_state(self, state: PortfolioState) -> None:
        d = {
            "mode": state.mode,
            "cash": state.cash,
            "realized_pnl_total": state.realized_pnl_total,
            "equity_high": state.equity_high,
            "sol_balance": state.sol_balance,
            "sol_gas_paid_total": state.sol_gas_paid_total,
            "positions": [asdict(p) for p in state.positions],
        }
        atomic_write_json(self._portfolio_path(state.mode), d)

    def load_portfolio_state(self, mode: Mode) -> PortfolioState | None:
        d = read_json_or_default(self._portfolio_path(mode), default=None)
        if d is None:
            return None
        return PortfolioState(
            mode=d["mode"],
            cash=d["cash"],
            realized_pnl_total=d["realized_pnl_total"],
            equity_high=d["equity_high"],
            sol_balance=d.get("sol_balance", 0.0),
            sol_gas_paid_total=d.get("sol_gas_paid_total", 0.0),
            positions=[PositionRecord(**p) for p in d.get("positions", [])],
        )

    # --- equity snapshots ---
    def _equity_path(self, mode: Mode) -> Path:
        return self._root / f"equity.{mode}.json"

    def append_equity_snapshot(
        self,
        mode: Mode,
        snapshot_at: datetime,
        equity: float,
        cash: float,
        positions_value: float,
    ) -> None:
        path = self._equity_path(mode)
        existing = read_json_or_default(path, default=[])
        existing.append(
            {
                "snapshot_at": snapshot_at.isoformat(),
                "equity": equity,
                "cash": cash,
                "positions_value": positions_value,
            }
        )
        atomic_write_json(path, existing)

    def list_equity_snapshots(self, mode: Mode, limit: int = 200) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = read_json_or_default(self._equity_path(mode), default=[])
        # chronological (oldest first), tail last `limit`
        if limit and len(rows) > limit:
            rows = rows[-limit:]
        return rows

    # --- ohlcv ---
    def _ohlcv_path(self, pair: str, timeframe: str) -> Path:
        return self._root / "ohlcv" / f"{_safe_pair(pair)}__{timeframe}.json"

    def upsert_ohlcv(self, candle: OHLCVCandle) -> None:
        path = self._ohlcv_path(candle.pair, candle.timeframe)
        existing = read_json_or_default(path, default=[])
        bucket_iso = candle.bucket_start.isoformat()
        # Replace if same bucket exists (last one, append-mostly assumption)
        idx = None
        for i in range(len(existing) - 1, -1, -1):
            if existing[i]["bucket_start"] == bucket_iso:
                idx = i
                break
        record = {
            "bucket_start": bucket_iso,
            "open": candle.open,
            "high": candle.high,
            "low": candle.low,
            "close": candle.close,
            "volume_quote": candle.volume_quote,
        }
        if idx is not None:
            existing[idx] = record
        else:
            existing.append(record)
        atomic_write_json(path, existing)

    def load_ohlcv(self, pair: str, timeframe: str, limit: int = 200) -> pd.DataFrame:
        rows = read_json_or_default(self._ohlcv_path(pair, timeframe), default=[])
        if limit and len(rows) > limit:
            rows = rows[-limit:]
        if not rows:
            return pd.DataFrame(columns=["timestamp", "open", "high", "low", "close", "volume"])
        return pd.DataFrame(
            {
                "timestamp": [datetime.fromisoformat(r["bucket_start"]) for r in rows],
                "open": [r["open"] for r in rows],
                "high": [r["high"] for r in rows],
                "low": [r["low"] for r in rows],
                "close": [r["close"] for r in rows],
                "volume": [r["volume_quote"] for r in rows],
            }
        )

    def load_ohlcv_for_pairs(
        self,
        pairs: list[str],
        timeframes: list[str],
        limit: int = 200,
    ) -> dict[str, dict[str, pd.DataFrame]]:
        out: dict[str, dict[str, pd.DataFrame]] = {}
        for pair in pairs:
            out[pair] = {}
            for tf in timeframes:
                out[pair][tf] = self.load_ohlcv(pair=pair, timeframe=tf, limit=limit)
        return out

    # --- risk state ---
    def _risk_path(self, mode: Mode) -> Path:
        return self._root / f"risk_state.{mode}.json"

    def save_risk_state(self, mode: Mode, state: Any) -> None:
        d = {
            "trades_per_day": {k.isoformat(): v for k, v in state.trades_per_day.items()},
            "daily_start_equity": {k.isoformat(): v for k, v in state.daily_start_equity.items()},
            "weekly_start_equity": {k.isoformat(): v for k, v in state.weekly_start_equity.items()},
            "day_paused_until": (
                state.day_paused_until.isoformat() if state.day_paused_until else None
            ),
            "week_paused_until": (
                state.week_paused_until.isoformat() if state.week_paused_until else None
            ),
            "kill_switch_active": state.kill_switch_active,
            "kill_switch_reason": state.kill_switch_reason,
        }
        atomic_write_json(self._risk_path(mode), d)

    def load_risk_state(self, mode: Mode) -> Any | None:
        from tradebot.core.risk import RiskState  # local import to avoid cycle

        d = read_json_or_default(self._risk_path(mode), default=None)
        if d is None:
            return None
        return RiskState(
            trades_per_day={
                _date.fromisoformat(k): v for k, v in d.get("trades_per_day", {}).items()
            },
            daily_start_equity={
                _date.fromisoformat(k): v for k, v in d.get("daily_start_equity", {}).items()
            },
            weekly_start_equity={
                _date.fromisoformat(k): v for k, v in d.get("weekly_start_equity", {}).items()
            },
            day_paused_until=(
                _date.fromisoformat(d["day_paused_until"]) if d.get("day_paused_until") else None
            ),
            week_paused_until=(
                _date.fromisoformat(d["week_paused_until"]) if d.get("week_paused_until") else None
            ),
            kill_switch_active=bool(d.get("kill_switch_active", False)),
            kill_switch_reason=d.get("kill_switch_reason", ""),
        )

    # --- mark history (rolling chart cache) ---
    def _mark_history_path(self, mode: Mode) -> Path:
        return self._root / f"mark_history.{mode}.json"

    def save_mark_history(
        self,
        mode: Mode,
        history: dict[str, list[tuple[datetime, float]]],
    ) -> None:
        d = {
            pair: [{"t": ts.isoformat(), "p": float(price)} for ts, price in points]
            for pair, points in history.items()
        }
        atomic_write_json(self._mark_history_path(mode), d)

    def load_mark_history(
        self,
        mode: Mode,
    ) -> dict[str, list[tuple[datetime, float]]]:
        d = read_json_or_default(self._mark_history_path(mode), default={})
        out: dict[str, list[tuple[datetime, float]]] = {}
        for pair, points in d.items():
            out[pair] = [(datetime.fromisoformat(p["t"]), float(p["p"])) for p in points]
        return out
