from __future__ import annotations

from datetime import UTC
from pathlib import Path

from fastapi.testclient import TestClient

from tradebot.config.defaults import default_config
from tradebot.dashboard.backtest_api import BacktestStore
from tradebot.dashboard.config_broker import ConfigBroker
from tradebot.dashboard.hub import DashboardHub
from tradebot.dashboard.server import build_app


def _gen_csv(n: int = 120) -> bytes:
    from datetime import datetime, timedelta

    base = datetime(2026, 5, 1, 0, 0, 0, tzinfo=UTC)
    rows = [b"timestamp,open,high,low,close,volume\n"]
    for i in range(n):
        ts = (base + timedelta(minutes=i)).isoformat()
        close = 100.0 + i * 0.1
        rows.append(f"{ts},{close - 0.1},{close + 0.5},{close - 0.5},{close},1000\n".encode())
    return b"".join(rows)


_CSV_BYTES = _gen_csv(120)


def _client(tmp_path: Path) -> TestClient:
    cfg = default_config()
    broker = ConfigBroker(path=tmp_path / "cfg.json", current=cfg)
    store = BacktestStore()
    hub = DashboardHub()
    app = build_app(hub, broker=broker, backtest_store=store)
    return TestClient(app)


def test_run_backtest_returns_result(tmp_path: Path):
    client = _client(tmp_path)
    files = {"csv_file": ("data.csv", _CSV_BYTES, "text/csv")}
    data = {
        "params": '{"pair": "SOL/USDC", "timeframe": "1m", "starting_cash": 100.0, '
        '"fee_bps": 30, "slippage_bps": 5, "warmup_bars": 50, '
        '"entry_threshold": 0.6, "exit_flip_threshold": -0.3}'
    }
    r = client.post("/api/backtest/run", files=files, data=data)
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["pair"] == "SOL/USDC"
    assert body["bars_processed"] == 70  # 120 - 50 warmup
    assert "id" in body
    assert "equity_curve" in body


def test_history_lists_recent_results(tmp_path: Path):
    client = _client(tmp_path)
    files = {"csv_file": ("data.csv", _CSV_BYTES, "text/csv")}
    data = {
        "params": '{"pair": "SOL/USDC", "timeframe": "1m", "starting_cash": 100.0, '
        '"fee_bps": 30, "slippage_bps": 5, "warmup_bars": 50, '
        '"entry_threshold": 0.6, "exit_flip_threshold": -0.3}'
    }
    client.post("/api/backtest/run", files=files, data=data)
    client.post("/api/backtest/run", files=files, data=data)
    r = client.get("/api/backtest/history")
    body = r.json()
    assert len(body) == 2
    assert "id" in body[0]


def test_result_by_id(tmp_path: Path):
    client = _client(tmp_path)
    files = {"csv_file": ("data.csv", _CSV_BYTES, "text/csv")}
    data = {
        "params": '{"pair": "SOL/USDC", "timeframe": "1m", "starting_cash": 100.0, '
        '"fee_bps": 30, "slippage_bps": 5, "warmup_bars": 50, '
        '"entry_threshold": 0.6, "exit_flip_threshold": -0.3}'
    }
    r1 = client.post("/api/backtest/run", files=files, data=data)
    bt_id = r1.json()["id"]
    r2 = client.get(f"/api/backtest/result/{bt_id}")
    assert r2.status_code == 200
    assert r2.json()["id"] == bt_id


def test_invalid_csv_returns_400(tmp_path: Path):
    client = _client(tmp_path)
    files = {"csv_file": ("bad.csv", b"not,a,valid,csv\n", "text/csv")}
    data = {
        "params": '{"pair": "X", "timeframe": "1m", "starting_cash": 100.0, '
        '"fee_bps": 30, "slippage_bps": 5, "warmup_bars": 50, '
        '"entry_threshold": 0.6, "exit_flip_threshold": -0.3}'
    }
    r = client.post("/api/backtest/run", files=files, data=data)
    assert r.status_code == 400
