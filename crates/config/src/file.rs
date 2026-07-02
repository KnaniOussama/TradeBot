use crate::error::ConfigError;
use crate::models::{
    AppConfig, DashboardConfig, RiskConfig, WatchlistConfig, WeightsConfig, WhalesConfig,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_config_version() -> u32 {
    1
}
fn default_data_dir() -> String {
    "data".into()
}

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
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let cfg: TradeBotConfig =
        serde_json::from_str(&text).map_err(|source| ConfigError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    cfg.validate()?;
    Ok(cfg)
}

/// Write a config as pretty JSON. Mirrors `tradebot/config/file.py::save_config`.
pub fn save_config(path: &Path, config: &TradeBotConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|source| ConfigError::InvalidJson {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::write(path, json).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}
