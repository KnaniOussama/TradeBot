//! Port of `tradebot/core/whale_activity.py`.
//!
//! Lives in `tradebot-signals` (not a `core` crate) to avoid a signals->core
//! dependency cycle: the tracker wraps Helius and is consumed by
//! [`crate::whale_follow::WhaleFollowSignal`] and, later, the trading loop.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use tracing::warn;

use tradebot_data::{get_recent_swaps_for_wallet, HeliusClient, WhaleSwap};

const DEFAULT_PER_WALLET_LIMIT: u32 = 20;
const DEFAULT_HISTORY_MAX: usize = 200;

/// Single source of truth for recent whale swap activity.
///
/// The trading loop calls [`WhaleActivityTracker::fetch_all`] once per
/// cycle. Multiple `WhaleFollowSignal` instances (one per pair) read from
/// this tracker so we don't repeat Helius calls per pair. The dashboard
/// reads [`WhaleActivityTracker::unmatched_swaps`] to surface activity in
/// tokens the bot doesn't currently watch.
pub struct WhaleActivityTracker {
    pub helius: HeliusClient,
    pub wallets: Vec<String>,
    pub per_wallet_limit: u32,
    pub history_max: usize,
    latest_per_wallet: Mutex<HashMap<String, Vec<WhaleSwap>>>,
    history: Mutex<VecDeque<WhaleSwap>>,
}

impl WhaleActivityTracker {
    pub fn new(helius: HeliusClient, wallets: Vec<String>) -> Self {
        Self {
            helius,
            wallets,
            per_wallet_limit: DEFAULT_PER_WALLET_LIMIT,
            history_max: DEFAULT_HISTORY_MAX,
            latest_per_wallet: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_per_wallet_limit(mut self, per_wallet_limit: u32) -> Self {
        self.per_wallet_limit = per_wallet_limit;
        self
    }

    pub fn with_history_max(mut self, history_max: usize) -> Self {
        self.history_max = history_max;
        self
    }

    /// Pull the latest swaps for every watched wallet. Best-effort:
    /// per-wallet failures are logged and skipped so one bad wallet doesn't
    /// kill the whole cycle.
    pub async fn fetch_all(&self) {
        if self.wallets.is_empty() {
            return;
        }
        for wallet in &self.wallets {
            let swaps = match get_recent_swaps_for_wallet(
                &self.helius,
                wallet,
                self.per_wallet_limit,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!(wallet = %wallet, error = %e, "whale_fetch_failed");
                    continue;
                }
            };
            // Dedupe against history by signature so the rolling buffer
            // doesn't accumulate duplicates across cycles.
            let seen_sigs: HashSet<String> = {
                let history = self.history.lock().unwrap();
                history.iter().map(|s| s.signature.clone()).collect()
            };
            let new_swaps: Vec<WhaleSwap> = swaps
                .iter()
                .filter(|s| !seen_sigs.contains(&s.signature))
                .cloned()
                .collect();
            self.latest_per_wallet
                .lock()
                .unwrap()
                .insert(wallet.clone(), swaps);
            let mut history = self.history.lock().unwrap();
            for s in new_swaps {
                if history.len() == self.history_max {
                    history.pop_front();
                }
                history.push_back(s);
            }
        }
    }

    /// Combined swap stream across all wallets, newest first.
    pub fn all_recent(&self) -> Vec<WhaleSwap> {
        let history = self.history.lock().unwrap();
        let mut out: Vec<WhaleSwap> = history.iter().cloned().collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
        out
    }

    /// Swaps where neither leg's mint is in our watchlist (off-radar
    /// activity).
    pub fn unmatched_swaps(&self, watched_mints: &HashSet<String>, limit: usize) -> Vec<WhaleSwap> {
        let mut out = Vec::new();
        for s in self.all_recent() {
            if watched_mints.contains(&s.in_mint) || watched_mints.contains(&s.out_mint) {
                continue;
            }
            out.push(s);
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const WALLET: &str = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const SOL: &str = "So11111111111111111111111111111111111111112";

    fn swap_tx(
        sent_mint: &str,
        sent_raw: i64,
        received_mint: &str,
        received_raw: i64,
        ts: i64,
    ) -> serde_json::Value {
        json!({
            "signature": format!("sig-{ts}"),
            "timestamp": ts,
            "tokenTransfers": [
                {
                    "mint": sent_mint,
                    "fromUserAccount": WALLET,
                    "toUserAccount": "Pool111",
                    "rawTokenAmount": {"tokenAmount": sent_raw.to_string(), "decimals": 6},
                    "tokenAmount": sent_raw as f64 / 1_000_000.0,
                },
                {
                    "mint": received_mint,
                    "fromUserAccount": "Pool111",
                    "toUserAccount": WALLET,
                    "rawTokenAmount": {"tokenAmount": received_raw.to_string(), "decimals": 9},
                    "tokenAmount": received_raw as f64 / 1_000_000_000.0,
                },
            ],
        })
    }

    #[tokio::test]
    async fn fetch_all_populates_history_newest_first() {
        let payload = vec![
            swap_tx(USDC, 10_000_000, SOL, 70_000_000, 1_714_742_400),
            swap_tx(SOL, 70_000_000, USDC, 10_500_000, 1_714_742_500),
        ];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/addresses/.*/transactions$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
            .mount(&server)
            .await;

        let helius = HeliusClient::with_base_url("k", server.uri());
        let tracker = WhaleActivityTracker::new(helius, vec![WALLET.to_string()]);
        tracker.fetch_all().await;

        let recent = tracker.all_recent();
        assert_eq!(recent.len(), 2);
        // Newest first: the swap at ts=1_714_742_500 comes before ts=1_714_742_400.
        assert!(recent[0].timestamp > recent[1].timestamp);
    }

    #[tokio::test]
    async fn fetch_all_dedupes_across_cycles() {
        let payload = vec![swap_tx(USDC, 10_000_000, SOL, 70_000_000, 1_714_742_400)];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/addresses/.*/transactions$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
            .mount(&server)
            .await;

        let helius = HeliusClient::with_base_url("k", server.uri());
        let tracker = WhaleActivityTracker::new(helius, vec![WALLET.to_string()]);
        tracker.fetch_all().await;
        tracker.fetch_all().await; // same swap fetched again, must not duplicate

        assert_eq!(tracker.all_recent().len(), 1);
    }

    #[tokio::test]
    async fn fetch_all_skips_failed_wallet_and_continues() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/addresses/.*/transactions$"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let helius = HeliusClient::with_base_url("k", server.uri());
        let tracker = WhaleActivityTracker::new(helius, vec![WALLET.to_string()]);
        tracker.fetch_all().await; // must not panic despite per-wallet failure
        assert!(tracker.all_recent().is_empty());
    }

    #[tokio::test]
    async fn fetch_all_noop_when_no_wallets() {
        let helius = HeliusClient::with_base_url("k", "http://127.0.0.1:1");
        let tracker = WhaleActivityTracker::new(helius, Vec::new());
        tracker.fetch_all().await;
        assert!(tracker.all_recent().is_empty());
    }

    fn swap(in_mint: &str, out_mint: &str, ts_secs: i64) -> WhaleSwap {
        WhaleSwap {
            wallet: WALLET.to_string(),
            timestamp: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
            signature: format!("sig-{ts_secs}"),
            in_mint: in_mint.to_string(),
            out_mint: out_mint.to_string(),
            in_amount_raw: 1,
            out_amount_raw: 1,
        }
    }

    #[test]
    fn unmatched_swaps_excludes_watched_mints() {
        let helius = HeliusClient::with_base_url("k", "http://127.0.0.1:1");
        let tracker = WhaleActivityTracker::new(helius, vec![WALLET.to_string()]);
        {
            let mut history = tracker.history.lock().unwrap();
            history.push_back(swap(USDC, SOL, 1));
            history.push_back(swap("OtherIn", "OtherMint", 2));
        }
        let watched = HashSet::from([SOL.to_string(), USDC.to_string()]);
        let unmatched = tracker.unmatched_swaps(&watched, 50);
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0].out_mint, "OtherMint");
    }
}
