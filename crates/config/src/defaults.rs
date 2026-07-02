use crate::file::TradeBotConfig;
use crate::models::{
    AppConfig, DashboardConfig, RiskConfig, WatchlistConfig, WatchlistEntry, WeightsConfig,
    WhalesConfig,
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
                WatchlistEntry {
                    symbol: "SOL".into(),
                    mint: "So11111111111111111111111111111111111111112".into(),
                    decimals: 9,
                },
                WatchlistEntry {
                    symbol: "JUP".into(),
                    mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".into(),
                    decimals: 6,
                },
                WatchlistEntry {
                    symbol: "JTO".into(),
                    mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL".into(),
                    decimals: 9,
                },
            ],
        },
        dashboard: DashboardConfig::default(),
        whales: WhalesConfig::default(),
    }
}

/// AppConfig with the same required fields the Python default uses, everything
/// else left at struct defaults.
fn minimal_app() -> AppConfig {
    let json =
        r#"{"rpc_url":"https://api.devnet.solana.com","helius_api_key_env":"HELIUS_API_KEY"}"#;
    serde_json::from_str(json).expect("minimal app config")
}
