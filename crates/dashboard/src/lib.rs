//! Rust port of `tradebot/dashboard`: an axum HTTP + websocket server that
//! serves the (unchanged) static frontend, streams `DashboardSnapshot`s
//! published on the `DashboardHub`, and exposes the config, backtest,
//! manual-sell, and pause/resume-control endpoints the frontend's
//! `app.js` calls.
//!
//! Route table (path / method -> handler; matches `app.js`'s `fetch(...)`
//! calls and its `new WebSocket(...)` target exactly):
//!
//! | Path                              | Method | Handler                        |
//! |------------------------------------|--------|--------------------------------|
//! | `/`                                | GET    | `assets::index_handler`        |
//! | `/static/{*path}`                  | GET    | `assets::static_handler`       |
//! | `/ws`                              | GET    | `routes::ws::ws_handler`       |
//! | `/api/state`                       | GET    | `routes::state::get_state`     |
//! | `/api/config`                      | GET    | `routes::config::get_config`   |
//! | `/api/config`                      | POST   | `routes::config::post_config`  |
//! | `/api/positions/{pair}/sell`       | POST   | `routes::positions::post_sell` |
//! | `/api/backtest/run`                | POST   | `routes::backtest::run`        |
//! | `/api/backtest/history`            | GET    | `routes::backtest::history`    |
//! | `/api/backtest/result/{id}`        | GET    | `routes::backtest::result_by_id` |
//! | `/api/control`                     | POST   | `routes::control::post_control`|
//!
//! The `/api/config`, `/api/positions/{pair}/sell`, and `/api/backtest/*`
//! rows are only registered when the corresponding piece of `AppState` is
//! present (`broker`, `manual_actions`, `backtest_store`), mirroring
//! `build_app()`'s conditional route registration in server.py.
//! `/api/control` has no Python equivalent; see `routes::control` for why
//! it exists anyway.

pub mod assets;
pub mod backtest_store;
pub mod config_broker;
pub mod control;
pub mod dto;
pub mod routes;
pub mod server;
pub mod state;

pub use backtest_store::BacktestStore;
pub use config_broker::ConfigBroker;
pub use control::ControlState;
pub use server::{build_router, spawn, DashboardHandle};
pub use state::AppState;
