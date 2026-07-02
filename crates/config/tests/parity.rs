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

    assert_eq!(
        rust, python,
        "Rust default_config diverged from Python default_config"
    );
}
