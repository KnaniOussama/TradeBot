"""Generates numeric-parity fixtures for the Rust TA signal port.

Runs the real Python tradebot.signals.ta functions against the OHLCV CSV
fixtures and dumps each component score plus the composite to JSON so the
Rust port can be asserted against it.

Run with the project venv:
    .venv/Scripts/python.exe crates/signals/tests/fixtures/gen_ta_fixtures.py
"""

import asyncio
import json
from datetime import UTC, datetime
from pathlib import Path

import pandas as pd

from tradebot.signals.base import MarketContext
from tradebot.signals.ta import (
    TASignal,
    _atr_momentum,
    _bb_position,
    _ema_cross,
    _macd_score,
    _rsi_score,
)

ROOT = Path("G:/TradeBot")
FIXTURES_DIR = ROOT / "tests" / "fixtures"
OUT_PATH = ROOT / "crates" / "signals" / "tests" / "fixtures" / "ta_parity.json"

CSV_NAMES = [
    "ohlcv_sol_uptrend.csv",
    "ohlcv_sol_downtrend.csv",
    "ohlcv_sol_choppy.csv",
]


async def compute_for(name: str) -> dict:
    df = pd.read_csv(FIXTURES_DIR / name)
    close = df["close"]

    rsi = _rsi_score(close)
    macd = _macd_score(close)
    ema_cross = _ema_cross(close)
    bb = _bb_position(close)
    atr_mom = _atr_momentum(df)

    sig = TASignal(timeframe="1m")
    ctx = MarketContext(
        pair="SOL/USDC",
        now=datetime(2026, 5, 1, tzinfo=UTC),
        ohlcv={"1m": df},
    )
    score = await sig.score(ctx)

    return {
        "rsi": rsi,
        "macd": macd,
        "ema_cross": ema_cross,
        "bb": bb,
        "atr_mom": atr_mom,
        "composite": score.score,
        "components": score.components,
    }


async def main() -> None:
    out = {}
    for name in CSV_NAMES:
        out[name] = await compute_for(name)
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))
    print(f"wrote {OUT_PATH}")
    print(json.dumps(out, indent=2, sort_keys=True))


if __name__ == "__main__":
    asyncio.run(main())
