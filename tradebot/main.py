from __future__ import annotations

import asyncio
import os
import sys
from pathlib import Path
from typing import Any

import click

from tradebot.core.aggregator import SignalAggregator
from tradebot.core.decision import DecisionEngine
from tradebot.core.loop import TradingLoop
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskManager, RiskState
from tradebot.dashboard.hub import DashboardHub
from tradebot.dashboard.server import run_server as run_dashboard_server
from tradebot.data.jupiter import JupiterClient
from tradebot.execution.demo import DemoExecutor
from tradebot.logging_setup import configure_logging, get_logger
from tradebot.wallet.keypair import BotKeypair


def _install_event_loop() -> None:
    if sys.platform == "win32":
        try:
            import winloop

            winloop.install()
            return
        except ImportError:
            pass
    else:
        try:
            import uvloop

            uvloop.install()
            return
        except ImportError:
            pass


@click.group()
def cli() -> None:
    """TradeBot CLI."""


@cli.command("init")
@click.option("--config", "config_path", default="tradebot.config.json", show_default=True)
def init_cmd(config_path: str) -> None:
    """Scaffold a default config file."""
    from tradebot.config.defaults import default_config
    from tradebot.config.file import save_config

    p = Path(config_path)
    if p.exists():
        click.echo(f"ERROR: {p} already exists; refusing to overwrite", err=True)
        raise SystemExit(2)
    cfg = default_config()
    save_config(p, cfg)
    click.echo(f"Wrote default config to {p}")
    click.echo("Edit it (or use the dashboard Settings tab after `tradebot start`).")


@cli.command("start")
@click.option(
    "--config",
    "config_path",
    default="tradebot.config.json",
    show_default=True,
    type=click.Path(exists=True),
)
@click.option("--mode", type=click.Choice(["demo", "real"]), default="demo", show_default=True)
@click.option("--confirm-real", is_flag=True, help="Required for --mode real (safety gate).")
@click.option("--keystore", type=click.Path(), default=None)
@click.option("--passphrase-env", default="TRADEBOT_PASSPHRASE")
@click.option(
    "--reset",
    is_flag=True,
    help="Wipe saved portfolio + risk state before starting (keeps trade log).",
)
def start_cmd(
    config_path: str,
    mode: str,
    confirm_real: bool,
    keystore: str | None,
    passphrase_env: str,
    reset: bool,
) -> None:
    """Launch engine + dashboard."""
    from tradebot.config.file import load_config

    cfg = load_config(Path(config_path))
    configure_logging(level=cfg.app.log_level, json_output=True)
    log = get_logger("main")

    if reset:
        from tradebot.storage.repo import JsonStorage, Mode

        storage = JsonStorage(root=Path(cfg.data_dir))
        _mode: Mode = mode  # type: ignore[assignment]
        for path in [
            storage._portfolio_path(_mode),  # noqa: SLF001
            storage._risk_path(_mode),  # noqa: SLF001
            storage._mark_history_path(_mode),  # noqa: SLF001
        ]:
            if path.exists():
                path.unlink()
        click.echo(f"Wiped saved state for mode={mode}")

    if mode == "real" and not confirm_real:
        log.error("real_mode_requires_confirm", hint="re-run with --confirm-real")
        raise SystemExit(2)

    bot_kp: BotKeypair | None = None
    if mode == "real":
        passphrase = os.environ.get(passphrase_env)
        if not passphrase:
            log.error("keystore_passphrase_missing", env_var=passphrase_env)
            raise SystemExit(2)
        from tradebot.wallet.keypair import load_bot_keypair

        ks_path = Path(keystore) if keystore else Path(cfg.app.keystore_path)
        bot_kp = load_bot_keypair(ks_path, passphrase=passphrase)
        log.info("keystore_loaded", address=bot_kp.address)

    _install_event_loop()
    asyncio.run(_run(mode=mode, cfg=cfg, cfg_path=Path(config_path), log=log, bot_keypair=bot_kp))


@cli.command("wallet")
@click.argument("action", type=click.Choice(["generate", "show"]))
@click.option("--keystore", type=click.Path(), required=True)
@click.option("--passphrase-env", default="TRADEBOT_PASSPHRASE")
def wallet_cmd(action: str, keystore: str, passphrase_env: str) -> None:
    """Manage the bot's encrypted keypair."""
    from tradebot.wallet.keypair import (
        generate_bot_keypair,
        load_bot_keypair,
        save_bot_keypair,
    )

    passphrase = os.environ.get(passphrase_env)
    if not passphrase:
        click.echo(f"ERROR: set {passphrase_env} in your environment first", err=True)
        raise SystemExit(2)
    path = Path(keystore)
    if action == "generate":
        if path.exists():
            click.echo(f"ERROR: refusing to overwrite existing keystore at {path}", err=True)
            raise SystemExit(2)
        kp = generate_bot_keypair()
        save_bot_keypair(kp, path=path, passphrase=passphrase)
        click.echo(f"Generated keystore at {path}")
        click.echo(f"Bot address: {kp.address}")
    elif action == "show":
        kp = load_bot_keypair(path, passphrase=passphrase)
        click.echo(f"Bot address: {kp.address}")


async def _run(
    *,
    mode: str,
    cfg: Any,
    cfg_path: Path,
    log: Any,
    bot_keypair: BotKeypair | None = None,
) -> None:
    from tradebot.storage.repo import JsonStorage

    storage = JsonStorage(root=Path(cfg.data_dir))
    saved = storage.load_portfolio_state(mode=mode)  # type: ignore[arg-type]
    if saved is not None:
        # Migration: state file pre-dates SOL tracking → seed from config.
        if saved.sol_balance == 0.0 and cfg.app.starting_sol_balance > 0.0:
            saved.sol_balance = cfg.app.starting_sol_balance
        portfolio = Portfolio.from_state(saved)
        log.info(
            "portfolio_resumed",
            cash=portfolio.cash,
            sol=portfolio.sol_balance,
            realized=portfolio.realized_pnl_total,
            positions=len(portfolio.open_positions()),
            equity_high=portfolio.equity_high,
        )
    else:
        portfolio = Portfolio(
            mode=mode,
            starting_cash=cfg.app.starting_capital_usd,
            starting_sol_balance=cfg.app.starting_sol_balance,
        )
        log.info("portfolio_fresh", capital=portfolio.cash, sol=portfolio.sol_balance)
    log.info("startup", mode=mode, data_dir=cfg.data_dir)

    base_mints: dict[str, tuple[str, int]] = {
        f"{e.symbol}/{cfg.watchlist.quote_symbol}": (e.mint, e.decimals)
        for e in cfg.watchlist.entries
    }
    pairs = list(base_mints.keys())
    timeframes = list(cfg.weights.timeframes.keys())

    from tradebot.data.rate_limiter import TokenBucketLimiter

    jup_limiter = TokenBucketLimiter(
        rate_per_sec=cfg.app.jupiter_rate_limit_rps,
        burst=cfg.app.jupiter_rate_limit_burst,
    )
    async with JupiterClient(
        base_url=cfg.app.jupiter_base_url,
        limiter=jup_limiter,
        max_429_retries=cfg.app.jupiter_max_429_retries,
    ) as jup:
        if mode == "real":
            from tradebot.data.rpc import SolanaRpcClient
            from tradebot.execution.real import RealExecutor

            assert bot_keypair is not None  # guarded by start_cmd
            rpc_cm = SolanaRpcClient(url=cfg.app.rpc_url)
            rpc = await rpc_cm.__aenter__()
            try:
                executor: Any = RealExecutor(
                    jupiter=jup,
                    rpc=rpc,
                    storage=storage,
                    keypair=bot_keypair,
                    base_mints=base_mints,
                    quote_mint=cfg.watchlist.quote_mint,
                    quote_decimals=6,
                    max_slippage_pct=cfg.risk.max_slippage_pct,
                    priority_fee_microlamports=cfg.app.priority_fee_microlamports,
                    confirmation_timeout_s=cfg.app.confirmation_timeout_s,
                )
                from tradebot.wallet.balance import get_token_balance
                from tradebot.wallet.reconcile import log_findings, reconcile

                findings = await reconcile(
                    portfolio=portfolio,
                    rpc=rpc,
                    bot_address=bot_keypair.address,
                    token_balance_fn=get_token_balance,
                    base_mints=base_mints,
                )
                log_findings(findings)
                await _run_loop(
                    cfg=cfg,
                    cfg_path=cfg_path,
                    mode=mode,
                    storage=storage,
                    portfolio=portfolio,
                    jup=jup,
                    executor=executor,
                    base_mints=base_mints,
                    pairs=pairs,
                    timeframes=timeframes,
                    log=log,
                    real_address=bot_keypair.address if bot_keypair else None,
                    jup_limiter=jup_limiter,
                )
            finally:
                await rpc_cm.__aexit__(None, None, None)
            return

        executor = DemoExecutor(
            jupiter=jup,
            storage=storage,
            base_mints=base_mints,
            quote_mint=cfg.watchlist.quote_mint,
            quote_decimals=6,
            max_slippage_pct=cfg.risk.max_slippage_pct,
            priority_fee_microlamports=cfg.app.priority_fee_microlamports,
            confirm_latency_s=cfg.app.simulated_confirm_latency_s,
        )
        await _run_loop(
            cfg=cfg,
            cfg_path=cfg_path,
            mode=mode,
            storage=storage,
            portfolio=portfolio,
            jup=jup,
            executor=executor,
            base_mints=base_mints,
            pairs=pairs,
            timeframes=timeframes,
            log=log,
            real_address=None,
            jup_limiter=jup_limiter,
        )


async def _run_loop(
    *,
    cfg: Any,
    cfg_path: Path,
    mode: str,
    storage: Any,
    portfolio: Portfolio,
    jup: JupiterClient,
    executor: Any,
    base_mints: dict[str, tuple[str, int]],
    pairs: list[str],
    timeframes: list[str],
    log: Any,
    real_address: str | None,
    jup_limiter: Any = None,
) -> None:
    from tradebot.dashboard.config_broker import ConfigBroker
    from tradebot.data.helius import HeliusClient
    from tradebot.signals.microstructure import MicrostructureSignal
    from tradebot.signals.onchain import OnChainSignal
    from tradebot.signals.ta import TASignal

    risk = RiskManager(cfg.risk)
    state = storage.load_risk_state(mode=mode) or RiskState()
    if state.kill_switch_active:
        log.warning(
            "kill_switch_resumed",
            reason=state.kill_switch_reason,
            hint="bot will not enter new positions until manually cleared",
        )
    engine = DecisionEngine(
        risk=risk,
        entry_threshold=cfg.app.entry_threshold,
        exit_flip_threshold=cfg.app.exit_flip_threshold,
    )

    # Signals are only INSTANTIATED if their weight > 0. A weight of 0 in the
    # aggregator just zeroes the contribution, but the signal would still make
    # network calls every cycle — burning the rate-limit budget for nothing.
    sig_weights = cfg.weights.signals
    micro_tf = timeframes[0] if timeframes else "1m"

    signals: list[Any] = []
    if sig_weights.get("ta", 0.0) > 0:
        signals.extend(TASignal(timeframe=tf) for tf in timeframes)
    else:
        log.info("ta_signal_disabled", reason="weight is 0")

    if sig_weights.get("microstructure", 0.0) > 0:
        for pair, (mint, base_decimals) in base_mints.items():
            signals.append(
                MicrostructureSignal(
                    pair=pair,
                    timeframe=micro_tf,
                    jupiter=jup,
                    base_mint=mint,
                    quote_mint=cfg.watchlist.quote_mint,
                    base_decimals=base_decimals,
                    quote_decimals=6,
                    probe_size_in_quote=10.0,
                )
            )
        log.info("microstructure_signal_enabled", pairs=len(base_mints))
    else:
        log.info(
            "microstructure_signal_disabled",
            reason="weight is 0 — saves 2 Jupiter calls per pair per cycle",
        )

    # Helius key resolution: prefer env var if the config value looks like a var name
    # (uppercase + underscores), otherwise treat the value as the literal API key.
    cfg_helius_value = cfg.app.helius_api_key_env or ""
    looks_like_env = cfg_helius_value.replace("_", "").isupper() and len(cfg_helius_value) > 0
    helius_key = (
        os.environ.get(cfg_helius_value) if looks_like_env else cfg_helius_value
    ) or None

    helius_cm: HeliusClient | None = None
    helius: HeliusClient | None = None
    onchain_weight = sig_weights.get("onchain", 0.0)
    whale_weight = sig_weights.get("whale_follow", 0.0)
    needs_helius = (
        helius_key is not None
        and (
            onchain_weight > 0
            or (whale_weight > 0 and getattr(cfg, "whales", None) and cfg.whales.enabled)
        )
    )
    if needs_helius:
        assert helius_key is not None  # narrowed by needs_helius
        helius_cm = HeliusClient(api_key=helius_key, base_url=cfg.app.helius_base_url)
        helius = await helius_cm.__aenter__()
        if onchain_weight > 0:
            for pair, (mint, _) in base_mints.items():
                signals.append(
                    OnChainSignal(
                        pair=pair,
                        timeframe=micro_tf,
                        helius=helius,
                        mint=mint,
                        dex_addresses=set(cfg.app.onchain_dex_addresses),
                        whale_min=cfg.app.onchain_whale_min,
                    )
                )
            log.info("onchain_signal_enabled", dex_addresses=len(cfg.app.onchain_dex_addresses))
        else:
            log.info("onchain_signal_disabled", reason="weight is 0")
    elif helius_key is None:
        log.info("helius_disabled", reason="no Helius key resolved")
    else:
        log.info("helius_disabled", reason="no consumer (onchain + whale weights are 0)")

    # Whale-follow: tracker fetches Helius once per cycle; per-pair signals consume it.
    # Skipped entirely if weight is 0, even if `whales.enabled = true` in config.
    whales_cfg = getattr(cfg, "whales", None)
    whale_tracker = None
    if (
        whales_cfg is not None
        and whales_cfg.enabled
        and helius is not None
        and whales_cfg.wallets
        and whale_weight > 0
    ):
        from tradebot.core.whale_activity import WhaleActivityTracker
        from tradebot.signals.whale_follow import WhaleFollowSignal

        whale_addresses = [w.address for w in whales_cfg.wallets]
        whale_tracker = WhaleActivityTracker(
            helius=helius,
            wallets=whale_addresses,
            per_wallet_limit=whales_cfg.per_wallet_swap_limit,
        )
        for pair, (mint, _) in base_mints.items():
            signals.append(
                WhaleFollowSignal(
                    pair=pair,
                    base_mint=mint,
                    quote_mint=cfg.watchlist.quote_mint,
                    tracker=whale_tracker,
                    lookback_seconds=whales_cfg.lookback_minutes * 60,
                    decay_half_life_s=whales_cfg.decay_half_life_minutes * 60,
                    timeframe=micro_tf,
                )
            )
        log.info(
            "whale_follow_enabled",
            wallets=len(whale_addresses),
            lookback_min=whales_cfg.lookback_minutes,
        )
    elif whales_cfg is not None and whales_cfg.enabled:
        log.warning(
            "whale_follow_skipped",
            reason="enabled but Helius key missing or no wallets configured",
        )

    # Birdeye: batched mark-price fetcher. Same env-var-or-literal-key resolution
    # as Helius. When present, a single Birdeye call per cycle replaces N Jupiter
    # mark probes — frees the Jupiter rate-limit budget for execution.
    from tradebot.data.birdeye import BirdeyeClient

    cfg_be_value = cfg.app.birdeye_api_key_env or ""
    be_looks_like_env = cfg_be_value.replace("_", "").isupper() and len(cfg_be_value) > 0
    birdeye_key = (
        os.environ.get(cfg_be_value) if be_looks_like_env else cfg_be_value
    ) or None
    birdeye_cm: BirdeyeClient | None = None
    birdeye: BirdeyeClient | None = None
    if birdeye_key:
        from tradebot.data.rate_limiter import TokenBucketLimiter

        birdeye_limiter = TokenBucketLimiter(
            rate_per_sec=cfg.app.birdeye_rate_limit_rps,
            burst=cfg.app.birdeye_rate_limit_burst,
        )
        birdeye_cm = BirdeyeClient(
            api_key=birdeye_key,
            base_url=cfg.app.birdeye_base_url,
            limiter=birdeye_limiter,
            max_429_retries=cfg.app.birdeye_max_429_retries,
        )
        birdeye = await birdeye_cm.__aenter__()
        log.info(
            "birdeye_marks_enabled",
            rps=cfg.app.birdeye_rate_limit_rps,
            burst=cfg.app.birdeye_rate_limit_burst,
        )
    else:
        log.info("birdeye_marks_disabled", reason="no Birdeye key configured")

    aggregator = SignalAggregator(
        signals=signals,
        timeframe_weights=cfg.weights.timeframes,
        signal_weights=cfg.weights.signals,
    )
    from tradebot.core.manual_actions import ManualActionQueue
    from tradebot.dashboard.backtest_api import BacktestStore

    hub = DashboardHub() if cfg.dashboard.enabled else None
    broker = ConfigBroker(path=cfg_path, current=cfg) if cfg.dashboard.enabled else None
    backtest_store = BacktestStore() if cfg.dashboard.enabled else None
    manual_actions = ManualActionQueue() if cfg.dashboard.enabled else None
    loop = TradingLoop(
        storage=storage,
        portfolio=portfolio,
        aggregator=aggregator,
        engine=engine,
        risk=risk,
        state=state,
        executor=executor,
        jupiter=jup,
        pairs=pairs,
        timeframes=timeframes,
        quote_mint=cfg.watchlist.quote_mint,
        quote_decimals=6,
        base_mints=base_mints,
        hub=hub,
        jup_limiter=jup_limiter,
        manual_actions=manual_actions,
        whale_tracker=whale_tracker,
        birdeye=birdeye,
    )
    saved_history = storage.load_mark_history(mode=mode)
    for pair, points in saved_history.items():
        if pair in loop._mark_history:  # noqa: SLF001
            for ts, price in points:
                loop._mark_history[pair].append((ts, price))  # noqa: SLF001
    if saved_history:
        log.info("mark_history_resumed", pairs=len(saved_history))
    loop_task = asyncio.create_task(loop.run_forever(interval_s=cfg.app.decision_interval_s))
    tasks: list[asyncio.Task[Any]] = [loop_task]
    if (
        birdeye is not None
        and hub is not None
        and getattr(cfg.app, "fast_tick_interval_s", 0.0) > 0
    ):
        fast_task = asyncio.create_task(
            loop.run_fast_ticks(interval_s=cfg.app.fast_tick_interval_s)
        )
        tasks.append(fast_task)
        log.info("fast_tick_enabled", interval_s=cfg.app.fast_tick_interval_s)
    if hub is not None:
        dashboard_task = asyncio.create_task(
            run_dashboard_server(
                hub,
                host=cfg.dashboard.host,
                port=cfg.dashboard.port,
                broker=broker,
                backtest_store=backtest_store,
                manual_actions=manual_actions,
            )
        )
        tasks.append(dashboard_task)
        log.info("dashboard_url", url=f"http://{cfg.dashboard.host}:{cfg.dashboard.port}")
    if mode == "real":
        log.warning("REAL_MODE_ACTIVE", address=real_address, rpc=cfg.app.rpc_url)
    try:
        await asyncio.gather(*tasks)
    except (KeyboardInterrupt, asyncio.CancelledError):
        log.info("shutdown_signal")
        loop.stop()
        for t in tasks:
            t.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    finally:
        if helius_cm is not None:
            await helius_cm.__aexit__(None, None, None)
        if birdeye_cm is not None:
            await birdeye_cm.__aexit__(None, None, None)


if __name__ == "__main__":
    cli()
