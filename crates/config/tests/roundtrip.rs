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
    assert_eq!(
        loaded.app.starting_capital_usd,
        cfg.app.starting_capital_usd
    );
    assert_eq!(
        loaded.risk.max_concurrent_positions,
        cfg.risk.max_concurrent_positions
    );
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
