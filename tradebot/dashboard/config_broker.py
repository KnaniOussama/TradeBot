from __future__ import annotations

from pathlib import Path

from tradebot.config.file import TradeBotConfig, load_config, save_config


class ConfigBroker:
    def __init__(self, path: Path, current: TradeBotConfig) -> None:
        self._path = Path(path)
        self._current = current

    def current(self) -> TradeBotConfig:
        return self._current

    def update(self, new_cfg: TradeBotConfig) -> None:
        save_config(self._path, new_cfg)
        self._current = load_config(self._path)
