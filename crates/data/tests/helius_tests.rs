use std::path::{Path, PathBuf};

use serde_json::Value;
use tradebot_data::{get_recent_swaps_for_wallet, HeliusClient};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WALLET: &str = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOL: &str = "So11111111111111111111111111111111111111112";

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

fn swap_tx(
    sent_mint: &str,
    sent_raw: i64,
    received_mint: &str,
    received_raw: i64,
    ts: i64,
) -> Value {
    serde_json::json!({
        "signature": "S".repeat(88),
        "timestamp": ts,
        "tokenTransfers": [
            {
                "mint": sent_mint,
                "fromUserAccount": WALLET,
                "toUserAccount": "Pool111",
                "rawTokenAmount": {"tokenAmount": sent_raw.to_string(), "decimals": 6},
                "tokenAmount": sent_raw as f64 / 1_000_000.0,
            },
            {
                "mint": received_mint,
                "fromUserAccount": "Pool111",
                "toUserAccount": WALLET,
                "rawTokenAmount": {"tokenAmount": received_raw.to_string(), "decimals": 9},
                "tokenAmount": received_raw as f64 / 1_000_000_000.0,
            },
        ],
    })
}

#[tokio::test]
async fn recent_token_transfers_parses_fixture() {
    let payload = load_fixture("helius_enhanced_txs.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v0/addresses/.*/transactions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = HeliusClient::with_base_url("test", server.uri());
    let transfers = client
        .recent_token_transfers("DEX1", 100)
        .await
        .expect("should succeed");
    assert_eq!(transfers.len(), 3);
    let t0 = &transfers[0];
    assert_eq!(t0.amount, 5000.0);
    assert_eq!(t0.mint, "So11111111111111111111111111111111111111112");
}

#[tokio::test]
async fn recent_token_transfers_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v0/addresses/.*/transactions$"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = HeliusClient::with_base_url("test", server.uri());
    let result = client.recent_token_transfers("X", 10).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn get_recent_swaps_parses_response() {
    let payload = vec![
        swap_tx(USDC, 10_000_000, SOL, 70_000_000, 1_714_742_400),
        swap_tx(SOL, 70_000_000, USDC, 10_500_000, 1_714_742_500),
    ];
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v0/addresses/.*/transactions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = HeliusClient::with_base_url("test-key", server.uri());
    let swaps = get_recent_swaps_for_wallet(&client, WALLET, 10)
        .await
        .expect("should succeed");
    assert_eq!(swaps.len(), 2);
    assert_eq!(swaps[0].in_mint, USDC);
    assert_eq!(swaps[0].out_mint, SOL);
    assert_eq!(swaps[1].in_mint, SOL);
    assert_eq!(swaps[1].out_mint, USDC);
}

#[tokio::test]
async fn get_recent_swaps_returns_empty_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v0/addresses/.*/transactions$"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = HeliusClient::with_base_url("test-key", server.uri());
    let swaps = get_recent_swaps_for_wallet(&client, WALLET, 10)
        .await
        .expect("should not error on http status");
    assert!(swaps.is_empty());
}
