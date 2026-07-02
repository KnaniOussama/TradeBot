//! Port of `tradebot/core/regime.py`: Wilder ADX/DMI-based market regime
//! classification.
//!
//! `Regime.adx` and all thresholds/periods here are `f64`/plain numbers,
//! not `Money`, since they are indicator values, not amounts.
//!
//! The tricky part of this port is matching pandas' exact
//! `Series.ewm(alpha=..., adjust=False).mean()` semantics, including its
//! default `ignore_na=False` behavior for the leading `NaN` produced by
//! `diff()` on the +DM/-DM series. Empirically (verified against a live
//! pandas install), that behavior is:
//!   - Leading NaNs before the first valid observation stay NaN.
//!   - A NaN after the series has been seeded holds the previous output
//!     value forward (visually unchanged).
//!   - When the next valid observation `x` arrives after a gap of `g`
//!     consecutive NaNs following a valid value `y`, the blended value is
//!     `(1-alpha)^(g+1) * y + (1 - (1-alpha)^(g+1)) * x` -- i.e. the
//!     position gap still counts toward the decay applied to `y`, it is
//!     not simply skipped.
//!
//! See `ewm_adjust_false` below, which implements exactly this rule.

use rust_decimal::prelude::ToPrimitive;
use tradebot_storage::Candle;

/// Market regime label. Mirrors the `RegimeLabel` literal type in
/// regime.py.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimeLabel {
    TrendingUp,
    TrendingDown,
    Chop,
    Neutral,
}

impl RegimeLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RegimeLabel::TrendingUp => "trending_up",
            RegimeLabel::TrendingDown => "trending_down",
            RegimeLabel::Chop => "chop",
            RegimeLabel::Neutral => "neutral",
        }
    }
}

/// Mirrors `Regime` in regime.py.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Regime {
    pub label: RegimeLabel,
    pub adx: f64,
    pub ema_fast_above_slow: bool,
}

/// Exponential weighted moving average matching pandas'
/// `Series.ewm(alpha=alpha, adjust=False).mean()` with the default
/// `ignore_na=False`. See the module doc comment for the derivation.
fn ewm_adjust_false(x: &[f64], alpha: f64) -> Vec<f64> {
    let decay = 1.0 - alpha;
    let mut out = Vec::with_capacity(x.len());
    let mut y: Option<f64> = None;
    let mut gap: i32 = 0;
    for &v in x {
        if v.is_nan() {
            match y {
                Some(prev) => {
                    out.push(prev);
                    gap += 1;
                }
                None => out.push(f64::NAN),
            }
            continue;
        }
        y = Some(match y {
            None => v,
            Some(prev) => {
                let w_old = decay.powi(gap + 1);
                w_old * prev + (1.0 - w_old) * v
            }
        });
        gap = 0;
        out.push(y.expect("just set"));
    }
    out
}

/// Mirrors pandas' `Series.replace(0, np.nan)`: only an exact-zero value is
/// replaced; NaN passes through unchanged.
fn replace_zero_with_nan(x: f64) -> f64 {
    if x == 0.0 {
        f64::NAN
    } else {
        x
    }
}

/// Computes Wilder's ADX series for `period`. Mirrors `_adx` in regime.py,
/// except the final `.fillna(0.0)` is left to the caller (which only reads
/// the last value).
fn adx_series(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Vec<f64> {
    let n = high.len();
    let alpha = 1.0 / period as f64;

    let mut plus_dm = vec![f64::NAN; n];
    let mut minus_dm = vec![f64::NAN; n];
    let mut tr = vec![0.0; n];

    for i in 0..n {
        if i == 0 {
            // high.diff() and close.shift() are both NaN at row 0; the
            // true-range max(axis=1) skips them (skipna=True default),
            // leaving just high-low.
            tr[i] = high[i] - low[i];
        } else {
            plus_dm[i] = (high[i] - high[i - 1]).max(0.0);
            minus_dm[i] = (low[i - 1] - low[i]).max(0.0);

            let a = high[i] - low[i];
            let b = (high[i] - close[i - 1]).abs();
            let c = (low[i] - close[i - 1]).abs();
            tr[i] = a.max(b).max(c);
        }
    }

    let atr = ewm_adjust_false(&tr, alpha);
    let plus_dm_ewm = ewm_adjust_false(&plus_dm, alpha);
    let minus_dm_ewm = ewm_adjust_false(&minus_dm, alpha);

    let mut dx = vec![0.0; n];
    for i in 0..n {
        let atr_div = replace_zero_with_nan(atr[i]);
        let plus_di = 100.0 * plus_dm_ewm[i] / atr_div;
        let minus_di = 100.0 * minus_dm_ewm[i] / atr_div;
        let denom = replace_zero_with_nan(plus_di + minus_di);
        dx[i] = 100.0 * (plus_di - minus_di).abs() / denom;
    }

    ewm_adjust_false(&dx, alpha)
}

/// EMA with `span`, matching pandas `Series.ewm(span=span,
/// adjust=False).mean()`: `alpha = 2 / (span + 1)`. The close series never
/// has NaNs, so this reduces to the plain recursion, but reuses
/// `ewm_adjust_false` for consistency.
fn ema_span(close: &[f64], span: u32) -> Vec<f64> {
    ewm_adjust_false(close, 2.0 / (span as f64 + 1.0))
}

/// Classifies the current market regime from OHLCV candles (oldest first,
/// matching the Python DataFrame row order). Returns
/// `Regime { label: Neutral, adx: 0.0, ema_fast_above_slow: false }` if
/// there isn't enough data for warmup. Mirrors `classify_regime` in
/// regime.py.
pub fn classify_regime(
    candles: &[Candle],
    adx_period: usize,
    ema_fast: u32,
    ema_slow: u32,
    trending_threshold: f64,
    chop_threshold: f64,
) -> Regime {
    let warmup = adx_period.max(ema_slow as usize) + 5;
    if candles.len() < warmup {
        return Regime {
            label: RegimeLabel::Neutral,
            adx: 0.0,
            ema_fast_above_slow: false,
        };
    }

    let high: Vec<f64> = candles
        .iter()
        .map(|c| c.high.to_f64().unwrap_or(f64::NAN))
        .collect();
    let low: Vec<f64> = candles
        .iter()
        .map(|c| c.low.to_f64().unwrap_or(f64::NAN))
        .collect();
    let close: Vec<f64> = candles
        .iter()
        .map(|c| c.close.to_f64().unwrap_or(f64::NAN))
        .collect();

    let adx = adx_series(&high, &low, &close, adx_period);
    let mut adx_val = *adx.last().expect("non-empty by warmup check");
    if adx_val.is_nan() {
        adx_val = 0.0;
    }

    let ema_fast_v = *ema_span(&close, ema_fast).last().expect("non-empty");
    let ema_slow_v = *ema_span(&close, ema_slow).last().expect("non-empty");
    let fast_above = ema_fast_v > ema_slow_v;

    let label = if adx_val >= trending_threshold {
        if fast_above {
            RegimeLabel::TrendingUp
        } else {
            RegimeLabel::TrendingDown
        }
    } else if adx_val <= chop_threshold {
        RegimeLabel::Chop
    } else {
        RegimeLabel::Neutral
    };

    Regime {
        label,
        adx: adx_val,
        ema_fast_above_slow: fast_above,
    }
}

/// `classify_regime` with the same defaults as regime.py:
/// `adx_period=14, ema_fast=20, ema_slow=50, trending_threshold=25.0,
/// chop_threshold=20.0`.
pub fn classify_regime_default(candles: &[Candle]) -> Regime {
    classify_regime(candles, 14, 20, 50, 25.0, 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use rust_decimal::Decimal;
    use std::path::Path;

    fn load_candles(path: &Path) -> Vec<Candle> {
        let text = std::fs::read_to_string(path).expect("read fixture csv");
        let mut lines = text.lines();
        let header = lines.next().expect("csv header");
        let cols: Vec<&str> = header.split(',').collect();
        let idx = |name: &str| cols.iter().position(|c| *c == name).expect("column");
        let (ts_i, o_i, h_i, l_i, c_i, v_i) = (
            idx("timestamp"),
            idx("open"),
            idx("high"),
            idx("low"),
            idx("close"),
            idx("volume"),
        );
        lines
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let fields: Vec<&str> = line.split(',').collect();
                Candle {
                    timestamp: DateTime::parse_from_rfc3339(fields[ts_i])
                        .expect("parse timestamp")
                        .with_timezone(&Utc),
                    open: fields[o_i].parse::<Decimal>().expect("parse open"),
                    high: fields[h_i].parse::<Decimal>().expect("parse high"),
                    low: fields[l_i].parse::<Decimal>().expect("parse low"),
                    close: fields[c_i].parse::<Decimal>().expect("parse close"),
                    volume: fields[v_i].parse::<Decimal>().expect("parse volume"),
                }
            })
            .collect()
    }

    fn fixture(name: &str) -> Vec<Candle> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        load_candles(&path)
    }

    #[test]
    fn classify_trending_up_on_uptrend_fixture() {
        let candles = fixture("ohlcv_sol_uptrend.csv");
        let r = classify_regime(&candles, 14, 20, 50, 25.0, 20.0);
        assert_eq!(r.label, RegimeLabel::TrendingUp);
        assert!(r.ema_fast_above_slow);
    }

    #[test]
    fn classify_trending_down_on_downtrend_fixture() {
        let candles = fixture("ohlcv_sol_downtrend.csv");
        let r = classify_regime(&candles, 14, 20, 50, 25.0, 20.0);
        assert_eq!(r.label, RegimeLabel::TrendingDown);
        assert!(!r.ema_fast_above_slow);
    }

    #[test]
    fn classify_returns_neutral_on_short_data() {
        let candles = fixture("ohlcv_sol_uptrend.csv");
        let short = &candles[..10];
        let r = classify_regime_default(short);
        assert_eq!(r.label, RegimeLabel::Neutral);
        assert_eq!(r.adx, 0.0);
        assert!(!r.ema_fast_above_slow);
    }

    #[test]
    fn regime_adx_field_populated() {
        let candles = fixture("ohlcv_sol_uptrend.csv");
        let r = classify_regime(&candles, 14, 20, 50, 25.0, 20.0);
        assert!(r.adx > 0.0);
    }

    /// Flat, dead-quiet OHLC (no range at all) should yield ADX == 0 and
    /// classify as chop. This isn't a Python parity case (it's a
    /// degenerate input the golden-vector fixtures don't exercise), just a
    /// sanity check that the chop branch and the zero-division guards
    /// behave.
    #[test]
    fn classify_chop_on_flat_candles() {
        let n = 60;
        let candles: Vec<Candle> = (0..n)
            .map(|i| Candle {
                timestamp: Utc::now() + chrono::Duration::minutes(i as i64),
                open: Decimal::new(100, 0),
                high: Decimal::new(100, 0),
                low: Decimal::new(100, 0),
                close: Decimal::new(100, 0),
                volume: Decimal::new(1000, 0),
            })
            .collect();
        let r = classify_regime_default(&candles);
        assert_eq!(r.label, RegimeLabel::Chop);
        assert_eq!(r.adx, 0.0);
    }

    #[test]
    fn ewm_adjust_false_matches_pandas_reference_vectors() {
        // Captured from a live pandas install: Series(vals).ewm(alpha=0.5,
        // adjust=False).mean(). See module doc comment.
        let cases: &[(&[f64], &[f64])] = &[
            (&[1.0, 2.0], &[1.0, 1.5]),
            (&[1.0, f64::NAN, 2.0], &[1.0, 1.0, 1.75]),
            (&[1.0, f64::NAN, f64::NAN, 2.0], &[1.0, 1.0, 1.0, 1.875]),
            (
                &[f64::NAN, 1.0, f64::NAN, f64::NAN, 2.0],
                &[f64::NAN, 1.0, 1.0, 1.0, 1.875],
            ),
            (
                &[1.0, 2.0, 3.0, f64::NAN, f64::NAN, 4.0],
                &[1.0, 1.5, 2.25, 2.25, 2.25, 3.78125],
            ),
        ];
        for (input, expected) in cases {
            let out = ewm_adjust_false(input, 0.5);
            for (o, e) in out.iter().zip(expected.iter()) {
                if e.is_nan() {
                    assert!(o.is_nan(), "expected NaN, got {o}");
                } else {
                    assert!((o - e).abs() < 1e-12, "got {o} expected {e}");
                }
            }
        }
    }
}
