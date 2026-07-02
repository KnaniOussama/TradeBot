//! Port of `tests/test_main.py` and `tests/test_main_wallet_cli.py`: runs
//! the built `tradebot` binary end-to-end and asserts on its exit code and
//! output, the way the Python tests drive `click`'s `CliRunner`.

use std::collections::BTreeMap;
use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bin() -> Command {
    Command::cargo_bin("tradebot").expect("tradebot binary built")
}

// --- init ---

#[test]
fn init_creates_default_config() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("tb.config.json");

    bin()
        .args(["init", "--config", config_path.to_str().unwrap()])
        .assert()
        .success();

    assert!(config_path.exists());
    let text = fs::read_to_string(&config_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["app"]["starting_capital_usd"], 50.0);
}

#[test]
fn init_refuses_overwrite() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("tb.config.json");
    fs::write(&config_path, "{}").unwrap();

    bin()
        .args(["init", "--config", config_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

// --- wallet ---

#[test]
fn wallet_generate_creates_keystore() {
    let dir = tempdir().unwrap();
    let keystore = dir.path().join("k.json");

    bin()
        .env("TRADEBOT_PASSPHRASE", "test-pp")
        .args([
            "wallet",
            "generate",
            "--keystore",
            keystore.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Bot address:"));

    assert!(keystore.exists());
}

#[test]
fn wallet_generate_refuses_overwrite() {
    let dir = tempdir().unwrap();
    let keystore = dir.path().join("k.json");
    fs::write(&keystore, "{}").unwrap();

    bin()
        .env("TRADEBOT_PASSPHRASE", "test-pp")
        .args([
            "wallet",
            "generate",
            "--keystore",
            keystore.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to overwrite"));
}

#[test]
fn wallet_show_prints_address() {
    let dir = tempdir().unwrap();
    let keystore = dir.path().join("k.json");

    bin()
        .env("TRADEBOT_PASSPHRASE", "test-pp")
        .args([
            "wallet",
            "generate",
            "--keystore",
            keystore.to_str().unwrap(),
        ])
        .assert()
        .success();

    bin()
        .env("TRADEBOT_PASSPHRASE", "test-pp")
        .args(["wallet", "show", "--keystore", keystore.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Bot address:"));
}

#[test]
fn wallet_no_passphrase_aborts() {
    let dir = tempdir().unwrap();
    let keystore = dir.path().join("k.json");

    bin()
        .env_remove("TRADEBOT_PASSPHRASE")
        .args([
            "wallet",
            "generate",
            "--keystore",
            keystore.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("TRADEBOT_PASSPHRASE"));
}

// --- start ---

#[test]
fn start_real_without_confirm_aborts() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("tb.config.json");
    bin()
        .args(["init", "--config", config_path.to_str().unwrap()])
        .assert()
        .success();

    bin()
        .env("TRADEBOT_PASSPHRASE", "x")
        .args([
            "start",
            "--config",
            config_path.to_str().unwrap(),
            "--mode",
            "real",
        ])
        .assert()
        .failure();
}

#[test]
fn start_missing_config_aborts() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("does-not-exist.json");

    bin()
        .args([
            "start",
            "--config",
            config_path.to_str().unwrap(),
            "--mode",
            "demo",
        ])
        .assert()
        .failure();
}

/// Runs `start --mode demo --max-cycles 2` against a config that points
/// Jupiter at a local mock server (so it is fully offline) and asserts the
/// bot completes a couple of decision cycles and persists portfolio/risk
/// state, mirroring the Python suite's "init then start for a couple
/// cycles" smoke coverage.
#[tokio::test]
async fn start_demo_runs_a_few_cycles_offline() {
    let quote_fixture = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/jupiter_quote_sol_usdc.json"),
    )
    .expect("jupiter quote fixture readable");
    let quote_json: serde_json::Value =
        serde_json::from_str(&quote_fixture).expect("fixture is valid json");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&quote_json))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let config_path = dir.path().join("tb.config.json");
    let data_dir = dir.path().join("data");

    let mut cfg = tradebot_config::default_config();
    cfg.data_dir = data_dir.to_str().unwrap().to_string();
    cfg.app.jupiter_base_url = server.uri();
    cfg.app.jupiter_max_429_retries = 0;
    cfg.dashboard.enabled = false;
    // Only the offline TA signal; microstructure/onchain would need extra
    // Jupiter/Helius calls this test does not mock.
    cfg.weights.signals = BTreeMap::from([("ta".to_string(), 1.0)]);
    tradebot_config::save_config(&config_path, &cfg).expect("save test config");

    bin()
        .args([
            "start",
            "--config",
            config_path.to_str().unwrap(),
            "--mode",
            "demo",
            "--max-cycles",
            "2",
        ])
        .assert()
        .success();

    assert!(data_dir.join("portfolio.demo.json").exists());
    assert!(data_dir.join("risk_state.demo.json").exists());
}
