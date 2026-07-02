from __future__ import annotations

import json
from pathlib import Path
from typing import TYPE_CHECKING

import uvicorn
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from pydantic import ValidationError

from tradebot.config.file import TradeBotConfig
from tradebot.dashboard.hub import DashboardHub
from tradebot.logging_setup import get_logger

if TYPE_CHECKING:
    from tradebot.core.manual_actions import ManualActionQueue
    from tradebot.dashboard.backtest_api import BacktestStore
    from tradebot.dashboard.config_broker import ConfigBroker

log = get_logger("dashboard.server")

STATIC_DIR = Path(__file__).parent / "static"


def build_app(
    hub: DashboardHub,
    broker: ConfigBroker | None = None,
    backtest_store: BacktestStore | None = None,
    manual_actions: ManualActionQueue | None = None,
) -> FastAPI:
    app = FastAPI(title="TradeBot Dashboard")

    @app.get("/", response_class=HTMLResponse)
    async def index() -> HTMLResponse:
        html = (STATIC_DIR / "index.html").read_text(encoding="utf-8")
        return HTMLResponse(content=html)

    @app.get("/api/state")
    async def state() -> JSONResponse:
        snap = hub.latest()
        return JSONResponse(content=snap.to_dict() if snap else {})

    @app.websocket("/ws")
    async def ws_endpoint(websocket: WebSocket) -> None:
        await websocket.accept()
        sub = hub.subscribe()
        # Send initial snapshot if available
        latest = hub.latest()
        if latest is not None:
            await websocket.send_text(json.dumps(latest.to_dict()))
        try:
            while True:
                snap = await sub.get()
                await websocket.send_text(json.dumps(snap.to_dict()))
        except WebSocketDisconnect:
            pass
        except Exception as e:
            log.warning("ws_send_failed", error=str(e))
        finally:
            hub.unsubscribe(sub)

    if broker is not None:

        @app.get("/api/config")
        async def get_config() -> JSONResponse:
            return JSONResponse(content=broker.current().model_dump())

        @app.post("/api/config")
        async def post_config(payload: dict) -> JSONResponse:  # type: ignore[type-arg]
            try:
                new_cfg = TradeBotConfig.model_validate(payload)
            except ValidationError as e:
                return JSONResponse(status_code=400, content={"error": str(e)})
            broker.update(new_cfg)
            return JSONResponse(
                content={
                    "ok": True,
                    "applied_at_next_cycle": True,
                    "note": "Restart the bot to apply most fields.",
                }
            )

    if manual_actions is not None:

        @app.post("/api/positions/{pair_path:path}/sell")
        async def post_manual_sell(pair_path: str, payload: dict | None = None) -> JSONResponse:  # type: ignore[type-arg]
            # Pair like "SOL/USDC" arrives URL-encoded; FastAPI already decoded.
            pair = pair_path
            snap = hub.latest()
            open_pairs: set[str] = set()
            if snap is not None:
                for pos in snap.positions:
                    p = pos.get("pair") if isinstance(pos, dict) else None
                    if p:
                        open_pairs.add(p)
            if open_pairs and pair not in open_pairs:
                return JSONResponse(
                    status_code=404,
                    content={"error": f"no open position for {pair}"},
                )
            reason = (payload or {}).get("reason", "") if isinstance(payload, dict) else ""
            req = manual_actions.request_exit(pair=pair, reason=str(reason))
            log.info("manual_exit_enqueued", pair=pair, reason=req.reason)
            return JSONResponse(
                content={
                    "ok": True,
                    "pair": pair,
                    "reason": req.reason,
                    "requested_at": req.requested_at.isoformat(),
                    "note": "will execute next cycle",
                }
            )

    if backtest_store is not None and broker is not None:
        from tradebot.dashboard.backtest_api import build_backtest_router

        app.include_router(
            build_backtest_router(backtest_store, get_config=lambda: broker.current())
        )

    if STATIC_DIR.exists():
        app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")

    return app


async def run_server(
    hub: DashboardHub,
    host: str,
    port: int,
    broker: ConfigBroker | None = None,
    backtest_store: BacktestStore | None = None,
    manual_actions: ManualActionQueue | None = None,
) -> None:
    config = uvicorn.Config(
        app=build_app(
            hub,
            broker=broker,
            backtest_store=backtest_store,
            manual_actions=manual_actions,
        ),
        host=host,
        port=port,
        log_level="warning",
        access_log=False,
        ws="wsproto",  # avoids noisy winloop+websockets-legacy traceback on Windows
    )
    server = uvicorn.Server(config)
    log.info("dashboard_starting", host=host, port=port)
    await server.serve()
