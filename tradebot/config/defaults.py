from __future__ import annotations

from tradebot.config.file import DashboardConfig, TradeBotConfig
from tradebot.config.models import (
    AppConfig,
    RiskConfig,
    WatchlistConfig,
    WatchlistEntry,
    WeightsConfig,
)


def default_config() -> TradeBotConfig:
    return TradeBotConfig(
        config_version=1,
        data_dir="data",
        app=AppConfig(
            rpc_url="https://api.devnet.solana.com",
            helius_api_key_env="HELIUS_API_KEY",
            log_level="INFO",
            starting_capital_usd=50.0,
            jupiter_base_url="https://quote-api.jup.ag/v6",
            price_poll_interval_s=5.0,
            decision_interval_s=10.0,
            jupiter_rate_limit_rps=0.9,
            jupiter_rate_limit_burst=5,
            jupiter_max_429_retries=3,
        ),
        risk=RiskConfig(),
        weights=WeightsConfig(
            timeframes={"5s": 0.10, "1m": 0.20, "15m": 0.30, "1h": 0.40},
            signals={"ta": 0.40, "microstructure": 0.30, "onchain": 0.30},
        ),
        watchlist=WatchlistConfig(
            quote_symbol="USDC",
            quote_mint="EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            entries=[
                WatchlistEntry(
                    symbol="SOL",
                    mint="So11111111111111111111111111111111111111112",
                    decimals=9,
                ),
                WatchlistEntry(
                    symbol="JUP",
                    mint="JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN",
                    decimals=6,
                ),
                WatchlistEntry(
                    symbol="JTO",
                    mint="jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL",
                    decimals=9,
                ),
            ],
        ),
        dashboard=DashboardConfig(),
    )
