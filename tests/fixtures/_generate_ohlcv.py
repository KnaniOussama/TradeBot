"""Generates deterministic OHLCV fixtures used by signal tests.

Run once: python tests/fixtures/_generate_ohlcv.py
"""

from datetime import UTC, datetime, timedelta
from pathlib import Path

import numpy as np
import pandas as pd

OUT_DIR = Path(__file__).parent
N_BARS = 200
START = datetime(2026, 5, 1, tzinfo=UTC)


def make_series(seed: int, drift: float, vol: float, start_price: float) -> pd.DataFrame:
    rng = np.random.default_rng(seed)
    rets = rng.normal(loc=drift, scale=vol, size=N_BARS)
    closes = start_price * np.exp(np.cumsum(rets))
    opens = np.concatenate([[start_price], closes[:-1]])
    highs = np.maximum(opens, closes) * (1 + rng.uniform(0, vol, size=N_BARS))
    lows = np.minimum(opens, closes) * (1 - rng.uniform(0, vol, size=N_BARS))
    volumes = rng.uniform(50_000, 200_000, size=N_BARS)
    timestamps = [START + timedelta(minutes=i) for i in range(N_BARS)]
    return pd.DataFrame(
        {
            "timestamp": [t.isoformat() for t in timestamps],
            "open": opens,
            "high": highs,
            "low": lows,
            "close": closes,
            "volume": volumes,
        }
    )


def main() -> None:
    make_series(seed=1, drift=0.002, vol=0.005, start_price=100.0).to_csv(
        OUT_DIR / "ohlcv_sol_uptrend.csv", index=False
    )
    make_series(seed=2, drift=-0.002, vol=0.005, start_price=100.0).to_csv(
        OUT_DIR / "ohlcv_sol_downtrend.csv", index=False
    )
    make_series(seed=3, drift=0.0, vol=0.008, start_price=100.0).to_csv(
        OUT_DIR / "ohlcv_sol_choppy.csv", index=False
    )


if __name__ == "__main__":
    main()
