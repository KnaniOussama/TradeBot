//! Port of `test_websocket_receives_published_snapshot` in
//! `tests/dashboard/test_server.py`. Websocket upgrades don't work over
//! `tower::ServiceExt::oneshot`, so this binds a real ephemeral-port
//! server (see `tests/support/mod.rs`) and connects with a websocket
//! client.

mod support;

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rust_decimal::Decimal;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use tradebot_common::Mode;
use tradebot_dashboard::AppState;
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

#[tokio::test]
async fn websocket_receives_published_snapshot() {
    let hub = Arc::new(DashboardHub::default());
    let (base_url, handle) = support::start(AppState::new(hub.clone())).await;
    let ws_url = base_url.replacen("http://", "ws://", 1) + "/ws";

    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(&ws_url))
        .await
        .expect("connect timed out")
        .expect("connect failed");

    hub.publish(snap("7.0"));

    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("recv timed out")
        .expect("stream ended")
        .expect("recv error");
    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text message, got {other:?}"),
    };
    let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["equity"], 7.0);

    let _ = ws.close(None).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn websocket_sends_latest_snapshot_immediately_on_connect() {
    let hub = Arc::new(DashboardHub::default());
    hub.publish(snap("42.0"));
    let (base_url, handle) = support::start(AppState::new(hub)).await;
    let ws_url = base_url.replacen("http://", "ws://", 1) + "/ws";

    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(&ws_url))
        .await
        .expect("connect timed out")
        .expect("connect failed");

    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("recv timed out")
        .expect("stream ended")
        .expect("recv error");
    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text message, got {other:?}"),
    };
    let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["equity"], 42.0);

    let _ = ws.close(None).await;
    handle.shutdown().await;
}
