//! `GET /ws`: subscribes to the `DashboardHub` broadcast and streams every
//! published `DashboardSnapshot` as JSON, sending the latest snapshot
//! immediately on connect if one is available. Mirrors `ws_endpoint()` in
//! server.py.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use tokio::sync::broadcast;

use crate::dto::snapshot_json;
use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.hub.subscribe();

    if let Some(latest) = state.hub.latest() {
        let payload = snapshot_json(&latest, &state.control).to_string();
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            snap = rx.recv() => {
                match snap {
                    Ok(snap) => {
                        let payload = snapshot_json(&snap, &state.control).to_string();
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(_)) => continue,
                    _ => break,
                }
            }
        }
    }
}
