import pytest

from tradebot.execution.gas import (
    DEFAULT_COMPUTE_UNITS_PER_SWAP,
    LAMPORTS_PER_SOL,
    SOLANA_BASE_FEE_LAMPORTS,
    gas_cost_sol,
)


def test_zero_priority_fee_is_just_base():
    cost = gas_cost_sol(priority_fee_microlamports=0)
    assert cost == pytest.approx(SOLANA_BASE_FEE_LAMPORTS / LAMPORTS_PER_SOL)


def test_priority_fee_scales_with_compute_units():
    # 1_000_000 microlamports/cu * 200_000 cu / 1_000_000 = 200_000 lamports
    cost = gas_cost_sol(priority_fee_microlamports=1_000_000)
    expected_lamports = SOLANA_BASE_FEE_LAMPORTS + (
        1_000_000 * DEFAULT_COMPUTE_UNITS_PER_SWAP // 1_000_000
    )
    assert cost == pytest.approx(expected_lamports / LAMPORTS_PER_SOL)


def test_negative_inputs_raise():
    with pytest.raises(ValueError):
        gas_cost_sol(priority_fee_microlamports=-1)
    with pytest.raises(ValueError):
        gas_cost_sol(priority_fee_microlamports=0, compute_units=-1)


def test_realistic_solana_priority_fee_returns_micro_sol():
    # 100k microlamports/cu (a typical priority fee) ≈ 0.000025 SOL
    cost = gas_cost_sol(priority_fee_microlamports=100_000)
    assert 1e-5 < cost < 1e-4
