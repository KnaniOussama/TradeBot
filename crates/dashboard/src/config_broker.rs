//! Port of `tradebot/dashboard/config_broker.py`: holds the config file
//! path and the currently-active `TradeBotConfig`, and applies validated
//! updates by writing to disk then reloading (so `current()` always
//! reflects exactly what is on disk, the same round-trip the Python
//! `ConfigBroker` does).

use std::path::PathBuf;
use std::sync::RwLock;

use tradebot_config::{load_config, save_config, ConfigError, TradeBotConfig};

pub struct ConfigBroker {
    path: PathBuf,
    current: RwLock<TradeBotConfig>,
}

impl ConfigBroker {
    pub fn new(path: PathBuf, current: TradeBotConfig) -> Self {
        Self {
            path,
            current: RwLock::new(current),
        }
    }

    /// The currently-active config. Mirrors `current()` in config_broker.py.
    pub fn current(&self) -> TradeBotConfig {
        self.current
            .read()
            .expect("config broker lock poisoned")
            .clone()
    }

    /// Saves `new_cfg` to disk, then reloads it as the new `current()`.
    /// Mirrors `update()` in config_broker.py.
    pub fn update(&self, new_cfg: TradeBotConfig) -> Result<(), ConfigError> {
        save_config(&self.path, &new_cfg)?;
        let reloaded = load_config(&self.path)?;
        *self.current.write().expect("config broker lock poisoned") = reloaded;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_config::default_config;

    #[test]
    fn update_saves_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        let cfg = default_config();
        save_config(&path, &cfg).unwrap();
        let broker = ConfigBroker::new(path.clone(), cfg);

        let mut new_cfg = broker.current();
        new_cfg.app.starting_capital_usd = 100.0;
        broker.update(new_cfg).unwrap();

        assert_eq!(broker.current().app.starting_capital_usd, 100.0);
        let reloaded = load_config(&path).unwrap();
        assert_eq!(reloaded.app.starting_capital_usd, 100.0);
    }
}
