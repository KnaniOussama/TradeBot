//! JSON shaping shared by the `/api/state` handler and the websocket
//! stream: both send the exact same payload shape, a `DashboardSnapshot`
//! with a `control` key merged in for the pause-toggle UI.

use std::sync::RwLock;

use serde_json::{json, Value};
use tradebot_engine::DashboardSnapshot;

use crate::control::ControlState;

/// Serializes `snap` and merges in the current `control` state, matching
/// the shape `render(snap)` in app.js expects (`snap.control.paused_buys` /
/// `snap.control.paused_sells`, both optional).
pub fn snapshot_json(snap: &DashboardSnapshot, control: &RwLock<ControlState>) -> Value {
    let mut value = serde_json::to_value(snap).expect("DashboardSnapshot always serializes");
    let c = *control.read().expect("control lock poisoned");
    if let Value::Object(ref mut map) = value {
        map.insert(
            "control".to_string(),
            json!({
                "paused_buys": c.paused_buys,
                "paused_sells": c.paused_sells,
            }),
        );
    }
    value
}
