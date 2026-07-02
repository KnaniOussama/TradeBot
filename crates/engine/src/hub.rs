//! Port of `tradebot/dashboard/hub.py`: a publish/subscribe hub for
//! dashboard snapshots.
//!
//! The Python `DashboardHub` keeps one bounded `asyncio.Queue` per
//! subscriber and drops the oldest queued item (then pushes the newest) when
//! a subscriber falls behind. `tokio::sync::broadcast` gives the same "drop
//! oldest, keep newest" behavior natively for a bounded ring buffer shared
//! across all subscribers: a slow receiver that falls behind observes
//! `RecvError::Lagged(n)` on its next `recv()` and then resumes from the
//! oldest value still buffered, rather than blocking the publisher. This is
//! why `hub.rs` uses `broadcast` instead of one queue per subscriber.

use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

use crate::snapshot::DashboardSnapshot;

/// Default bound on the number of not-yet-delivered snapshots buffered per
/// subscriber before older ones are dropped. Mirrors `subscriber_queue_size`
/// in hub.py.
const DEFAULT_SUBSCRIBER_QUEUE_SIZE: usize = 4;

/// Publish/subscribe hub for `DashboardSnapshot`s. Mirrors `DashboardHub` in
/// hub.py. Cheap to share: wrap in `Arc<DashboardHub>` and clone the `Arc`
/// into every task that needs to publish or subscribe.
pub struct DashboardHub {
    latest: RwLock<Option<Arc<DashboardSnapshot>>>,
    sender: broadcast::Sender<Arc<DashboardSnapshot>>,
}

impl DashboardHub {
    /// Builds a hub whose broadcast ring buffer holds up to
    /// `subscriber_queue_size` undelivered snapshots per subscriber.
    pub fn new(subscriber_queue_size: usize) -> Self {
        let (sender, _rx) = broadcast::channel(subscriber_queue_size.max(1));
        Self {
            latest: RwLock::new(None),
            sender,
        }
    }

    /// The most recently published snapshot, if any. Mirrors `latest()` in
    /// hub.py.
    pub fn latest(&self) -> Option<Arc<DashboardSnapshot>> {
        self.latest
            .read()
            .expect("dashboard hub latest lock poisoned")
            .clone()
    }

    /// Registers a new subscriber. Mirrors `subscribe()` in hub.py; the
    /// returned receiver auto-unregisters when dropped (there is no
    /// separate `unsubscribe` call needed, unlike the Python `asyncio.Queue`
    /// version).
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<DashboardSnapshot>> {
        self.sender.subscribe()
    }

    /// Publishes a new snapshot: updates `latest()` and pushes it to every
    /// subscriber, dropping the oldest buffered value for any subscriber
    /// that has fallen behind. Mirrors `publish()` in hub.py.
    pub fn publish(&self, snapshot: DashboardSnapshot) {
        let snapshot = Arc::new(snapshot);
        *self
            .latest
            .write()
            .expect("dashboard hub latest lock poisoned") = Some(snapshot.clone());
        // Err(SendError) just means there are currently no subscribers;
        // `latest()` still reflects the new snapshot, matching Python
        // (which iterates an empty subscriber list and does nothing).
        let _ = self.sender.send(snapshot);
    }
}

impl Default for DashboardHub {
    fn default() -> Self {
        Self::new(DEFAULT_SUBSCRIBER_QUEUE_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_common::Mode;

    fn snap(equity: &str) -> DashboardSnapshot {
        let equity: rust_decimal::Decimal = equity.parse().unwrap();
        DashboardSnapshot {
            mode: Mode::Demo,
            now: "2026-05-03T12:00:00+00:00".to_string(),
            cash: equity,
            equity,
            equity_high: equity,
            drawdown_pct: 0.0,
            realized_pnl_total: rust_decimal::Decimal::ZERO,
            sol_balance: rust_decimal::Decimal::new(5, 2),
            sol_gas_paid_total: rust_decimal::Decimal::ZERO,
            sol_mark: rust_decimal::Decimal::new(140, 0),
            kill_switch_active: false,
            kill_switch_reason: String::new(),
            positions: Vec::new(),
            recent_trades: Vec::new(),
            equity_history: Vec::new(),
            signals: Vec::new(),
            pair_charts: Vec::new(),
            decisions: Vec::new(),
            whale_activity: Vec::new(),
            limiter: None,
        }
    }

    /// Waits for the next snapshot on `rx`, transparently skipping over any
    /// `Lagged` notifications (mirrors the Python hub's "drop oldest,
    /// deliver newest" semantics from the subscriber's point of view).
    async fn recv_latest(
        rx: &mut broadcast::Receiver<Arc<DashboardSnapshot>>,
    ) -> Arc<DashboardSnapshot> {
        loop {
            match rx.recv().await {
                Ok(snap) => return snap,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("unexpected recv error: {e}"),
            }
        }
    }

    #[tokio::test]
    async fn hub_starts_with_no_snapshot() {
        let hub = DashboardHub::default();
        assert!(hub.latest().is_none());
    }

    #[tokio::test]
    async fn publish_updates_latest() {
        let hub = DashboardHub::default();
        hub.publish(snap("50.0"));
        let latest = hub.latest().expect("latest set after publish");
        assert_eq!(latest.mode, Mode::Demo);
    }

    #[tokio::test]
    async fn subscriber_receives_published_snapshot() {
        let hub = DashboardHub::default();
        let mut sub = hub.subscribe();
        hub.publish(snap("42.0"));
        let received =
            tokio::time::timeout(std::time::Duration::from_secs(1), recv_latest(&mut sub))
                .await
                .expect("timed out waiting for snapshot");
        assert_eq!(received.equity, "42.0".parse().unwrap());
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive() {
        let hub = DashboardHub::default();
        let mut s1 = hub.subscribe();
        let mut s2 = hub.subscribe();
        hub.publish(snap("100.0"));
        let a = recv_latest(&mut s1).await;
        let b = recv_latest(&mut s2).await;
        assert_eq!(a.equity, b.equity);
        assert_eq!(a.equity, "100.0".parse().unwrap());
    }

    #[tokio::test]
    async fn full_subscriber_drops_oldest_silently() {
        let hub = DashboardHub::new(1);
        let mut sub = hub.subscribe();
        hub.publish(snap("1.0"));
        hub.publish(snap("2.0"));
        // Newest wins: the lagged first value is skipped transparently.
        let received = recv_latest(&mut sub).await;
        assert_eq!(received.equity, "2.0".parse().unwrap());
    }

    #[tokio::test]
    async fn dropped_receiver_does_not_block_publish() {
        let hub = DashboardHub::default();
        {
            let _sub = hub.subscribe();
            // subscriber dropped here
        }
        hub.publish(snap("7.0"));
        assert_eq!(hub.latest().unwrap().equity, "7.0".parse().unwrap());
    }
}
