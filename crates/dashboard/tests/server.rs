//! Port of `tests/dashboard/test_server.py`: static index serving, the
//! `/api/state` bootstrap endpoint, config GET/POST, and manual sell.
//! Uses `tower::ServiceExt::oneshot` against the built `Router` directly
//! (no network socket needed for these).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use rust_decimal::Decimal;
use tower::ServiceExt;

use tradebot_common::Mode;
use tradebot_config::{default_config, load_config, save_config};
use tradebot_core::ManualActionQueue;
use tradebot_dashboard::{build_router, AppState, ConfigBroker};
use tradebot_engine::{DashboardHub, DashboardSnapshot};

fn snap(equity: &str) -> DashboardSnapshot {
    let equity: Decimal = equity.parse().unwrap();
    DashboardSnapshot {
        mode: Mode::Demo,
        now: "2026-05-03T12:00:00+00:00".to_string(),
        cash: equity,
        equity,
        equity_high: equity,
        drawdown_pct: 0.0,
        realized_pnl_total: Decimal::ZERO,
        sol_balance: Decimal::new(5, 2),
        sol_gas_paid_total: Decimal::ZERO,
        sol_mark: Decimal::new(140, 0),
        kill_switch_active: false,
        kill_switch_reason: String::new(),
        positions: Vec::new(),
        recent_trades: Vec::new(),
        equity_history: Vec::new(),
        signals: Vec::new(),
        pair_charts: Vec::new(),
        decisions: Vec::new(),
        whale_activity: Vec::new(),
        limiter: None,
    }
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = body_bytes(resp).await;
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn get_index_serves_html() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("text/html"));
    let body = String::from_utf8(body_bytes(resp).await).unwrap();
    assert!(body.contains("TradeBot"));
}

#[tokio::test]
async fn get_state_returns_empty_object_before_any_publish() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body, serde_json::json!({}));
}

#[tokio::test]
async fn get_state_returns_latest_after_publish() {
    let hub = Arc::new(DashboardHub::default());
    hub.publish(snap("99.5"));
    let app = build_router(AppState::new(hub));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["equity"], 99.5);
}

fn broker_state(hub: Arc<DashboardHub>, path: &std::path::Path) -> AppState {
    let cfg = default_config();
    save_config(path, &cfg).unwrap();
    let broker = Arc::new(ConfigBroker::new(path.to_path_buf(), cfg));
    AppState::new(hub).with_broker(broker)
}

#[tokio::test]
async fn get_config_returns_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cfg.json");
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(broker_state(hub, &path));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["app"]["starting_capital_usd"], 50.0);
}

#[tokio::test]
async fn post_config_validates_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cfg.json");
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(broker_state(hub, &path));

    let mut new_cfg = serde_json::to_value(default_config()).unwrap();
    new_cfg["app"]["starting_capital_usd"] = serde_json::json!(100.0);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&new_cfg).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let loaded = load_config(&path).unwrap();
    assert_eq!(loaded.app.starting_capital_usd, 100.0);
}

#[tokio::test]
async fn post_invalid_config_returns_400() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cfg.json");
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(broker_state(hub, &path));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"garbage": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn config_routes_absent_without_broker() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manual_sell_enqueues_into_manual_action_queue() {
    let hub = Arc::new(DashboardHub::default());
    let manual_actions = Arc::new(ManualActionQueue::new());
    let app = build_router(AppState::new(hub).with_manual_actions(manual_actions.clone()));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/positions/SOL%2FUSDC/sell")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason": "took profit"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["pair"], "SOL/USDC");
    assert_eq!(body["reason"], "took profit");

    let drained = manual_actions.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].pair, "SOL/USDC");
    assert_eq!(drained[0].reason, "took profit");
}

#[tokio::test]
async fn manual_sell_404_for_pair_with_no_open_position() {
    let hub = Arc::new(DashboardHub::default());
    hub.publish(snap("50.0")); // has positions: [] -> empty, so 404 only fires when
                               // there ARE open positions and this pair isn't one.
    let mut with_position = snap("50.0");
    with_position
        .positions
        .push(tradebot_engine::PositionSnapshot {
            pair: "SOL/USDC".to_string(),
            base_amount: Decimal::ONE,
            avg_entry_price: Decimal::new(100, 0),
            mark_price: Decimal::new(110, 0),
            unrealized_pnl_quote: Decimal::TEN,
            unrealized_pnl_pct: 0.1,
            lineage: Vec::new(),
        });
    hub.publish(with_position);

    let manual_actions = Arc::new(ManualActionQueue::new());
    let app = build_router(AppState::new(hub).with_manual_actions(manual_actions));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/positions/BONK%2FUSDC/sell")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason": ""}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manual_sell_routes_absent_without_manual_actions() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/positions/SOL%2FUSDC/sell")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
