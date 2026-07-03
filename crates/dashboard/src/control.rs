//! In-memory pause/resume control state for the trading-controls toggles in
//! the top of the dashboard UI.
//!
//! Neither the Python reference (`tradebot/dashboard/server.py`) nor the
//! rest of the Rust engine implements a backend for `/api/control` yet: the
//! frontend already treats a 404 there as "not wired up" and stays
//! optimistic (see `postControl` in app.js). This module provides a real,
//! in-process-only backend for it so the toggles round-trip, while leaving
//! actually gating buy/sell decisions in the trading loop on this flag as
//! future work (tracked in the crate-level report, not implemented here).

use serde::{Deserialize, Serialize};

/// Current pause state for buys/sells, as shown by the pause-toggle buttons
/// and merged into every published snapshot as the `control` field.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlState {
    #[serde(default)]
    pub paused_buys: bool,
    #[serde(default)]
    pub paused_sells: bool,
}
