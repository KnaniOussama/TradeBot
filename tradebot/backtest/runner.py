from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

import pandas as pd

from tradebot.backtest.executor import SyntheticExecutor
from tradebot.backtest.metrics import compute_metrics
from tradebot.config.models import RiskConfig
from tradebot.core.aggregator import SignalAggregator
from tradebot.core.decision import DecisionEngine
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskManager, RiskState
from tradebot.execution.base import Order
from tradebot.signals.base import MarketContext


@dataclass
class BacktestTrade:
    timestamp: str
    side: str
    base_amount: float
    quote_amount: float
    price: float
    fee_quote: float


@dataclass
class BacktestResult:
    id: str
    pair: str
    starting_cash: float
    final_equity: float
    total_return_pct: float
    realized_pnl: float
    n_trades: int
    n_wins: int
    n_losses: int
    max_drawdown_pct: float
    sharpe: float
    bars_processed: int
    equity_curve: list[dict[str, Any]] = field(default_factory=list)
    trades: list[BacktestTrade] = field(default_factory=list)
    params: dict[str, Any] = field(default_factory=dict)
    completed_at: str = ""


@dataclass
class BacktestParams:
    pair: str
    timeframe: str = "1m"
    starting_cash: float = 50.0
    fee_bps: int = 30
    slippage_bps: int = 5
    warmup_bars: int = 50
    entry_threshold: float = 0.6
    exit_flip_threshold: float = -0.3
    bar_seconds: int = 60


async def run_backtest(
    *,
    ohlcv: pd.DataFrame,
    params: BacktestParams,
    signals_factory: Callable[[], list[Any]],
    timeframe_weights: dict[str, float],
    signal_weights: dict[str, float],
    risk: RiskConfig,
    backtest_id: str,
) -> BacktestResult:
    portfolio = Portfolio(mode="backtest", starting_cash=params.starting_cash)
    state = RiskState()
    risk_mgr = RiskManager(risk)
    aggregator = SignalAggregator(
        signals=signals_factory(),
        timeframe_weights=timeframe_weights,
        signal_weights=signal_weights,
    )
    engine = DecisionEngine(
        risk=risk_mgr,
        entry_threshold=params.entry_threshold,
        exit_flip_threshold=params.exit_flip_threshold,
    )
    executor = SyntheticExecutor(fee_bps=params.fee_bps, slippage_bps=params.slippage_bps)

    trades: list[BacktestTrade] = []
    equity_curve: list[tuple[datetime, float]] = []

    for i in range(params.warmup_bars, len(ohlcv)):
        bar = ohlcv.iloc[i]
        ts_val = bar["timestamp"]
        now = ts_val.to_pydatetime() if hasattr(ts_val, "to_pydatetime") else ts_val
        window = ohlcv.iloc[: i + 1]
        ctx = MarketContext(pair=params.pair, now=now, ohlcv={params.timeframe: window})
        agg_score = await aggregator.aggregate(ctx)
        marks = {params.pair: float(bar["close"])}
        slippages = {params.pair: params.slippage_bps / 10_000}
        risk_mgr.update_state(portfolio, state, current_equity=portfolio.equity(marks), now=now)
        actions, _observations = engine.decide(
            scores=[agg_score],
            marks=marks,
            slippages=slippages,
            portfolio=portfolio,
            state=state,
            now=now,
        )
        for action in actions:
            order = (
                Order(pair=params.pair, side="buy", size_quote=action.size_quote)
                if action.kind == "enter"
                else Order(pair=params.pair, side="sell", size_base=action.size_base)
            )
            fill = await executor.execute(
                order=order, portfolio=portfolio, now=now, mark=marks[params.pair]
            )
            risk_mgr.record_trade(state=state, now=now)
            trades.append(
                BacktestTrade(
                    timestamp=now.isoformat(),
                    side=fill.side,
                    base_amount=fill.base_amount,
                    quote_amount=fill.quote_amount,
                    price=fill.price,
                    fee_quote=fill.fee_quote,
                )
            )
        equity_curve.append((now, portfolio.equity(marks)))

    metrics = compute_metrics(
        equity_curve=equity_curve,
        trades=trades,
        starting_cash=params.starting_cash,
        bar_seconds=params.bar_seconds,
    )
    return BacktestResult(
        id=backtest_id,
        pair=params.pair,
        starting_cash=params.starting_cash,
        final_equity=float(metrics["final_equity"]),
        total_return_pct=float(metrics["total_return_pct"]),
        realized_pnl=portfolio.realized_pnl_total,
        n_trades=len(trades),
        n_wins=int(metrics["n_wins"]),
        n_losses=int(metrics["n_losses"]),
        max_drawdown_pct=float(metrics["max_drawdown_pct"]),
        sharpe=float(metrics["sharpe"]),
        bars_processed=len(ohlcv) - params.warmup_bars,
        equity_curve=[{"t": ts.isoformat(), "e": eq} for ts, eq in equity_curve],
        trades=trades,
        params={
            "pair": params.pair,
            "timeframe": params.timeframe,
            "starting_cash": params.starting_cash,
            "fee_bps": params.fee_bps,
            "slippage_bps": params.slippage_bps,
            "warmup_bars": params.warmup_bars,
            "entry_threshold": params.entry_threshold,
            "exit_flip_threshold": params.exit_flip_threshold,
        },
        completed_at=datetime.now().isoformat(),
    )
