//! Port of `tradebot/core/aggregator.py`: runs every configured signal
//! against a `MarketContext` and combines the results into one composite
//! score per pair.
//!
//! Weighting is two-level: within a signal, its per-timeframe scores are
//! averaged using `timeframe_weights` (falling back to an equal-weight
//! average if none of its timeframes have a positive total weight); the
//! per-signal average is then weighted by `signal_weights` and summed.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tradebot_signals::{clamp_score, MarketContext, Signal, SignalScore};

/// Composite score for one pair at one instant, plus the individual signal
/// scores it was built from. Mirrors `AggregatedScore` in aggregator.py.
#[derive(Debug, Clone)]
pub struct AggregatedScore {
    pub pair: String,
    pub composite: f64,
    pub sampled_at: DateTime<Utc>,
    pub scores: Vec<SignalScore>,
}

/// Runs a set of signals and combines their scores into one composite.
/// Mirrors `SignalAggregator` in aggregator.py.
pub struct SignalAggregator {
    signals: Vec<Box<dyn Signal>>,
    timeframe_weights: HashMap<String, f64>,
    signal_weights: HashMap<String, f64>,
}

impl SignalAggregator {
    pub fn new(
        signals: Vec<Box<dyn Signal>>,
        timeframe_weights: HashMap<String, f64>,
        signal_weights: HashMap<String, f64>,
    ) -> Self {
        Self {
            signals,
            timeframe_weights,
            signal_weights,
        }
    }

    /// Read access to the per-timeframe weight map, needed by the trading
    /// loop to pick the "regime timeframe" (the highest-weight timeframe)
    /// for per-pair regime classification. Mirrors direct `self._tf_weights`
    /// access in loop.py.
    pub fn timeframe_weights(&self) -> &HashMap<String, f64> {
        &self.timeframe_weights
    }

    /// Scores `ctx` with every configured signal and returns the combined
    /// result. Mirrors `SignalAggregator.aggregate` in aggregator.py.
    pub async fn aggregate(&self, ctx: &MarketContext) -> AggregatedScore {
        let mut scores: Vec<SignalScore> = Vec::with_capacity(self.signals.len());
        for sig in &self.signals {
            scores.push(sig.score(ctx).await);
        }

        // Group scores by signal name; compute per-signal tf-weighted
        // average, then weight by signal_weight. Single-TF signals are
        // naturally unpenalised.
        let mut by_signal: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
        for s in &scores {
            let tw = self
                .timeframe_weights
                .get(s.timeframe.as_str())
                .copied()
                .unwrap_or(0.0);
            by_signal
                .entry(s.signal.as_str())
                .or_default()
                .push((s.score, tw));
        }

        let mut composite = 0.0;
        for (signal_name, points) in &by_signal {
            let total_w: f64 = points.iter().map(|(_, w)| *w).sum();
            let avg_score = if total_w <= 0.0 {
                // Treat as equal-weighted if no tf weights match.
                points.iter().map(|(score, _)| *score).sum::<f64>() / points.len().max(1) as f64
            } else {
                points.iter().map(|(score, w)| score * w).sum::<f64>() / total_w
            };
            composite += avg_score
                * self
                    .signal_weights
                    .get(*signal_name)
                    .copied()
                    .unwrap_or(0.0);
        }

        AggregatedScore {
            pair: ctx.pair.clone(),
            composite: clamp_score(composite),
            sampled_at: ctx.now,
            scores,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap as StdHashMap;

    struct StubSignal {
        name: &'static str,
        timeframe: &'static str,
        fixed: f64,
    }

    #[async_trait]
    impl Signal for StubSignal {
        fn name(&self) -> &str {
            self.name
        }
        fn timeframe(&self) -> &str {
            self.timeframe
        }
        async fn score(&self, ctx: &MarketContext) -> SignalScore {
            SignalScore::new(
                self.name,
                ctx.pair.clone(),
                self.timeframe,
                self.fixed,
                ctx.now,
                StdHashMap::new(),
            )
            .expect("fixed score in range")
        }
    }

    fn ctx(pair: &str) -> MarketContext {
        MarketContext::new(pair, Utc::now(), StdHashMap::new())
    }

    fn weights(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[tokio::test]
    async fn combines_two_signals_one_timeframe() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 0.6,
                }),
                Box::new(StubSignal {
                    name: "microstructure",
                    timeframe: "1m",
                    fixed: 0.4,
                }),
            ],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 0.5), ("microstructure", 0.5)]),
        );
        let out = agg.aggregate(&ctx("SOL/USDC")).await;
        assert_eq!(out.pair, "SOL/USDC");
        assert!((out.composite - 0.5).abs() < 1e-9);
        assert_eq!(out.scores.len(), 2);
    }

    #[tokio::test]
    async fn combines_two_timeframes() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 0.2,
                }),
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1h",
                    fixed: 0.8,
                }),
            ],
            weights(&[("1m", 0.3), ("1h", 0.7)]),
            weights(&[("ta", 1.0)]),
        );
        let out = agg.aggregate(&ctx("SOL/USDC")).await;
        assert!((out.composite - 0.62).abs() < 1e-9);
    }

    #[tokio::test]
    async fn clamps_to_unit_range() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 1.0,
                }),
                Box::new(StubSignal {
                    name: "micro",
                    timeframe: "1m",
                    fixed: 1.0,
                }),
            ],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 0.6), ("micro", 0.6)]),
        );
        let out = agg.aggregate(&ctx("X")).await;
        assert!((-1.0..=1.0).contains(&out.composite));
    }

    #[tokio::test]
    async fn skips_unknown_signal_weight() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 0.5,
                }),
                Box::new(StubSignal {
                    name: "ghost",
                    timeframe: "1m",
                    fixed: 1.0,
                }),
            ],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
        );
        let out = agg.aggregate(&ctx("X")).await;
        assert!((out.composite - 0.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn records_per_signal_scores() {
        let agg = SignalAggregator::new(
            vec![Box::new(StubSignal {
                name: "ta",
                timeframe: "1m",
                fixed: 0.4,
            })],
            weights(&[("1m", 1.0)]),
            weights(&[("ta", 1.0)]),
        );
        let out = agg.aggregate(&ctx("X")).await;
        assert_eq!(out.scores[0].signal, "ta");
        assert_eq!(out.scores[0].score, 0.4);
    }

    #[tokio::test]
    async fn single_timeframe_signal_uses_full_signal_weight() {
        let agg = SignalAggregator::new(
            vec![Box::new(StubSignal {
                name: "microstructure",
                timeframe: "1m",
                fixed: 0.6,
            })],
            weights(&[("1m", 0.2), ("1h", 0.8)]),
            weights(&[("ta", 0.5), ("microstructure", 0.5)]),
        );
        let out = agg.aggregate(&ctx("SOL/USDC")).await;
        assert!((out.composite - 0.30).abs() < 1e-9);
    }

    #[tokio::test]
    async fn multi_timeframe_signal_averages_by_tf_weight() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 0.4,
                }),
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1h",
                    fixed: 0.8,
                }),
            ],
            weights(&[("1m", 0.25), ("1h", 0.75)]),
            weights(&[("ta", 1.0)]),
        );
        let out = agg.aggregate(&ctx("SOL/USDC")).await;
        assert!((out.composite - 0.7).abs() < 1e-9);
    }

    #[tokio::test]
    async fn mixed_signals_combine_correctly() {
        let agg = SignalAggregator::new(
            vec![
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1m",
                    fixed: 0.4,
                }),
                Box::new(StubSignal {
                    name: "ta",
                    timeframe: "1h",
                    fixed: 0.8,
                }),
                Box::new(StubSignal {
                    name: "microstructure",
                    timeframe: "1m",
                    fixed: 1.0,
                }),
            ],
            weights(&[("1m", 0.25), ("1h", 0.75)]),
            weights(&[("ta", 0.6), ("microstructure", 0.4)]),
        );
        let out = agg.aggregate(&ctx("SOL/USDC")).await;
        assert!((out.composite - 0.82).abs() < 1e-9);
    }
}
