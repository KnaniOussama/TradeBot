//! Port of `tradebot/signals/onchain.py`: whale-transfer flow and DEX
//! transfer-count surge scoring, driven by Helius enhanced transactions.

use std::collections::{BTreeMap, HashMap, HashSet};

use async_trait::async_trait;
use tracing::warn;

use tradebot_data::{HeliusClient, TokenTransfer};

use crate::base::{clamp_score, rolling_zscore, MarketContext, Signal, SignalScore};

/// Net whale flow score: positive = net DEX -> wallet (accumulation),
/// negative = net wallet -> DEX (distribution). Transfers below `whale_min`
/// are ignored.
pub fn net_whale_flow(
    transfers: &[TokenTransfer],
    mint: &str,
    dex_addresses: &HashSet<String>,
    whale_min: f64,
) -> f64 {
    let relevant: Vec<&TokenTransfer> = transfers
        .iter()
        .filter(|t| t.mint == mint && t.amount >= whale_min)
        .collect();
    if relevant.is_empty() {
        return 0.0;
    }
    let out_of_dex: f64 = relevant
        .iter()
        .filter(|t| dex_addresses.contains(&t.from_addr) && !dex_addresses.contains(&t.to_addr))
        .map(|t| t.amount)
        .sum();
    let into_dex: f64 = relevant
        .iter()
        .filter(|t| dex_addresses.contains(&t.to_addr) && !dex_addresses.contains(&t.from_addr))
        .map(|t| t.amount)
        .sum();
    let total = out_of_dex + into_dex;
    if total <= 0.0 {
        return 0.0;
    }
    let net = (out_of_dex - into_dex) / total;
    clamp_score(net)
}

/// Z-score of recent transfer count vs. a rolling baseline. `counts` is one
/// entry per minute bucket, ordered oldest to newest.
pub fn transfer_count_zscore(counts: &[f64], window: usize) -> f64 {
    if counts.len() < window + 1 {
        return 0.0;
    }
    let z = rolling_zscore(counts, window);
    let last = *z.last().unwrap();
    if !last.is_finite() {
        return 0.0;
    }
    clamp_score(last / 3.0)
}

/// Bucket transfers touching `mint` into 60-second buckets, sorted oldest to
/// newest, and return the per-bucket count series.
fn transfer_counts_by_bucket(transfers: &[TokenTransfer], mint: &str) -> Vec<f64> {
    let mut buckets: BTreeMap<i64, i64> = BTreeMap::new();
    for t in transfers.iter().filter(|t| t.mint == mint) {
        let bucket = (t.timestamp / 60) * 60;
        *buckets.entry(bucket).or_insert(0) += 1;
    }
    buckets.into_values().map(|c| c as f64).collect()
}

fn default_weights() -> HashMap<String, f64> {
    HashMap::from([
        ("whale_flow".to_string(), 0.70),
        ("transfer_z".to_string(), 0.30),
    ])
}

/// On-chain composite signal: whale-transfer flow and transfer-count surge,
/// driven by a single primary DEX address's recent transaction history.
pub struct OnChainSignal {
    pub pair: String,
    pub timeframe: String,
    pub helius: HeliusClient,
    pub mint: String,
    pub dex_addresses: HashSet<String>,
    pub whale_min: f64,
    pub lookback_limit: u32,
    pub name: String,
    pub weights: HashMap<String, f64>,
}

impl OnChainSignal {
    pub fn new(
        pair: impl Into<String>,
        timeframe: impl Into<String>,
        helius: HeliusClient,
        mint: impl Into<String>,
        dex_addresses: HashSet<String>,
    ) -> Self {
        Self {
            pair: pair.into(),
            timeframe: timeframe.into(),
            helius,
            mint: mint.into(),
            dex_addresses,
            whale_min: 1000.0,
            lookback_limit: 100,
            name: "onchain".to_string(),
            weights: default_weights(),
        }
    }

    pub fn with_whale_min(mut self, whale_min: f64) -> Self {
        self.whale_min = whale_min;
        self
    }

    pub fn with_lookback_limit(mut self, lookback_limit: u32) -> Self {
        self.lookback_limit = lookback_limit;
        self
    }

    /// Query the first DEX address for recent activity. v1 simplification.
    async fn fetch_recent_transfers(&self) -> Vec<TokenTransfer> {
        let primary = match self.dex_addresses.iter().next() {
            Some(a) => a,
            None => return Vec::new(),
        };
        match self
            .helius
            .recent_token_transfers(primary, self.lookback_limit)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "helius_fetch_failed");
                Vec::new()
            }
        }
    }
}

#[async_trait]
impl Signal for OnChainSignal {
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
                HashMap::new(),
            )
            .expect("zero score is always in range");
        }

        let transfers = self.fetch_recent_transfers().await;
        let whale = net_whale_flow(&transfers, &self.mint, &self.dex_addresses, self.whale_min);

        let mut transfer_z = 0.0;
        if !transfers.is_empty() {
            let counts = transfer_counts_by_bucket(&transfers, &self.mint);
            if !counts.is_empty() {
                transfer_z = transfer_count_zscore(&counts, 20);
            }
        }

        let mut components = HashMap::new();
        components.insert("whale_flow".to_string(), whale);
        components.insert("transfer_z".to_string(), transfer_z);

        let composite: f64 = components
            .iter()
            .map(|(k, v)| v * self.weights.get(k).copied().unwrap_or(0.0))
            .sum();

        SignalScore::new(
            self.name.clone(),
            ctx.pair.clone(),
            self.timeframe.clone(),
            clamp_score(composite),
            ctx.now,
            components,
        )
        .expect("clamp_score always produces a value in [-1, 1]")
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn transfer(ts: i64, mint: &str, from: &str, to: &str, amount: f64) -> TokenTransfer {
        TokenTransfer {
            signature: ts.to_string(),
            timestamp: ts,
            mint: mint.to_string(),
            from_addr: from.to_string(),
            to_addr: to.to_string(),
            amount,
        }
    }

    #[test]
    fn net_whale_flow_outflow_from_dex_positive() {
        let transfers = vec![
            transfer(1, "M", "DEX1", "Whale1", 10000.0), // DEX -> wallet (accumulation)
            transfer(2, "M", "DEX1", "Whale2", 5000.0),
            transfer(3, "M", "Retail", "DEX1", 50.0), // tiny retail, ignored
        ];
        let dex = HashSet::from(["DEX1".to_string()]);
        let score = net_whale_flow(&transfers, "M", &dex, 1000.0);
        assert!(score > 0.0);
    }

    #[test]
    fn net_whale_flow_inflow_to_dex_negative() {
        let transfers = vec![transfer(1, "M", "Whale1", "DEX1", 10000.0)]; // selling
        let dex = HashSet::from(["DEX1".to_string()]);
        let score = net_whale_flow(&transfers, "M", &dex, 1000.0);
        assert!(score < 0.0);
    }

    #[test]
    fn net_whale_flow_balanced_near_zero() {
        let transfers = vec![
            transfer(1, "M", "DEX1", "W1", 5000.0),
            transfer(2, "M", "W2", "DEX1", 5000.0),
        ];
        let dex = HashSet::from(["DEX1".to_string()]);
        let score = net_whale_flow(&transfers, "M", &dex, 1000.0);
        assert!(score.abs() < 0.05);
    }

    #[test]
    fn transfer_count_zscore_surge_positive() {
        // 50 baseline minutes with 1 transfer each, then a recent burst.
        let mut counts = vec![1.0; 50];
        counts.push(20.0);
        let score = transfer_count_zscore(&counts, 20);
        assert!(score > 0.3, "expected > 0.3, got {score}");
    }

    #[tokio::test]
    async fn onchain_signal_bullish_whale_outflow() {
        let payload = json!([
            {
                "signature": "s1",
                "timestamp": 1746288000,
                "tokenTransfers": [
                    {
                        "fromUserAccount": "DEX1",
                        "toUserAccount": "W1",
                        "mint": "MintX",
                        "tokenAmount": 10000.0,
                    }
                ],
            },
            {
                "signature": "s2",
                "timestamp": 1746288060,
                "tokenTransfers": [
                    {
                        "fromUserAccount": "DEX1",
                        "toUserAccount": "W2",
                        "mint": "MintX",
                        "tokenAmount": 8000.0,
                    }
                ],
            },
        ]);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/addresses/.*/transactions$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
            .mount(&server)
            .await;

        let helius = HeliusClient::with_base_url("k", server.uri());
        let sig = OnChainSignal::new(
            "X/USDC",
            "1m",
            helius,
            "MintX",
            HashSet::from(["DEX1".to_string()]),
        )
        .with_whale_min(1000.0)
        .with_lookback_limit(100);
        let ctx = MarketContext::new("X/USDC", Utc::now(), HashMap::new());
        let score = sig.score(&ctx).await;
        assert_eq!(score.signal, "onchain");
        assert!(score.score > 0.0);
        assert!(score.components.contains_key("whale_flow"));
    }

    #[tokio::test]
    async fn onchain_signal_helius_failure_returns_neutral() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/addresses/.*/transactions$"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let helius = HeliusClient::with_base_url("k", server.uri());
        let sig = OnChainSignal::new(
            "X/USDC",
            "1m",
            helius,
            "MintX",
            HashSet::from(["DEX1".to_string()]),
        )
        .with_whale_min(1000.0);
        let ctx = MarketContext::new("X/USDC", Utc::now(), HashMap::new());
        let score = sig.score(&ctx).await;
        assert_eq!(score.score, 0.0);
    }
}
