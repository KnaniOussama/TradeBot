//! Port of `tradebot/core/manual_actions.py`: a thread-safe queue of
//! user-initiated trade actions.
//!
//! The dashboard server enqueues from HTTP request handlers; the trading
//! loop drains at the top of each cycle and converts entries into normal
//! `Action`s that run through the same executor (so slippage gates, gas
//! accounting, and trade logging all apply consistently).

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// A user-requested exit. Mirrors `ManualExitRequest` in manual_actions.py.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualExitRequest {
    pub pair: String,
    pub reason: String,
    pub requested_at: DateTime<Utc>,
}

/// Single-process queue of manual exit requests. Mirrors
/// `ManualActionQueue` in manual_actions.py; uses a `Mutex<VecDeque<_>>`
/// since the Python version is used from both HTTP handlers and the trading
/// loop on the same process.
#[derive(Debug, Default)]
pub struct ManualActionQueue {
    pending: Mutex<VecDeque<ManualExitRequest>>,
}

impl ManualActionQueue {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueues an exit request for `pair`. An empty `reason` defaults to
    /// "manual sell".
    pub fn request_exit(
        &self,
        pair: impl Into<String>,
        reason: impl Into<String>,
    ) -> ManualExitRequest {
        let reason = reason.into();
        let req = ManualExitRequest {
            pair: pair.into(),
            reason: if reason.is_empty() {
                "manual sell".to_string()
            } else {
                reason
            },
            requested_at: Utc::now(),
        };
        self.pending
            .lock()
            .expect("manual action queue mutex poisoned")
            .push_back(req.clone());
        req
    }

    /// Returns and clears all pending requests, in the order they were
    /// enqueued.
    pub fn drain(&self) -> Vec<ManualExitRequest> {
        let mut pending = self
            .pending
            .lock()
            .expect("manual action queue mutex poisoned");
        pending.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.pending
            .lock()
            .expect("manual action queue mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_starts_empty() {
        let q = ManualActionQueue::new();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert_eq!(q.drain(), Vec::new());
    }

    #[test]
    fn request_exit_appends_and_drain_clears() {
        let q = ManualActionQueue::new();
        let req = q.request_exit("SOL/USDC", "took profit");
        assert_eq!(req.pair, "SOL/USDC");
        assert_eq!(req.reason, "took profit");
        assert_eq!(q.len(), 1);
        let drained = q.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].pair, "SOL/USDC");
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn empty_reason_defaults_to_manual_sell() {
        let q = ManualActionQueue::new();
        let req = q.request_exit("BONK/USDC", "");
        assert_eq!(req.reason, "manual sell");
    }

    #[test]
    fn multiple_requests_drain_in_order() {
        let q = ManualActionQueue::new();
        q.request_exit("A/USDC", "r1");
        q.request_exit("B/USDC", "r2");
        q.request_exit("C/USDC", "r3");
        let drained = q.drain();
        let pairs: Vec<&str> = drained.iter().map(|r| r.pair.as_str()).collect();
        assert_eq!(pairs, vec!["A/USDC", "B/USDC", "C/USDC"]);
    }
}
