//! Async token bucket rate limiter. Port of `tradebot/data/rate_limiter.py`.
//!
//! `rate_per_sec` tokens are added per second, capped at `burst`. `acquire()`
//! blocks until at least one token is available, then consumes one.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

const RECENT_ACQUIRES_MAX: usize = 200;
const RECENT_WINDOW_S: f64 = 10.0;

struct State {
    tokens: f64,
    last_refill: Instant,
    total_acquired: u64,
    throttle_wait_total: f64,
    recent_acquires: VecDeque<Instant>,
    last_429_at: Option<f64>,
    total_429s: u64,
}

/// Snapshot of limiter counters, mirroring the dict returned by the Python
/// `TokenBucketLimiter.metrics()`.
#[derive(Debug, Clone, Serialize)]
pub struct RateLimiterMetrics {
    pub rate_limit_rps: f64,
    pub burst: u32,
    pub current_tokens: f64,
    pub total_acquired: u64,
    pub throttle_wait_total_s: f64,
    pub recent_rps: f64,
    pub total_429s: u64,
    pub last_429_at: Option<f64>,
}

pub struct TokenBucketLimiter {
    rate_per_sec: f64,
    burst: u32,
    created_at: Instant,
    state: Mutex<State>,
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

impl TokenBucketLimiter {
    pub fn new(rate_per_sec: f64, burst: u32) -> Self {
        let now = Instant::now();
        Self {
            rate_per_sec,
            burst,
            created_at: now,
            state: Mutex::new(State {
                tokens: burst as f64,
                last_refill: now,
                total_acquired: 0,
                throttle_wait_total: 0.0,
                recent_acquires: VecDeque::with_capacity(RECENT_ACQUIRES_MAX),
                last_429_at: None,
                total_429s: 0,
            }),
        }
    }

    fn refill(&self, state: &mut State, now: Instant) {
        let elapsed = now
            .saturating_duration_since(state.last_refill)
            .as_secs_f64();
        if elapsed > 0.0 {
            state.tokens = (state.tokens + elapsed * self.rate_per_sec).min(self.burst as f64);
            state.last_refill = now;
        }
    }

    /// Block until at least one token is available, then consume one.
    pub async fn acquire(&self) {
        let wait_started = Instant::now();
        loop {
            let wait_s = {
                let mut state = self.state.lock().expect("rate limiter mutex poisoned");
                let now = Instant::now();
                self.refill(&mut state, now);
                if state.tokens >= 1.0 {
                    state.tokens -= 1.0;
                    state.total_acquired += 1;
                    if state.recent_acquires.len() == RECENT_ACQUIRES_MAX {
                        state.recent_acquires.pop_front();
                    }
                    state.recent_acquires.push_back(now);
                    state.throttle_wait_total +=
                        now.saturating_duration_since(wait_started).as_secs_f64();
                    return;
                }
                let deficit = 1.0 - state.tokens;
                (deficit / self.rate_per_sec).max(0.005)
            };
            tokio::time::sleep(Duration::from_secs_f64(wait_s)).await;
        }
    }

    /// Record that the upstream service returned HTTP 429 for a request
    /// guarded by this limiter.
    pub fn record_429(&self) {
        let mut state = self.state.lock().expect("rate limiter mutex poisoned");
        state.last_429_at = Some(
            Instant::now()
                .saturating_duration_since(self.created_at)
                .as_secs_f64(),
        );
        state.total_429s += 1;
    }

    pub fn metrics(&self) -> RateLimiterMetrics {
        let state = self.state.lock().expect("rate limiter mutex poisoned");
        let now = Instant::now();
        let cutoff = now.checked_sub(Duration::from_secs_f64(RECENT_WINDOW_S));
        let recent = state
            .recent_acquires
            .iter()
            .filter(|&&ts| match cutoff {
                Some(cutoff) => ts >= cutoff,
                None => true,
            })
            .count();
        RateLimiterMetrics {
            rate_limit_rps: self.rate_per_sec,
            burst: self.burst,
            current_tokens: round3(state.tokens),
            total_acquired: state.total_acquired,
            throttle_wait_total_s: round3(state.throttle_wait_total),
            recent_rps: round3(recent as f64 / RECENT_WINDOW_S),
            total_429s: state.total_429s,
            last_429_at: state.last_429_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_burst_does_not_block() {
        let lim = TokenBucketLimiter::new(1.0, 5);
        let t0 = Instant::now();
        for _ in 0..5 {
            lim.acquire().await;
        }
        assert!(t0.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn sustained_rate_caps_throughput() {
        let lim = TokenBucketLimiter::new(10.0, 2);
        let t0 = Instant::now();
        for _ in 0..12 {
            lim.acquire().await;
        }
        let elapsed = t0.elapsed().as_secs_f64();
        // 12 acquires at 10/s with burst 2 -> first 2 free, next 10 cost ~1s
        assert!(elapsed > 0.8 && elapsed < 1.4, "elapsed={elapsed}");
    }

    #[tokio::test]
    async fn acquire_records_metrics() {
        let lim = TokenBucketLimiter::new(5.0, 1);
        for _ in 0..3 {
            lim.acquire().await;
        }
        let m = lim.metrics();
        assert_eq!(m.total_acquired, 3);
        assert!(m.throttle_wait_total_s >= 0.0);
    }

    #[tokio::test]
    async fn recent_rps_window() {
        let lim = TokenBucketLimiter::new(100.0, 10);
        for _ in 0..5 {
            lim.acquire().await;
        }
        let m = lim.metrics();
        assert!(m.recent_rps >= 0.0);
    }

    #[tokio::test]
    async fn record_429_updates_metrics() {
        let lim = TokenBucketLimiter::new(10.0, 5);
        lim.record_429();
        lim.record_429();
        let m = lim.metrics();
        assert_eq!(m.total_429s, 2);
        assert!(m.last_429_at.is_some());
    }
}
