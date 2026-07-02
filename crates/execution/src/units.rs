//! Conversions between human-readable `Money` amounts and the integer
//! base-unit amounts Jupiter and the Solana RPC speak.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tradebot_common::Money;

use crate::base::ExecutionError;

fn pow10(decimals: u32) -> Decimal {
    Decimal::from(10u64.pow(decimals))
}

/// Converts a human-readable amount to base units, truncating toward zero
/// like Python's `int(amount * 10**decimals)`.
pub(crate) fn to_units(amount: Money, decimals: u32) -> Result<u64, ExecutionError> {
    let scaled = (amount * pow10(decimals)).trunc();
    scaled
        .to_u64()
        .ok_or_else(|| ExecutionError::Invalid(format!("amount out of range: {amount}")))
}

/// Converts base units back to a human-readable `Money` amount.
pub(crate) fn from_units(units: u64, decimals: u32) -> Money {
    Decimal::from(units) / pow10(decimals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_units_scales_and_truncates() {
        let units = to_units(Decimal::new(15, 1), 9).unwrap(); // 1.5 SOL
        assert_eq!(units, 1_500_000_000);
    }

    #[test]
    fn from_units_scales_down() {
        let amount = from_units(15_050_000, 6); // 15.05 USDC
        assert_eq!(amount, Decimal::new(1505, 2));
    }

    #[test]
    fn round_trip_is_exact_for_clean_amounts() {
        let original = Decimal::new(1, 1); // 0.1
        let units = to_units(original, 9).unwrap();
        assert_eq!(units, 100_000_000);
        assert_eq!(from_units(units, 9), original);
    }
}
