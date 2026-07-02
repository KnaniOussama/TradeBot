from __future__ import annotations

import json
from pathlib import Path

from pydantic import BaseModel, ValidationError

from tradebot.config.models import (
    AppConfig,
    RiskConfig,
    WatchlistConfig,
    WeightsConfig,
    WhalesConfig,
)


class DashboardConfig(BaseModel):
    enabled: bool = True
    host: str = "127.0.0.1"
    port: int = 8765


class TradeBotConfig(BaseModel):
    config_version: int = 1
    data_dir: str = "data"
    app: AppConfig
    risk: RiskConfig
    weights: WeightsConfig
    watchlist: WatchlistConfig
    dashboard: DashboardConfig
    whales: WhalesConfig = WhalesConfig()


def load_config(path: Path) -> TradeBotConfig:
    path = Path(path)
    if not path.exists():
        raise FileNotFoundError(f"config not found: {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise ValueError(f"invalid JSON in {path}: {e}") from e
    try:
        return TradeBotConfig.model_validate(data)
    except ValidationError as e:
        raise ValueError(f"config validation failed: {e}") from e


def save_config(path: Path, config: TradeBotConfig) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(config.model_dump_json(indent=2), encoding="utf-8")
