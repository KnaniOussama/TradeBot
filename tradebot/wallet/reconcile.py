from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any, Literal

from tradebot.core.portfolio import Portfolio
from tradebot.logging_setup import get_logger

log = get_logger("wallet.reconcile")


@dataclass(frozen=True)
class ReconcileFinding:
    kind: Literal["position_mismatch", "unexpected_balance", "low_sol"]
    pair: str | None
    message: str
    expected: float | None = None
    observed: float | None = None


_MISMATCH_TOLERANCE = 1e-6


async def reconcile(
    portfolio: Portfolio,
    rpc: Any,
    bot_address: str,
    token_balance_fn: Callable[..., Awaitable[float]],
    base_mints: dict[str, tuple[str, int]],
    min_sol_for_fees: float = 0.01,
) -> list[ReconcileFinding]:
    findings: list[ReconcileFinding] = []

    sol = await rpc.get_balance_sol(bot_address)
    if sol < min_sol_for_fees:
        findings.append(
            ReconcileFinding(
                kind="low_sol",
                pair=None,
                message=(
                    f"bot wallet has only {sol:.6f} SOL; need >= {min_sol_for_fees} for tx fees"
                ),
                observed=sol,
                expected=min_sol_for_fees,
            )
        )

    for pair, (mint, _decimals) in base_mints.items():
        observed = await token_balance_fn(rpc, bot_address, mint)
        pos = portfolio.position_for(pair)
        expected = pos.base_amount if pos is not None else 0.0

        if pos is not None and abs(observed - expected) > _MISMATCH_TOLERANCE:
            findings.append(
                ReconcileFinding(
                    kind="position_mismatch",
                    pair=pair,
                    message=f"{pair}: portfolio expects {expected:.6f}, wallet has {observed:.6f}",
                    expected=expected,
                    observed=observed,
                )
            )
        elif pos is None and observed > _MISMATCH_TOLERANCE:
            findings.append(
                ReconcileFinding(
                    kind="unexpected_balance",
                    pair=pair,
                    message=f"{pair}: wallet has {observed:.6f} but portfolio holds no position",
                    expected=0.0,
                    observed=observed,
                )
            )

    return findings


def log_findings(findings: list[ReconcileFinding]) -> None:
    if not findings:
        log.info("reconcile_clean", findings=0)
        return
    for f in findings:
        log.warning(
            "reconcile_finding",
            kind=f.kind,
            pair=f.pair,
            message=f.message,
            expected=f.expected,
            observed=f.observed,
        )
    log.warning("reconcile_summary", findings=len(findings))
