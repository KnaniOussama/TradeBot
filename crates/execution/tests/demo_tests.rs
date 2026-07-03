//! Port of `tests/execution/test_demo.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tempfile::tempdir;
use tradebot_common::Mode;
use tradebot_core::{Portfolio, PortfolioError};
use tradebot_data::JupiterClient;
use tradebot_execution::{DemoExecutor, ExecutionError, Executor, Order};
use tradebot_storage::JsonStorage;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

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

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn base_mints() -> HashMap<String, (String, u32)> {
    let mut m = HashMap::new();
    m.insert("SOL/USDC".to_string(), (SOL_MINT.to_string(), 9));
    m
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 3, 0, 0, 0).unwrap()
}

#[tokio::test]
async fn demo_buy_simulates_fill() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("1.0"));
    let ex = DemoExecutor::new(jupiter, &storage, base_mints(), USDC_MINT, 6, 1.0, 0, 0.0);

    let order = Order::buy("SOL/USDC", dec("10.0"));
    let fill = ex.execute(&order, &mut p, now()).await.unwrap();

    assert_eq!(fill.pair, "SOL/USDC");
    assert_eq!(fill.side, tradebot_storage::Side::Buy);
    assert!(fill.base_amount > Decimal::ZERO);
    assert_eq!(fill.quote_amount, dec("10.0"));
    assert!(p.cash < dec("100.0"));
    assert!(p.position_for("SOL/USDC").is_some());
}

#[tokio::test]
async fn demo_sell_returns_quote_to_cash() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("1.0"));
    p.apply_fill(
        "SOL/USDC",
        tradebot_storage::Side::Buy,
        dec("0.1"),
        dec("10.0"),
        Decimal::ZERO,
    )
    .unwrap();
    let ex = DemoExecutor::new(jupiter, &storage, base_mints(), USDC_MINT, 6, 1.0, 0, 0.0);

    let order = Order::sell("SOL/USDC", dec("0.1"));
    let fill = ex.execute(&order, &mut p, now()).await.unwrap();

    assert_eq!(fill.side, tradebot_storage::Side::Sell);
    assert!(fill.quote_amount > Decimal::ZERO);
    assert!(p.position_for("SOL/USDC").is_none());
}

#[tokio::test]
async fn demo_persists_trade() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("1.0"));
    let ex = DemoExecutor::new(jupiter, &storage, base_mints(), USDC_MINT, 6, 1.0, 0, 0.0);

    ex.execute(&Order::buy("SOL/USDC", dec("10.0")), &mut p, now())
        .await
        .unwrap();

    assert_eq!(storage.list_trades(Mode::Demo, 10).len(), 1);
}

#[tokio::test]
async fn demo_unknown_pair_raises() {
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new("https://example.invalid", None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("1.0"));
    let ex = DemoExecutor::new(jupiter, &storage, HashMap::new(), USDC_MINT, 6, 1.0, 0, 0.0);

    let result = ex
        .execute(&Order::buy("UNKNOWN/USDC", dec("10.0")), &mut p, Utc::now())
        .await;
    assert!(matches!(result, Err(ExecutionError::Invalid(msg)) if msg.contains("unknown pair")));
}

#[tokio::test]
async fn demo_charges_sol_gas_on_fill() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("0.01"));
    let ex = DemoExecutor::new(jupiter, &storage, base_mints(), USDC_MINT, 6, 1.0, 0, 0.0);

    ex.execute(&Order::buy("SOL/USDC", dec("10.0")), &mut p, Utc::now())
        .await
        .unwrap();

    // Base fee 5000 lamports = 5e-6 SOL deducted exactly once.
    assert_eq!(p.sol_balance, dec("0.01") - dec("0.000005"));
    assert_eq!(p.sol_gas_paid_total, dec("0.000005"));
}

#[tokio::test]
async fn demo_rejects_when_sol_balance_zero() {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
    let ex = DemoExecutor::new(jupiter, &storage, base_mints(), USDC_MINT, 6, 1.0, 0, 0.0);

    let result = ex
        .execute(&Order::buy("SOL/USDC", dec("10.0")), &mut p, Utc::now())
        .await;
    match result {
        Err(ExecutionError::Portfolio(PortfolioError::InsufficientSol { .. })) => {}
        other => panic!("expected insufficient SOL error, got {other:?}"),
    }
}

#[tokio::test]
async fn demo_rejects_when_drift_exceeds_max_slippage() {
    // Trigger quote: out=15_050_000. Fill quote: out=14_500_000 (~3.6% worse).
    let trigger_payload = load_fixture("jupiter_quote_sol_usdc.json");
    let mut drifted_payload = trigger_payload.clone();
    drifted_payload["outAmount"] = Value::String("14500000".to_string());

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&trigger_payload))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&drifted_payload))
        .with_priority(2)
        .mount(&server)
        .await;

    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();
    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let mut p = Portfolio::new(Mode::Demo, dec("100.0"), dec("0.01"));
    let ex = DemoExecutor::new(
        jupiter,
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        0.01, // 1%, drift is ~3.6%, should fail
        0,
        0.0,
    );

    let result = ex
        .execute(&Order::buy("SOL/USDC", dec("10.0")), &mut p, Utc::now())
        .await;
    match result {
        Err(ExecutionError::Invalid(msg)) => assert!(msg.contains("drift"), "msg={msg}"),
        other => panic!("expected drift error, got {other:?}"),
    }
    // No state mutation when fill rejected.
    assert_eq!(p.cash, dec("100.0"));
    assert_eq!(p.sol_balance, dec("0.01"));
}
