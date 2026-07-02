from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from typing import TYPE_CHECKING, Literal

from tradebot.core.aggregator import AggregatedScore
from tradebot.core.portfolio import Portfolio
from tradebot.core.risk import RiskManager, RiskState

if TYPE_CHECKING:
    from tradebot.core.regime import Regime
    from tradebot.core.sizing import KellyStats


class ExitReason(StrEnum):
    SIGNAL_FLIP = "signal_flip"
    TRAILING_STOP = "trailing_stop"
    PER_TRADE_KILL = "per_trade_kill"
    TAKE_PROFIT_LADDER = "take_profit_ladder"
    MANUAL = "manual"


@dataclass(frozen=True)
class Action:
    kind: Literal["enter", "exit"]
    pair: str
    size_quote: float = 0.0  # for "enter": notional quote to deploy
    size_base: float = 0.0  # for "exit": base amount to sell
    confidence: float = 0.0
    reason: ExitReason | None = None


@dataclass(frozen=True)
class Observation:
    """One row per pair per cycle: what the bot saw and what it decided."""

    timestamp: str
    pair: str
    composite: float
    mark: float
    regime: str | None  # "trending_up" | "trending_down" | "chop" | "neutral" | None
    decision: str  # "enter" | "exit" | "hold"
    reason: str  # human-readable
    size_quote: float = 0.0
    size_base: float = 0.0


class DecisionEngine:
    def __init__(
        self,
        risk: RiskManager,
        entry_threshold: float = 0.6,
        exit_flip_threshold: float = -0.3,
    ) -> None:
        self._risk = risk
        self._entry_threshold = entry_threshold
        self._exit_flip_threshold = exit_flip_threshold
        self._peaks: dict[str, float] = {}
        self._laddered_pairs: set[str] = set()

    def update_position_peaks(self, marks: dict[str, float], portfolio: Portfolio) -> None:
        for pos in portfolio.open_positions():
            mark = marks.get(pos.pair)
            if mark is None:
                continue
            current_peak = self._peaks.get(pos.pair, pos.avg_entry_price)
            if mark > current_peak:
                self._peaks[pos.pair] = mark

    def _peak_for(self, pair: str, fallback: float) -> float:
        return self._peaks.get(pair, fallback)

    def decide(
        self,
        scores: list[AggregatedScore],
        marks: dict[str, float],
        slippages: dict[str, float],
        portfolio: Portfolio,
        state: RiskState,
        now: datetime,
        regimes: dict[str, Regime] | None = None,
        kelly_stats: dict[str, KellyStats] | None = None,
    ) -> tuple[list[Action], list[Observation]]:
        actions: list[Action] = []
        # observations: keyed by pair; emit at most one per pair
        obs_map: dict[str, Observation] = {}
        ts = now.isoformat()

        def _regime_label(pair: str) -> str | None:
            if regimes is None:
                return None
            r = regimes.get(pair)
            return r.label if r is not None else None

        # Forget laddered pairs that are no longer open
        open_pairs = {p.pair for p in portfolio.open_positions()}
        self._laddered_pairs &= open_pairs

        # Update peaks first so trailing stop sees fresh highs
        self.update_position_peaks(marks=marks, portfolio=portfolio)

        # Build a lookup of composite by pair for quick access in exit logic
        score_map: dict[str, AggregatedScore] = {s.pair: s for s in scores}

        # 1. Exit logic for existing positions
        for pos in portfolio.open_positions():
            mark = marks.get(pos.pair, pos.avg_entry_price)
            entry = pos.avg_entry_price
            unrealized_loss_pct = max(0.0, (entry - mark) / entry) if entry > 0 else 0.0
            composite = score_map[pos.pair].composite if pos.pair in score_map else 0.0
            regime_lbl = _regime_label(pos.pair)

            # Take-profit ladder (only once per opening of a position)
            tp_pct = self._risk._cfg.tp_ladder_pct  # noqa: SLF001
            tp_frac = self._risk._cfg.tp_ladder_fraction  # noqa: SLF001
            if (
                pos.pair not in self._laddered_pairs
                and entry > 0
                and (mark - entry) / entry >= tp_pct
                and tp_frac > 0
            ):
                size_out = pos.base_amount * tp_frac
                actions.append(
                    Action(
                        kind="exit",
                        pair=pos.pair,
                        size_base=size_out,
                        reason=ExitReason.TAKE_PROFIT_LADDER,
                    )
                )
                self._laddered_pairs.add(pos.pair)
                obs_map[pos.pair] = Observation(
                    timestamp=ts,
                    pair=pos.pair,
                    composite=composite,
                    mark=mark if mark != 0.0 else 0.0,
                    regime=regime_lbl,
                    decision="exit",
                    reason=f"take-profit ladder triggered at +{tp_pct * 100:.1f}%",
                    size_base=size_out,
                )
                continue  # skip other exits this cycle for this position

            # per-trade kill
            if self._risk.check_per_trade_kill(unrealized_loss_pct) == "kill":
                actions.append(
                    Action(
                        kind="exit",
                        pair=pos.pair,
                        size_base=pos.base_amount,
                        reason=ExitReason.PER_TRADE_KILL,
                    )
                )
                obs_map[pos.pair] = Observation(
                    timestamp=ts,
                    pair=pos.pair,
                    composite=composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="exit",
                    reason=f"per-trade kill: loss {unrealized_loss_pct * 100:.2f}% exceeded limit",
                    size_base=pos.base_amount,
                )
                continue
            # trailing stop
            peak = self._peak_for(pos.pair, fallback=entry)
            trailing_pct = self._risk._cfg.trailing_stop_pct  # noqa: SLF001 (internal access)
            if peak > 0 and (peak - mark) / peak >= trailing_pct:
                actions.append(
                    Action(
                        kind="exit",
                        pair=pos.pair,
                        size_base=pos.base_amount,
                        reason=ExitReason.TRAILING_STOP,
                    )
                )
                obs_map[pos.pair] = Observation(
                    timestamp=ts,
                    pair=pos.pair,
                    composite=composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="exit",
                    reason=(
                        f"trailing stop: dropped {((peak - mark) / peak) * 100:.2f}%"
                        f" from peak {peak:.4f}"
                    ),
                    size_base=pos.base_amount,
                )
                continue
            # signal flip while in profit
            agg = score_map.get(pos.pair)
            if agg is not None and agg.composite < self._exit_flip_threshold and mark > entry:
                actions.append(
                    Action(
                        kind="exit",
                        pair=pos.pair,
                        size_base=pos.base_amount,
                        reason=ExitReason.SIGNAL_FLIP,
                    )
                )
                obs_map[pos.pair] = Observation(
                    timestamp=ts,
                    pair=pos.pair,
                    composite=composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="exit",
                    reason=(
                        f"signal flip: composite {agg.composite:.3f}"
                        f" below exit threshold {self._exit_flip_threshold:.3f}"
                    ),
                    size_base=pos.base_amount,
                )
                continue

            # No exit triggered: emit hold observation for open position
            if pos.pair not in obs_map:
                obs_map[pos.pair] = Observation(
                    timestamp=ts,
                    pair=pos.pair,
                    composite=composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="hold",
                    reason="trailing stop OK / no exit triggered",
                )

        # 2. Entry logic: skip if kill switch active
        if state.kill_switch_active:
            # Emit hold observations for scored pairs without open positions
            for s in scores:
                if s.pair not in obs_map:
                    mark = marks.get(s.pair, 0.0)
                    regime_lbl = _regime_label(s.pair)
                    obs_map[s.pair] = Observation(
                        timestamp=ts,
                        pair=s.pair,
                        composite=s.composite,
                        mark=mark,
                        regime=regime_lbl,
                        decision="hold",
                        reason="kill switch active: entries blocked",
                    )
            return actions, list(obs_map.values())

        for s in scores:
            if s.pair in obs_map:
                # Position was already handled in exit logic above; skip entry for same pair
                continue

            mark = marks.get(s.pair, 0.0)
            regime_lbl = _regime_label(s.pair)

            if s.composite < self._entry_threshold:
                obs_map[s.pair] = Observation(
                    timestamp=ts,
                    pair=s.pair,
                    composite=s.composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="hold",
                    reason=(
                        f"composite {s.composite:.3f} below threshold {self._entry_threshold:.3f}"
                    ),
                )
                continue

            # Regime gate: block entry if chop and regime_block_chop is enabled
            if self._risk._cfg.regime_filter_enabled and self._risk._cfg.regime_block_chop:  # noqa: SLF001
                regime = (regimes or {}).get(s.pair)
                if regime is not None and regime.label == "chop":
                    obs_map[s.pair] = Observation(
                        timestamp=ts,
                        pair=s.pair,
                        composite=s.composite,
                        mark=mark,
                        regime=regime_lbl,
                        decision="hold",
                        reason=f"blocked: chop regime (ADX={regime.adx:.1f})",
                    )
                    continue  # skip entry in choppy market

            slippage = slippages.get(s.pair, 0.0)
            confidence = max(
                0.0, min(1.0, (s.composite - self._entry_threshold) / (1.0 - self._entry_threshold))
            )
            # Non-sizing risk checks (slippage, max trades, position count, etc.)
            gates = self._risk.evaluate_entry(
                pair=s.pair,
                confidence=confidence,
                portfolio=portfolio,
                state=state,
                slippage_pct=slippage,
                now=now,
            )
            if not gates.allowed:
                obs_map[s.pair] = Observation(
                    timestamp=ts,
                    pair=s.pair,
                    composite=s.composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="hold",
                    reason=(
                        f"entry gate blocked:"
                        f" {gates.reason if hasattr(gates, 'reason') else 'risk checks failed'}"
                    ),
                )
                continue

            # Kelly sizing (if enabled and stats available)
            if self._risk._cfg.use_kelly_sizing:  # noqa: SLF001
                from tradebot.core.sizing import kelly_size

                stats = (kelly_stats or {}).get(s.pair)
                if stats is not None:
                    size = kelly_size(
                        available_cash=portfolio.cash,
                        confidence=confidence,
                        stats=stats,
                        fallback_min=self._risk._cfg.per_trade_size_min,  # noqa: SLF001
                        fallback_max=self._risk._cfg.per_trade_size_max,  # noqa: SLF001
                    )
                else:
                    size = gates.size_quote
            else:
                size = gates.size_quote

            if size > 0:
                actions.append(
                    Action(
                        kind="enter",
                        pair=s.pair,
                        size_quote=size,
                        confidence=confidence,
                    )
                )
                obs_map[s.pair] = Observation(
                    timestamp=ts,
                    pair=s.pair,
                    composite=s.composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="enter",
                    reason=(
                        f"entry: composite {s.composite:.3f}"
                        f" >= threshold {self._entry_threshold:.3f}"
                    ),
                    size_quote=size,
                )
            else:
                obs_map[s.pair] = Observation(
                    timestamp=ts,
                    pair=s.pair,
                    composite=s.composite,
                    mark=mark,
                    regime=regime_lbl,
                    decision="hold",
                    reason="size computed as zero: insufficient cash or sizing limit",
                )

        return actions, list(obs_map.values())
