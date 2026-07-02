from __future__ import annotations

import json
import uuid
from collections import deque
from collections.abc import Callable
from dataclasses import asdict

from fastapi import APIRouter, File, Form, HTTPException, UploadFile
from fastapi.responses import JSONResponse

from tradebot.backtest.data_loader import DataLoaderError, load_csv_bytes
from tradebot.backtest.runner import BacktestParams, BacktestResult, run_backtest
from tradebot.config.file import TradeBotConfig
from tradebot.logging_setup import get_logger
from tradebot.signals.ta import TASignal

log = get_logger("dashboard.backtest")


class BacktestStore:
    def __init__(self, max_results: int = 20) -> None:
        self._results: deque[BacktestResult] = deque(maxlen=max_results)

    def add(self, result: BacktestResult) -> None:
        self._results.append(result)

    def list_summary(self) -> list[dict[str, object]]:
        return [
            {
                "id": r.id,
                "pair": r.pair,
                "completed_at": r.completed_at,
                "total_return_pct": r.total_return_pct,
                "sharpe": r.sharpe,
                "n_trades": r.n_trades,
                "max_drawdown_pct": r.max_drawdown_pct,
            }
            for r in reversed(self._results)
        ]

    def get(self, id_: str) -> BacktestResult | None:
        for r in self._results:
            if r.id == id_:
                return r
        return None


def build_backtest_router(
    store: BacktestStore, get_config: Callable[[], TradeBotConfig]
) -> APIRouter:
    """Returns a router; `get_config` is a zero-arg callable returning the active TradeBotConfig."""
    router = APIRouter(prefix="/api/backtest")

    @router.post("/run")
    async def run(
        csv_file: UploadFile = File(),  # noqa: B008
        params: str = Form(),  # noqa: B008
    ) -> JSONResponse:
        # Parse params JSON
        try:
            params_dict = json.loads(params)
            bt_params = BacktestParams(**params_dict)
        except (json.JSONDecodeError, TypeError, ValueError) as e:
            raise HTTPException(status_code=400, detail=f"invalid params: {e}") from e

        # Load CSV
        try:
            content = await csv_file.read()
            df = load_csv_bytes(content)
        except DataLoaderError as e:
            raise HTTPException(status_code=400, detail=f"invalid CSV: {e}") from e

        # Use weights from current live config
        cfg: TradeBotConfig = get_config()

        backtest_id = uuid.uuid4().hex[:12]
        log.info(
            "backtest_run_start",
            id=backtest_id,
            pair=bt_params.pair,
            bars=len(df),
            warmup=bt_params.warmup_bars,
        )

        # For v1, signals = TA only (microstructure + onchain need live data feeds, not replayable)
        result = await run_backtest(
            ohlcv=df,
            params=bt_params,
            signals_factory=lambda: [TASignal(timeframe=bt_params.timeframe)],
            timeframe_weights={bt_params.timeframe: 1.0},
            signal_weights={"ta": 1.0},
            risk=cfg.risk,
            backtest_id=backtest_id,
        )
        store.add(result)
        log.info(
            "backtest_run_done",
            id=backtest_id,
            return_pct=result.total_return_pct,
            sharpe=result.sharpe,
            trades=result.n_trades,
        )
        return JSONResponse(content=_result_to_dict(result))

    @router.get("/history")
    async def history() -> JSONResponse:
        return JSONResponse(content=store.list_summary())

    @router.get("/result/{id_}")
    async def result_by_id(id_: str) -> JSONResponse:
        r = store.get(id_)
        if r is None:
            raise HTTPException(status_code=404, detail="not found")
        return JSONResponse(content=_result_to_dict(r))

    return router


def _result_to_dict(r: BacktestResult) -> dict[str, object]:
    d: dict[str, object] = asdict(r)
    return d
