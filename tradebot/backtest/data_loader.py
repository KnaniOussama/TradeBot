from __future__ import annotations

import io
from pathlib import Path

import pandas as pd

REQUIRED_COLS = {"timestamp", "open", "high", "low", "close", "volume"}


class DataLoaderError(Exception):
    pass


def _validate(df: pd.DataFrame) -> pd.DataFrame:
    missing = REQUIRED_COLS - set(df.columns)
    if missing:
        raise DataLoaderError(f"missing required columns: {sorted(missing)}")
    if len(df) == 0:
        raise DataLoaderError("CSV has no data rows")
    df["timestamp"] = pd.to_datetime(df["timestamp"], utc=True)
    df = df.sort_values("timestamp").reset_index(drop=True)
    return df


def load_csv_path(path: Path) -> pd.DataFrame:
    try:
        df = pd.read_csv(path)
    except (OSError, pd.errors.ParserError) as e:
        raise DataLoaderError(f"failed to read CSV: {e}") from e
    return _validate(df)


def load_csv_bytes(content: bytes) -> pd.DataFrame:
    try:
        df = pd.read_csv(io.BytesIO(content))
    except pd.errors.ParserError as e:
        raise DataLoaderError(f"failed to parse CSV: {e}") from e
    return _validate(df)
