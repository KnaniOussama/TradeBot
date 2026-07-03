//! `GET /api/state`: bootstrap snapshot the frontend fetches once over REST
//! while the websocket connects. Mirrors `state()` in server.py.

use axum::extract::State;
use axum::response::Json;
use serde_json::{json, Value};

use crate::dto::snapshot_json;
use crate::state::AppState;

pub async fn get_state(State(state): State<AppState>) -> Json<Value> {
    match state.hub.latest() {
        Some(snap) => Json(snapshot_json(&snap, &state.control)),
        None => Json(json!({})),
    }
}
