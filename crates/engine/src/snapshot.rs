//! Port of `tradebot/dashboard/state.py`: the `DashboardSnapshot` data model
//! and `build_snapshot(...)`.
//!
//! JSON field names on every DTO here are load-bearing: the existing JS
//! frontend (`tradebot/dashboard/static/app.js`, ported in a later phase)
//! reads this shape by field name, so every struct field is named exactly
//! like its Python dict-key counterpart and left in `serde`'s default
//! snake_case rendering (no renames needed).
//!
//! Money amounts (cash, equity, prices, trade sizes) stay `Money`
//! (`rust_decimal::Decimal`); ratios and scores (`drawdown_pct`,
//! `unrealized_pnl_pct`, `composite`, `change_pct`, signal scores) stay
//! `f64`, consistent with the rest of the Rust port.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::Serialize;

use tradebot_common::{Mode, Money};
use tradebot_core::portfolio::{default_sol_fallback_price, SOL_PAIR};
use tradebot_core::{AggregatedScore, Observation, Portfolio, RiskState};
use tradebot_data::RateLimiterMetrics;
use tradebot_storage::{EquitySnapshot, JsonStorage, Side, Trade};

/// One leg of a position's fill history, oldest first. Mirrors the lineage
/// leg dicts built in `_annotate_trades` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineageLeg {
    pub ts: String,
    pub side: Side,
    pub base: Money,
    pub quote: Money,
    pub price: Money,
    pub realized_pnl: Option<Money>,
    pub badge: String,
}

/// One trade annotated with a badge (`OPEN` / `ADD` / `TRIM n%` / `CLOSE
/// +-$x.xx`) and realized P&L. Mirrors `trade_to_dict` plus the
/// `badge`/`realized_pnl` fields `_annotate_trades` adds in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnnotatedTrade {
    pub pair: String,
    pub side: Side,
    pub base_amount: Money,
    pub quote_amount: Money,
    pub price: Money,
    pub fee_quote: Money,
    pub slippage_pct: f64,
    pub opened_at: String,
    pub confidence: Option<f64>,
    pub badge: String,
    pub realized_pnl: Option<Money>,
}

/// Per-pair state threaded through the chronological trade walk. Mirrors
/// the `{"base": ..., "cost_per_unit": ..., "lineage": [...]}` dict in
/// `_annotate_trades` in state.py.
struct PairLedger {
    base: Money,
    cost_per_unit: Money,
    lineage: Vec<LineageLeg>,
}

/// Amount-comparison epsilon matching Python's `1e-9` in `_annotate_trades`.
fn epsilon() -> Money {
    Decimal::new(1, 9)
}

/// Walks `trades_chrono` (oldest first) per pair, assigning a badge and
/// realized P&L to each trade and tracking the lineage of legs that make up
/// each currently-open position. Mirrors `_annotate_trades` in state.py.
fn annotate_trades(
    trades_chrono: &[Trade],
) -> (Vec<AnnotatedTrade>, HashMap<String, Vec<LineageLeg>>) {
    let eps = epsilon();
    let mut state: HashMap<String, PairLedger> = HashMap::new();
    let mut annotated: Vec<AnnotatedTrade> = Vec::with_capacity(trades_chrono.len());

    for t in trades_chrono {
        let ps = state.entry(t.pair.clone()).or_insert_with(|| PairLedger {
            base: Decimal::ZERO,
            cost_per_unit: Decimal::ZERO,
            lineage: Vec::new(),
        });

        let (badge, realized_pnl): (String, Option<Money>) = match t.side {
            Side::Buy => {
                let badge = if ps.base <= eps {
                    "OPEN".to_string()
                } else {
                    "ADD".to_string()
                };
                if ps.base <= eps {
                    ps.lineage.clear();
                }
                let new_base = ps.base + t.base_amount;
                ps.cost_per_unit =
                    (ps.cost_per_unit * ps.base + t.price * t.base_amount) / new_base;
                ps.base = new_base;
                (badge, None)
            }
            Side::Sell => {
                let base_before = ps.base;
                if base_before <= eps {
                    // Selling without a tracked position (shouldn't normally happen).
                    ("CLOSE".to_string(), Some(Decimal::ZERO))
                } else {
                    let frac = (t.base_amount / base_before).min(Decimal::ONE);
                    let cost = ps.cost_per_unit * t.base_amount;
                    let realized = t.quote_amount - cost;
                    ps.base = (base_before - t.base_amount).max(Decimal::ZERO);
                    let badge = if ps.base <= eps {
                        let sign = if realized >= Decimal::ZERO { "+" } else { "" };
                        let realized_f = realized.to_f64().unwrap_or(0.0);
                        format!("CLOSE {sign}${realized_f:.2}")
                    } else {
                        let pct = (frac.to_f64().unwrap_or(0.0) * 100.0).round();
                        format!("TRIM {pct}%")
                    };
                    (badge, Some(realized))
                }
            }
        };

        let opened_at = t.opened_at.to_rfc3339();
        ps.lineage.push(LineageLeg {
            ts: opened_at.clone(),
            side: t.side,
            base: t.base_amount,
            quote: t.quote_amount,
            price: t.price,
            realized_pnl,
            badge: badge.clone(),
        });
        annotated.push(AnnotatedTrade {
            pair: t.pair.clone(),
            side: t.side,
            base_amount: t.base_amount,
            quote_amount: t.quote_amount,
            price: t.price,
            fee_quote: t.fee_quote,
            slippage_pct: t.slippage_pct,
            opened_at,
            confidence: t.confidence,
            badge,
            realized_pnl,
        });
        if t.side == Side::Sell && ps.base <= eps {
            // Position fully closed; lineage reserved for the next OPEN.
            ps.lineage.clear();
        }
    }

    let open_lineages = state
        .into_iter()
        .filter(|(_, ps)| ps.base > eps)
        .map(|(pair, ps)| (pair, ps.lineage))
        .collect();
    (annotated, open_lineages)
}

/// One open position, with mark-to-market P&L and its fill lineage. Mirrors
/// the position dicts built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PositionSnapshot {
    pub pair: String,
    pub base_amount: Money,
    pub avg_entry_price: Money,
    pub mark_price: Money,
    pub unrealized_pnl_quote: Money,
    pub unrealized_pnl_pct: f64,
    pub lineage: Vec<LineageLeg>,
}

/// One signal's sub-scores for one pair. Mirrors the `components` list in
/// the signal dict built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalComponent {
    pub signal: String,
    pub timeframe: String,
    pub score: f64,
    pub components: HashMap<String, f64>,
}

/// One pair's composite signal score plus its components. Mirrors the
/// signal dict built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalSnapshot {
    pub pair: String,
    pub composite: f64,
    pub sampled_at: String,
    pub components: Vec<SignalComponent>,
}

/// One point on a pair's rolling mark-price chart. Mirrors the `{"t":
/// ..., "p": ...}` dicts built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartPoint {
    pub t: String,
    pub p: Money,
}

/// One pair's rolling mark-price chart summary. Mirrors the pair_chart
/// dicts built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairChart {
    pub pair: String,
    pub last: Money,
    pub change_pct: f64,
    pub high: Money,
    pub low: Money,
    pub points: Vec<ChartPoint>,
}

/// One decision-cycle observation, ready for the dashboard's decision log.
/// Mirrors the decision dicts built in `build_snapshot` in state.py.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionEntry {
    pub timestamp: String,
    pub pair: String,
    pub composite: f64,
    pub mark: Money,
    pub regime: Option<String>,
    pub decision: String,
    pub reason: String,
    pub size_quote: Money,
    pub size_base: Money,
}

/// One whale swap in a token outside the bot's watchlist. Mirrors the
/// `unmatched_swaps` dicts built in loop.py before calling `build_snapshot`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhaleActivityEntry {
    pub ts: String,
    pub wallet: String,
    pub in_mint: String,
    pub out_mint: String,
    pub in_amount_raw: u64,
    pub out_amount_raw: u64,
    pub signature: String,
}

/// Full dashboard snapshot published each cycle. Mirrors `DashboardSnapshot`
/// in state.py; field names are a hard requirement for the JS frontend.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardSnapshot {
    pub mode: Mode,
    pub now: String,
    pub cash: Money,
    pub equity: Money,
    pub equity_high: Money,
    pub drawdown_pct: f64,
    pub realized_pnl_total: Money,
    pub sol_balance: Money,
    pub sol_gas_paid_total: Money,
    pub sol_mark: Money,
    pub kill_switch_active: bool,
    pub kill_switch_reason: String,
    pub positions: Vec<PositionSnapshot>,
    pub recent_trades: Vec<AnnotatedTrade>,
    pub equity_history: Vec<EquitySnapshot>,
    pub signals: Vec<SignalSnapshot>,
    pub pair_charts: Vec<PairChart>,
    pub decisions: Vec<DecisionEntry>,
    pub whale_activity: Vec<WhaleActivityEntry>,
    pub limiter: Option<RateLimiterMetrics>,
}

/// Rolling mark-price history per pair, oldest first. Mirrors the
/// `mark_history: dict[str, list[tuple[datetime, float]]]` parameter to
/// `build_snapshot` in state.py.
pub type MarkHistorySnapshot = BTreeMap<String, Vec<(DateTime<Utc>, Money)>>;

/// The optional keyword arguments `build_snapshot` takes in state.py,
/// bundled into one struct since Rust has no default-argument syntax.
/// `Default::default()` reproduces the Python defaults (`equity_history_limit
/// = 200`, `recent_trades_limit = 50`, everything else `None`/empty).
pub struct SnapshotOptions {
    pub equity_history_limit: usize,
    pub recent_trades_limit: usize,
    pub mark_history: Option<MarkHistorySnapshot>,
    pub observations: Option<Vec<Observation>>,
    pub limiter_metrics: Option<RateLimiterMetrics>,
    pub whale_activity: Option<Vec<WhaleActivityEntry>>,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            equity_history_limit: 200,
            recent_trades_limit: 50,
            mark_history: None,
            observations: None,
            limiter_metrics: None,
            whale_activity: None,
        }
    }
}

/// Assembles a `DashboardSnapshot` from current portfolio/risk state, live
/// marks, aggregated signal scores, and (optionally) chart history, buffered
/// decision observations, rate-limiter metrics, and whale activity. Mirrors
/// `build_snapshot` in state.py.
///
/// Unlike the Python version this is not `async`: every input here
/// (`JsonStorage`'s reads, the trade-lineage walk) is synchronous CPU/file
/// work in the Rust port, so there is nothing to await.
pub fn build_snapshot(
    storage: &JsonStorage,
    portfolio: &mut Portfolio,
    risk_state: &RiskState,
    marks: &HashMap<String, Money>,
    scores: &[AggregatedScore],
    now: DateTime<Utc>,
    options: SnapshotOptions,
) -> DashboardSnapshot {
    let equity = portfolio.equity(marks);
    portfolio.update_equity_high(equity);
    let drawdown = portfolio.drawdown_pct(equity);

    let mut positions: Vec<PositionSnapshot> = portfolio
        .open_positions()
        .into_iter()
        .map(|pos| {
            let mark = marks.get(&pos.pair).copied().unwrap_or(pos.avg_entry_price);
            let unrealized = (mark - pos.avg_entry_price) * pos.base_amount;
            let unrealized_pct = if pos.avg_entry_price > Decimal::ZERO {
                ((mark - pos.avg_entry_price) / pos.avg_entry_price)
                    .to_f64()
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            PositionSnapshot {
                pair: pos.pair.clone(),
                base_amount: pos.base_amount,
                avg_entry_price: pos.avg_entry_price,
                mark_price: mark,
                unrealized_pnl_quote: unrealized,
                unrealized_pnl_pct: unrealized_pct,
                lineage: Vec::new(),
            }
        })
        .collect();

    let signals: Vec<SignalSnapshot> = scores
        .iter()
        .map(|s| SignalSnapshot {
            pair: s.pair.clone(),
            composite: s.composite,
            sampled_at: s.sampled_at.to_rfc3339(),
            components: s
                .scores
                .iter()
                .map(|cs| SignalComponent {
                    signal: cs.signal.clone(),
                    timeframe: cs.timeframe.clone(),
                    score: cs.score,
                    components: cs.components.clone(),
                })
                .collect(),
        })
        .collect();

    let mode = portfolio.mode;
    // Pull all trades for the lineage walk (newest first), then reverse to
    // chronological order.
    let mut trades_chrono = storage.list_trades(mode, 10_000);
    trades_chrono.reverse();
    let (annotated_chrono, open_lineages) = annotate_trades(&trades_chrono);
    // recent_trades wants newest-first, capped.
    let mut recent_trades = annotated_chrono;
    recent_trades.reverse();
    recent_trades.truncate(options.recent_trades_limit);
    // Attach lineage to each open position.
    for pos in &mut positions {
        pos.lineage = open_lineages.get(&pos.pair).cloned().unwrap_or_default();
    }
    let equity_history = storage.list_equity_snapshots(mode, options.equity_history_limit);

    let mut pair_charts: Vec<PairChart> = Vec::new();
    if let Some(mark_history) = &options.mark_history {
        for (pair, points) in mark_history {
            if points.is_empty() {
                continue;
            }
            let series: Vec<ChartPoint> = points
                .iter()
                .map(|(ts, price)| ChartPoint {
                    t: ts.to_rfc3339(),
                    p: *price,
                })
                .collect();
            let first = series[0].p;
            let last = series[series.len() - 1].p;
            let change_pct = if first > Decimal::ZERO {
                ((last - first) / first).to_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            let high = series.iter().map(|c| c.p).max().expect("non-empty series");
            let low = series.iter().map(|c| c.p).min().expect("non-empty series");
            pair_charts.push(PairChart {
                pair: pair.clone(),
                last,
                change_pct,
                high,
                low,
                points: series,
            });
        }
    }

    let mut decisions: Vec<DecisionEntry> = Vec::new();
    if let Some(observations) = &options.observations {
        let start = observations.len().saturating_sub(100);
        for o in observations[start..].iter().rev() {
            decisions.push(DecisionEntry {
                timestamp: o.timestamp.clone(),
                pair: o.pair.clone(),
                composite: o.composite,
                mark: o.mark,
                regime: o.regime.clone(),
                decision: o.decision.clone(),
                reason: o.reason.clone(),
                size_quote: o.size_quote,
                size_base: o.size_base,
            });
        }
    }

    let sol_mark = marks
        .get(SOL_PAIR)
        .copied()
        .unwrap_or_else(default_sol_fallback_price);

    DashboardSnapshot {
        mode,
        now: now.to_rfc3339(),
        cash: portfolio.cash,
        equity,
        equity_high: portfolio.equity_high,
        drawdown_pct: drawdown,
        realized_pnl_total: portfolio.realized_pnl_total,
        sol_balance: portfolio.sol_balance,
        sol_gas_paid_total: portfolio.sol_gas_paid_total,
        sol_mark,
        kill_switch_active: risk_state.kill_switch_active,
        kill_switch_reason: risk_state.kill_switch_reason.clone(),
        positions,
        recent_trades,
        equity_history,
        signals,
        pair_charts,
        decisions,
        whale_activity: options.whale_activity.unwrap_or_default(),
        limiter: options.limiter_metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;
    use tradebot_common::Mode;
    use tradebot_storage::Side;

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn storage() -> (tempfile::TempDir, JsonStorage) {
        let dir = tempdir().unwrap();
        let s = JsonStorage::new(dir.path()).unwrap();
        (dir, s)
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
    }

    fn trade(
        pair: &str,
        side: Side,
        base: &str,
        quote: &str,
        price: &str,
        at: DateTime<Utc>,
    ) -> Trade {
        Trade {
            mode: Mode::Demo,
            pair: pair.to_string(),
            side,
            base_amount: dec(base),
            quote_amount: dec(quote),
            price: dec(price),
            fee_quote: Decimal::ZERO,
            slippage_pct: 0.0,
            tx_signature: None,
            opened_at: at,
            confidence: None,
            notes: None,
        }
    }

    #[test]
    fn build_snapshot_minimal() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions::default(),
        );
        assert_eq!(snap.mode, Mode::Demo);
        assert_eq!(snap.cash, dec("50.0"));
        assert_eq!(snap.equity, dec("50.0"));
        assert!(snap.positions.is_empty());
        assert!(snap.recent_trades.is_empty());
        assert!(snap.equity_history.is_empty());
    }

    #[test]
    fn build_snapshot_includes_positions() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let marks = HashMap::from([("SOL/USDC".to_string(), dec("120.0"))]);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &marks,
            &[],
            now(),
            SnapshotOptions::default(),
        );
        assert_eq!(snap.positions.len(), 1);
        let pos = &snap.positions[0];
        assert_eq!(pos.pair, "SOL/USDC");
        assert_eq!(pos.base_amount, dec("0.1"));
        assert_eq!(pos.avg_entry_price, dec("100.0"));
        assert_eq!(pos.mark_price, dec("120.0"));
        assert_eq!(pos.unrealized_pnl_quote, dec("2.0"));
        assert!((pos.unrealized_pnl_pct - 0.20).abs() < 1e-9);
    }

    #[test]
    fn build_snapshot_includes_recent_trades() {
        let (_dir, storage) = storage();
        storage
            .append_trade(&trade("SOL/USDC", Side::Buy, "0.1", "10.0", "100.0", now()))
            .unwrap();
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions::default(),
        );
        assert_eq!(snap.recent_trades.len(), 1);
        let t = &snap.recent_trades[0];
        assert_eq!(t.pair, "SOL/USDC");
        assert_eq!(t.side, Side::Buy);
        assert_eq!(t.price, dec("100.0"));
    }

    #[test]
    fn build_snapshot_equity_history() {
        let (_dir, storage) = storage();
        for i in 0..5 {
            storage
                .append_equity_snapshot(
                    Mode::Demo,
                    &EquitySnapshot {
                        snapshot_at: now() + chrono::Duration::minutes(i),
                        equity: dec("50.0") + Decimal::from(i),
                        cash: dec("50.0") + Decimal::from(i),
                        positions_value: Decimal::ZERO,
                    },
                )
                .unwrap();
        }
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions {
                equity_history_limit: 10,
                ..SnapshotOptions::default()
            },
        );
        assert_eq!(snap.equity_history.len(), 5);
        assert_eq!(snap.equity_history[0].equity, dec("50.0"));
        assert_eq!(snap.equity_history[4].equity, dec("54.0"));
    }

    #[test]
    fn build_snapshot_signal_scores() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let scores = vec![AggregatedScore {
            pair: "SOL/USDC".to_string(),
            composite: 0.65,
            sampled_at: now(),
            scores: Vec::new(),
        }];
        let marks = HashMap::from([("SOL/USDC".to_string(), dec("150.0"))]);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &marks,
            &scores,
            now(),
            SnapshotOptions::default(),
        );
        assert_eq!(snap.signals.len(), 1);
        assert_eq!(snap.signals[0].pair, "SOL/USDC");
        assert_eq!(snap.signals[0].composite, 0.65);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let snap = DashboardSnapshot {
            mode: Mode::Demo,
            now: "2026-05-03T12:00:00+00:00".to_string(),
            cash: dec("50.0"),
            equity: dec("50.0"),
            equity_high: dec("50.0"),
            drawdown_pct: 0.0,
            realized_pnl_total: Decimal::ZERO,
            sol_balance: dec("0.05"),
            sol_gas_paid_total: Decimal::ZERO,
            sol_mark: dec("140.0"),
            kill_switch_active: false,
            kill_switch_reason: String::new(),
            positions: Vec::new(),
            recent_trades: Vec::new(),
            equity_history: Vec::new(),
            signals: Vec::new(),
            pair_charts: Vec::new(),
            decisions: Vec::new(),
            whale_activity: Vec::new(),
            limiter: None,
        };
        let s = serde_json::to_string(&snap).unwrap();
        assert!(s.contains("\"demo\""));
        assert!(s.contains("\"cash\":50.0"));
    }

    #[test]
    fn build_snapshot_includes_decisions_newest_first_and_capped() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let obs = vec![
            Observation {
                timestamp: "2026-05-03T12:00:00+00:00".to_string(),
                pair: "SOL/USDC".to_string(),
                composite: 0.8,
                mark: dec("150.0"),
                regime: Some("trending_up".to_string()),
                decision: "enter".to_string(),
                reason: "composite 0.800 >= threshold 0.600".to_string(),
                size_quote: dec("30.0"),
                size_base: Decimal::ZERO,
            },
            Observation {
                timestamp: "2026-05-03T12:01:00+00:00".to_string(),
                pair: "SOL/USDC".to_string(),
                composite: 0.3,
                mark: dec("149.0"),
                regime: Some("chop".to_string()),
                decision: "hold".to_string(),
                reason: "composite 0.300 below threshold 0.600".to_string(),
                size_quote: Decimal::ZERO,
                size_base: Decimal::ZERO,
            },
        ];
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions {
                observations: Some(obs),
                ..SnapshotOptions::default()
            },
        );
        assert_eq!(snap.decisions.len(), 2);
        assert_eq!(snap.decisions[0].timestamp, "2026-05-03T12:01:00+00:00");
        assert_eq!(snap.decisions[1].timestamp, "2026-05-03T12:00:00+00:00");
    }

    #[test]
    fn build_snapshot_decisions_empty_by_default() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions::default(),
        );
        assert!(snap.decisions.is_empty());
    }

    #[test]
    fn build_snapshot_limiter_metrics_roundtrip() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let metrics = RateLimiterMetrics {
            rate_limit_rps: 0.9,
            burst: 5,
            current_tokens: 4.5,
            total_acquired: 10,
            throttle_wait_total_s: 0.05,
            recent_rps: 0.3,
            total_429s: 0,
            last_429_at: None,
        };
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions {
                limiter_metrics: Some(metrics),
                ..SnapshotOptions::default()
            },
        );
        let l = snap.limiter.expect("limiter set");
        assert_eq!(l.rate_limit_rps, 0.9);
        assert_eq!(l.total_acquired, 10);
    }

    #[test]
    fn build_snapshot_limiter_none_by_default() {
        let (_dir, storage) = storage();
        let mut p = Portfolio::new(Mode::Demo, dec("50.0"), Decimal::ZERO);
        let snap = build_snapshot(
            &storage,
            &mut p,
            &RiskState::default(),
            &HashMap::new(),
            &[],
            now(),
            SnapshotOptions::default(),
        );
        assert!(snap.limiter.is_none());
    }

    // --- trade lineage / badge annotation ---

    #[test]
    fn annotate_open_then_full_close_tags_close_badge_and_pnl() {
        let t1 = trade(
            "SOL/USDC",
            Side::Buy,
            "1.0",
            "100.0",
            "100.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 0, 0).unwrap(),
        );
        let t2 = trade(
            "SOL/USDC",
            Side::Sell,
            "1.0",
            "110.0",
            "110.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 5, 0).unwrap(),
        );
        let (annotated, open_lineages) = annotate_trades(&[t1, t2]);
        assert_eq!(annotated[0].badge, "OPEN");
        assert!(annotated[0].realized_pnl.is_none());
        assert!(annotated[1].badge.starts_with("CLOSE +$10"));
        assert_eq!(annotated[1].realized_pnl, Some(dec("10.0")));
        assert!(!open_lineages.contains_key("SOL/USDC"));
    }

    #[test]
    fn annotate_open_then_partial_sell_tags_trim_and_keeps_lineage() {
        let t1 = trade(
            "SOL/USDC",
            Side::Buy,
            "2.0",
            "200.0",
            "100.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 0, 0).unwrap(),
        );
        let t2 = trade(
            "SOL/USDC",
            Side::Sell,
            "1.0",
            "110.0",
            "110.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 5, 0).unwrap(),
        );
        let (annotated, open_lineages) = annotate_trades(&[t1, t2]);
        assert_eq!(annotated[1].badge, "TRIM 50%");
        assert_eq!(annotated[1].realized_pnl, Some(dec("10.0")));
        assert!(open_lineages.contains_key("SOL/USDC"));
        assert_eq!(open_lineages["SOL/USDC"].len(), 2);
    }

    #[test]
    fn annotate_dca_buy_after_open_tags_add() {
        let t1 = trade(
            "SOL/USDC",
            Side::Buy,
            "1.0",
            "100.0",
            "100.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 0, 0).unwrap(),
        );
        let t2 = trade(
            "SOL/USDC",
            Side::Buy,
            "1.0",
            "120.0",
            "120.0",
            Utc.with_ymd_and_hms(2026, 5, 3, 10, 5, 0).unwrap(),
        );
        let (annotated, _) = annotate_trades(&[t1, t2]);
        assert_eq!(annotated[0].badge, "OPEN");
        assert_eq!(annotated[1].badge, "ADD");
    }
}
