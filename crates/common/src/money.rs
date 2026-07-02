//! Money is a fixed-point decimal. The `serde-float` feature on `rust_decimal`
//! makes it (de)serialize as a JSON number (e.g. `50.0`), matching the float
//! representation the Python engine writes into `data/` state files.

pub type Money = rust_decimal::Decimal;

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn money_serializes_as_json_number() {
        let m: Money = Decimal::new(500, 1); // 50.0
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, "50.0");
    }

    #[test]
    fn money_deserializes_from_json_number() {
        let m: Money = serde_json::from_str("50").unwrap();
        assert_eq!(m, Decimal::new(50, 0));
    }

    #[test]
    fn money_arithmetic_is_exact() {
        let a = Decimal::new(10, 2); // 0.10
        let b = Decimal::new(20, 2); // 0.20
        assert_eq!(a + b, Decimal::new(30, 2)); // 0.30 exactly, no float drift
    }
}
