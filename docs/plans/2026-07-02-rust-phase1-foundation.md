# Rust Rewrite — Phase 1 (Foundation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Rust Cargo workspace and the two foundation crates — `tradebot-common` (money type, Mode/Timeframe enums, logging) and `tradebot-config` (the full config schema with validation, load/save, and defaults) — with a Rust CI job, all passing.

**Architecture:** A Cargo workspace at the repo root with library crates under `crates/` and one placeholder binary. Foundation primitives live in `tradebot-common`; the config schema is a faithful serde port of the Python pydantic models in `tradebot/config/`, validated after deserialize. The Python code stays untouched and serves as the parity oracle.

**Tech Stack:** Rust (stable, edition 2021), `serde`/`serde_json`, `rust_decimal` (serde-float), `thiserror`, `tracing`/`tracing-subscriber`.

**Prerequisite:** A Rust toolchain (`rustup` + stable `cargo`). If `cargo --version` fails, install via https://rustup.rs before starting.

**Scope note:** The Python `tradebot/config/loader.py` (per-file YAML loader + `ConfigBundle`) is intentionally NOT ported. The runtime (`tradebot/main.py`) loads config only through the unified JSON path (`tradebot/config/file.py::load_config`). The YAML loader is dead code; porting it would violate YAGNI.

---

### Task 1: Cargo workspace skeleton

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `crates/tradebot/Cargo.toml`
- Create: `crates/tradebot/src/main.rs`
- Modify: `.gitignore`

- [ ] **Step 1: Create the workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
version = "0.1.0"
license = "MIT"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rust_decimal = { version = "1", features = ["serde-float"] }
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
```

- [ ] **Step 2: Pin the toolchain**

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create the placeholder binary crate**

Create `crates/tradebot/Cargo.toml`:

```toml
[package]
name = "tradebot"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "tradebot"
path = "src/main.rs"
```

Create `crates/tradebot/src/main.rs`:

```rust
fn main() {
    println!("tradebot {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Ignore the build directory**

Add these lines to `.gitignore` (under a new `# --- Rust ---` heading):

```gitignore
# --- Rust ---
/target/
**/*.rs.bk
```

Note: keep `Cargo.lock` tracked (this workspace produces a binary).

- [ ] **Step 5: Verify the workspace builds**

Run: `cargo build`
Expected: compiles with no errors; produces `target/debug/tradebot`.

Run: `cargo run -p tradebot`
Expected: prints `tradebot 0.1.0`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml crates/tradebot .gitignore
git commit -m "build: add Cargo workspace and placeholder binary"
```

---

### Task 2: `tradebot-common` — Money type

**Files:**
- Create: `crates/common/Cargo.toml`
- Create: `crates/common/src/lib.rs`
- Create: `crates/common/src/money.rs`

- [ ] **Step 1: Create the crate manifest**

Create `crates/common/Cargo.toml`:

```toml
[package]
name = "tradebot-common"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
rust_decimal = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 2: Create the lib root exposing the money module**

Create `crates/common/src/lib.rs`:

```rust
pub mod money;

pub use money::Money;
```

- [ ] **Step 3: Write the failing test for Money JSON formatting**

Create `crates/common/src/money.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they fail (then pass)**

Run: `cargo test -p tradebot-common money`
Expected initially: FAIL if the `serde-float` feature is not active (serialized value would be a quoted string `"50.0"` instead of `50.0`). If it fails that way, confirm `rust_decimal` in the root `Cargo.toml` has `features = ["serde-float"]`, then re-run.
Expected after: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/common
git commit -m "feat(common): add Money decimal type with JSON-number serialization"
```

---

### Task 3: `tradebot-common` — Mode and Timeframe enums

**Files:**
- Create: `crates/common/src/types.rs`
- Modify: `crates/common/src/lib.rs`

- [ ] **Step 1: Write the failing tests for Mode**

Create `crates/common/src/types.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Trading mode. Serializes to the lowercase strings the Python engine uses
/// in state-file names (`portfolio.demo.json`, etc.) and config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Demo,
    Real,
    Backtest,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Mode::Demo => "demo",
            Mode::Real => "real",
            Mode::Backtest => "backtest",
        };
        f.write_str(s)
    }
}

/// Candle timeframe. Serializes to the exact tokens used as config keys and
/// OHLCV filename suffixes (`5s`, `1m`, `15m`, `1h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    #[serde(rename = "5s")]
    FiveS,
    #[serde(rename = "1m")]
    OneM,
    #[serde(rename = "15m")]
    FifteenM,
    #[serde(rename = "1h")]
    OneH,
}

impl Timeframe {
    /// Bucket width in seconds. Mirrors `_TIMEFRAME_SECONDS` in the Python loop.
    pub fn seconds(self) -> u32 {
        match self {
            Timeframe::FiveS => 5,
            Timeframe::OneM => 60,
            Timeframe::FifteenM => 900,
            Timeframe::OneH => 3600,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Mode::Demo).unwrap(), "\"demo\"");
        assert_eq!(serde_json::to_string(&Mode::Real).unwrap(), "\"real\"");
        assert_eq!(
            serde_json::to_string(&Mode::Backtest).unwrap(),
            "\"backtest\""
        );
    }

    #[test]
    fn mode_display_matches_serialization() {
        assert_eq!(Mode::Demo.to_string(), "demo");
    }

    #[test]
    fn timeframe_roundtrips_through_tokens() {
        for (tf, token) in [
            (Timeframe::FiveS, "\"5s\""),
            (Timeframe::OneM, "\"1m\""),
            (Timeframe::FifteenM, "\"15m\""),
            (Timeframe::OneH, "\"1h\""),
        ] {
            assert_eq!(serde_json::to_string(&tf).unwrap(), token);
            let back: Timeframe = serde_json::from_str(token).unwrap();
            assert_eq!(back, tf);
        }
    }

    #[test]
    fn timeframe_seconds() {
        assert_eq!(Timeframe::FiveS.seconds(), 5);
        assert_eq!(Timeframe::OneM.seconds(), 60);
        assert_eq!(Timeframe::FifteenM.seconds(), 900);
        assert_eq!(Timeframe::OneH.seconds(), 3600);
    }
}
```

- [ ] **Step 2: Export the types module**

Update `crates/common/src/lib.rs`:

```rust
pub mod money;
pub mod types;

pub use money::Money;
pub use types::{Mode, Timeframe};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p tradebot-common types`
Expected: PASS (4 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/common
git commit -m "feat(common): add Mode and Timeframe enums with token serialization"
```

---

### Task 4: `tradebot-common` — logging init

**Files:**
- Create: `crates/common/src/logging.rs`
- Modify: `crates/common/src/lib.rs`

- [ ] **Step 1: Write the failing test for level parsing**

Create `crates/common/src/logging.rs`:

```rust
use tracing::level_filters::LevelFilter;

/// Parse a config `log_level` string (as used by the Python engine: "INFO",
/// "DEBUG", etc.) into a tracing filter. Unknown values fall back to INFO.
pub fn parse_level(level: &str) -> LevelFilter {
    match level.to_ascii_uppercase().as_str() {
        "TRACE" => LevelFilter::TRACE,
        "DEBUG" => LevelFilter::DEBUG,
        "INFO" => LevelFilter::INFO,
        "WARN" | "WARNING" => LevelFilter::WARN,
        "ERROR" => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    }
}

/// Initialize global JSON logging at the given level. Safe to call once; a
/// second call is a no-op (returns without panicking).
pub fn init_logging(level: &str, json: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::builder()
        .with_default_directive(parse_level(level).into())
        .from_env_lossy();

    let builder = fmt().with_env_filter(filter);
    let result = if json {
        builder.json().try_init()
    } else {
        builder.try_init()
    };
    // Ignore "already initialized" — callers may invoke this more than once
    // across tests or re-entrant setup.
    let _ = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn parses_known_levels_case_insensitively() {
        assert_eq!(parse_level("info"), LevelFilter::INFO);
        assert_eq!(parse_level("DEBUG"), LevelFilter::DEBUG);
        assert_eq!(parse_level("WARNING"), LevelFilter::WARN);
        assert_eq!(parse_level("error"), LevelFilter::ERROR);
    }

    #[test]
    fn unknown_level_falls_back_to_info() {
        assert_eq!(parse_level("bogus"), LevelFilter::INFO);
    }

    #[test]
    fn init_logging_does_not_panic_when_called_twice() {
        init_logging("INFO", true);
        init_logging("DEBUG", true);
    }
}
```

- [ ] **Step 2: Export the logging module**

Update `crates/common/src/lib.rs`:

```rust
pub mod logging;
pub mod money;
pub mod types;

pub use logging::{init_logging, parse_level};
pub use money::Money;
pub use types::{Mode, Timeframe};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p tradebot-common logging`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/common
git commit -m "feat(common): add tracing-based JSON logging init"
```

---

### Task 5: `tradebot-config` — crate + error type + leaf models

**Files:**
- Create: `crates/config/Cargo.toml`
- Create: `crates/config/src/lib.rs`
- Create: `crates/config/src/error.rs`
- Create: `crates/config/src/models.rs`

- [ ] **Step 1: Create the crate manifest**

Create `crates/config/Cargo.toml`:

```toml
[package]
name = "tradebot-config"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
tradebot-common = { path = "../common" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: Define the error type**

Create `crates/config/src/error.rs`:

```rust
use std::path::PathBuf;

/// Errors from loading, parsing, or validating a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config not found: {0}")]
    NotFound(PathBuf),
    #[error("invalid JSON in {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("config validation failed: {0}")]
    Validation(String),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
```

- [ ] **Step 3: Create the lib root**

Create `crates/config/src/lib.rs`:

```rust
pub mod error;
pub mod models;

pub use error::ConfigError;
```

- [ ] **Step 4: Write the failing tests for the watchlist/whale leaf models**

Create `crates/config/src/models.rs`:

```rust
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
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p tradebot-config models::tests`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/config
git commit -m "feat(config): add ConfigError and watchlist/whale leaf models"
```

---

### Task 6: `tradebot-config` — WhalesConfig, WatchlistConfig, RiskConfig

**Files:**
- Modify: `crates/config/src/models.rs`

- [ ] **Step 1: Add the three structs with serde defaults and validation**

Append to `crates/config/src/models.rs` (before the `#[cfg(test)]` block):

```rust
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
```

- [ ] **Step 2: Add tests for defaults and validators**

Add to the `#[cfg(test)] mod tests` block in `crates/config/src/models.rs`:

```rust
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
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p tradebot-config`
Expected: PASS (all prior tests plus 4 new).

- [ ] **Step 4: Commit**

```bash
git add crates/config
git commit -m "feat(config): add whales, watchlist, and risk config models"
```

---

### Task 7: `tradebot-config` — WeightsConfig, AppConfig, DashboardConfig

**Files:**
- Modify: `crates/config/src/models.rs`

- [ ] **Step 1: Add WeightsConfig with sum-to-one validation**

Append to `crates/config/src/models.rs` (before the test block):

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightsConfig {
    pub timeframes: BTreeMap<String, f64>,
    pub signals: BTreeMap<String, f64>,
}

impl WeightsConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (name, map) in [("timeframes", &self.timeframes), ("signals", &self.signals)] {
            let total: f64 = map.values().sum();
            if (total - 1.0).abs() > 1e-6 {
                return Err(invalid(format!("{name} weights must sum to 1.0, got {total}")));
            }
        }
        Ok(())
    }
}
```

Note: `BTreeMap` (not `HashMap`) keeps a deterministic key order, so serialized
weights are reproducible for the golden-file test in Task 9.

- [ ] **Step 2: Add AppConfig and DashboardConfig**

Append to `crates/config/src/models.rs` (before the test block):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub rpc_url: String,
    pub helius_api_key_env: String,
    #[serde(default = "app_log_level")] pub log_level: String,
    #[serde(default = "app_starting_capital")] pub starting_capital_usd: f64,
    #[serde(default = "app_jupiter_base_url")] pub jupiter_base_url: String,
    #[serde(default = "app_price_poll")] pub price_poll_interval_s: f64,
    #[serde(default = "app_decision_interval")] pub decision_interval_s: f64,
    #[serde(default = "app_jup_rps")] pub jupiter_rate_limit_rps: f64,
    #[serde(default = "app_jup_burst")] pub jupiter_rate_limit_burst: u32,
    #[serde(default = "app_jup_retries")] pub jupiter_max_429_retries: u32,
    #[serde(default = "app_entry_threshold")] pub entry_threshold: f64,
    #[serde(default = "app_exit_flip")] pub exit_flip_threshold: f64,
    #[serde(default = "app_helius_base_url")] pub helius_base_url: String,
    #[serde(default)] pub onchain_dex_addresses: Vec<String>,
    #[serde(default = "app_onchain_whale_min")] pub onchain_whale_min: f64,
    #[serde(default)] pub birdeye_api_key_env: String,
    #[serde(default = "app_birdeye_base_url")] pub birdeye_base_url: String,
    #[serde(default = "app_birdeye_rps")] pub birdeye_rate_limit_rps: f64,
    #[serde(default = "app_birdeye_burst")] pub birdeye_rate_limit_burst: u32,
    #[serde(default = "app_birdeye_retries")] pub birdeye_max_429_retries: u32,
    #[serde(default)] pub fast_tick_interval_s: f64,
    #[serde(default = "app_sim_fee_bps")] pub simulated_fee_bps: u32,
    #[serde(default = "ri_true")] pub dashboard_enabled: bool,
    #[serde(default = "app_dash_host")] pub dashboard_host: String,
    #[serde(default = "app_dash_port")] pub dashboard_port: u16,
    #[serde(default = "app_keystore_path")] pub keystore_path: String,
    #[serde(default)] pub priority_fee_microlamports: u64,
    #[serde(default = "app_confirmation_timeout")] pub confirmation_timeout_s: f64,
    #[serde(default = "app_starting_sol")] pub starting_sol_balance: f64,
    #[serde(default = "app_sim_confirm_latency")] pub simulated_confirm_latency_s: f64,
}

fn app_log_level() -> String { "INFO".into() }
fn app_starting_capital() -> f64 { 50.0 }
fn app_jupiter_base_url() -> String { "https://lite-api.jup.ag/swap/v1".into() }
fn app_price_poll() -> f64 { 5.0 }
fn app_decision_interval() -> f64 { 10.0 }
fn app_jup_rps() -> f64 { 0.9 }
fn app_jup_burst() -> u32 { 5 }
fn app_jup_retries() -> u32 { 3 }
fn app_entry_threshold() -> f64 { 0.6 }
fn app_exit_flip() -> f64 { -0.3 }
fn app_helius_base_url() -> String { "https://api.helius.xyz".into() }
fn app_onchain_whale_min() -> f64 { 1000.0 }
fn app_birdeye_base_url() -> String { "https://public-api.birdeye.so".into() }
fn app_birdeye_rps() -> f64 { 0.9 }
fn app_birdeye_burst() -> u32 { 2 }
fn app_birdeye_retries() -> u32 { 2 }
fn app_sim_fee_bps() -> u32 { 10 }
fn app_dash_host() -> String { "127.0.0.1".into() }
fn app_dash_port() -> u16 { 8765 }
fn app_keystore_path() -> String { "keystore/bot.keystore.json".into() }
fn app_confirmation_timeout() -> f64 { 30.0 }
fn app_starting_sol() -> f64 { 0.05 }
fn app_sim_confirm_latency() -> f64 { 1.0 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "ri_true")] pub enabled: bool,
    #[serde(default = "app_dash_host")] pub host: String,
    #[serde(default = "app_dash_port")] pub port: u16,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self { enabled: true, host: app_dash_host(), port: app_dash_port() }
    }
}
```

- [ ] **Step 2b: Add tests for weights and app config**

Add to the test block:

```rust
    #[test]
    fn weights_reject_unnormalized() {
        let w = WeightsConfig {
            timeframes: BTreeMap::from([("5s".into(), 0.5), ("1m".into(), 0.5), ("15m".into(), 0.5), ("1h".into(), 0.5)]),
            signals: BTreeMap::from([("ta".into(), 1.0)]),
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn weights_accept_normalized() {
        let w = WeightsConfig {
            timeframes: BTreeMap::from([("5s".into(), 0.1), ("1m".into(), 0.2), ("15m".into(), 0.3), ("1h".into(), 0.4)]),
            signals: BTreeMap::from([("ta".into(), 0.4), ("microstructure".into(), 0.3), ("onchain".into(), 0.3)]),
        };
        assert!(w.validate().is_ok());
    }

    #[test]
    fn app_config_applies_defaults_from_minimal_json() {
        let json = r#"{"rpc_url":"https://example.com/rpc","helius_api_key_env":"HELIUS_API_KEY"}"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(a.starting_capital_usd, 50.0);
        assert_eq!(a.dashboard_port, 8765);
        assert_eq!(a.exit_flip_threshold, -0.3);
    }

    #[test]
    fn app_config_ignores_unknown_fields() {
        // Mirrors pydantic's extra="ignore" — old configs with removed fields (e.g. db_path) still load.
        let json = r#"{"rpc_url":"x","helius_api_key_env":"y","db_path":"tradebot.db"}"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(a.rpc_url, "x");
    }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p tradebot-config`
Expected: PASS (all prior plus 4 new).

- [ ] **Step 4: Commit**

```bash
git add crates/config
git commit -m "feat(config): add weights, app, and dashboard config models"
```

---

### Task 8: `tradebot-config` — TradeBotConfig, default_config, load/save

**Files:**
- Create: `crates/config/src/file.rs`
- Create: `crates/config/src/defaults.rs`
- Modify: `crates/config/src/lib.rs`

- [ ] **Step 1: Add the top-level config and load/save in file.rs**

Create `crates/config/src/file.rs`:

```rust
use crate::error::ConfigError;
use crate::models::{AppConfig, DashboardConfig, RiskConfig, WatchlistConfig, WeightsConfig, WhalesConfig};
use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_config_version() -> u32 { 1 }
fn default_data_dir() -> String { "data".into() }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeBotConfig {
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    pub app: AppConfig,
    pub risk: RiskConfig,
    pub weights: WeightsConfig,
    pub watchlist: WatchlistConfig,
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub whales: WhalesConfig,
}

impl TradeBotConfig {
    /// Run all cross-field validation. Called by `load_config` after deserialize.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.risk.validate()?;
        self.weights.validate()?;
        self.watchlist.validate()?;
        self.whales.validate()?;
        Ok(())
    }
}

/// Load and validate a config file. Mirrors `tradebot/config/file.py::load_config`.
pub fn load_config(path: &Path) -> Result<TradeBotConfig, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::NotFound(path.to_path_buf()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })?;
    let cfg: TradeBotConfig = serde_json::from_str(&text)
        .map_err(|source| ConfigError::InvalidJson { path: path.to_path_buf(), source })?;
    cfg.validate()?;
    Ok(cfg)
}

/// Write a config as pretty JSON. Mirrors `tradebot/config/file.py::save_config`.
pub fn save_config(path: &Path, config: &TradeBotConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| ConfigError::Io { path: parent.to_path_buf(), source })?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|source| ConfigError::InvalidJson { path: path.to_path_buf(), source })?;
    std::fs::write(path, json)
        .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })?;
    Ok(())
}
```

- [ ] **Step 2: Add default_config in defaults.rs**

Create `crates/config/src/defaults.rs`:

```rust
use crate::file::TradeBotConfig;
use crate::models::{
    AppConfig, DashboardConfig, RiskConfig, WatchlistConfig, WatchlistEntry, WeightsConfig, WhalesConfig,
};
use std::collections::BTreeMap;

/// The scaffolded default config. Mirrors `tradebot/config/defaults.py::default_config`.
pub fn default_config() -> TradeBotConfig {
    let mut app = minimal_app();
    app.jupiter_base_url = "https://quote-api.jup.ag/v6".into();

    TradeBotConfig {
        config_version: 1,
        data_dir: "data".into(),
        app,
        risk: RiskConfig::default(),
        weights: WeightsConfig {
            timeframes: BTreeMap::from([
                ("5s".into(), 0.10),
                ("1m".into(), 0.20),
                ("15m".into(), 0.30),
                ("1h".into(), 0.40),
            ]),
            signals: BTreeMap::from([
                ("ta".into(), 0.40),
                ("microstructure".into(), 0.30),
                ("onchain".into(), 0.30),
            ]),
        },
        watchlist: WatchlistConfig {
            quote_symbol: "USDC".into(),
            quote_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            entries: vec![
                WatchlistEntry { symbol: "SOL".into(), mint: "So11111111111111111111111111111111111111112".into(), decimals: 9 },
                WatchlistEntry { symbol: "JUP".into(), mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".into(), decimals: 6 },
                WatchlistEntry { symbol: "JTO".into(), mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL".into(), decimals: 9 },
            ],
        },
        dashboard: DashboardConfig::default(),
        whales: WhalesConfig::default(),
    }
}

/// AppConfig with the same required fields the Python default uses, everything
/// else left at struct defaults.
fn minimal_app() -> AppConfig {
    let json = r#"{"rpc_url":"https://api.devnet.solana.com","helius_api_key_env":"HELIUS_API_KEY"}"#;
    serde_json::from_str(json).expect("minimal app config")
}
```

- [ ] **Step 3: Export the new modules**

Update `crates/config/src/lib.rs`:

```rust
pub mod defaults;
pub mod error;
pub mod file;
pub mod models;

pub use defaults::default_config;
pub use error::ConfigError;
pub use file::{load_config, save_config, TradeBotConfig};
```

- [ ] **Step 4: Write tests for load/save/default (create the tests dir)**

Create `crates/config/tests/roundtrip.rs`:

```rust
use std::path::PathBuf;
use tradebot_config::{default_config, load_config, save_config, ConfigError};

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("tradebot_cfg_test_{}_{}", std::process::id(), name));
    p
}

#[test]
fn default_config_is_valid_and_has_expected_fields() {
    let cfg = default_config();
    assert_eq!(cfg.app.starting_capital_usd, 50.0);
    assert_eq!(cfg.dashboard.port, 8765);
    assert!(cfg.watchlist.entries.iter().any(|e| e.symbol == "SOL"));
    assert!(cfg.validate().is_ok());
}

#[test]
fn save_and_load_roundtrip() {
    let cfg = default_config();
    let path = tmp("roundtrip.json");
    save_config(&path, &cfg).unwrap();
    let loaded = load_config(&path).unwrap();
    assert_eq!(loaded.app.starting_capital_usd, cfg.app.starting_capital_usd);
    assert_eq!(loaded.risk.max_concurrent_positions, cfg.risk.max_concurrent_positions);
    assert_eq!(loaded, cfg);
    std::fs::remove_file(&path).ok();
}

#[test]
fn load_missing_file_is_not_found() {
    let err = load_config(&tmp("does_not_exist.json")).unwrap_err();
    assert!(matches!(err, ConfigError::NotFound(_)));
}

#[test]
fn load_invalid_json_errors() {
    let path = tmp("bad.json");
    std::fs::write(&path, "{ not json").unwrap();
    let err = load_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::InvalidJson { .. }));
    std::fs::remove_file(&path).ok();
}

#[test]
fn load_schema_violation_errors() {
    // app present but weights don't sum to 1.0 -> validation error
    let path = tmp("badweights.json");
    let mut cfg = default_config();
    cfg.weights.signals.insert("ta".into(), 0.9); // now sums > 1
    // write raw so load re-validates
    save_config(&path, &cfg).unwrap();
    let err = load_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::Validation(_)));
    std::fs::remove_file(&path).ok();
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p tradebot-config`
Expected: PASS (unit tests plus 5 integration tests).

- [ ] **Step 6: Commit**

```bash
git add crates/config
git commit -m "feat(config): add TradeBotConfig, default_config, and load/save"
```

---

### Task 9: Cross-language parity — Rust default matches Python default

**Files:**
- Create: `crates/config/tests/fixtures/python_default_config.json`
- Create: `crates/config/tests/parity.rs`

- [ ] **Step 1: Capture the Python default config as a fixture**

Run this from the repo root (uses the existing Python venv):

```bash
.venv/Scripts/python.exe -c "from tradebot.config.defaults import default_config; from tradebot.config.file import save_config; from pathlib import Path; save_config(Path('crates/config/tests/fixtures/python_default_config.json'), default_config())"
```

Expected: creates `crates/config/tests/fixtures/python_default_config.json`.
Confirm it exists and contains `"starting_capital_usd": 50.0`.

- [ ] **Step 2: Write the parity test comparing structure, not byte order**

Create `crates/config/tests/parity.rs`:

```rust
use tradebot_config::default_config;

/// The Rust `default_config()` must produce the same config the Python
/// `default_config()` does. Compare as parsed JSON values so object key order
/// and formatting differences don't cause false failures.
#[test]
fn rust_default_matches_python_default() {
    let fixture = include_str!("fixtures/python_default_config.json");
    let python: serde_json::Value = serde_json::from_str(fixture).unwrap();

    let rust_str = serde_json::to_string(&default_config()).unwrap();
    let rust: serde_json::Value = serde_json::from_str(&rust_str).unwrap();

    assert_eq!(rust, python, "Rust default_config diverged from Python default_config");
}
```

- [ ] **Step 3: Run the parity test**

Run: `cargo test -p tradebot-config --test parity`
Expected: PASS.

If it fails, the assertion diff shows exactly which field or default diverged
(for example a wrong numeric default or a missing field). Fix the Rust model to
match Python, not the fixture.

- [ ] **Step 4: Commit**

```bash
git add crates/config/tests
git commit -m "test(config): parity test — Rust default matches Python default"
```

---

### Task 10: Rust CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add a Rust job alongside the existing Python job**

Add this job under the existing `jobs:` map in `.github/workflows/ci.yml` (keep the Python `test` job as-is):

```yaml
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        run: rustup toolchain install stable --profile minimal --component rustfmt clippy

      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: Format check
        run: cargo fmt --all --check

      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings

      - name: Tests
        run: cargo test --all
```

- [ ] **Step 2: Verify locally that the same commands pass**

Run: `cargo fmt --all --check`
Expected: no output, exit 0. (If it reports diffs, run `cargo fmt --all` and re-commit.)

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings, exit 0.

Run: `cargo test --all`
Expected: all tests pass across `tradebot-common` and `tradebot-config`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add Rust fmt/clippy/test job"
```

---

## Self-Review

**Spec coverage (against the Phase 1 line in the design doc — "workspace, `tradebot-config`, tracing logging, error types, the Decimal money type, and the Rust CI job"):**
- Workspace → Task 1
- Decimal money type → Task 2
- Mode/Timeframe shared types → Task 3 (needed by config + later phases)
- tracing logging → Task 4
- error types → Task 5 (`ConfigError`)
- `tradebot-config` full schema → Tasks 5–8
- default_config + load/save → Task 8
- Cross-language parity → Task 9 (de-risks the whole rewrite early)
- Rust CI job → Task 10

**Placeholder scan:** No TBD/TODO. Every code step contains complete, compilable code. Validation, defaults, and error handling are shown explicitly, not described.

**Type consistency:** `ConfigError` variants (`NotFound`, `InvalidJson`, `Validation`, `Io`) are defined in Task 5 and used consistently in Tasks 6–8. `TradeBotConfig` is defined in Task 8 and consumed in Tasks 8–9. `default_config` / `load_config` / `save_config` signatures match across tasks. `ri_true()` is defined in Task 6 and reused in Task 7 (AppConfig/DashboardConfig booleans) — deliberate reuse, same crate module.

**Known follow-ups (out of Phase 1 scope, tracked for later phases):**
- `rust_decimal`'s `serde-float` feature name should be confirmed against the resolved crate version during Task 2; the test in Task 2 Step 4 catches a mismatch.
- Money is defined but not yet consumed; the storage crate (Phase 2) is its first real user, where number-format parity with Python state files gets exercised.
