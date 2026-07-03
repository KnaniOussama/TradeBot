//! Shared application state threaded through every axum handler via
//! `State<AppState>`. Mirrors the optional constructor arguments of
//! `build_app()` in server.py: `broker`, `backtest_store`, and
//! `manual_actions` are each `Option`, and routes that depend on a missing
//! one are simply not registered by `build_router` (see `server.rs`),
//! matching FastAPI's conditional `@app.get`/`@app.post` registration.

use std::sync::{Arc, Mutex, RwLock};

use tradebot_core::ManualActionQueue;
use tradebot_engine::DashboardHub;

use crate::backtest_store::BacktestStore;
use crate::config_broker::ConfigBroker;
use crate::control::ControlState;

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<DashboardHub>,
    pub broker: Option<Arc<ConfigBroker>>,
    pub backtest_store: Option<Arc<Mutex<BacktestStore>>>,
    pub manual_actions: Option<Arc<ManualActionQueue>>,
    pub control: Arc<RwLock<ControlState>>,
}

impl AppState {
    pub fn new(hub: Arc<DashboardHub>) -> Self {
        Self {
            hub,
            broker: None,
            backtest_store: None,
            manual_actions: None,
            control: Arc::new(RwLock::new(ControlState::default())),
        }
    }

    pub fn with_broker(mut self, broker: Arc<ConfigBroker>) -> Self {
        self.broker = Some(broker);
        self
    }

    pub fn with_backtest_store(mut self, store: Arc<Mutex<BacktestStore>>) -> Self {
        self.backtest_store = Some(store);
        self
    }

    pub fn with_manual_actions(mut self, manual_actions: Arc<ManualActionQueue>) -> Self {
        self.manual_actions = Some(manual_actions);
        self
    }
}
