from __future__ import annotations

import asyncio
from collections import deque
from datetime import UTC, datetime
from typing import TYPE_CHECKING, cast

from tradebot.core.aggregator import AggregatedScore, SignalAggregator
from tradebot.core.decision import Action, DecisionEngine, ExitReason, Observation
from tradebot.core.manual_actions import ManualActionQueue
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskManager, RiskState
from tradebot.core.whale_activity import WhaleActivityTracker
from tradebot.dashboard.hub import DashboardHub
from tradebot.dashboard.state import build_snapshot
from tradebot.data.jupiter import JupiterClient
from tradebot.execution.base import ExecutionError, Order
from tradebot.execution.demo import DemoExecutor
from tradebot.logging_setup import get_logger
from tradebot.signals.base import MarketContext
from tradebot.storage.repo import JsonStorage, Mode, OHLCVCandle

if TYPE_CHECKING:
    from tradebot.data.birdeye import BirdeyeClient
    from tradebot.data.rate_limiter import TokenBucketLimiter

log = get_logger("loop")


_TIMEFRAME_SECONDS: dict[str, int] = {"5s": 5, "1m": 60, "15m": 900, "1h": 3600}


def _bucket_floor(ts: datetime, timeframe: str) -> datetime:
    secs = _TIMEFRAME_SECONDS.get(timeframe)
    if secs is None:
        return ts
    epoch = int(ts.timestamp())
    floored = epoch - (epoch % secs)
    return datetime.fromtimestamp(floored, tz=ts.tzinfo or UTC)


class TradingLoop:
    def __init__(
        self,
        storage: JsonStorage,
        portfolio: Portfolio,
        aggregator: SignalAggregator,
        engine: DecisionEngine,
        risk: RiskManager,
        state: RiskState,
        executor: DemoExecutor,
        jupiter: JupiterClient,
        pairs: list[str],
        timeframes: list[str],
        quote_mint: str,
        quote_decimals: int,
        base_mints: dict[str, tuple[str, int]],
        ohlcv_limit: int = 200,
        hub: DashboardHub | None = None,
        chart_history_max: int = 120,
        jup_limiter: TokenBucketLimiter | None = None,
        manual_actions: ManualActionQueue | None = None,
        whale_tracker: WhaleActivityTracker | None = None,
        birdeye: BirdeyeClient | None = None,
    ) -> None:
        self._storage = storage
        self._portfolio = portfolio
        self._aggregator = aggregator
        self._engine = engine
        self._risk = risk
        self._state = state
        self._executor = executor
        self._jup = jupiter
        self._pairs = pairs
        self._timeframes = timeframes
        self._quote_mint = quote_mint
        self._quote_decimals = quote_decimals
        self._base_mints = base_mints
        self._ohlcv_limit = ohlcv_limit
        self._hub = hub
        self._chart_history_max = chart_history_max
        self._jup_limiter = jup_limiter
        self._manual_actions = manual_actions
        self._whale_tracker = whale_tracker
        self._birdeye = birdeye
        self._mark_history: dict[str, deque[tuple[datetime, float]]] = {
            p: deque(maxlen=chart_history_max) for p in pairs
        }
        self._observations: deque[Observation] = deque(maxlen=200)
        self._stop = asyncio.Event()
        # Latest marks observed by the fast-tick task (or last decision cycle).
        # Used by the fast-tick republish to compute up-to-date equity/P&L while
        # waiting for the next decision cycle.
        self._latest_marks: dict[str, float] = {}
        # Cached scored signals from the last decision cycle, reused by fast-tick
        # republishes so the dashboard's signal panel doesn't go blank between cycles.
        self._latest_scored: list[AggregatedScore] = []

    def stop(self) -> None:
        self._stop.set()

    async def _live_marks(self) -> tuple[dict[str, float], dict[str, float]]:
        """Returns (marks_per_pair, slippage_per_pair).

        Prefers Birdeye (one batched call covers the whole watchlist for a
        flat HTTP cost). Falls back to per-pair Jupiter probes if Birdeye is
        unavailable or returns nothing. The DemoExecutor's slippage gate
        catches per-trade slippage at fill time, so when Birdeye is the
        source we report 0 here (Birdeye doesn't expose price impact).
        """
        if self._birdeye is not None:
            mints = [m for m, _ in self._base_mints.values()]
            try:
                prices = await self._birdeye.multi_price(mints)
            except Exception as e:  # noqa: BLE001
                log.warning("birdeye_marks_failed", error=str(e))
                prices = {}
            if prices:
                marks_b: dict[str, float] = {}
                for pair, (mint, _) in self._base_mints.items():
                    if mint in prices:
                        marks_b[pair] = prices[mint]
                if marks_b:
                    slips_b = {pair: 0.0 for pair in marks_b}
                    return marks_b, slips_b
            log.info("birdeye_marks_empty_falling_back_to_jupiter")

        marks: dict[str, float] = {}
        slips: dict[str, float] = {}
        probe_quote = 1.0
        for pair, (mint, base_decimals) in self._base_mints.items():
            try:
                in_units = int(probe_quote * (10**self._quote_decimals))
                q = await self._jup.quote(
                    input_mint=self._quote_mint,
                    output_mint=mint,
                    amount=in_units,
                    slippage_bps=50,
                )
                out_human = q.out_amount / (10**base_decimals)
                if out_human > 0:
                    marks[pair] = probe_quote / out_human
                slips[pair] = q.price_impact_pct
            except Exception as e:  # noqa: BLE001
                log.warning("mark_quote_failed", pair=pair, error=str(e))
        return marks, slips

    async def run_one_cycle(self, now: datetime) -> None:
        # 0. Refresh whale activity (one Helius call per watched wallet, shared
        #    across every pair's WhaleFollowSignal this cycle).
        if self._whale_tracker is not None:
            try:
                await self._whale_tracker.fetch_all()
            except Exception as e:  # noqa: BLE001
                log.warning("whale_tracker_fetch_failed", error=str(e))
        # 1. Load OHLCV for all (pair, timeframe)
        ohlcv = self._storage.load_ohlcv_for_pairs(
            pairs=self._pairs,
            timeframes=self._timeframes,
            limit=self._ohlcv_limit,
        )
        # 2. Get live marks + slippages
        marks, slippages = await self._live_marks()
        for pair, price in marks.items():
            self._mark_history.setdefault(pair, deque(maxlen=self._chart_history_max)).append(
                (now, price)
            )
        # 2b. Persist marks as OHLCV candles per timeframe so signals have data.
        for pair, price in marks.items():
            for tf in self._timeframes:
                bucket = _bucket_floor(now, tf)
                existing_df = ohlcv.get(pair, {}).get(tf)
                if existing_df is not None and len(existing_df) > 0:
                    last_ts = existing_df["timestamp"].iloc[-1]
                    if last_ts == bucket:
                        last = existing_df.iloc[-1]
                        candle = OHLCVCandle(
                            pair=pair,
                            timeframe=tf,
                            bucket_start=bucket,
                            open=float(last["open"]),
                            high=max(float(last["high"]), price),
                            low=min(float(last["low"]), price),
                            close=price,
                            volume_quote=float(last["volume"]),
                        )
                    else:
                        candle = OHLCVCandle(
                            pair=pair,
                            timeframe=tf,
                            bucket_start=bucket,
                            open=price,
                            high=price,
                            low=price,
                            close=price,
                            volume_quote=0.0,
                        )
                else:
                    candle = OHLCVCandle(
                        pair=pair,
                        timeframe=tf,
                        bucket_start=bucket,
                        open=price,
                        high=price,
                        low=price,
                        close=price,
                        volume_quote=0.0,
                    )
                self._storage.upsert_ohlcv(candle)
        # Reload OHLCV so signals see the freshly-appended candle.
        ohlcv = self._storage.load_ohlcv_for_pairs(
            pairs=self._pairs,
            timeframes=self._timeframes,
            limit=self._ohlcv_limit,
        )
        # 3. Aggregate signals per pair
        scored = []
        for pair in self._pairs:
            ctx = MarketContext(pair=pair, now=now, ohlcv=ohlcv.get(pair, {}))
            scored.append(await self._aggregator.aggregate(ctx))
        # 4. Update risk state with current equity
        equity = self._portfolio.equity(marks)
        self._risk.update_state(self._portfolio, self._state, current_equity=equity, now=now)
        # 4b. Classify regime per pair (highest-weight TF)
        from tradebot.core.regime import Regime, classify_regime  # noqa: PLC0415

        regimes: dict[str, Regime] = {}
        regime_tf = max(self._aggregator._tf_weights.items(), key=lambda kv: kv[1])[0]  # noqa: SLF001
        for pair in self._pairs:
            df = ohlcv.get(pair, {}).get(regime_tf)
            if df is not None and len(df) > 0:
                regimes[pair] = classify_regime(df)
        # 4c. Compute Kelly stats per pair from trade history
        from tradebot.core.sizing import KellyStats, compute_kelly_stats  # noqa: PLC0415

        kelly_stats: dict[str, KellyStats] = {}
        for pair in self._pairs:
            returns = self._storage.round_trip_returns(
                mode=cast("Mode", self._portfolio.mode), pair=pair, limit=200
            )
            kelly_stats[pair] = compute_kelly_stats(returns)
        # 5. Decide
        actions, observations = self._engine.decide(
            scores=scored,
            marks=marks,
            slippages=slippages,
            portfolio=self._portfolio,
            state=self._state,
            now=now,
            regimes=regimes,
            kelly_stats=kelly_stats,
        )
        # 5b. Drain manual exit requests (user clicked "sell now" in dashboard).
        if self._manual_actions is not None:
            score_by_pair = {s.pair: s for s in scored}
            for req in self._manual_actions.drain():
                pos = self._portfolio.position_for(req.pair)
                if pos is None or pos.base_amount <= 1e-9:
                    log.warning("manual_exit_no_position", pair=req.pair, reason=req.reason)
                    continue
                # Insert at the front so manual exits run before any auto exits/entries.
                actions.insert(
                    0,
                    Action(
                        kind="exit",
                        pair=req.pair,
                        size_base=pos.base_amount,
                        reason=ExitReason.MANUAL,
                    ),
                )
                composite = score_by_pair[req.pair].composite if req.pair in score_by_pair else 0.0
                observations.insert(
                    0,
                    Observation(
                        timestamp=now.isoformat(),
                        pair=req.pair,
                        composite=composite,
                        mark=marks.get(req.pair, pos.avg_entry_price),
                        regime=regimes[req.pair].label if req.pair in regimes else None,
                        decision="exit",
                        reason=f"manual sell: {req.reason}",
                        size_base=pos.base_amount,
                    ),
                )

        for o in observations:
            self._observations.append(o)
            log.info(
                "cycle_observation",
                pair=o.pair,
                decision=o.decision,
                reason=o.reason,
                composite=round(o.composite, 4),
                mark=round(o.mark, 6),
                regime=o.regime,
                size_quote=round(o.size_quote, 4) if o.size_quote else None,
                size_base=round(o.size_base, 6) if o.size_base else None,
            )
        # 6. Execute
        for action in actions:
            await self._execute_action(action, now=now)
        # 7. Snapshot equity
        positions_value = sum(
            p.base_amount * marks.get(p.pair, p.avg_entry_price)
            for p in self._portfolio.open_positions()
        )
        self._storage.append_equity_snapshot(
            mode=cast("Mode", self._portfolio.mode),
            snapshot_at=now,
            equity=equity,
            cash=self._portfolio.cash,
            positions_value=positions_value,
        )
        # 7b. Persist resumable state
        from tradebot.storage.repo import PortfolioState, PositionRecord

        positions_records = [
            PositionRecord(
                pair=p.pair,
                base_amount=p.base_amount,
                avg_entry_price=p.avg_entry_price,
                fees_paid_quote=p.fees_paid_quote,
            )
            for p in self._portfolio.open_positions()
        ]
        self._storage.save_portfolio_state(
            PortfolioState(
                mode=cast("Mode", self._portfolio.mode),
                cash=self._portfolio.cash,
                realized_pnl_total=self._portfolio.realized_pnl_total,
                equity_high=self._portfolio.equity_high,
                sol_balance=self._portfolio.sol_balance,
                sol_gas_paid_total=self._portfolio.sol_gas_paid_total,
                positions=positions_records,
            )
        )
        self._storage.save_risk_state(mode=cast("Mode", self._portfolio.mode), state=self._state)
        self._storage.save_mark_history(
            mode=cast("Mode", self._portfolio.mode),
            history={pair: list(buf) for pair, buf in self._mark_history.items() if buf},
        )
        # Cache for fast-tick republishes.
        self._latest_marks = dict(marks)
        self._latest_scored = list(scored)
        # 8. Publish dashboard snapshot
        if self._hub is not None:
            mark_history_dict: dict[str, list[tuple[datetime, float]]] = {
                pair: list(buf) for pair, buf in self._mark_history.items() if buf
            }
            # Compute unmatched whale swaps (tokens NOT in our watchlist) for the
            # dashboard's Whale Watch panel.
            unmatched_swaps: list[dict[str, object]] = []
            if self._whale_tracker is not None:
                watched_mints = {self._quote_mint, *(m for m, _ in self._base_mints.values())}
                for s in self._whale_tracker.unmatched_swaps(watched_mints, limit=30):
                    unmatched_swaps.append(
                        {
                            "ts": s.timestamp.isoformat(),
                            "wallet": s.wallet,
                            "in_mint": s.in_mint,
                            "out_mint": s.out_mint,
                            "in_amount_raw": s.in_amount_raw,
                            "out_amount_raw": s.out_amount_raw,
                            "signature": s.signature,
                        }
                    )
            snap = await build_snapshot(
                storage=self._storage,
                portfolio=self._portfolio,
                risk_state=self._state,
                marks=marks,
                scores=scored,
                now=now,
                mark_history=mark_history_dict,
                observations=list(self._observations),
                limiter_metrics=self._jup_limiter.metrics() if self._jup_limiter else None,
                whale_activity=unmatched_swaps,
            )
            await self._hub.publish(snap)

    async def _execute_action(self, action: Action, now: datetime) -> None:
        try:
            if action.kind == "enter":
                order = Order(pair=action.pair, side="buy", size_quote=action.size_quote)
            else:
                order = Order(pair=action.pair, side="sell", size_base=action.size_base)
            await self._executor.execute(order=order, portfolio=self._portfolio, now=now)
            self._risk.record_trade(state=self._state, now=now)
        except ExecutionError as e:
            log.warning("execution_failed", pair=action.pair, kind=action.kind, error=str(e))

    async def run_forever(self, interval_s: float) -> None:
        """Fixed-cadence loop: cycle N starts at start_time + N*interval_s.

        If a cycle takes longer than `interval_s` it runs back-to-back
        (limited only by the shared rate limiter); shorter cycles sleep
        for the remainder. This keeps wall-clock cadence predictable.
        """
        loop = asyncio.get_running_loop()
        next_start = loop.time()
        while not self._stop.is_set():
            cycle_started = loop.time()
            try:
                await self.run_one_cycle(now=datetime.now(UTC))
            except Exception as e:
                log.error("cycle_failed", error=str(e))
            cycle_duration = loop.time() - cycle_started
            if cycle_duration > interval_s:
                log.warning(
                    "cycle_overrun",
                    duration_s=round(cycle_duration, 3),
                    interval_s=interval_s,
                    hint="rate-limited or slow network; cycles running back-to-back",
                )
            next_start += interval_s
            sleep_s = max(0.0, next_start - loop.time())
            if sleep_s > 0:
                try:
                    await asyncio.wait_for(self._stop.wait(), timeout=sleep_s)
                except TimeoutError:
                    pass
            else:
                # Cycle overran: reset the schedule to "now" so we don't burn CPU catching up.
                next_start = loop.time()

    async def run_fast_ticks(self, interval_s: float) -> None:
        """Lightweight Birdeye-only loop that refreshes chart prices and republishes
        the dashboard snapshot at sub-second cadence between decision cycles.

        Skips entirely when:
          - no Birdeye client (would need Jupiter probes, too expensive at 1s)
          - no dashboard hub (nothing to publish to)
          - no cached scored signals yet (first decision cycle hasn't run)

        Each tick:
          1. Birdeye `multi_price` for every watched pair (1 HTTP call, free tier).
          2. Append to `mark_history` so the chart gains new points.
          3. Rebuild the snapshot using *cached* scored signals + fresh marks/equity.
          4. Publish via the hub (websocket clients receive the update).
        """
        if self._birdeye is None or self._hub is None:
            log.info(
                "fast_tick_disabled",
                reason="needs both Birdeye client and dashboard hub",
            )
            return
        loop = asyncio.get_running_loop()
        next_start = loop.time()
        mints = [m for m, _ in self._base_mints.values()]
        mint_to_pair = {m: pair for pair, (m, _) in self._base_mints.items()}
        while not self._stop.is_set():
            try:
                if self._latest_scored:  # wait until first decision cycle has run
                    prices = await self._birdeye.multi_price(mints)
                    if prices:
                        now = datetime.now(UTC)
                        for mint, price in prices.items():
                            pair = mint_to_pair.get(mint)
                            if pair is None:
                                continue
                            self._latest_marks[pair] = price
                            self._mark_history.setdefault(
                                pair, deque(maxlen=self._chart_history_max)
                            ).append((now, price))
                        await self._publish_fast_snapshot(now=now)
            except Exception as e:  # noqa: BLE001
                log.warning("fast_tick_failed", error=str(e))
            next_start += interval_s
            sleep_s = max(0.0, next_start - loop.time())
            if sleep_s > 0:
                try:
                    await asyncio.wait_for(self._stop.wait(), timeout=sleep_s)
                except TimeoutError:
                    pass
            else:
                next_start = loop.time()

    async def _publish_fast_snapshot(self, now: datetime) -> None:
        """Rebuild and publish a snapshot using cached scored + fresh marks."""
        assert self._hub is not None  # narrowed by caller
        mark_history_dict: dict[str, list[tuple[datetime, float]]] = {
            pair: list(buf) for pair, buf in self._mark_history.items() if buf
        }
        unmatched_swaps: list[dict[str, object]] = []
        if self._whale_tracker is not None:
            watched_mints = {self._quote_mint, *(m for m, _ in self._base_mints.values())}
            for s in self._whale_tracker.unmatched_swaps(watched_mints, limit=30):
                unmatched_swaps.append(
                    {
                        "ts": s.timestamp.isoformat(),
                        "wallet": s.wallet,
                        "in_mint": s.in_mint,
                        "out_mint": s.out_mint,
                        "in_amount_raw": s.in_amount_raw,
                        "out_amount_raw": s.out_amount_raw,
                        "signature": s.signature,
                    }
                )
        snap = await build_snapshot(
            storage=self._storage,
            portfolio=self._portfolio,
            risk_state=self._state,
            marks=self._latest_marks,
            scores=self._latest_scored,
            now=now,
            mark_history=mark_history_dict,
            observations=list(self._observations),
            limiter_metrics=self._jup_limiter.metrics() if self._jup_limiter else None,
            whale_activity=unmatched_swaps,
        )
        await self._hub.publish(snap)
