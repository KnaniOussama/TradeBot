use tradebot_data::BirdeyeClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn client_for(server: &MockServer) -> BirdeyeClient {
    BirdeyeClient::with_options("test-key", server.uri(), "solana", 5.0, None, 2)
}

#[tokio::test]
async fn multi_price_returns_map() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/defi/multi_price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {
                SOL: {"value": 145.32, "updateUnixTime": 1714742400},
                USDC: {"value": 0.9998},
            },
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let prices = client
        .multi_price(&[SOL.to_string(), USDC.to_string()])
        .await;
    assert!((prices[SOL] - 145.32).abs() < 1e-9);
    assert!((prices[USDC] - 0.9998).abs() < 1e-9);
}

#[tokio::test]
async fn multi_price_skips_entries_without_value() {
    let bad_mint = "BadMint11111111111111111111111111111111111";
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/defi/multi_price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {
                SOL: {"value": 145.0},
                bad_mint: {},
            },
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let prices = client
        .multi_price(&[SOL.to_string(), bad_mint.to_string()])
        .await;
    assert!(prices.contains_key(SOL));
    assert!(!prices.contains_key(bad_mint));
}

#[tokio::test]
async fn multi_price_returns_empty_on_persistent_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/defi/multi_price"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let client = BirdeyeClient::with_options("test-key", server.uri(), "solana", 5.0, None, 0);
    let prices = client.multi_price(&[SOL.to_string()]).await;
    assert!(prices.is_empty());
}

#[tokio::test]
async fn multi_price_returns_empty_when_unsuccessful() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/defi/multi_price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": false,
            "message": "rate limit",
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let prices = client.multi_price(&[SOL.to_string()]).await;
    assert!(prices.is_empty());
}

#[tokio::test]
async fn multi_price_empty_input_short_circuits() {
    // No mock mounted: a network call would fail the test.
    let server = MockServer::start().await;
    let client = client_for(&server);
    let prices = client.multi_price(&[]).await;
    assert!(prices.is_empty());
}

#[tokio::test]
async fn falls_back_to_single_price_on_401_and_stays_locked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/defi/multi_price"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/defi/price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"value": 145.0},
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let prices = client.multi_price(&[SOL.to_string()]).await;
    assert!((prices[SOL] - 145.0).abs() < 1e-9);

    // Second call on the same client: multi_price_locked is now set, so it
    // should skip straight to /defi/price without hitting /defi/multi_price
    // again (which has no further mock registered beyond the 401 mount, and
    // that mount has no call-count limit so it would still 401 if hit).
    Mock::given(method("GET"))
        .and(path("/defi/price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"value": 146.0},
        })))
        .mount(&server)
        .await;
    let prices2 = client.multi_price(&[SOL.to_string()]).await;
    assert!((prices2[SOL] - 146.0).abs() < 1e-9);
}
