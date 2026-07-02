from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, TypeVar

import yaml
from pydantic import BaseModel, ValidationError

from tradebot.config.models import (
    AppConfig,
    RiskConfig,
    WatchlistConfig,
    WeightsConfig,
)

T = TypeVar("T", bound=BaseModel)


@dataclass(frozen=True)
class ConfigBundle:
    app: AppConfig
    risk: RiskConfig
    weights: WeightsConfig
    watchlist: WatchlistConfig


def _read_yaml(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"config file not found: {path}")
    try:
        with path.open("r", encoding="utf-8") as f:
            data = yaml.safe_load(f)
    except yaml.YAMLError as e:
        raise ValueError(f"invalid YAML in {path}: {e}") from e
    if not isinstance(data, dict):
        raise ValueError(f"config root must be a mapping in {path}")
    return data


def _load(path: Path, model: type[T]) -> T:  # noqa: UP047
    data = _read_yaml(path)
    try:
        return model.model_validate(data)
    except ValidationError as e:
        raise ValueError(f"config validation failed for {path}: {e}") from e


def load_app_config(path: Path) -> AppConfig:
    return _load(path, AppConfig)


def load_risk_config(path: Path) -> RiskConfig:
    return _load(path, RiskConfig)


def load_weights(path: Path) -> WeightsConfig:
    return _load(path, WeightsConfig)


def load_watchlist(path: Path) -> WatchlistConfig:
    return _load(path, WatchlistConfig)


def load_bundle(
    app_path: Path,
    risk_path: Path,
    weights_path: Path,
    watchlist_path: Path,
) -> ConfigBundle:
    return ConfigBundle(
        app=load_app_config(app_path),
        risk=load_risk_config(risk_path),
        weights=load_weights(weights_path),
        watchlist=load_watchlist(watchlist_path),
    )
