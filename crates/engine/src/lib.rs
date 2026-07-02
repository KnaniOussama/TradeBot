//! Rust port of `tradebot/core/loop.py`, `tradebot/dashboard/state.py`, and
//! `tradebot/dashboard/hub.py`: the trading loop, the dashboard snapshot
//! model, and the in-process publish/subscribe hub that connects them.
//!
//! `loop.rs` (module `r#loop`, since `loop` is a Rust keyword) owns the
//! cycle: live marks, OHLCV upsert, signal aggregation, regime
//! classification, Kelly sizing, the decision engine, order execution,
//! equity/state persistence, and snapshot publishing. `snapshot.rs` builds
//! the `DashboardSnapshot` the JS frontend consumes. `hub.rs` fans a
//! published snapshot out to subscribers over a `tokio::sync::broadcast`
//! channel.

pub mod hub;
pub mod r#loop;
pub mod snapshot;

pub use hub::DashboardHub;
pub use r#loop::{EngineError, StopSignal, TradingLoop, TradingLoopOptions};
pub use snapshot::{
    build_snapshot, AnnotatedTrade, ChartPoint, DashboardSnapshot, DecisionEntry, LineageLeg,
    MarkHistorySnapshot, PairChart, PositionSnapshot, SignalComponent, SignalSnapshot,
    SnapshotOptions, WhaleActivityEntry,
};
