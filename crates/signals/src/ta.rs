//! Port of `tradebot/signals/ta.py`: the technical-analysis signal.
//!
//! All indicator helpers operate on plain `f64` price series extracted from
//! the candle series in a `MarketContext`. The EMA recursion and rolling
//! windows are written to match pandas' `ewm(adjust=False)` and
//! `rolling(...)` behavior exactly (see the doc comments on each function),
//! since this signal is numeric-parity tested against the Python
//! implementation.

use std::collections::HashMap;

use async_trait::async_trait;
use rust_decimal::prelude::ToPrimitive;

use crate::base::{clamp_score, MarketContext, Signal, SignalScore};

/// Exponential moving average with `adjust=False` semantics:
/// `alpha = 2 / (span + 1)`, `y[0] = x[0]`, `y[t] = alpha*x[t] + (1-alpha)*y[t-1]`.
fn ema_span(x: &[f64], span: u32) -> Vec<f64> {
    let alpha = 2.0 / (span as f64 + 1.0);
    ema_alpha(x, alpha)
}

/// Exponential moving average with an explicit alpha and `adjust=False`
/// semantics: `y[0] = x[0]`, `y[t] = alpha*x[t] + (1-alpha)*y[t-1]`.
fn ema_alpha(x: &[f64], alpha: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len());
    if x.is_empty() {
        return out;
    }
    let mut prev = x[0];
    out.push(prev);
    for &v in &x[1..] {
        prev = alpha * v + (1.0 - alpha) * prev;
        out.push(prev);
    }
    out
}

/// RSI mean-reversion score. `delta = close.diff()`; the leading NaN at
/// index 0 is dropped and the EWM recursion is seeded from the first real
/// delta (index 1). `avg_loss` is floored at `1e-12` only at the point of
/// division, not fed back into the recursion (matches the Python
/// `avg_loss.where(avg_loss > 1e-12, other=1e-12)`, which is applied to the
/// already-computed ewm series, not to the ewm input).
pub fn rsi_score(close: &[f64], period: usize) -> f64 {
    let n = close.len();
    if n < period + 1 {
        return 0.0;
    }
    let mut gain = Vec::with_capacity(n - 1);
    let mut loss = Vec::with_capacity(n - 1);
    for i in 1..n {
        let delta = close[i] - close[i - 1];
        gain.push(delta.max(0.0));
        loss.push((-delta).max(0.0));
    }
    let alpha = 1.0 / period as f64;
    let avg_gain = *ema_alpha(&gain, alpha).last().unwrap();
    let mut avg_loss = *ema_alpha(&loss, alpha).last().unwrap();
    if avg_loss <= 1e-12 {
        avg_loss = 1e-12;
    }
    let rsi = 100.0 - (100.0 / (1.0 + avg_gain / avg_loss));
    clamp_score((50.0 - rsi) / 20.0)
}

/// MACD-line-vs-zero score, normalized by the population stdev of the last
/// (up to) 50 macd-line values.
pub fn macd_score(close: &[f64], fast: u32, slow: u32, signal: u32) -> f64 {
    let n = close.len();
    if n < (slow + signal) as usize {
        return 0.0;
    }
    let ema_fast = ema_span(close, fast);
    let ema_slow = ema_span(close, slow);
    let macd_line: Vec<f64> = ema_fast
        .iter()
        .zip(ema_slow.iter())
        .map(|(f, s)| f - s)
        .collect();

    let tail_len = macd_line.len().min(50);
    let tail = &macd_line[macd_line.len() - tail_len..];
    let mean = tail.iter().sum::<f64>() / tail_len as f64;
    let var = tail.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / tail_len as f64;
    let std = var.sqrt();
    if std < 1e-12 {
        return 0.0;
    }
    let last = *macd_line.last().unwrap();
    clamp_score(last / (2.0 * std))
}

/// Fast/slow EMA crossover score, scaled so a 2% relative diff maps to +-1.
pub fn ema_cross(close: &[f64], fast: u32, slow: u32) -> f64 {
    let n = close.len();
    if n < slow as usize + 5 {
        return 0.0;
    }
    let last_fast = *ema_span(close, fast).last().unwrap();
    let last_slow = *ema_span(close, slow).last().unwrap();
    let diff = (last_fast - last_slow) / last_slow;
    clamp_score(diff * 50.0)
}

/// Bollinger Band mean-reversion position score (inverted: +1 near lower
/// band, -1 near upper band).
pub fn bb_position(close: &[f64], period: usize, stds: f64) -> f64 {
    let n = close.len();
    if n < period {
        return 0.0;
    }
    let window = &close[n - period..n];
    let mean = window.iter().sum::<f64>() / period as f64;
    let var = window.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let std = var.sqrt();
    let upper = mean + stds * std;
    let lower = mean - stds * std;
    if upper - lower < 1e-12 {
        return 0.0;
    }
    let last = close[n - 1];
    let pos = 2.0 * (last - lower) / (upper - lower) - 1.0;
    clamp_score(-pos)
}

/// ATR-normalized momentum score over `period` bars.
pub fn atr_momentum(high: &[f64], low: &[f64], close: &[f64], period: usize) -> f64 {
    let n = close.len();
    if n < period + 5 {
        return 0.0;
    }
    let mut tr = Vec::with_capacity(n);
    tr.push(high[0] - low[0]);
    for i in 1..n {
        let a = high[i] - low[i];
        let b = (high[i] - close[i - 1]).abs();
        let c = (low[i] - close[i - 1]).abs();
        tr.push(a.max(b).max(c));
    }
    let atr = ema_alpha(&tr, 1.0 / period as f64);
    let last_atr = *atr.last().unwrap();
    if last_atr < 1e-12 {
        return 0.0;
    }
    let momentum = (close[n - 1] - close[n - period]) / last_atr;
    clamp_score(momentum / 5.0)
}

fn default_weights() -> HashMap<String, f64> {
    HashMap::from([
        ("rsi".to_string(), 0.15),
        ("macd".to_string(), 0.30),
        ("ema_cross".to_string(), 0.30),
        ("bb".to_string(), 0.10),
        ("atr_mom".to_string(), 0.15),
    ])
}

/// Technical-analysis composite signal: a weighted blend of RSI, MACD,
/// EMA-cross, Bollinger-band position, and ATR-momentum scores.
pub struct TASignal {
    pub timeframe: String,
    pub name: String,
    pub weights: HashMap<String, f64>,
}

impl TASignal {
    pub fn new(timeframe: impl Into<String>) -> Self {
        Self {
            timeframe: timeframe.into(),
            name: "ta".to_string(),
            weights: default_weights(),
        }
    }
}

#[async_trait]
impl Signal for TASignal {
    fn name(&self) -> &str {
        &self.name
    }

    fn timeframe(&self) -> &str {
        &self.timeframe
    }

    async fn score(&self, ctx: &MarketContext) -> SignalScore {
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

        let close: Vec<f64> = candles
            .iter()
            .map(|c| c.close.to_f64().unwrap_or(f64::NAN))
            .collect();
        let high: Vec<f64> = candles
            .iter()
            .map(|c| c.high.to_f64().unwrap_or(f64::NAN))
            .collect();
        let low: Vec<f64> = candles
            .iter()
            .map(|c| c.low.to_f64().unwrap_or(f64::NAN))
            .collect();

        let mut components = HashMap::new();
        components.insert("rsi".to_string(), rsi_score(&close, 14));
        components.insert("macd".to_string(), macd_score(&close, 12, 26, 9));
        components.insert("ema_cross".to_string(), ema_cross(&close, 20, 50));
        components.insert("bb".to_string(), bb_position(&close, 20, 2.0));
        components.insert("atr_mom".to_string(), atr_momentum(&high, &low, &close, 14));

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
    use super::*;

    fn rising_series(n: usize) -> Vec<f64> {
        (0..n).map(|i| 100.0 + i as f64).collect()
    }

    fn falling_series(n: usize) -> Vec<f64> {
        (0..n).map(|i| 100.0 - i as f64).collect()
    }

    #[test]
    fn rsi_score_overbought_negative() {
        let s = rising_series(50);
        let score = rsi_score(&s, 14);
        assert!(score < -0.3, "expected < -0.3, got {score}");
    }

    #[test]
    fn rsi_score_oversold_positive() {
        let s = falling_series(50);
        let score = rsi_score(&s, 14);
        assert!(score > 0.3, "expected > 0.3, got {score}");
    }

    #[tokio::test]
    async fn ta_signal_missing_timeframe_returns_zero() {
        let ctx = MarketContext::new("SOL/USDC", chrono::Utc::now(), HashMap::new());
        let sig = TASignal::new("1m");
        let score = sig.score(&ctx).await;
        assert_eq!(score.score, 0.0);
    }

    #[tokio::test]
    async fn ta_signal_insufficient_data_returns_zero() {
        use rust_decimal::Decimal;
        use tradebot_storage::Candle;

        let candles: Vec<Candle> = (0..5)
            .map(|_| Candle {
                timestamp: chrono::Utc::now(),
                open: Decimal::new(1000, 1),
                high: Decimal::new(1010, 1),
                low: Decimal::new(990, 1),
                close: Decimal::new(1005, 1),
                volume: Decimal::new(10000, 1),
            })
            .collect();
        let ctx = MarketContext::new(
            "SOL/USDC",
            chrono::Utc::now(),
            HashMap::from([("1m".to_string(), candles)]),
        );
        let sig = TASignal::new("1m");
        let score = sig.score(&ctx).await;
        assert_eq!(score.score, 0.0);
    }
}
