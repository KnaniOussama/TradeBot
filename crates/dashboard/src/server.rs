//! Builds the axum `Router` and runs it. Mirrors `build_app()` /
//! `run_server()` in server.py.

use std::io;
use std::net::SocketAddr;

use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::assets::{index_handler, static_handler};
use crate::routes::{backtest, config, control, positions, state as state_routes, ws};
use crate::state::AppState;

/// Builds the router. Routes that depend on an optional piece of state
/// (`broker`, `backtest_store`, `manual_actions`) are only registered when
/// that state is present, mirroring the conditional `@app.get`/`@app.post`
/// registration in `build_app()` in server.py: hitting an unregistered
/// route 404s exactly like it does in the Python app.
pub fn build_router(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws::ws_handler))
        .route("/api/state", get(state_routes::get_state));

    if state.broker.is_some() {
        router = router.route(
            "/api/config",
            get(config::get_config).post(config::post_config),
        );
    }

    if state.manual_actions.is_some() {
        router = router.route("/api/positions/{pair}/sell", post(positions::post_sell));
    }

    if state.backtest_store.is_some() && state.broker.is_some() {
        router = router
            .route("/api/backtest/run", post(backtest::run))
            .route("/api/backtest/history", get(backtest::history))
            .route("/api/backtest/result/{id}", get(backtest::result_by_id));
    }

    // Not present in server.py (see routes/control.rs docs); registered
    // unconditionally since it needs no optional dependency beyond the
    // control state every `AppState` always carries.
    router = router.route("/api/control", post(control::post_control));

    router = router.route("/static/{*path}", get(static_handler));

    router.with_state(state)
}

/// A running dashboard server. Dropping this without calling
/// [`DashboardHandle::shutdown`] leaves the server running detached; call
/// `shutdown` for a clean stop (used from the CLI's Ctrl-C path).
pub struct DashboardHandle {
    join: JoinHandle<()>,
    shutdown_tx: oneshot::Sender<()>,
}

impl DashboardHandle {
    /// Signals graceful shutdown and waits for the server task to exit.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.join.await;
    }
}

/// Binds `host:port` and spawns the server as a background task, returning
/// the handle plus the address actually bound (useful when `port` is `0`,
/// e.g. in tests). Mirrors `run_server()` in server.py, except binding
/// happens synchronously here (before returning) so a bad host/port
/// surfaces immediately to the caller instead of failing silently in the
/// background.
pub async fn spawn(
    state: AppState,
    host: &str,
    port: u16,
) -> io::Result<(DashboardHandle, SocketAddr)> {
    let listener = TcpListener::bind((host, port)).await?;
    let addr = listener.local_addr()?;
    let router = build_router(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let join = tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "dashboard_server_failed");
        }
    });

    Ok((DashboardHandle { join, shutdown_tx }, addr))
}
