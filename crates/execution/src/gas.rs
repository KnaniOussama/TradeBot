//! Port of `tradebot/execution/gas.py`: gas / priority-fee -> SOL cost
//! accounting.

use rust_decimal::Decimal;
use tradebot_common::Money;

pub const LAMPORTS_PER_SOL: i64 = 1_000_000_000;
pub const SOLANA_BASE_FEE_LAMPORTS: i64 = 5_000;
pub const DEFAULT_COMPUTE_UNITS_PER_SWAP: i64 = 200_000;

/// Raised when `gas_cost_sol` is given a negative input. Mirrors the
/// `ValueError` raised by `gas_cost_sol` in gas.py.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("negative gas inputs")]
pub struct NegativeGasInputs;

/// Solana transaction cost in SOL for one Jupiter swap.
///
/// `base_fee + priority_fee`, where:
///   `priority_fee_lamports = priority_fee_microlamports * compute_units / 1_000_000`
///
/// With `priority_fee_microlamports = 0` this returns the bare 5000-lamport
/// base fee (~0.000005 SOL).
pub fn gas_cost_sol(
    priority_fee_microlamports: i64,
    compute_units: i64,
) -> Result<Money, NegativeGasInputs> {
    if priority_fee_microlamports < 0 || compute_units < 0 {
        return Err(NegativeGasInputs);
    }
    let priority_lamports = (priority_fee_microlamports * compute_units) / 1_000_000;
    let total_lamports = SOLANA_BASE_FEE_LAMPORTS + priority_lamports;
    Ok(Decimal::from(total_lamports) / Decimal::from(LAMPORTS_PER_SOL))
}

/// `gas_cost_sol` with `compute_units` defaulted to
/// `DEFAULT_COMPUTE_UNITS_PER_SWAP`, matching the Python default argument.
pub fn gas_cost_sol_for_swap(priority_fee_microlamports: i64) -> Result<Money, NegativeGasInputs> {
    gas_cost_sol(priority_fee_microlamports, DEFAULT_COMPUTE_UNITS_PER_SWAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_priority_fee_is_just_base() {
        let cost = gas_cost_sol_for_swap(0).unwrap();
        assert_eq!(
            cost,
            Decimal::from(SOLANA_BASE_FEE_LAMPORTS) / Decimal::from(LAMPORTS_PER_SOL)
        );
    }

    #[test]
    fn priority_fee_scales_with_compute_units() {
        // 1_000_000 microlamports/cu * 200_000 cu / 1_000_000 = 200_000 lamports
        let cost = gas_cost_sol_for_swap(1_000_000).unwrap();
        let expected_lamports =
            SOLANA_BASE_FEE_LAMPORTS + (1_000_000 * DEFAULT_COMPUTE_UNITS_PER_SWAP / 1_000_000);
        assert_eq!(
            cost,
            Decimal::from(expected_lamports) / Decimal::from(LAMPORTS_PER_SOL)
        );
    }

    #[test]
    fn negative_inputs_raise() {
        assert!(gas_cost_sol(-1, DEFAULT_COMPUTE_UNITS_PER_SWAP).is_err());
        assert!(gas_cost_sol(0, -1).is_err());
    }

    #[test]
    fn realistic_solana_priority_fee_returns_micro_sol() {
        // 100k microlamports/cu (a typical priority fee) ~= 0.000025 SOL
        let cost = gas_cost_sol_for_swap(100_000).unwrap();
        let lower = Decimal::new(1, 5); // 1e-5
        let upper = Decimal::new(1, 4); // 1e-4
        assert!(cost > lower && cost < upper, "cost={cost}");
    }
}
