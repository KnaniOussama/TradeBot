//! Port of `tests/core/test_loop.py`.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use tradebot_common::Mode;
use tradebot_config::models::RiskConfig;
use tradebot_core::{DecisionEngine, Portfolio, RiskManager, RiskState, SignalAggregator};
use tradebot_data::JupiterClient;
use tradebot_engine::{DashboardHub, TradingLoop, TradingLoopOptions};
use tradebot_execution::DemoExecutor;
use tradebot_signals::{MarketContext, Signal, SignalScore};
use tradebot_storage::JsonStorage;

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

fn base_mints() -> HashMap<String, (String, u32)> {
    let mut m = HashMap::new();
    m.insert("SOL/USDC".to_string(), (SOL_MINT.to_string(), 9));
    m
}

fn cycle_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
}

/// Always scores 0.9 on "ta"/"1m", strong enough to trigger an entry.
struct BullSignal;

#[async_trait]
impl Signal for BullSignal {
    fn name(&self) -> &str {
        "ta"
    }
    fn timeframe(&self) -> &str {
        "1m"
    }
    async fn score(&self, ctx: &MarketContext) -> SignalScore {
        SignalScore::new("ta", ctx.pair.clone(), "1m", 0.9, ctx.now, HashMap::new())
            .expect("fixed score in range")
    }
}

async fn mock_jupiter_server() -> MockServer {
    let payload = load_fixture("jupiter_quote_sol_usdc.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn one_cycle_enters_position() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        vec![Box::new(BullSignal)],
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::from([("ta".to_string(), 1.0)]),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions::default(),
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    assert!(loop_.portfolio().position_for("SOL/USDC").is_some());
    assert_eq!(storage.list_trades(Mode::Demo, 100).len(), 1);
}

#[tokio::test]
async fn one_cycle_writes_equity_snapshot() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        Vec::new(),
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::new(),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions::default(),
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    assert_eq!(storage.list_equity_snapshots(Mode::Demo, 10).len(), 1);
}

#[tokio::test]
async fn one_cycle_publishes_to_hub() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path()).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        Vec::new(),
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::new(),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
    let hub = std::sync::Arc::new(DashboardHub::default());

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions {
            hub: Some(hub.clone()),
            ..TradingLoopOptions::default()
        },
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    let snap = hub.latest().expect("snapshot published");
    assert_eq!(snap.mode, Mode::Demo);
    assert!(snap.equity > Decimal::ZERO);
}

#[tokio::test]
async fn one_cycle_persists_portfolio_risk_and_mark_history() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path().join("data")).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        Vec::new(),
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::new(),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions::default(),
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    let saved_portfolio = storage.load_portfolio_state(Mode::Demo).unwrap();
    assert_eq!(saved_portfolio.cash, Decimal::new(100, 0));
    let saved_history = storage.load_mark_history(Mode::Demo);
    assert!(saved_history.contains_key("SOL/USDC"));
    assert_eq!(saved_history["SOL/USDC"].len(), 1);
}

#[tokio::test]
async fn one_cycle_buffers_observations() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path().join("data")).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        vec![Box::new(BullSignal)],
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::from([("ta".to_string(), 1.0)]),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions::default(),
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    assert!(!loop_.observations().is_empty());
}

#[tokio::test]
async fn one_cycle_publishes_decisions_in_snapshot() {
    let server = mock_jupiter_server().await;
    let storage_dir = tempdir().unwrap();
    let storage = JsonStorage::new(storage_dir.path().join("data2")).unwrap();

    let portfolio = Portfolio::new(Mode::Demo, Decimal::new(100, 0), Decimal::ONE);
    let risk = RiskManager::new(RiskConfig::default());
    let state = RiskState::default();
    let aggregator = SignalAggregator::new(
        vec![Box::new(BullSignal)],
        IndexMap::from([("1m".to_string(), 1.0)]),
        IndexMap::from([("ta".to_string(), 1.0)]),
    );
    let engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
    let hub = std::sync::Arc::new(DashboardHub::default());

    let jupiter = JupiterClient::new(server.uri(), None, 3);
    let ex = DemoExecutor::new(
        jupiter.clone(),
        &storage,
        base_mints(),
        USDC_MINT,
        6,
        1.0,
        10,
        0.0,
    );

    let mut loop_ = TradingLoop::new(
        &storage,
        portfolio,
        aggregator,
        engine,
        risk,
        state,
        ex,
        jupiter,
        vec!["SOL/USDC".to_string()],
        vec!["1m".to_string()],
        USDC_MINT.to_string(),
        6,
        base_mints(),
        TradingLoopOptions {
            hub: Some(hub.clone()),
            ..TradingLoopOptions::default()
        },
    );

    loop_.run_one_cycle(cycle_now()).await.unwrap();

    let snap = hub.latest().expect("snapshot published");
    assert!(!snap.decisions.is_empty());
}
