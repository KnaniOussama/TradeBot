from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field, field_validator, model_validator

Timeframe = Literal["5s", "1m", "15m", "1h"]
Mode = Literal["demo", "real", "backtest"]


class WatchlistEntry(BaseModel):
    symbol: str = Field(min_length=1, max_length=16)
    mint: str = Field(min_length=32, max_length=64)
    decimals: int = Field(ge=0, le=18)


class WhaleEntry(BaseModel):
    address: str = Field(min_length=32, max_length=64)
    label: str = Field(default="", max_length=64)


class WhalesConfig(BaseModel):
    enabled: bool = False
    wallets: list[WhaleEntry] = []
    lookback_minutes: int = Field(default=30, ge=1, le=1440)
    decay_half_life_minutes: float = Field(default=10.0, gt=0, le=720)
    per_wallet_swap_limit: int = Field(default=20, ge=1, le=100)


class WatchlistConfig(BaseModel):
    quote_symbol: str
    quote_mint: str
    entries: list[WatchlistEntry]

    @model_validator(mode="after")
    def _no_duplicate_symbols(self) -> WatchlistConfig:
        seen: set[str] = set()
        for e in self.entries:
            if e.symbol in seen:
                raise ValueError(f"duplicate symbol: {e.symbol}")
            seen.add(e.symbol)
        return self


class RiskConfig(BaseModel):
    max_concurrent_positions: int = 3
    per_trade_size_min: float = 0.30
    per_trade_size_max: float = 0.50
    trailing_stop_pct: float = 0.02
    take_profit_pct: float = 0.02
    daily_loss_limit_pct: float = 0.08
    weekly_loss_limit_pct: float = 0.10
    per_trade_kill_pct: float = 0.03
    drawdown_circuit_pct: float = 0.15
    max_slippage_pct: float = 0.01
    max_trades_per_day: int = 10
    tp_ladder_pct: float = 0.02
    tp_ladder_fraction: float = 0.5
    # Phase 10: regime filter
    regime_filter_enabled: bool = True
    regime_block_chop: bool = True
    # Phase 10: Kelly sizing
    use_kelly_sizing: bool = True

    @model_validator(mode="after")
    def _size_range_ordered(self) -> RiskConfig:
        if self.per_trade_size_min > self.per_trade_size_max:
            raise ValueError("per_trade_size_min must be <= per_trade_size_max")
        return self


class WeightsConfig(BaseModel):
    timeframes: dict[str, float]
    signals: dict[str, float]

    @field_validator("timeframes", "signals")
    @classmethod
    def _sum_to_one(cls, v: dict[str, float]) -> dict[str, float]:
        total = sum(v.values())
        if abs(total - 1.0) > 1e-6:
            raise ValueError(f"weights must sum to 1.0, got {total}")
        return v


class AppConfig(BaseModel):
    rpc_url: str
    helius_api_key_env: str
    log_level: str = "INFO"
    starting_capital_usd: float = 50.0
    jupiter_base_url: str = "https://lite-api.jup.ag/swap/v1"
    price_poll_interval_s: float = 5.0
    decision_interval_s: float = 10.0
    jupiter_rate_limit_rps: float = 0.9  # sustained req/s budget; lite-api limit is ~1.0
    jupiter_rate_limit_burst: int = 5  # token bucket capacity
    jupiter_max_429_retries: int = 3
    entry_threshold: float = 0.6
    exit_flip_threshold: float = -0.3
    helius_base_url: str = "https://api.helius.xyz"
    onchain_dex_addresses: list[str] = []
    onchain_whale_min: float = 1000.0
    # Birdeye is used for live mark-prices (one batched call per cycle covers the
    # whole watchlist), freeing the Jupiter rate-limit budget for microstructure
    # probes and trade execution. Without a key the bot falls back to per-pair
    # Jupiter probes (the original behavior).
    birdeye_api_key_env: str = ""
    birdeye_base_url: str = "https://public-api.birdeye.so"
    # Birdeye free Standard tier is ~1 rps. The Starter tier is ~30 rps.
    # Set to 0.9 (free) or 25+ (paid). Limiter applies to BOTH the decision
    # cycle and the fast-tick task — they share one budget.
    birdeye_rate_limit_rps: float = 0.9
    birdeye_rate_limit_burst: int = 2
    birdeye_max_429_retries: int = 2
    # Fast chart-refresh task: polls Birdeye every N seconds and republishes
    # the snapshot so the dashboard chart updates between (slower) decision
    # cycles. On free-tier Birdeye (~1 rps) with 8 mints, ONE full refresh
    # takes ~8s — set this to a multiple of (mints / rps) or larger.
    # Recommended: 0 (disabled) or 10+ on free tier; 1-2 on paid Starter tier.
    # Set to 0 to disable.
    fast_tick_interval_s: float = 0.0
    simulated_fee_bps: int = 10
    dashboard_enabled: bool = True
    dashboard_host: str = "127.0.0.1"
    dashboard_port: int = 8765
    keystore_path: str = "keystore/bot.keystore.json"
    priority_fee_microlamports: int = 0
    confirmation_timeout_s: float = 30.0
    # Demo wallet starting SOL (for gas accounting). 0.05 SOL ≈ $7 at $140/SOL.
    starting_sol_balance: float = 0.05
    # Demo: synthetic delay between trigger-quote and fill-quote, mirroring real-mode
    # quote→confirmation latency. Pass 0 in tests to skip the sleep.
    simulated_confirm_latency_s: float = 1.0
