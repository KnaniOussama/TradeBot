//! Port of `tests/execution/test_real.py`.
//!
//! The full-path test stubs out `TxSigner` (like Python's `_stub_sign_and_send`)
//! since the recorded `jupiter_swap_response.json` fixture's `swapTransaction`
//! is a 5-byte placeholder, not a real serialized `VersionedTransaction` --
//! see `signing_tests.rs` for coverage of the actual deserialize/sign/send
//! mechanics against a self-constructed transaction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::Value;
use tempfile::tempdir;
use tradebot_common::Mode;
use tradebot_core::Portfolio;
use tradebot_data::{JupiterClient, SolanaRpcClient};
use tradebot_execution::{ExecutionError, Executor, Order, RealExecutor, TxSigner};
use tradebot_storage::{JsonStorage, Side};
use tradebot_wallet::{generate_bot_keypair, BotKeypair};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn base_mints() -> HashMap<String, (String, u32)> {
    let mut m = HashMap::new();
    m.insert("SOL/USDC".to_string(), (SOL_MINT.to_string(), 9));
    m
}

struct StubSigner {
    signature: String,
}

#[async_trait]
impl TxSigner for StubSigner {
    async fn sign_and_send(
        &self,
        _serialized_tx_b64: &str,
        _keypair: &BotKeypair,
        _rpc: &SolanaRpcClient,
    ) -> Result<String, ExecutionError> {
        Ok(self.signature.clone())
    }
}

#[tokio::test]
async fn real_buy_full_path() {
    let quote_payload = load_fixture("jupiter_quote_sol_usdc.json");
    let swap_payload = load_fixture("jupiter_swap_response.json");
    let sig_status_payload = load_fixture("rpc_get_signature_statuses.json");

    let jupiter_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&quote_payload))
        .mount(&jupiter_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/swap"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&swap_payload))
        .mount(&jupiter_server)
        .await;

    let rpc_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("getSignatureStatuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&sig_status_payload))
        .mount(&rpc_server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(jupiter_server.uri(), None, 3);
    let rpc = SolanaRpcClient::new(rpc_server.uri());
    let kp = generate_bot_keypair();
    let mut p = Portfolio::new(Mode::Real, dec("100.0"), dec("1.0"));

    let ex = RealExecutor::new(
        jupiter,
        rpc,
        &storage,
        &kp,
        base_mints(),
        USDC_MINT,
        6,
        0.10,
        0,
        2.0,
        Some(Box::new(StubSigner {
            signature: "REALSIG".to_string(),
        })),
    );

    let order = Order::buy("SOL/USDC", dec("10.0"));
    let fill = ex.execute(&order, &mut p, Utc::now()).await.unwrap();

    assert_eq!(fill.tx_signature, Some("REALSIG".to_string()));
    assert_eq!(fill.side, Side::Buy);
    assert!(fill.base_amount > Decimal::ZERO);
    assert!(p.cash < dec("100.0"));
    assert!(p.position_for("SOL/USDC").is_some());

    let trades = storage.list_trades(Mode::Real, 10);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].tx_signature, Some("REALSIG".to_string()));
}

#[tokio::test]
async fn real_rejects_high_slippage() {
    let thin_payload = load_fixture("jupiter_quote_thin.json");

    let jupiter_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&thin_payload))
        .mount(&jupiter_server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(jupiter_server.uri(), None, 3);
    let rpc = SolanaRpcClient::new("https://example.invalid");
    let kp = generate_bot_keypair();
    let mut p = Portfolio::new(Mode::Real, dec("100.0"), dec("1.0"));

    let ex = RealExecutor::new(
        jupiter,
        rpc,
        &storage,
        &kp,
        base_mints(),
        USDC_MINT,
        6,
        0.01,
        0,
        2.0,
        Some(Box::new(StubSigner {
            signature: "UNUSED".to_string(),
        })),
    );

    let result = ex
        .execute(&Order::buy("SOL/USDC", dec("10.0")), &mut p, Utc::now())
        .await;
    match result {
        Err(ExecutionError::Invalid(msg)) => assert!(msg.contains("slippage"), "msg={msg}"),
        other => panic!("expected slippage error, got {other:?}"),
    }
}

#[tokio::test]
async fn real_rejects_unknown_pair() {
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new("https://example.invalid", None, 3);
    let rpc = SolanaRpcClient::new("https://example.invalid");
    let kp = generate_bot_keypair();
    let mut p = Portfolio::new(Mode::Real, dec("100.0"), dec("1.0"));

    let ex = RealExecutor::new(
        jupiter,
        rpc,
        &storage,
        &kp,
        HashMap::new(),
        USDC_MINT,
        6,
        0.01,
        0,
        2.0,
        Some(Box::new(StubSigner {
            signature: "UNUSED".to_string(),
        })),
    );

    let result = ex
        .execute(&Order::buy("X/USDC", dec("10.0")), &mut p, Utc::now())
        .await;
    match result {
        Err(ExecutionError::Invalid(msg)) => assert!(msg.contains("unknown pair"), "msg={msg}"),
        other => panic!("expected unknown pair error, got {other:?}"),
    }
}
