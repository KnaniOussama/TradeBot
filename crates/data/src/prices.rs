//! Jupiter-quote-based price feed. Port of `tradebot/data/prices.py`.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, watch};
use tracing::warn;

use crate::error::JupiterError;
use crate::jupiter::JupiterClient;

const SUBSCRIBER_QUEUE_CAPACITY: usize = 256;

/// One sampled price observation, published to subscribers of a `PriceFeed`.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceTick {
    pub pair: String,
    pub price: f64,
    pub sampled_at: DateTime<Utc>,
    pub in_amount_quote: f64,
    pub price_impact_pct: f64,
}

#[derive(Debug, Clone)]
struct PairSpec {
    symbol: String,
    mint: String,
    decimals: u32,
}

/// Polls Jupiter quotes for a set of base/quote pairs and publishes
/// `PriceTick`s to subscribers.
///
/// `run()` takes `&self`, so the feed can be shared behind an `Arc` and
/// polled from a background task while `add_pair`/`subscribe`/`stop` are
/// called from the owning task, mirroring how the Python asyncio version is
/// driven from a single event loop.
pub struct PriceFeed {
    jupiter: JupiterClient,
    quote_mint: String,
    quote_decimals: u32,
    quote_symbol: String,
    poll_interval_s: f64,
    sample_size_in_quote: f64,
    slippage_bps: u32,
    pairs: Mutex<Vec<PairSpec>>,
    subscribers: Mutex<Vec<mpsc::Sender<PriceTick>>>,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,
}

impl PriceFeed {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        jupiter: JupiterClient,
        quote_mint: impl Into<String>,
        quote_decimals: u32,
        poll_interval_s: f64,
        sample_size_in_quote: f64,
        slippage_bps: u32,
        quote_symbol: impl Into<String>,
    ) -> Self {
        let (stop_tx, stop_rx) = watch::channel(false);
        Self {
            jupiter,
            quote_mint: quote_mint.into(),
            quote_decimals,
            quote_symbol: quote_symbol.into(),
            poll_interval_s,
            sample_size_in_quote,
            slippage_bps,
            pairs: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            stop_tx,
            stop_rx,
        }
    }

    pub fn add_pair(&self, symbol: impl Into<String>, mint: impl Into<String>, decimals: u32) {
        self.pairs
            .lock()
            .expect("pairs mutex poisoned")
            .push(PairSpec {
                symbol: symbol.into(),
                mint: mint.into(),
                decimals,
            });
    }

    pub fn subscribe(&self) -> mpsc::Receiver<PriceTick> {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE_CAPACITY);
        self.subscribers
            .lock()
            .expect("subscribers mutex poisoned")
            .push(tx);
        rx
    }

    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    pub async fn run(&self) {
        let mut stop_rx = self.stop_rx.clone();
        while !*stop_rx.borrow() {
            let specs: Vec<PairSpec> = self.pairs.lock().expect("pairs mutex poisoned").clone();
            for spec in &specs {
                if let Err(e) = self.poll_pair(spec).await {
                    let pair = format!("{}/{}", spec.symbol, self.quote_symbol);
                    warn!(pair, error = %e, "price_poll_failed");
                }
            }
            tokio::select! {
                _ = stop_rx.changed() => {}
                _ = tokio::time::sleep(Duration::from_secs_f64(self.poll_interval_s)) => {}
            }
        }
    }

    async fn poll_pair(&self, spec: &PairSpec) -> Result<(), JupiterError> {
        // Quote: input = quote token (e.g. USDC), output = base token, then
        // invert. amount_quote_units = self.sample tokens' worth of the
        // quote mint, expressed in its smallest unit.
        let amount_quote_units =
            (self.sample_size_in_quote * 10f64.powi(self.quote_decimals as i32)) as u64;
        let q = self
            .jupiter
            .quote(
                &self.quote_mint,
                &spec.mint,
                amount_quote_units,
                self.slippage_bps,
            )
            .await?;
        // quote tells us how many base tokens we get per `sample_size_in_quote`.
        // price = quote_in / base_out (per 1 base token).
        let out_human = q.out_amount as f64 / 10f64.powi(spec.decimals as i32);
        if out_human == 0.0 {
            return Ok(());
        }
        let price = self.sample_size_in_quote / out_human;
        let tick = PriceTick {
            pair: format!("{}/{}", spec.symbol, self.quote_symbol),
            price,
            sampled_at: Utc::now(),
            in_amount_quote: self.sample_size_in_quote,
            price_impact_pct: q.price_impact_pct,
        };
        let subscribers = self.subscribers.lock().expect("subscribers mutex poisoned");
        for sub in subscribers.iter() {
            if sub.try_send(tick.clone()).is_err() {
                warn!(pair = tick.pair.as_str(), "subscriber_queue_full");
            }
        }
        Ok(())
    }
}
