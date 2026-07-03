use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tradebot_data::{JupiterClient, TokenBucketLimiter};
use wiremock::matchers::{method, path};
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
async fn quote_parses_response() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 3);
    let q = client
        .quote(
            "So11111111111111111111111111111111111111112",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            100_000_000,
            50,
        )
        .await
        .expect("quote should succeed");

    assert_eq!(q.in_amount, 100_000_000);
    assert_eq!(q.out_amount, 15_050_000);
    assert_eq!(q.price_impact_pct, 0.0012);
    assert_eq!(q.slippage_bps, 50);
    assert_eq!(q.route_labels, vec!["Raydium".to_string()]);
}

#[tokio::test]
async fn quote_implied_price_helper() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 3);
    let q = client
        .quote(
            "So11111111111111111111111111111111111111112",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            100_000_000,
            50,
        )
        .await
        .expect("quote should succeed");

    // 0.1 SOL (9 decimals) -> 15.05 USDC (6 decimals) => price 150.5 USDC/SOL
    let price = q.implied_price(9, 6);
    assert!((price - 150.5).abs() < 1e-6, "price={price}");
}

#[tokio::test]
async fn quote_http_error_raises() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(serde_json::json!({"error": "boom"})),
        )
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 3);
    let result = client.quote(&"A".repeat(43), &"B".repeat(43), 1, 50).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn quote_retries_on_429_then_succeeds() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .with_priority(2)
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 2);
    let q = client
        .quote(&"A".repeat(43), &"B".repeat(43), 1_000_000, 50)
        .await
        .expect("should succeed after retry");
    assert_eq!(q.in_amount, 100_000_000);
}

#[tokio::test]
async fn quote_gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 2);
    let result = client
        .quote(&"A".repeat(43), &"B".repeat(43), 1_000_000, 50)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn quote_with_limiter_acquires_token() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let limiter = Arc::new(TokenBucketLimiter::new(10.0, 1));
    let client = JupiterClient::new(server.uri(), Some(limiter.clone()), 3);
    client
        .quote(&"A".repeat(43), &"B".repeat(43), 1_000_000, 50)
        .await
        .expect("quote should succeed");

    assert_eq!(limiter.metrics().total_acquired, 1);
}

#[tokio::test]
async fn build_swap_returns_serialized_tx() {
    let quote_payload = load_fixture("jupiter_quote_sol_usdc.json");
    let swap_payload = load_fixture("jupiter_swap_response.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/swap"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&swap_payload))
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 3);
    let quote = client_quote_from_fixture(&quote_payload);
    let out = client
        .build_swap(&quote, "11111111111111111111111111111111", 10_000, true)
        .await
        .expect("build_swap should succeed");

    assert_eq!(out.serialized_tx_b64, "AQABAgM=");
    assert_eq!(out.last_valid_block_height, 350000999);
    assert_eq!(out.prioritization_fee_lamports, 5000);
}

#[tokio::test]
async fn build_swap_http_error() {
    let quote_payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/swap"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(serde_json::json!({"error": "boom"})),
        )
        .mount(&server)
        .await;

    let client = JupiterClient::new(server.uri(), None, 3);
    let quote = client_quote_from_fixture(&quote_payload);
    let result = client
        .build_swap(&quote, "11111111111111111111111111111111", 0, true)
        .await;
    assert!(result.is_err());
}

fn client_quote_from_fixture(payload: &Value) -> tradebot_data::JupiterQuote {
    tradebot_data::JupiterQuote {
        input_mint: payload["inputMint"].as_str().unwrap().to_string(),
        output_mint: payload["outputMint"].as_str().unwrap().to_string(),
        in_amount: payload["inAmount"].as_str().unwrap().parse().unwrap(),
        out_amount: payload["outAmount"].as_str().unwrap().parse().unwrap(),
        other_amount_threshold: payload["otherAmountThreshold"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        slippage_bps: payload["slippageBps"].as_u64().unwrap() as u32,
        price_impact_pct: payload["priceImpactPct"].as_str().unwrap().parse().unwrap(),
        route_labels: vec!["Raydium".to_string()],
        raw: payload.clone(),
    }
}
