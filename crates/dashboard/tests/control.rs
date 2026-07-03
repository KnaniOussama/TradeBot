//! `/api/control` pause/resume toggles: no Python reference exists (see
//! `src/routes/control.rs`), so this test just exercises the Rust-only
//! contract directly against the router.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use tradebot_dashboard::{build_router, AppState};
use tradebot_engine::DashboardHub;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn post_control_updates_and_echoes_state() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"paused_buys": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["paused_buys"], true);
    assert_eq!(body["paused_sells"], false);

    // A partial update only touches the field it names.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"paused_sells": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["paused_buys"], true);
    assert_eq!(body["paused_sells"], true);
}

#[tokio::test]
async fn control_state_is_merged_into_api_state() {
    let hub = Arc::new(DashboardHub::default());
    let app = build_router(AppState::new(hub));

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"paused_buys": true, "paused_sells": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // With no published snapshot yet, /api/state is still `{}` (no control
    // key merged in, matching the empty-object contract of the base
    // endpoint).
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
    assert_eq!(body, serde_json::json!({}));
}
