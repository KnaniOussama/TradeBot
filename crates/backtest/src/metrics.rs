//! Port of `tradebot/backtest/metrics.py`: total return, Sharpe ratio, max
//! drawdown, and win/loss counts from an equity curve and a trade list.
//!
//! Sharpe uses per-bar returns, population variance (ddof=0, matching
//! Python's `sum((r-mean)**2 for r in rets) / len(rets)`), and annualizes by
//! `sqrt(bars_per_year)` where `bars_per_year = 365*24*3600 / bar_seconds`.
//! Equity values are `Money` (Decimal) in the curve/starting_cash the runner
//! hands in, but every computation here is a ratio, so it is done in `f64`
//! after a single lossless-enough conversion, matching the fact that the
//! Python reference computes entirely in floats from the start.

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use tradebot_common::Money;
use tradebot_storage::Side;

/// Computed backtest metrics. Mirrors the dict returned by
/// `compute_metrics` in metrics.py.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub final_equity: f64,
    pub total_return_pct: f64,
    pub n_wins: i64,
    pub n_losses: i64,
    pub max_drawdown_pct: f64,
    pub sharpe: f64,
}

/// The minimal trade view `compute_metrics` needs for win/loss counting:
/// side and fill price. Mirrors the `getattr(t, "side", None) or t["side"]`
/// duck-typing in metrics.py.
#[derive(Debug, Clone, Copy)]
pub struct TradeOutcome {
    pub side: Side,
    pub price: Money,
}

fn money_to_f64(m: Money) -> f64 {
    m.to_f64().unwrap_or(0.0)
}

/// Computes final equity, total return, max drawdown, Sharpe, and win/loss
/// counts. Mirrors `compute_metrics` in metrics.py.
pub fn compute_metrics(
    equity_curve: &[(DateTime<Utc>, Money)],
    trades: &[TradeOutcome],
    starting_cash: Money,
    bar_seconds: u32,
) -> Metrics {
    let starting_cash_f = money_to_f64(starting_cash);

    if equity_curve.is_empty() {
        return Metrics {
            final_equity: starting_cash_f,
            total_return_pct: 0.0,
            n_wins: 0,
            n_losses: 0,
            max_drawdown_pct: 0.0,
            sharpe: 0.0,
        };
    }

    let equities: Vec<f64> = equity_curve.iter().map(|(_, e)| money_to_f64(*e)).collect();
    let final_equity = *equities.last().unwrap();
    let total_return = if starting_cash_f > 0.0 {
        (final_equity - starting_cash_f) / starting_cash_f
    } else {
        0.0
    };

    // Max drawdown.
    let mut peak = equities[0];
    let mut max_dd = 0.0_f64;
    for &e in &equities {
        if e > peak {
            peak = e;
        }
        let dd = if peak > 0.0 { (peak - e) / peak } else { 0.0 };
        if dd > max_dd {
            max_dd = dd;
        }
    }

    // Sharpe: per-bar returns, population variance, annualized.
    let sharpe = if equities.len() < 2 {
        0.0
    } else {
        let rets: Vec<f64> = (1..equities.len())
            .map(|i| equities[i] / equities[i - 1] - 1.0)
            .collect();
        let n = rets.len() as f64;
        let mean = rets.iter().sum::<f64>() / n;
        let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        let std = var.sqrt();
        let bars_per_year = 365.0 * 24.0 * 3600.0 / bar_seconds as f64;
        if std > 0.0 {
            (mean / std) * bars_per_year.sqrt()
        } else {
            0.0
        }
    };

    // Win/loss from round trips: pair each sell with the most recent buy.
    let mut wins = 0i64;
    let mut losses = 0i64;
    let mut last_buy_price: Option<Money> = None;
    for t in trades {
        match t.side {
            Side::Buy => last_buy_price = Some(t.price),
            Side::Sell => {
                if let Some(buy_price) = last_buy_price {
                    if t.price > buy_price {
                        wins += 1;
                    } else {
                        losses += 1;
                    }
                    last_buy_price = None;
                }
            }
        }
    }

    Metrics {
        final_equity,
        total_return_pct: total_return,
        n_wins: wins,
        n_losses: losses,
        max_drawdown_pct: max_dd,
        sharpe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(i: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 1, 0, (i % 60) as u32, (i / 60) as u32)
            .unwrap()
    }

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    #[test]
    fn empty_equity_curve_returns_defaults() {
        let result = compute_metrics(&[], &[], dec("100.0"), 60);
        assert_eq!(result.final_equity, 100.0);
        assert_eq!(result.n_wins, 0);
        assert_eq!(result.n_losses, 0);
        assert_eq!(result.max_drawdown_pct, 0.0);
        assert_eq!(result.sharpe, 0.0);
        assert_eq!(result.total_return_pct, 0.0);
    }

    #[test]
    fn max_drawdown_computed_correctly() {
        let curve = vec![
            (ts(0), dec("100.0")),
            (ts(1), dec("120.0")),
            (ts(2), dec("90.0")),
            (ts(3), dec("100.0")),
        ];
        let result = compute_metrics(&curve, &[], dec("100.0"), 60);
        assert!((result.max_drawdown_pct - 0.25).abs() < 1e-6);
    }

    #[test]
    fn total_return_positive() {
        let curve = vec![(ts(0), dec("100.0")), (ts(1), dec("110.0"))];
        let result = compute_metrics(&curve, &[], dec("100.0"), 60);
        assert!((result.total_return_pct - 0.1).abs() < 1e-9);
        assert_eq!(result.final_equity, 110.0);
    }

    #[test]
    fn win_loss_counted_from_round_trips() {
        let trades = vec![
            TradeOutcome {
                side: Side::Buy,
                price: dec("100.0"),
            },
            TradeOutcome {
                side: Side::Sell,
                price: dec("120.0"),
            },
            TradeOutcome {
                side: Side::Buy,
                price: dec("130.0"),
            },
            TradeOutcome {
                side: Side::Sell,
                price: dec("120.0"),
            },
        ];
        let curve = vec![(ts(0), dec("100.0")), (ts(1), dec("110.0"))];
        let result = compute_metrics(&curve, &trades, dec("100.0"), 60);
        assert_eq!(result.n_wins, 1);
        assert_eq!(result.n_losses, 1);
    }

    #[test]
    fn sharpe_nonzero_with_returns() {
        let flat: Vec<(DateTime<Utc>, Money)> = (0..10).map(|i| (ts(i), dec("100.0"))).collect();
        let result_flat = compute_metrics(&flat, &[], dec("100.0"), 60);
        assert_eq!(result_flat.sharpe, 0.0);

        let growing: Vec<(DateTime<Utc>, Money)> = (0..50)
            .map(|i| {
                let v = 100.0 + i as f64 * 0.5;
                (ts(i), rust_decimal::Decimal::from_f64_retain(v).unwrap())
            })
            .collect();
        let result_growing = compute_metrics(&growing, &[], dec("100.0"), 60);
        assert!(result_growing.sharpe > 0.0);
    }
}
