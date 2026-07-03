//! `POST /api/control`: pause/resume the buy/sell toggles at the top of the
//! dashboard. Not present in the Python reference (`server.py` has no
//! `/api/control` route at all; app.js's `postControl()` already treats a
//! 404 there as "not wired up" and stays optimistic client-side). This is a
//! genuine backend for it: it stores the flags and echoes them back in
//! every snapshot's `control` field (see `dto.rs`).
//!
//! Actually gating new buy/sell decisions in the trading loop on this flag
//! is not implemented here; see the crate-level docs.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

#[derive(Debug, Deserialize, Default)]
struct ControlPayload {
    paused_buys: Option<bool>,
    paused_sells: Option<bool>,
}

pub async fn post_control(State(state): State<AppState>, body: Bytes) -> Response {
    let payload: ControlPayload = if body.is_empty() {
        ControlPayload::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    };

    let updated = {
        let mut c = state.control.write().expect("control lock poisoned");
        if let Some(buys) = payload.paused_buys {
            c.paused_buys = buys;
        }
        if let Some(sells) = payload.paused_sells {
            c.paused_sells = sells;
        }
        *c
    };

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "paused_buys": updated.paused_buys,
            "paused_sells": updated.paused_sells,
        })),
    )
        .into_response()
}
