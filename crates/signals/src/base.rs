//! Port of `tradebot/signals/base.py`: shared signal types and helpers.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tradebot_storage::Candle;

/// Everything a signal needs to score a single pair at a single instant.
///
/// `ohlcv` maps a timeframe token (e.g. "1m") to its candle series, mirroring
/// the Python `dict[str, pd.DataFrame]`. `extras` mirrors the Python
/// `dict[str, object]` grab-bag; it is unused by any signal ported so far
/// (only `ta.py` is in scope for this phase), so it is kept as a plain JSON
/// value map for forward compatibility rather than given typed accessors.
#[derive(Debug, Clone, Default)]
pub struct MarketContext {
    pub pair: String,
    pub now: DateTime<Utc>,
    pub ohlcv: HashMap<String, Vec<Candle>>,
    pub extras: HashMap<String, serde_json::Value>,
}

impl MarketContext {
    pub fn new(
        pair: impl Into<String>,
        now: DateTime<Utc>,
        ohlcv: HashMap<String, Vec<Candle>>,
    ) -> Self {
        Self {
            pair: pair.into(),
            now,
            ohlcv,
            extras: HashMap::new(),
        }
    }
}

/// Error returned when constructing a [`SignalScore`] with a score outside
/// the valid `[-1, 1]` range.
#[derive(Debug, Clone, thiserror::Error)]
#[error("score out of range [-1, 1]: {0}")]
pub struct ScoreOutOfRange(pub f64);

/// A named sub-score for one signal at one point in time.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalScore {
    pub signal: String,
    pub pair: String,
    pub timeframe: String,
    /// In `[-1, +1]`.
    pub score: f64,
    pub sampled_at: DateTime<Utc>,
    /// Named sub-scores for transparency.
    pub components: HashMap<String, f64>,
}

impl SignalScore {
    /// Builds a `SignalScore`, validating that `score` is within `[-1, 1]`.
    ///
    /// Mirrors the `__post_init__` validation on the Python dataclass, which
    /// raises `ValueError` for an out-of-range score.
    pub fn new(
        signal: impl Into<String>,
        pair: impl Into<String>,
        timeframe: impl Into<String>,
        score: f64,
        sampled_at: DateTime<Utc>,
        components: HashMap<String, f64>,
    ) -> Result<Self, ScoreOutOfRange> {
        if !(-1.0..=1.0).contains(&score) {
            return Err(ScoreOutOfRange(score));
        }
        Ok(Self {
            signal: signal.into(),
            pair: pair.into(),
            timeframe: timeframe.into(),
            score,
            sampled_at,
            components,
        })
    }
}

/// A signal source. Stateless beyond internal config; emits a `SignalScore`.
///
/// `async_trait` is used so this stays object-safe: later phases hold
/// `Vec<Box<dyn Signal>>`.
#[async_trait]
pub trait Signal: Send + Sync {
    fn name(&self) -> &str;
    fn timeframe(&self) -> &str;

    async fn score(&self, ctx: &MarketContext) -> SignalScore;
}

/// Clip to `[-1, 1]`; treat NaN/inf as neutral `0.0`.
pub fn clamp_score(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-1.0, 1.0)
}

/// Z-score against a rolling window. Returns NaN for the first `window - 1`
/// entries. Zero-variance windows return `0.0` (instead of NaN/inf) so
/// callers don't get garbage.
pub fn rolling_zscore(series: &[f64], window: usize) -> Vec<f64> {
    let n = series.len();
    let mut out = vec![f64::NAN; n];
    if window == 0 {
        return out;
    }
    for i in 0..n {
        if i + 1 < window {
            continue;
        }
        let start = i + 1 - window;
        let win = &series[start..=i];
        let mean = win.iter().sum::<f64>() / window as f64;
        let var = win.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / window as f64;
        let std = var.sqrt();
        out[i] = if std <= 1e-12 {
            0.0
        } else {
            (series[i] - mean) / std
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_score_clips_to_range() {
        assert_eq!(clamp_score(0.5), 0.5);
        assert_eq!(clamp_score(2.0), 1.0);
        assert_eq!(clamp_score(-3.0), -1.0);
        assert_eq!(clamp_score(0.0), 0.0);
    }

    #[test]
    fn clamp_score_handles_nan() {
        assert_eq!(clamp_score(f64::NAN), 0.0);
    }

    #[test]
    fn clamp_score_handles_inf() {
        assert_eq!(clamp_score(f64::INFINITY), 0.0);
        assert_eq!(clamp_score(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn signal_score_holds_fields() {
        let s = SignalScore::new(
            "ta",
            "SOL/USDC",
            "1m",
            0.5,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            HashMap::from([("rsi".to_string(), 0.3), ("macd".to_string(), 0.7)]),
        )
        .unwrap();
        assert_eq!(s.score, 0.5);
        assert_eq!(s.components["rsi"], 0.3);
    }

    #[test]
    fn signal_score_rejects_out_of_range() {
        let result = SignalScore::new("ta", "SOL/USDC", "1m", 1.5, Utc::now(), HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn rolling_zscore_normal_case() {
        let series: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        let z = rolling_zscore(&series, 5);
        assert!(z.last().unwrap() > &0.0);
        assert!(z[0].is_nan());
    }

    #[test]
    fn rolling_zscore_zero_variance() {
        let series = vec![5.0; 20];
        let z = rolling_zscore(&series, 5);
        let last = *z.last().unwrap();
        assert!(last.is_finite());
        assert!(last.abs() < 1e9);
    }

    #[test]
    fn market_context_minimal() {
        let ctx = MarketContext::new(
            "SOL/USDC",
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            HashMap::from([("1m".to_string(), Vec::new())]),
        );
        assert_eq!(ctx.pair, "SOL/USDC");
        assert!(ctx.ohlcv.contains_key("1m"));
    }
}
