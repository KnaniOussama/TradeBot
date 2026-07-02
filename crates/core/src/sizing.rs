//! Port of `tradebot/core/sizing.py`: Kelly-criterion position sizing, with
//! a linear-by-confidence fallback when there isn't enough trade history.
//!
//! `KellyStats` and all fractions/rates here are `f64` (per the Money-vs-f64
//! split: these are ratios, not amounts). `kelly_size` returns `Money`
//! since it computes a quote-currency order size from `available_cash`.
//!
//! Note (Decimal-vs-float tolerance): the computed fraction (`frac` /
//! `confidence_scaled`, clamped) is an `f64`, converted to `Decimal` via
//! `Decimal::from_f64_retain` before multiplying by `available_cash`. That
//! conversion retains the exact (imprecise) binary value of the f64, so a
//! "clean" fraction like `0.42` becomes a long-tailed Decimal
//! (`0.41999999999999998...`) rather than the exact literal `0.42`. The
//! resulting `Money` differs from the mathematically exact answer by at
//! most the f64's own relative epsilon (~1e-15), i.e. a few units in the
//! 13th-or-later decimal place of the result -- not visible at any
//! sane display precision, but tests compare against the exact decimal
//! literal, so they use a `1e-6` absolute tolerance to stay robust to it.

use rust_decimal::Decimal;
use tradebot_common::Money;

use crate::clamp01;

/// Kelly statistics computed from a set of round-trip trade returns.
/// Mirrors `KellyStats` in sizing.py.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KellyStats {
    pub n_round_trips: usize,
    /// Win rate, 0..1.
    pub win_rate: f64,
    /// Average winning trade return as a fraction.
    pub avg_win_pct: f64,
    /// Average losing trade return as a fraction (positive number).
    pub avg_loss_pct: f64,
    /// Raw Kelly fraction f* = (p*b - q) / b where b = avg_win / avg_loss.
    pub kelly_fraction: f64,
}

/// Computes Kelly statistics from a list of round-trip returns. Each entry
/// is a fractional return: positive for wins, negative for losses. Returns
/// `n_round_trips = 0` for an empty slice. Mirrors `compute_kelly_stats` in
/// sizing.py.
pub fn compute_kelly_stats(round_trip_returns: &[f64]) -> KellyStats {
    if round_trip_returns.is_empty() {
        return KellyStats {
            n_round_trips: 0,
            win_rate: 0.0,
            avg_win_pct: 0.0,
            avg_loss_pct: 0.0,
            kelly_fraction: 0.0,
        };
    }

    let wins: Vec<f64> = round_trip_returns
        .iter()
        .copied()
        .filter(|&r| r > 0.0)
        .collect();
    let losses: Vec<f64> = round_trip_returns
        .iter()
        .copied()
        .filter(|&r| r < 0.0)
        .map(|r| -r)
        .collect();
    let n = round_trip_returns.len();
    let win_rate = wins.len() as f64 / n as f64;
    let avg_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().sum::<f64>() / wins.len() as f64
    };
    let avg_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().sum::<f64>() / losses.len() as f64
    };

    if avg_loss <= 1e-9 {
        // No losses recorded, be conservative, don't trust the kelly.
        return KellyStats {
            n_round_trips: n,
            win_rate,
            avg_win_pct: avg_win,
            avg_loss_pct: 0.0,
            kelly_fraction: 0.0,
        };
    }

    let b = avg_win / avg_loss;
    let kelly = if b > 0.0 {
        (win_rate * b - (1.0 - win_rate)) / b
    } else {
        0.0
    };
    KellyStats {
        n_round_trips: n,
        win_rate,
        avg_win_pct: avg_win,
        avg_loss_pct: avg_loss,
        kelly_fraction: kelly.max(0.0),
    }
}

/// Tunables for `kelly_size`, mirroring the keyword defaults on
/// `kelly_size` in sizing.py.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KellySizeParams {
    /// Half-Kelly (or other) safety cap applied to the raw Kelly fraction.
    pub cap_fraction: f64,
    /// Floor on the confidence-scaled Kelly fraction.
    pub min_fraction: f64,
    /// Absolute ceiling regardless of Kelly output.
    pub max_fraction: f64,
    /// Minimum round trips needed before trusting Kelly.
    pub min_round_trips: usize,
    /// Fallback linear-sizing floor (used below `min_round_trips`).
    pub fallback_min: f64,
    /// Fallback linear-sizing ceiling (used below `min_round_trips`).
    pub fallback_max: f64,
}

impl Default for KellySizeParams {
    fn default() -> Self {
        Self {
            cap_fraction: 0.5,
            min_fraction: 0.05,
            max_fraction: 0.5,
            min_round_trips: 20,
            fallback_min: 0.30,
            fallback_max: 0.50,
        }
    }
}

/// Computes a trade size in quote units using the Kelly fraction. Falls
/// back to linear-by-confidence sizing (matching the legacy
/// `RiskManager::size_for`) when insufficient trade history is available
/// (`< min_round_trips`).
///
/// Note: partial sells are treated as full closes when building round-trip
/// returns elsewhere in the pipeline; this is a documented approximation
/// carried over from sizing.py, not something this function itself does.
pub fn kelly_size(
    available_cash: Money,
    confidence: f64,
    stats: &KellyStats,
    params: &KellySizeParams,
) -> Money {
    let frac = if stats.n_round_trips < params.min_round_trips {
        let c = clamp01(confidence);
        params.fallback_min + c * (params.fallback_max - params.fallback_min)
    } else {
        // Half-Kelly (or configured cap), scaled by confidence (weak
        // signals trade smaller).
        let raw = stats.kelly_fraction * params.cap_fraction;
        let confidence_scaled = raw * clamp01(confidence);
        confidence_scaled
            .max(params.min_fraction)
            .min(params.max_fraction)
    };
    available_cash * Decimal::from_f64_retain(frac).unwrap_or(Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::ToPrimitive;

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn approx(actual: Money, expected: f64, tol: f64) {
        let a = actual.to_f64().unwrap();
        assert!(
            (a - expected).abs() <= tol,
            "actual={a} expected={expected}"
        );
    }

    #[test]
    fn compute_kelly_stats_basic() {
        // 6 wins of +5%, 4 losses of -2% -> win_rate=0.6, b=2.5, kelly=0.44.
        let mut returns = vec![0.05; 6];
        returns.extend(vec![-0.02; 4]);
        let s = compute_kelly_stats(&returns);
        assert_eq!(s.n_round_trips, 10);
        assert!((s.win_rate - 0.6).abs() < 1e-9);
        // b = 0.05/0.02 = 2.5; kelly = (0.6*2.5 - 0.4)/2.5 = 1.1/2.5 = 0.44
        assert!((s.kelly_fraction - 0.44).abs() < 1e-3);
    }

    #[test]
    fn compute_kelly_stats_empty() {
        let s = compute_kelly_stats(&[]);
        assert_eq!(s.n_round_trips, 0);
        assert_eq!(s.kelly_fraction, 0.0);
        assert_eq!(s.win_rate, 0.0);
    }

    #[test]
    fn compute_kelly_stats_all_wins() {
        // No losses -> conservative, kelly_fraction=0.0.
        let returns = vec![0.05; 5];
        let s = compute_kelly_stats(&returns);
        assert_eq!(s.n_round_trips, 5);
        assert_eq!(s.win_rate, 1.0);
        assert_eq!(s.kelly_fraction, 0.0);
    }

    #[test]
    fn compute_kelly_stats_all_losses() {
        // All losses -> kelly_fraction=0 (negative kelly clamped).
        let returns = vec![-0.02; 5];
        let s = compute_kelly_stats(&returns);
        assert_eq!(s.n_round_trips, 5);
        assert_eq!(s.win_rate, 0.0);
        assert_eq!(s.kelly_fraction, 0.0);
    }

    #[test]
    fn kelly_size_falls_back_on_insufficient_history() {
        // When n_round_trips < min_round_trips, use linear sizing fallback.
        let s = KellyStats {
            n_round_trips: 0,
            win_rate: 0.0,
            avg_win_pct: 0.0,
            avg_loss_pct: 0.0,
            kelly_fraction: 0.0,
        };
        // confidence=0.6 -> frac = 0.30 + 0.6*(0.50-0.30) = 0.42 -> 42.0
        let params = KellySizeParams {
            fallback_min: 0.30,
            fallback_max: 0.50,
            ..KellySizeParams::default()
        };
        let size = kelly_size(dec("100.0"), 0.6, &s, &params);
        approx(size, 42.0, 0.01);
    }

    #[test]
    fn kelly_size_uses_kelly_when_sufficient_history() {
        // With 20+ trades and positive kelly, size should be > 0.
        let mut returns = vec![0.05; 15];
        returns.extend(vec![-0.02; 10]); // 25 trades
        let s = compute_kelly_stats(&returns);
        assert_eq!(s.n_round_trips, 25);
        let size = kelly_size(dec("1000.0"), 1.0, &s, &KellySizeParams::default());
        assert!(size > Decimal::ZERO);
        assert!(size <= dec("500.0")); // never exceeds max_fraction (0.5)
    }

    #[test]
    fn kelly_size_confidence_zero_returns_min_fraction() {
        // confidence=0 -> smallest possible kelly-scaled size (min_fraction).
        let mut returns = vec![0.05; 15];
        returns.extend(vec![-0.02; 10]); // 25 trades, positive kelly
        let s = compute_kelly_stats(&returns);
        let params = KellySizeParams {
            min_fraction: 0.05,
            max_fraction: 0.5,
            ..KellySizeParams::default()
        };
        let size = kelly_size(dec("1000.0"), 0.0, &s, &params);
        approx(size, 1000.0 * 0.05, 1e-6);
    }

    #[test]
    fn kelly_size_clamps_at_max_fraction() {
        // Even a huge kelly fraction stays within max_fraction.
        let s = KellyStats {
            n_round_trips: 30,
            win_rate: 0.9,
            avg_win_pct: 0.1,
            avg_loss_pct: 0.01,
            kelly_fraction: 8.0, // absurdly large
        };
        let params = KellySizeParams {
            max_fraction: 0.5,
            ..KellySizeParams::default()
        };
        let size = kelly_size(dec("1000.0"), 1.0, &s, &params);
        approx(size, 1000.0 * 0.5, 1e-6);
    }
}
