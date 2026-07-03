//! Shared test helpers: spins up a real dashboard server on an ephemeral
//! port for tests that need a live socket (websocket, multipart uploads).

use tradebot_dashboard::{spawn, AppState, DashboardHandle};

pub async fn start(state: AppState) -> (String, DashboardHandle) {
    let (handle, addr) = spawn(state, "127.0.0.1", 0)
        .await
        .expect("dashboard test server failed to bind");
    (format!("http://{addr}"), handle)
}
