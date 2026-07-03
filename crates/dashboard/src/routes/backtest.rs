//! `POST /api/backtest/run`, `GET /api/backtest/history`, `GET
//! /api/backtest/result/{id}`: the Backtest tab's endpoints. Mirrors
//! `build_backtest_router()` in backtest_api.py.

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use indexmap::IndexMap;
use rand::RngCore;
use serde_json::json;

use tradebot_backtest::{load_csv_bytes, run_backtest, BacktestParams};
use tradebot_signals::{Signal, TASignal};

use crate::state::AppState;

/// A random 12-hex-char id, filling the same role as Python's
/// `uuid.uuid4().hex[:12]` (a short, human-typeable run id, not a
/// cryptographic identifier).
fn random_backtest_id() -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bad_request(detail: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail.into() })),
    )
        .into_response()
}

pub async fn run(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut csv_bytes: Option<Vec<u8>> = None;
    let mut params_text: Option<String> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return bad_request(format!("invalid multipart body: {e}")),
        };
        match field.name() {
            Some("csv_file") => match field.bytes().await {
                Ok(b) => csv_bytes = Some(b.to_vec()),
                Err(e) => return bad_request(format!("invalid csv_file field: {e}")),
            },
            Some("params") => match field.text().await {
                Ok(t) => params_text = Some(t),
                Err(e) => return bad_request(format!("invalid params field: {e}")),
            },
            _ => {}
        }
    }

    let params_text = match params_text {
        Some(p) => p,
        None => return bad_request("missing params field"),
    };
    let bt_params: BacktestParams = match serde_json::from_str(&params_text) {
        Ok(p) => p,
        Err(e) => return bad_request(format!("invalid params: {e}")),
    };

    let csv_bytes = match csv_bytes {
        Some(b) => b,
        None => return bad_request("missing csv_file field"),
    };
    let candles = match load_csv_bytes(&csv_bytes) {
        Ok(c) => c,
        Err(e) => return bad_request(format!("invalid CSV: {e}")),
    };

    let cfg = state
        .broker
        .as_ref()
        .expect("route only registered when a broker is present")
        .current();

    let backtest_id = random_backtest_id();
    let timeframe = bt_params.timeframe.clone();
    // v1: signals = TA only (microstructure + onchain need live data feeds,
    // not replayable), matching backtest_api.py.
    let signals: Vec<Box<dyn Signal>> = vec![Box::new(TASignal::new(timeframe.clone()))];
    let timeframe_weights = IndexMap::from([(timeframe, 1.0)]);
    let signal_weights = IndexMap::from([("ta".to_string(), 1.0)]);

    let result = match run_backtest(
        &candles,
        bt_params,
        signals,
        timeframe_weights,
        signal_weights,
        cfg.risk.clone(),
        backtest_id,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return bad_request(e.to_string()),
    };

    if let Some(store) = &state.backtest_store {
        store
            .lock()
            .expect("backtest store lock poisoned")
            .add(result.clone());
    }

    (StatusCode::OK, Json(result)).into_response()
}

pub async fn history(State(state): State<AppState>) -> Response {
    let store = state
        .backtest_store
        .as_ref()
        .expect("route only registered when a backtest store is present");
    let list = store
        .lock()
        .expect("backtest store lock poisoned")
        .list_summary();
    (StatusCode::OK, Json(list)).into_response()
}

pub async fn result_by_id(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let store = state
        .backtest_store
        .as_ref()
        .expect("route only registered when a backtest store is present");
    let store = store.lock().expect("backtest store lock poisoned");
    match store.get(&id) {
        Some(r) => (StatusCode::OK, Json(r.clone())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": "not found" })),
        )
            .into_response(),
    }
}
