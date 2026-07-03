use std::path::{Path, PathBuf};

use serde_json::Value;
use tradebot_data::{RpcError, SolanaRpcClient};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

#[tokio::test]
async fn get_balance_lamports_parses_response() {
    let payload = load_fixture("rpc_get_balance.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let lamports = client
        .get_balance_lamports("11111111111111111111111111111111")
        .await
        .expect("should succeed");
    assert_eq!(lamports, 1_234_567_890);
}

#[tokio::test]
async fn get_balance_sol_converts_from_lamports() {
    let payload = load_fixture("rpc_get_balance.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let sol = client
        .get_balance_sol("11111111111111111111111111111111")
        .await
        .expect("should succeed");
    assert!((sol - 1.23456789).abs() < 1e-9, "sol={sol}");
}

#[tokio::test]
async fn rpc_json_error_raises() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": {"code": -32601, "message": "Method not found"},
            "id": 1,
        })))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let result = client
        .get_balance_lamports("11111111111111111111111111111111")
        .await;
    match result {
        Err(RpcError::JsonRpc { code, message }) => {
            assert_eq!(code, -32601);
            assert_eq!(message, "Method not found");
        }
        other => panic!("expected JsonRpc error, got {other:?}"),
    }
}

#[tokio::test]
async fn get_token_accounts_by_owner_parses_response() {
    let payload = load_fixture("rpc_get_token_accounts_by_owner.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let accounts = client
        .get_token_accounts_by_owner(
            "OwnerAddr1111111111111111111111111111111111",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        )
        .await
        .expect("should succeed");
    assert_eq!(accounts.len(), 1);
    let ui_amount = accounts[0]["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmount"]
        .as_f64()
        .unwrap();
    assert_eq!(ui_amount, 1.5);
}

#[tokio::test]
async fn send_raw_transaction_returns_signature() {
    let payload = load_fixture("rpc_send_tx.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let sig = client
        .send_raw_transaction("AQABAgM=", false)
        .await
        .expect("should succeed");
    assert!(sig.starts_with("5J7q9X3z2Y8w4"));
}

#[tokio::test]
async fn get_latest_blockhash_parses_response() {
    let payload = load_fixture("rpc_get_latest_blockhash.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let bh = client.get_latest_blockhash().await.expect("should succeed");
    assert!(bh.blockhash.starts_with("5J7q9X3z2Y8w4"));
    assert_eq!(bh.last_valid_block_height, 350_001_010);
}

#[tokio::test]
async fn confirm_signature_success() {
    let payload = load_fixture("rpc_get_signature_statuses.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let ok = client
        .confirm_signature("sig123", 2.0, 0.05)
        .await
        .expect("should confirm");
    assert!(ok);
}

#[tokio::test]
async fn confirm_signature_fails_on_err() {
    let mut payload = load_fixture("rpc_get_signature_statuses.json");
    payload["result"]["value"][0]["err"] = serde_json::json!({"InstructionError": [0, "Custom"]});
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let result = client.confirm_signature("sig123", 2.0, 0.05).await;
    assert!(matches!(result, Err(RpcError::ConfirmationFailed { .. })));
}

#[tokio::test]
async fn confirm_signature_times_out() {
    let pending = serde_json::json!({
        "jsonrpc": "2.0",
        "result": {"context": {"slot": 1}, "value": [null]},
        "id": 1,
    });
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&pending))
        .mount(&server)
        .await;

    let client = SolanaRpcClient::new(server.uri());
    let result = client.confirm_signature("sig123", 0.2, 0.05).await;
    assert!(matches!(result, Err(RpcError::ConfirmationTimeout { .. })));
}
