from __future__ import annotations

from pathlib import Path

import pytest

from tradebot.backtest.data_loader import (
    DataLoaderError,
    load_csv_bytes,
    load_csv_path,
)

_CSV = (
    "timestamp,open,high,low,close,volume\n"
    "2026-05-01T00:00:00+00:00,100,101,99,100.5,1000\n"
    "2026-05-01T00:01:00+00:00,100.5,102,100,101.5,1100\n"
)


def test_load_csv_path(tmp_path: Path):
    p = tmp_path / "x.csv"
    p.write_text(_CSV)
    df = load_csv_path(p)
    assert len(df) == 2
    assert df["close"].iloc[-1] == 101.5


def test_load_csv_bytes_roundtrips():
    df = load_csv_bytes(_CSV.encode("utf-8"))
    assert len(df) == 2


def test_missing_columns_raises():
    bad = b"timestamp,close\n2026-05-01T00:00:00+00:00,100\n"
    with pytest.raises(DataLoaderError):
        load_csv_bytes(bad)


def test_empty_csv_raises():
    with pytest.raises(DataLoaderError):
        load_csv_bytes(b"timestamp,open,high,low,close,volume\n")


def test_unsorted_timestamps_get_sorted():
    csv = (
        "timestamp,open,high,low,close,volume\n"
        "2026-05-01T00:01:00+00:00,100.5,102,100,101.5,1100\n"
        "2026-05-01T00:00:00+00:00,100,101,99,100.5,1000\n"
    )
    df = load_csv_bytes(csv.encode("utf-8"))
    assert df["close"].iloc[0] == 100.5
    assert df["close"].iloc[-1] == 101.5
