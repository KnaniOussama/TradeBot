use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tradebot_data::{JupiterClient, PriceFeed, PriceTick};
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
async fn price_feed_emits_tick() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let jup = JupiterClient::new(server.uri(), None, 3);
    let feed = Arc::new(PriceFeed::new(
        jup,
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        6,
        0.01,
        1.0,
        50,
        "USDC",
    ));
    feed.add_pair("SOL", "So11111111111111111111111111111111111111112", 9);
    let mut sub = feed.subscribe();

    let feed_bg = feed.clone();
    let task = tokio::spawn(async move { feed_bg.run().await });

    let tick: PriceTick = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("should not time out")
        .expect("channel should yield a tick");
    feed.stop();
    task.await.expect("run task should not panic");

    assert_eq!(tick.pair, "SOL/USDC");
    assert!(tick.price > 0.0);
}

#[tokio::test]
async fn price_feed_multiple_subscribers_get_same_tick() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let jup = JupiterClient::new(server.uri(), None, 3);
    let feed = Arc::new(PriceFeed::new(
        jup,
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        6,
        0.01,
        1.0,
        50,
        "USDC",
    ));
    feed.add_pair("SOL", "So11111111111111111111111111111111111111112", 9);
    let mut s1 = feed.subscribe();
    let mut s2 = feed.subscribe();

    let feed_bg = feed.clone();
    let task = tokio::spawn(async move { feed_bg.run().await });

    let t1 = tokio::time::timeout(Duration::from_secs(2), s1.recv())
        .await
        .expect("should not time out")
        .expect("channel should yield a tick");
    let t2 = tokio::time::timeout(Duration::from_secs(2), s2.recv())
        .await
        .expect("should not time out")
        .expect("channel should yield a tick");
    feed.stop();
    task.await.expect("run task should not panic");

    assert_eq!(t1.pair, "SOL/USDC");
    assert_eq!(t2.pair, "SOL/USDC");
}

#[tokio::test]
async fn price_feed_stop_terminates_run() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let jup = JupiterClient::new(server.uri(), None, 3);
    let feed = Arc::new(PriceFeed::new(
        jup,
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        6,
        0.01,
        1.0,
        50,
        "USDC",
    ));
    feed.add_pair("SOL", "So11111111111111111111111111111111111111112", 9);

    let feed_bg = feed.clone();
    let task = tokio::spawn(async move { feed_bg.run().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    feed.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("run should terminate promptly after stop()")
        .expect("run task should not panic");
}
