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

fn default_lookback_minutes() -> u32 { 30 }
fn default_decay_half_life_minutes() -> f64 { 10.0 }
fn default_per_wallet_swap_limit() -> u32 { 20 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhalesConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub wallets: Vec<WhaleEntry>,
    #[serde(default = "default_lookback_minutes")]
    pub lookback_minutes: u32,
    #[serde(default = "default_decay_half_life_minutes")]
    pub decay_half_life_minutes: f64,
    #[serde(default = "default_per_wallet_swap_limit")]
    pub per_wallet_swap_limit: u32,
}

impl Default for WhalesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            wallets: Vec::new(),
            lookback_minutes: default_lookback_minutes(),
            decay_half_life_minutes: default_decay_half_life_minutes(),
            per_wallet_swap_limit: default_per_wallet_swap_limit(),
        }
    }
}

impl WhalesConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        for w in &self.wallets {
            w.validate()?;
        }
        if !(1..=1440).contains(&self.lookback_minutes) {
            return Err(invalid("lookback_minutes out of range [1,1440]"));
        }
        if !(self.decay_half_life_minutes > 0.0 && self.decay_half_life_minutes <= 720.0) {
            return Err(invalid("decay_half_life_minutes out of range (0,720]"));
        }
        if !(1..=100).contains(&self.per_wallet_swap_limit) {
            return Err(invalid("per_wallet_swap_limit out of range [1,100]"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchlistConfig {
    pub quote_symbol: String,
    pub quote_mint: String,
    pub entries: Vec<WatchlistEntry>,
}

impl WatchlistConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = std::collections::HashSet::new();
        for e in &self.entries {
            e.validate()?;
            if !seen.insert(&e.symbol) {
                return Err(invalid(format!("duplicate symbol: {}", e.symbol)));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskConfig {
    #[serde(default = "ri_max_concurrent")] pub max_concurrent_positions: u32,
    #[serde(default = "ri_size_min")] pub per_trade_size_min: f64,
    #[serde(default = "ri_size_max")] pub per_trade_size_max: f64,
    #[serde(default = "ri_trailing")] pub trailing_stop_pct: f64,
    #[serde(default = "ri_take_profit")] pub take_profit_pct: f64,
    #[serde(default = "ri_daily")] pub daily_loss_limit_pct: f64,
    #[serde(default = "ri_weekly")] pub weekly_loss_limit_pct: f64,
    #[serde(default = "ri_kill")] pub per_trade_kill_pct: f64,
    #[serde(default = "ri_drawdown")] pub drawdown_circuit_pct: f64,
    #[serde(default = "ri_slippage")] pub max_slippage_pct: f64,
    #[serde(default = "ri_max_trades")] pub max_trades_per_day: u32,
    #[serde(default = "ri_tp_ladder_pct")] pub tp_ladder_pct: f64,
    #[serde(default = "ri_tp_ladder_frac")] pub tp_ladder_fraction: f64,
    #[serde(default = "ri_true")] pub regime_filter_enabled: bool,
    #[serde(default = "ri_true")] pub regime_block_chop: bool,
    #[serde(default = "ri_true")] pub use_kelly_sizing: bool,
}

fn ri_max_concurrent() -> u32 { 3 }
fn ri_size_min() -> f64 { 0.30 }
fn ri_size_max() -> f64 { 0.50 }
fn ri_trailing() -> f64 { 0.02 }
fn ri_take_profit() -> f64 { 0.02 }
fn ri_daily() -> f64 { 0.08 }
fn ri_weekly() -> f64 { 0.10 }
fn ri_kill() -> f64 { 0.03 }
fn ri_drawdown() -> f64 { 0.15 }
fn ri_slippage() -> f64 { 0.01 }
fn ri_max_trades() -> u32 { 10 }
fn ri_tp_ladder_pct() -> f64 { 0.02 }
fn ri_tp_ladder_frac() -> f64 { 0.5 }
fn ri_true() -> bool { true }

impl Default for RiskConfig {
    fn default() -> Self {
        serde_json::from_str("{}").expect("RiskConfig all-default deserialize")
    }
}

impl RiskConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.per_trade_size_min > self.per_trade_size_max {
            return Err(invalid("per_trade_size_min must be <= per_trade_size_max"));
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

    #[test]
    fn risk_config_defaults_match_python() {
        let r = RiskConfig::default();
        assert_eq!(r.max_concurrent_positions, 3);
        assert_eq!(r.per_trade_size_min, 0.30);
        assert_eq!(r.per_trade_size_max, 0.50);
        assert_eq!(r.trailing_stop_pct, 0.02);
        assert_eq!(r.daily_loss_limit_pct, 0.08);
        assert_eq!(r.weekly_loss_limit_pct, 0.10);
        assert_eq!(r.per_trade_kill_pct, 0.03);
        assert_eq!(r.drawdown_circuit_pct, 0.15);
        assert_eq!(r.max_slippage_pct, 0.01);
        assert_eq!(r.max_trades_per_day, 10);
        assert!(r.regime_filter_enabled);
        assert!(r.use_kelly_sizing);
    }

    #[test]
    fn risk_config_rejects_inverted_size_range() {
        let r = RiskConfig { per_trade_size_min: 0.6, per_trade_size_max: 0.4, ..RiskConfig::default() };
        assert!(r.validate().is_err());
    }

    #[test]
    fn whales_config_default_is_disabled() {
        let w = WhalesConfig::default();
        assert!(!w.enabled);
        assert_eq!(w.lookback_minutes, 30);
        assert_eq!(w.per_wallet_swap_limit, 20);
    }

    #[test]
    fn watchlist_rejects_duplicate_symbols() {
        let w = WatchlistConfig {
            quote_symbol: "USDC".into(),
            quote_mint: "E".repeat(44),
            entries: vec![
                WatchlistEntry { symbol: "SOL".into(), mint: "A".repeat(43), decimals: 9 },
                WatchlistEntry { symbol: "SOL".into(), mint: "B".repeat(43), decimals: 9 },
            ],
        };
        assert!(w.validate().is_err());
    }
}
