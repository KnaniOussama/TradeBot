//! Port of `tradebot/signals/whale_follow.py`.
//!
//! Scores a pair by recent activity from a curated list of "smart-money"
//! wallets. The shared [`WhaleActivityTracker`] does the Helius fetch once
//! per cycle; this signal just queries the resulting swap list. Logic:
//!   - For the requested pair, count swaps that touched its base mint:
//!       * whale BUYS the base (in_mint=quote, out_mint=base) -> +1 contribution
//!       * whale SELLS the base (in_mint=base, out_mint=quote) -> -1 contribution
//!   - Each contribution is weighted by `exp(-dt * ln2 / half_life)` so
//!     older swaps fade.
//!   - Score = clamp(sum_contributions / sum_decay_weights, -1, 1).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use tradebot_data::WhaleSwap;

use crate::base::{clamp_score, MarketContext, Signal, SignalScore};
use crate::whale_activity::WhaleActivityTracker;

const DEFAULT_LOOKBACK_SECONDS: i64 = 1800;
const DEFAULT_DECAY_HALF_LIFE_S: f64 = 600.0;

/// Whale-follow composite signal: time-decayed net buy/sell pressure from a
/// curated wallet list, read from a shared [`WhaleActivityTracker`].
pub struct WhaleFollowSignal {
    pub pair: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub tracker: Arc<WhaleActivityTracker>,
    pub lookback_seconds: i64,
    pub decay_half_life_s: f64,
    pub name: String,
    pub timeframe: String,
    pub weights: HashMap<String, f64>,
}

impl WhaleFollowSignal {
    pub fn new(
        pair: impl Into<String>,
        base_mint: impl Into<String>,
        quote_mint: impl Into<String>,
        tracker: Arc<WhaleActivityTracker>,
    ) -> Self {
        Self {
            pair: pair.into(),
            base_mint: base_mint.into(),
            quote_mint: quote_mint.into(),
            tracker,
            lookback_seconds: DEFAULT_LOOKBACK_SECONDS,
            decay_half_life_s: DEFAULT_DECAY_HALF_LIFE_S,
            name: "whale_follow".to_string(),
            timeframe: "1m".to_string(),
            weights: HashMap::new(),
        }
    }

    pub fn with_lookback_seconds(mut self, lookback_seconds: i64) -> Self {
        self.lookback_seconds = lookback_seconds;
        self
    }

    pub fn with_decay_half_life_s(mut self, decay_half_life_s: f64) -> Self {
        self.decay_half_life_s = decay_half_life_s;
        self
    }

    fn score_from_swaps(&self, swaps: &[WhaleSwap], now: DateTime<Utc>) -> f64 {
        if swaps.is_empty() {
            return 0.0;
        }
        let cutoff = now - chrono::Duration::seconds(self.lookback_seconds);
        let half_life = self.decay_half_life_s.max(1.0);
        let mut relevant = 0.0;
        let mut contribution = 0.0;
        for s in swaps {
            if s.timestamp < cutoff {
                continue;
            }
            let dt = (now - s.timestamp).num_milliseconds() as f64 / 1000.0;
            let decay = (-dt * std::f64::consts::LN_2 / half_life).exp();
            if s.out_mint == self.base_mint && s.in_mint == self.quote_mint {
                contribution += decay; // buy
                relevant += decay;
            } else if s.in_mint == self.base_mint && s.out_mint == self.quote_mint {
                contribution -= decay; // sell
                relevant += decay;
            }
        }
        if relevant <= 0.0 {
            return 0.0;
        }
        contribution / relevant
    }
}

#[async_trait]
impl Signal for WhaleFollowSignal {
    fn name(&self) -> &str {
        &self.name
    }

    fn timeframe(&self) -> &str {
        &self.timeframe
    }

    async fn score(&self, ctx: &MarketContext) -> SignalScore {
        if ctx.pair != self.pair {
            return SignalScore::new(
                self.name.clone(),
                ctx.pair.clone(),
                self.timeframe.clone(),
                0.0,
                ctx.now,
                HashMap::from([("reason".to_string(), 0.0)]),
            )
            .expect("zero score is always in range");
        }
        let swaps = self.tracker.all_recent();
        let raw = self.score_from_swaps(&swaps, ctx.now);
        let score = clamp_score(raw);
        SignalScore::new(
            self.name.clone(),
            ctx.pair.clone(),
            self.timeframe.clone(),
            score,
            ctx.now,
            HashMap::from([
                ("raw".to_string(), raw),
                ("swap_count".to_string(), swaps.len() as f64),
                (
                    "wallet_count".to_string(),
                    self.tracker.wallets.len() as f64,
                ),
            ]),
        )
        .expect("clamp_score always produces a value in [-1, 1]")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use tradebot_data::HeliusClient;

    use super::*;

    const SOL: &str = "So11111111111111111111111111111111111111112";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const WALLET_A: &str = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
    }

    fn swap(in_mint: &str, out_mint: &str, age_seconds: i64) -> WhaleSwap {
        WhaleSwap {
            wallet: WALLET_A.to_string(),
            timestamp: now() - chrono::Duration::seconds(age_seconds),
            signature: "sig".to_string(),
            in_mint: in_mint.to_string(),
            out_mint: out_mint.to_string(),
            in_amount_raw: 1_000_000,
            out_amount_raw: 1_000_000,
        }
    }

    fn make_signal(wallets: Vec<String>) -> WhaleFollowSignal {
        let helius = HeliusClient::with_base_url("test", "http://127.0.0.1:1");
        let tracker = Arc::new(WhaleActivityTracker::new(helius, wallets));
        WhaleFollowSignal::new("SOL/USDC", SOL, USDC, tracker)
    }

    #[test]
    fn score_buy_swap_is_positive() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        let s = swap(USDC, SOL, 0);
        let score = sig.score_from_swaps(&[s], now());
        assert!((score - 1.0).abs() < 1e-9, "got {score}");
    }

    #[test]
    fn score_sell_swap_is_negative() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        let s = swap(SOL, USDC, 0);
        let score = sig.score_from_swaps(&[s], now());
        assert!((score - -1.0).abs() < 1e-9, "got {score}");
    }

    #[test]
    fn score_balanced_buy_and_sell_returns_zero() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        let swaps = vec![swap(USDC, SOL, 0), swap(SOL, USDC, 0)];
        let score = sig.score_from_swaps(&swaps, now());
        assert!(score.abs() < 1e-9, "got {score}");
    }

    #[test]
    fn unrelated_swap_does_not_contribute() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        let other_mint = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";
        let swaps = vec![swap(USDC, other_mint, 0)];
        assert_eq!(sig.score_from_swaps(&swaps, now()), 0.0);
    }

    #[test]
    fn old_swap_outside_lookback_ignored() {
        let sig = make_signal(vec![WALLET_A.to_string()]).with_lookback_seconds(600);
        let old = swap(USDC, SOL, 3600); // 1 hour old > 10 minute lookback
        assert_eq!(sig.score_from_swaps(&[old], now()), 0.0);
    }

    #[test]
    fn decay_makes_old_swap_count_less() {
        let sig = make_signal(vec![WALLET_A.to_string()])
            .with_lookback_seconds(3600)
            .with_decay_half_life_s(300.0);
        let fresh_buy = swap(USDC, SOL, 0);
        let old_sell = swap(SOL, USDC, 900); // 3 half-lives -> ~12.5%
        let score = sig.score_from_swaps(&[fresh_buy, old_sell], now());
        // Fresh buy dominates; net positive but not 1.0.
        assert!(score > 0.5 && score < 1.0, "got {score}");
    }

    #[test]
    fn no_swaps_returns_zero() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        assert_eq!(sig.score_from_swaps(&[], now()), 0.0);
    }

    #[tokio::test]
    async fn score_protocol_returns_zero_for_other_pair() {
        let sig = make_signal(vec![WALLET_A.to_string()]);
        let ctx = MarketContext::new("BONK/USDC", now(), HashMap::new());
        let out = sig.score(&ctx).await;
        assert_eq!(out.score, 0.0);
        assert_eq!(out.signal, "whale_follow");
    }
}
