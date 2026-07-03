//! Port of `tradebot/signals/microstructure.py`: the order-book microstructure
//! signal. Probes Jupiter quotes to estimate depth imbalance and blends that
//! with VWAP deviation and volume z-score computed from the recent candles.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use rust_decimal::prelude::ToPrimitive;
use tracing::warn;

use tradebot_data::JupiterClient;
use tradebot_storage::Candle;

use crate::base::{clamp_score, rolling_zscore, MarketContext, Signal, SignalScore};

fn to_f64(c: rust_decimal::Decimal) -> f64 {
    c.to_f64().unwrap_or(f64::NAN)
}

/// VWAP mean-reversion deviation score over the last `period` candles.
/// A 2% deviation above/below VWAP maps to the max magnitude; the sign is
/// inverted (mean-reverting): trading above VWAP is bearish.
pub fn vwap_deviation(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period {
        return 0.0;
    }
    let tail = &candles[candles.len() - period..];
    let pv: f64 = tail
        .iter()
        .map(|c| to_f64(c.close) * to_f64(c.volume))
        .sum();
    let v: f64 = tail.iter().map(|c| to_f64(c.volume)).sum();
    if v <= 0.0 {
        return 0.0;
    }
    let vwap = pv / v;
    let last = to_f64(candles.last().unwrap().close);
    if vwap <= 0.0 {
        return 0.0;
    }
    let dev = (last - vwap) / vwap;
    clamp_score(-dev * 50.0)
}

/// Volume-surge z-score over the last `window` candles, signed by the last
/// bar's close direction.
pub fn volume_zscore(candles: &[Candle], window: usize) -> f64 {
    if candles.len() < window + 1 {
        return 0.0;
    }
    let volumes: Vec<f64> = candles.iter().map(|c| to_f64(c.volume)).collect();
    let z = rolling_zscore(&volumes, window);
    let last_z = *z.last().unwrap();
    let n = candles.len();
    let last_change = to_f64(candles[n - 1].close) - to_f64(candles[n - 2].close);
    let direction = if last_change > 0.0 {
        1.0
    } else if last_change < 0.0 {
        -1.0
    } else {
        0.0
    };
    clamp_score((last_z / 3.0) * direction)
}

struct CacheEntry {
    token: i64,
    value: f64,
}

fn default_weights() -> HashMap<String, f64> {
    HashMap::from([
        ("depth_imbalance".to_string(), 0.35),
        ("vwap_dev".to_string(), 0.35),
        ("volume_z".to_string(), 0.30),
    ])
}

/// Order-book microstructure composite signal: depth-probe imbalance, VWAP
/// deviation, and volume z-score.
///
/// The depth probe result is cached per "cycle token" (see
/// [`MicrostructureSignal::set_cycle_token`]) so repeated `score` calls
/// within the same trading cycle don't re-query Jupiter.
pub struct MicrostructureSignal {
    pub pair: String,
    pub timeframe: String,
    pub jupiter: JupiterClient,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_decimals: u32,
    pub quote_decimals: u32,
    pub probe_size_in_quote: f64,
    pub name: String,
    pub weights: HashMap<String, f64>,
    cycle_token: AtomicI64,
    depth_cache: Mutex<Option<CacheEntry>>,
}

impl MicrostructureSignal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pair: impl Into<String>,
        timeframe: impl Into<String>,
        jupiter: JupiterClient,
        base_mint: impl Into<String>,
        quote_mint: impl Into<String>,
        base_decimals: u32,
        quote_decimals: u32,
        probe_size_in_quote: f64,
    ) -> Self {
        Self {
            pair: pair.into(),
            timeframe: timeframe.into(),
            jupiter,
            base_mint: base_mint.into(),
            quote_mint: quote_mint.into(),
            base_decimals,
            quote_decimals,
            probe_size_in_quote,
            name: "microstructure".to_string(),
            weights: default_weights(),
            cycle_token: AtomicI64::new(0),
            depth_cache: Mutex::new(None),
        }
    }

    /// Call at the top of each cycle to enable within-cycle depth probe
    /// caching.
    pub fn set_cycle_token(&self, token: i64) {
        self.cycle_token.store(token, Ordering::SeqCst);
    }

    fn store_cache(&self, token: i64, value: f64) -> f64 {
        *self.depth_cache.lock().unwrap() = Some(CacheEntry { token, value });
        value
    }

    /// Probe Jupiter both ways at the same quote notional. Higher slippage
    /// on a side means that side is thinner. Bias toward the thinner side
    /// (i.e., if buying is hard / asks are thin, that's bullish).
    ///
    /// Result is cached per cycle token to avoid duplicate HTTP calls when
    /// multiple timeframe instances exist for the same pair.
    async fn depth_imbalance(&self) -> f64 {
        let token = self.cycle_token.load(Ordering::SeqCst);
        {
            let cache = self.depth_cache.lock().unwrap();
            if let Some(entry) = cache.as_ref() {
                if entry.token == token {
                    return entry.value;
                }
            }
        }

        let quote_units_in =
            (self.probe_size_in_quote * 10f64.powi(self.quote_decimals as i32)) as u64;
        let buy_q = match self
            .jupiter
            .quote(&self.quote_mint, &self.base_mint, quote_units_in, 200)
            .await
        {
            Ok(q) => q,
            Err(e) => {
                warn!(error = %e, "depth_probe_failed");
                return self.store_cache(token, 0.0);
            }
        };
        let base_out = buy_q.out_amount;
        if base_out == 0 {
            return self.store_cache(token, 0.0);
        }
        let sell_q = match self
            .jupiter
            .quote(&self.base_mint, &self.quote_mint, base_out, 200)
            .await
        {
            Ok(q) => q,
            Err(e) => {
                warn!(error = %e, "depth_probe_failed");
                return self.store_cache(token, 0.0);
            }
        };

        // If sell side has more impact than buy side, sellers are scarce -> bullish.
        let diff = sell_q.price_impact_pct - buy_q.price_impact_pct;
        let score = clamp_score(diff * 50.0);
        self.store_cache(token, score)
    }
}

#[async_trait]
impl Signal for MicrostructureSignal {
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
        let candles = match ctx.ohlcv.get(&self.timeframe) {
            Some(c) if !c.is_empty() => c,
            _ => {
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
        };

        let mut components = HashMap::new();
        components.insert("depth_imbalance".to_string(), self.depth_imbalance().await);
        components.insert("vwap_dev".to_string(), vwap_deviation(candles, 20));
        components.insert("volume_z".to_string(), volume_zscore(candles, 20));

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
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use rust_decimal::Decimal;
    use serde_json::Value;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
    }

    fn load_fixture(name: &str) -> Value {
        let text = std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
    }

    fn flat_candles(n: usize, last_close: f64, last_volume: f64) -> Vec<Candle> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let close = if i == n - 1 { last_close } else { 100.0 };
            let volume = if i == n - 1 { last_volume } else { 1000.0 };
            out.push(Candle {
                timestamp: Utc::now(),
                open: Decimal::from_f64_retain(100.0).unwrap(),
                high: Decimal::from_f64_retain(100.0).unwrap(),
                low: Decimal::from_f64_retain(100.0).unwrap(),
                close: Decimal::from_f64_retain(close).unwrap(),
                volume: Decimal::from_f64_retain(volume).unwrap(),
            });
        }
        out
    }

    #[test]
    fn vwap_deviation_above_vwap_negative() {
        let candles = flat_candles(30, 110.0, 1000.0);
        let score = vwap_deviation(&candles, 20);
        assert!(score < -0.3, "expected < -0.3, got {score}");
    }

    #[test]
    fn vwap_deviation_at_vwap_zero() {
        let candles = flat_candles(30, 100.0, 1000.0);
        let score = vwap_deviation(&candles, 20);
        assert!(score.abs() < 0.05, "expected ~0, got {score}");
    }

    #[test]
    fn volume_zscore_surge_with_up_close_positive() {
        let candles = flat_candles(50, 101.0, 10000.0);
        let score = volume_zscore(&candles, 20);
        assert!(score > 0.3, "expected > 0.3, got {score}");
    }

    #[test]
    fn volume_zscore_surge_with_down_close_negative() {
        let candles = flat_candles(50, 99.0, 10000.0);
        let score = volume_zscore(&candles, 20);
        assert!(score < -0.3, "expected < -0.3, got {score}");
    }

    #[tokio::test]
    async fn microstructure_signal_thin_market_emits_score() {
        let thin = load_fixture("jupiter_quote_thin.json");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&thin))
            .mount(&server)
            .await;

        let jupiter = JupiterClient::new(server.uri(), None, 3);
        let sig = MicrostructureSignal::new(
            "SOL/USDC",
            "1m",
            jupiter,
            "So11111111111111111111111111111111111111112",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            9,
            6,
            10.0,
        );
        let candles = flat_candles(30, 100.0, 1000.0);
        let ctx = MarketContext::new(
            "SOL/USDC",
            Utc::now(),
            HashMap::from([("1m".to_string(), candles)]),
        );
        let score = sig.score(&ctx).await;
        assert_eq!(score.signal, "microstructure");
        assert!(score.score >= -1.0 && score.score <= 1.0);
        assert!(score.components.contains_key("depth_imbalance"));
        assert!(score.components.contains_key("vwap_dev"));
        assert!(score.components.contains_key("volume_z"));
    }

    #[tokio::test]
    async fn microstructure_signal_no_quote_returns_partial() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let jupiter = JupiterClient::new(server.uri(), None, 3);
        let sig = MicrostructureSignal::new(
            "X/Y",
            "1m",
            jupiter,
            "A".repeat(43),
            "B".repeat(43),
            9,
            6,
            10.0,
        );
        let candles = flat_candles(30, 100.0, 1000.0);
        let ctx = MarketContext::new(
            "X/Y",
            Utc::now(),
            HashMap::from([("1m".to_string(), candles)]),
        );
        let score = sig.score(&ctx).await;
        // When depth probe fails, depth_imbalance should be 0 but other components still computed.
        assert_eq!(score.components["depth_imbalance"], 0.0);
    }

    #[tokio::test]
    async fn microstructure_caches_depth_within_cycle() {
        // Only the buy+sell probe from the first score() call should hit the
        // server. If caching is broken, the second score() call would issue
        // two more requests and this expectation would fail at MockServer
        // shutdown.
        let normal = load_fixture("jupiter_quote_sol_usdc.json");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&normal))
            .expect(2)
            .mount(&server)
            .await;

        let jupiter = JupiterClient::new(server.uri(), None, 3);
        let sig = MicrostructureSignal::new(
            "X/Y",
            "1m",
            jupiter,
            "A".repeat(43),
            "B".repeat(43),
            9,
            6,
            10.0,
        );
        let candles = flat_candles(30, 100.0, 1000.0);
        sig.set_cycle_token(123);
        let ctx = MarketContext::new(
            "X/Y",
            Utc::now(),
            HashMap::from([("1m".to_string(), candles)]),
        );
        sig.score(&ctx).await;
        sig.score(&ctx).await; // second call within same cycle: must not re-query Jupiter
    }
}
