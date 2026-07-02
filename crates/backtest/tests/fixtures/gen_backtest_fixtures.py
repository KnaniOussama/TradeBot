"""Generates numeric-parity fixtures for the Rust backtest port.

Runs the real Python tradebot.backtest.metrics.compute_metrics function on a
fixed synthetic equity curve/trade list, and the real
tradebot.backtest.runner.run_backtest end-to-end (with the real TASignal and
default RiskConfig) against tests/fixtures/backtest_sol_uptrend.csv, and
dumps both results to JSON so the Rust port can be asserted against them.

Run with the project venv:
    .venv/Scripts/python.exe crates/backtest/tests/fixtures/gen_backtest_fixtures.py
"""

import asyncio
import json
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pandas as pd

from tradebot.backtest.metrics import compute_metrics
from tradebot.backtest.runner import BacktestParams, run_backtest
from tradebot.config.models import RiskConfig
from tradebot.signals.ta import TASignal

ROOT = Path("G:/TradeBot")
FIXTURES_DIR = ROOT / "tests" / "fixtures"
OUT_PATH = ROOT / "crates" / "backtest" / "tests" / "fixtures" / "backtest_parity.json"


def gen_metrics_golden() -> dict:
    """A fixed equity curve + trade list, run through the real compute_metrics."""
    base = datetime(2026, 5, 1, tzinfo=UTC)
    equity_values = [
        100.0, 102.5, 101.0, 105.25, 103.75, 108.0, 106.5, 99.0, 101.25, 97.5,
        100.0, 104.0, 110.5, 108.25, 112.0, 109.75, 115.0, 113.5, 118.25, 121.0,
    ]
    curve = [(base + timedelta(minutes=i), v) for i, v in enumerate(equity_values)]
    trades = [
        {"side": "buy", "price": 100.0},
        {"side": "sell", "price": 108.0},
        {"side": "buy", "price": 106.5},
        {"side": "sell", "price": 99.0},
        {"side": "buy", "price": 101.25},
        {"side": "sell", "price": 118.25},
    ]
    result = compute_metrics(
        equity_curve=curve, trades=trades, starting_cash=100.0, bar_seconds=60
    )
    return {
        "equity_values": equity_values,
        "starting_cash": 100.0,
        "bar_seconds": 60,
        "result": result,
    }


async def gen_runner_parity(entry_threshold: float) -> dict:
    """End-to-end backtest against the real fixture CSV with the real TASignal."""
    df = pd.read_csv(FIXTURES_DIR / "backtest_sol_uptrend.csv")
    df["timestamp"] = pd.to_datetime(df["timestamp"], utc=True)

    params = BacktestParams(
        pair="SOL/USDC",
        timeframe="1m",
        starting_cash=100.0,
        fee_bps=30,
        slippage_bps=5,
        warmup_bars=50,
        entry_threshold=entry_threshold,
        exit_flip_threshold=-0.3,
        bar_seconds=60,
    )
    result = await run_backtest(
        ohlcv=df,
        params=params,
        signals_factory=lambda: [TASignal(timeframe="1m")],
        timeframe_weights={"1m": 1.0},
        signal_weights={"ta": 1.0},
        risk=RiskConfig(),
        backtest_id="parity_run",
    )
    return {
        "entry_threshold": entry_threshold,
        "n_bars": len(df),
        "bars_processed": result.bars_processed,
        "n_trades": result.n_trades,
        "n_wins": result.n_wins,
        "n_losses": result.n_losses,
        "final_equity": result.final_equity,
        "total_return_pct": result.total_return_pct,
        "max_drawdown_pct": result.max_drawdown_pct,
        "sharpe": result.sharpe,
        "realized_pnl": result.realized_pnl,
        "trades": [
            {
                "side": t.side,
                "base_amount": t.base_amount,
                "quote_amount": t.quote_amount,
                "price": t.price,
                "fee_quote": t.fee_quote,
            }
            for t in result.trades
        ],
    }


async def main() -> None:
    out = {
        "metrics_golden": gen_metrics_golden(),
        "runner_parity": await gen_runner_parity(0.6),
        "runner_parity_active": await gen_runner_parity(0.2),
    }
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))
    print(f"wrote {OUT_PATH}")
    print(json.dumps(out, indent=2, sort_keys=True))


if __name__ == "__main__":
    asyncio.run(main())
