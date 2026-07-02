"""Generates numeric-parity fixtures for the Rust regime classifier port.

Runs the real Python tradebot.core.regime.classify_regime against the OHLCV
CSV fixtures and dumps the label, adx, and ema_fast_above_slow fields to
JSON so the Rust port can be asserted against it.

Run with the project venv:
    .venv/Scripts/python.exe crates/core/tests/fixtures/gen_regime_fixtures.py
"""

import json
from pathlib import Path

import pandas as pd

from tradebot.core.regime import classify_regime

ROOT = Path("G:/TradeBot")
FIXTURES_DIR = ROOT / "crates" / "core" / "tests" / "fixtures"
OUT_PATH = FIXTURES_DIR / "regime_parity.json"

CSV_NAMES = [
    "ohlcv_sol_uptrend.csv",
    "ohlcv_sol_downtrend.csv",
    "ohlcv_sol_choppy.csv",
]


def compute_for(name: str) -> dict:
    df = pd.read_csv(FIXTURES_DIR / name)
    r = classify_regime(df, adx_period=14, ema_fast=20, ema_slow=50)
    return {
        "label": r.label,
        "adx": r.adx,
        "ema_fast_above_slow": r.ema_fast_above_slow,
    }


def main() -> None:
    out = {name: compute_for(name) for name in CSV_NAMES}
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))
    print(f"wrote {OUT_PATH}")
    print(json.dumps(out, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
