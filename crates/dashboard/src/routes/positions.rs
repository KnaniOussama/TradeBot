//! `POST /api/positions/{pair}/sell`: enqueues a manual exit request.
//! Mirrors `post_manual_sell()` in server.py.
//!
//! app.js always calls this with `encodeURIComponent(pair)`, so a pair like
//! `SOL/USDC` arrives as a single percent-encoded path segment
//! (`SOL%2FUSDC`), not multiple segments; axum's `Path` extractor
//! percent-decodes it back to `SOL/USDC` before this handler runs, so a
//! plain `{pair}` route (rather than a `{*pair}` wildcard, which server.py
//! needs because Starlette does not auto-decode `%2F` in path params) is
//! enough here.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

#[derive(Debug, Deserialize, Default)]
struct SellPayload {
    #[serde(default)]
    reason: String,
}

pub async fn post_sell(
    State(state): State<AppState>,
    Path(pair): Path<String>,
    body: Bytes,
) -> Response {
    let payload: SellPayload = if body.is_empty() {
        SellPayload::default()
    } else {
        serde_json::from_slice(&body).unwrap_or_default()
    };

    if let Some(snap) = state.hub.latest() {
        let open_pairs: HashSet<&str> = snap.positions.iter().map(|p| p.pair.as_str()).collect();
        if !open_pairs.is_empty() && !open_pairs.contains(pair.as_str()) {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("no open position for {pair}") })),
            )
                .into_response();
        }
    }

    let manual_actions = state
        .manual_actions
        .as_ref()
        .expect("route only registered when manual_actions is present");
    let req = manual_actions.request_exit(pair.clone(), payload.reason);

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "pair": pair,
            "reason": req.reason,
            "requested_at": req.requested_at.to_rfc3339(),
            "note": "will execute next cycle",
        })),
    )
        .into_response()
}
