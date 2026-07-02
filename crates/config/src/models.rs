use crate::error::ConfigError;
use serde::{Deserialize, Serialize};

fn invalid(msg: impl Into<String>) -> ConfigError {
    ConfigError::Validation(msg.into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub symbol: String,
    pub mint: String,
    pub decimals: u8,
}

impl WatchlistEntry {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=16).contains(&self.symbol.len()) {
            return Err(invalid(format!("symbol length out of range: {}", self.symbol)));
        }
        if !(32..=64).contains(&self.mint.len()) {
            return Err(invalid(format!("mint length out of range: {}", self.mint)));
        }
        if self.decimals > 18 {
            return Err(invalid(format!("decimals out of range: {}", self.decimals)));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhaleEntry {
    pub address: String,
    #[serde(default)]
    pub label: String,
}

impl WhaleEntry {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(32..=64).contains(&self.address.len()) {
            return Err(invalid(format!("whale address length out of range: {}", self.address)));
        }
        if self.label.len() > 64 {
            return Err(invalid("whale label too long (max 64)"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mint43() -> String {
        "A".repeat(43)
    }

    #[test]
    fn watchlist_entry_accepts_valid() {
        let e = WatchlistEntry { symbol: "SOL".into(), mint: mint43(), decimals: 9 };
        assert!(e.validate().is_ok());
    }

    #[test]
    fn watchlist_entry_rejects_bad_decimals() {
        let e = WatchlistEntry { symbol: "SOL".into(), mint: mint43(), decimals: 19 };
        assert!(e.validate().is_err());
    }

    #[test]
    fn watchlist_entry_rejects_short_mint() {
        let e = WatchlistEntry { symbol: "SOL".into(), mint: "abc".into(), decimals: 9 };
        assert!(e.validate().is_err());
    }

    #[test]
    fn whale_entry_defaults_label_to_empty() {
        let w: WhaleEntry = serde_json::from_str(&format!("{{\"address\":\"{}\"}}", mint43())).unwrap();
        assert_eq!(w.label, "");
    }
}
