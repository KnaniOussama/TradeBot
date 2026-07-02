from __future__ import annotations

LAMPORTS_PER_SOL = 1_000_000_000
SOLANA_BASE_FEE_LAMPORTS = 5_000
DEFAULT_COMPUTE_UNITS_PER_SWAP = 200_000


def gas_cost_sol(
    priority_fee_microlamports: int,
    compute_units: int = DEFAULT_COMPUTE_UNITS_PER_SWAP,
) -> float:
    """Solana transaction cost in SOL for one Jupiter swap.

    base_fee + priority_fee, where:
      priority_fee_lamports = priority_fee_microlamports * compute_units / 1_000_000

    With priority_fee_microlamports=0 this returns the bare 5000-lamport base
    fee (~0.000005 SOL).
    """
    if priority_fee_microlamports < 0 or compute_units < 0:
        raise ValueError("negative gas inputs")
    priority_lamports = (priority_fee_microlamports * compute_units) // 1_000_000
    total_lamports = SOLANA_BASE_FEE_LAMPORTS + priority_lamports
    return total_lamports / LAMPORTS_PER_SOL
