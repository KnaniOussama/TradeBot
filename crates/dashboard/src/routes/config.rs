//! `GET`/`POST /api/config`: the Settings tab's read/save endpoints.
//! Mirrors `get_config()` / `post_config()` in server.py.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;
use tradebot_config::TradeBotConfig;

use crate::state::AppState;

pub async fn get_config(State(state): State<AppState>) -> Response {
    let broker = state
        .broker
        .as_ref()
        .expect("route only registered when a broker is present");
    (StatusCode::OK, Json(broker.current())).into_response()
}

pub async fn post_config(State(state): State<AppState>, body: Bytes) -> Response {
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let cfg: TradeBotConfig = match serde_json::from_value(value) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Err(e) = cfg.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let broker = state
        .broker
        .as_ref()
        .expect("route only registered when a broker is present");
    if let Err(e) = broker.update(cfg) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "applied_at_next_cycle": true,
            "note": "Restart the bot to apply most fields.",
        })),
    )
        .into_response()
}
