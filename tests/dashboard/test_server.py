import asyncio
import json

import pytest
from fastapi.testclient import TestClient

from tradebot.dashboard.hub import DashboardHub
from tradebot.dashboard.server import build_app
from tradebot.dashboard.state import DashboardSnapshot


def _snap(equity: float = 50.0) -> DashboardSnapshot:
    return DashboardSnapshot(
        mode="demo",
        now="2026-05-03T12:00:00+00:00",
        cash=equity,
        equity=equity,
        equity_high=equity,
        drawdown_pct=0.0,
        realized_pnl_total=0.0,
        sol_balance=0.05,
        sol_gas_paid_total=0.0,
        sol_mark=140.0,
        kill_switch_active=False,
        kill_switch_reason="",
    )


def test_get_index_serves_html():
    hub = DashboardHub()
    app = build_app(hub)
    with TestClient(app) as c:
        r = c.get("/")
        assert r.status_code == 200
        assert "text/html" in r.headers["content-type"]
        assert "TradeBot" in r.text


def test_get_state_returns_snapshot():
    hub = DashboardHub()
    app = build_app(hub)
    with TestClient(app) as c:
        # Empty state initially
        r = c.get("/api/state")
        assert r.status_code == 200
        body = r.json()
        assert body == {} or body.get("snapshot") is None


@pytest.mark.asyncio
async def test_get_state_returns_latest_after_publish():
    hub = DashboardHub()
    await hub.publish(_snap(equity=99.5))
    app = build_app(hub)
    with TestClient(app) as c:
        r = c.get("/api/state")
        body = r.json()
        assert body["equity"] == 99.5


def test_websocket_receives_published_snapshot():
    hub = DashboardHub()
    app = build_app(hub)
    with TestClient(app) as client:
        with client.websocket_connect("/ws") as ws:
            asyncio.run(hub.publish(_snap(equity=7.0)))
            payload = json.loads(ws.receive_text())
            assert payload["equity"] == 7.0


def test_get_config_returns_current(tmp_path):
    from tradebot.config.defaults import default_config
    from tradebot.config.file import save_config
    from tradebot.dashboard.config_broker import ConfigBroker

    p = tmp_path / "cfg.json"
    cfg = default_config()
    save_config(p, cfg)
    broker = ConfigBroker(path=p, current=cfg)

    hub = DashboardHub()
    app = build_app(hub, broker=broker)
    with TestClient(app) as c:
        r = c.get("/api/config")
        assert r.status_code == 200
        body = r.json()
        assert body["app"]["starting_capital_usd"] == 50.0


def test_post_config_validates_and_saves(tmp_path):
    from tradebot.config.defaults import default_config
    from tradebot.config.file import load_config, save_config
    from tradebot.dashboard.config_broker import ConfigBroker

    p = tmp_path / "cfg.json"
    cfg = default_config()
    save_config(p, cfg)
    broker = ConfigBroker(path=p, current=cfg)

    hub = DashboardHub()
    app = build_app(hub, broker=broker)
    with TestClient(app) as c:
        new = cfg.model_dump()
        new["app"]["starting_capital_usd"] = 100.0
        r = c.post("/api/config", json=new)
        assert r.status_code == 200
        # Reload from disk to confirm save
        loaded = load_config(p)
        assert loaded.app.starting_capital_usd == 100.0


def test_post_invalid_config_returns_400(tmp_path):
    from tradebot.config.defaults import default_config
    from tradebot.config.file import save_config
    from tradebot.dashboard.config_broker import ConfigBroker

    p = tmp_path / "cfg.json"
    save_config(p, default_config())
    broker = ConfigBroker(path=p, current=default_config())

    hub = DashboardHub()
    app = build_app(hub, broker=broker)
    with TestClient(app) as c:
        r = c.post("/api/config", json={"garbage": True})
        assert r.status_code == 400
