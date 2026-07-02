//! Port of `tradebot/core/decision.py`: the bot's brain. Turns aggregated
//! signal scores, current marks, and portfolio/risk state into a list of
//! `Action`s (enter/exit) plus one `Observation` per pair explaining what
//! was decided and why.
//!
//! Composite scores, confidence, slippage, and all `_pct` values stay
//! `f64`; marks, peaks, and order sizes (`size_quote` / `size_base`) stay
//! `Money` (Decimal), consistent with the rest of this crate.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tradebot_common::Money;

use crate::aggregator::AggregatedScore;
use crate::portfolio::Portfolio;
use crate::regime::{Regime, RegimeLabel};
use crate::risk::{KillAction, RiskManager, RiskState};
use crate::sizing::{kelly_size, KellySizeParams, KellyStats};

/// Why a position was exited. Mirrors `ExitReason` (a `StrEnum`) in
/// decision.py.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    SignalFlip,
    TrailingStop,
    PerTradeKill,
    TakeProfitLadder,
    Manual,
}

impl ExitReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExitReason::SignalFlip => "signal_flip",
            ExitReason::TrailingStop => "trailing_stop",
            ExitReason::PerTradeKill => "per_trade_kill",
            ExitReason::TakeProfitLadder => "take_profit_ladder",
            ExitReason::Manual => "manual",
        }
    }
}

/// Whether an `Action` opens or closes a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Enter,
    Exit,
}

/// One thing to do: open or close a position. Mirrors `Action` in
/// decision.py.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub kind: ActionKind,
    pub pair: String,
    /// For `Enter`: notional quote to deploy.
    pub size_quote: Money,
    /// For `Exit`: base amount to sell.
    pub size_base: Money,
    pub confidence: f64,
    pub reason: Option<ExitReason>,
}

impl Action {
    fn enter(pair: impl Into<String>, size_quote: Money, confidence: f64) -> Self {
        Self {
            kind: ActionKind::Enter,
            pair: pair.into(),
            size_quote,
            size_base: Decimal::ZERO,
            confidence,
            reason: None,
        }
    }

    fn exit(pair: impl Into<String>, size_base: Money, reason: ExitReason) -> Self {
        Self {
            kind: ActionKind::Exit,
            pair: pair.into(),
            size_quote: Decimal::ZERO,
            size_base,
            confidence: 0.0,
            reason: Some(reason),
        }
    }
}

/// One row per pair per cycle: what the bot saw and what it decided.
/// Mirrors `Observation` in decision.py.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub timestamp: String,
    pub pair: String,
    pub composite: f64,
    pub mark: Money,
    /// "trending_up" | "trending_down" | "chop" | "neutral" | None.
    pub regime: Option<String>,
    /// "enter" | "exit" | "hold".
    pub decision: String,
    pub reason: String,
    pub size_quote: Money,
    pub size_base: Money,
}

/// The bot's decision engine: turns aggregated scores into actions. Mirrors
/// `DecisionEngine` in decision.py.
pub struct DecisionEngine {
    risk: RiskManager,
    entry_threshold: f64,
    exit_flip_threshold: f64,
    peaks: HashMap<String, Money>,
    laddered_pairs: HashSet<String>,
}

impl DecisionEngine {
    pub fn new(risk: RiskManager, entry_threshold: f64, exit_flip_threshold: f64) -> Self {
        Self {
            risk,
            entry_threshold,
            exit_flip_threshold,
            peaks: HashMap::new(),
            laddered_pairs: HashSet::new(),
        }
    }

    /// Raises the tracked peak mark for every open position that has a
    /// fresh mark above its current peak (seeded from avg_entry_price).
    pub fn update_position_peaks(&mut self, marks: &HashMap<String, Money>, portfolio: &Portfolio) {
        for pos in portfolio.open_positions() {
            let mark = match marks.get(&pos.pair) {
                Some(m) => *m,
                None => continue,
            };
            let current_peak = *self.peaks.get(&pos.pair).unwrap_or(&pos.avg_entry_price);
            if mark > current_peak {
                self.peaks.insert(pos.pair.clone(), mark);
            }
        }
    }

    fn peak_for(&self, pair: &str, fallback: Money) -> Money {
        *self.peaks.get(pair).unwrap_or(&fallback)
    }

    /// Core decision loop: exits for open positions, then entries for
    /// everything else, emitting one `Observation` per pair. Mirrors
    /// `DecisionEngine.decide` in decision.py.
    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &mut self,
        scores: &[AggregatedScore],
        marks: &HashMap<String, Money>,
        slippages: &HashMap<String, f64>,
        portfolio: &Portfolio,
        state: &RiskState,
        now: DateTime<Utc>,
        regimes: Option<&HashMap<String, Regime>>,
        kelly_stats: Option<&HashMap<String, KellyStats>>,
    ) -> (Vec<Action>, Vec<Observation>) {
        let ts = now.to_rfc3339();
        let cfg = self.risk.cfg().clone();

        let regime_label = |pair: &str| -> Option<String> {
            regimes
                .and_then(|r| r.get(pair))
                .map(|r| r.label.as_str().to_string())
        };

        // Forget laddered pairs that are no longer open.
        let open_pairs: HashSet<String> = portfolio
            .open_positions()
            .iter()
            .map(|p| p.pair.clone())
            .collect();
        self.laddered_pairs.retain(|p| open_pairs.contains(p));

        // Update peaks first so trailing stop sees fresh highs.
        self.update_position_peaks(marks, portfolio);

        // Build a lookup of composite by pair for quick access in exit logic.
        let score_map: HashMap<&str, &AggregatedScore> =
            scores.iter().map(|s| (s.pair.as_str(), s)).collect();

        let mut actions: Vec<Action> = Vec::new();
        let mut obs_map: HashMap<String, Observation> = HashMap::new();

        // 1. Exit logic for existing positions.
        for pos in portfolio.open_positions() {
            let mark = marks.get(&pos.pair).copied().unwrap_or(pos.avg_entry_price);
            let entry = pos.avg_entry_price;
            let unrealized_loss_pct = if entry > Decimal::ZERO {
                ((entry - mark) / entry).to_f64().unwrap_or(0.0).max(0.0)
            } else {
                0.0
            };
            let composite = score_map
                .get(pos.pair.as_str())
                .map(|s| s.composite)
                .unwrap_or(0.0);
            let regime_lbl = regime_label(&pos.pair);

            // Take-profit ladder (only once per opening of a position).
            let tp_pct = cfg.tp_ladder_pct;
            let tp_frac = cfg.tp_ladder_fraction;
            let tp_gain_pct = if entry > Decimal::ZERO {
                ((mark - entry) / entry).to_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            if !self.laddered_pairs.contains(&pos.pair)
                && entry > Decimal::ZERO
                && tp_gain_pct >= tp_pct
                && tp_frac > 0.0
            {
                let size_out =
                    pos.base_amount * Decimal::from_f64_retain(tp_frac).unwrap_or(Decimal::ZERO);
                actions.push(Action::exit(
                    pos.pair.clone(),
                    size_out,
                    ExitReason::TakeProfitLadder,
                ));
                self.laddered_pairs.insert(pos.pair.clone());
                obs_map.insert(
                    pos.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: pos.pair.clone(),
                        composite,
                        mark,
                        regime: regime_lbl,
                        decision: "exit".to_string(),
                        reason: format!("take-profit ladder triggered at +{:.1}%", tp_pct * 100.0),
                        size_quote: Decimal::ZERO,
                        size_base: size_out,
                    },
                );
                continue;
            }

            // Per-trade kill.
            if self.risk.check_per_trade_kill(unrealized_loss_pct) == KillAction::Kill {
                actions.push(Action::exit(
                    pos.pair.clone(),
                    pos.base_amount,
                    ExitReason::PerTradeKill,
                ));
                obs_map.insert(
                    pos.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: pos.pair.clone(),
                        composite,
                        mark,
                        regime: regime_lbl,
                        decision: "exit".to_string(),
                        reason: format!(
                            "per-trade kill: loss {:.2}% exceeded limit",
                            unrealized_loss_pct * 100.0
                        ),
                        size_quote: Decimal::ZERO,
                        size_base: pos.base_amount,
                    },
                );
                continue;
            }

            // Trailing stop.
            let peak = self.peak_for(&pos.pair, entry);
            let trailing_pct = cfg.trailing_stop_pct;
            if peak > Decimal::ZERO {
                let drop_pct = ((peak - mark) / peak).to_f64().unwrap_or(0.0);
                if drop_pct >= trailing_pct {
                    actions.push(Action::exit(
                        pos.pair.clone(),
                        pos.base_amount,
                        ExitReason::TrailingStop,
                    ));
                    obs_map.insert(
                        pos.pair.clone(),
                        Observation {
                            timestamp: ts.clone(),
                            pair: pos.pair.clone(),
                            composite,
                            mark,
                            regime: regime_lbl,
                            decision: "exit".to_string(),
                            reason: format!(
                                "trailing stop: dropped {:.2}% from peak {:.4}",
                                drop_pct * 100.0,
                                peak.to_f64().unwrap_or(0.0)
                            ),
                            size_quote: Decimal::ZERO,
                            size_base: pos.base_amount,
                        },
                    );
                    continue;
                }
            }

            // Signal flip while in profit.
            if let Some(agg) = score_map.get(pos.pair.as_str()) {
                if agg.composite < self.exit_flip_threshold && mark > entry {
                    actions.push(Action::exit(
                        pos.pair.clone(),
                        pos.base_amount,
                        ExitReason::SignalFlip,
                    ));
                    obs_map.insert(
                        pos.pair.clone(),
                        Observation {
                            timestamp: ts.clone(),
                            pair: pos.pair.clone(),
                            composite,
                            mark,
                            regime: regime_lbl,
                            decision: "exit".to_string(),
                            reason: format!(
                                "signal flip: composite {:.3} below exit threshold {:.3}",
                                agg.composite, self.exit_flip_threshold
                            ),
                            size_quote: Decimal::ZERO,
                            size_base: pos.base_amount,
                        },
                    );
                    continue;
                }
            }

            // No exit triggered: emit hold observation for open position.
            obs_map
                .entry(pos.pair.clone())
                .or_insert_with(|| Observation {
                    timestamp: ts.clone(),
                    pair: pos.pair.clone(),
                    composite,
                    mark,
                    regime: regime_lbl,
                    decision: "hold".to_string(),
                    reason: "trailing stop OK / no exit triggered".to_string(),
                    size_quote: Decimal::ZERO,
                    size_base: Decimal::ZERO,
                });
        }

        // 2. Entry logic: skip if kill switch active.
        if state.kill_switch_active {
            for s in scores {
                obs_map
                    .entry(s.pair.clone())
                    .or_insert_with(|| Observation {
                        timestamp: ts.clone(),
                        pair: s.pair.clone(),
                        composite: s.composite,
                        mark: marks.get(&s.pair).copied().unwrap_or(Decimal::ZERO),
                        regime: regime_label(&s.pair),
                        decision: "hold".to_string(),
                        reason: "kill switch active: entries blocked".to_string(),
                        size_quote: Decimal::ZERO,
                        size_base: Decimal::ZERO,
                    });
            }
            return (actions, obs_map.into_values().collect());
        }

        for s in scores {
            if obs_map.contains_key(&s.pair) {
                // Position was already handled in exit logic above: skip
                // entry for the same pair.
                continue;
            }

            let mark = marks.get(&s.pair).copied().unwrap_or(Decimal::ZERO);
            let regime_lbl = regime_label(&s.pair);

            if s.composite < self.entry_threshold {
                obs_map.insert(
                    s.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: s.pair.clone(),
                        composite: s.composite,
                        mark,
                        regime: regime_lbl,
                        decision: "hold".to_string(),
                        reason: format!(
                            "composite {:.3} below threshold {:.3}",
                            s.composite, self.entry_threshold
                        ),
                        size_quote: Decimal::ZERO,
                        size_base: Decimal::ZERO,
                    },
                );
                continue;
            }

            // Regime gate: block entry if chop and regime_block_chop is
            // enabled.
            if cfg.regime_filter_enabled && cfg.regime_block_chop {
                if let Some(regime) = regimes.and_then(|r| r.get(&s.pair)) {
                    if regime.label == RegimeLabel::Chop {
                        obs_map.insert(
                            s.pair.clone(),
                            Observation {
                                timestamp: ts.clone(),
                                pair: s.pair.clone(),
                                composite: s.composite,
                                mark,
                                regime: regime_lbl,
                                decision: "hold".to_string(),
                                reason: format!("blocked: chop regime (ADX={:.1})", regime.adx),
                                size_quote: Decimal::ZERO,
                                size_base: Decimal::ZERO,
                            },
                        );
                        continue; // skip entry in choppy market
                    }
                }
            }

            let slippage = slippages.get(&s.pair).copied().unwrap_or(0.0);
            let confidence = ((s.composite - self.entry_threshold) / (1.0 - self.entry_threshold))
                .clamp(0.0, 1.0);

            // Non-sizing risk checks (slippage, max trades, position count,
            // etc).
            let gates = self
                .risk
                .evaluate_entry(&s.pair, confidence, portfolio, state, slippage, now);
            if !gates.allowed {
                obs_map.insert(
                    s.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: s.pair.clone(),
                        composite: s.composite,
                        mark,
                        regime: regime_lbl,
                        decision: "hold".to_string(),
                        reason: format!("entry gate blocked: {}", gates.reason),
                        size_quote: Decimal::ZERO,
                        size_base: Decimal::ZERO,
                    },
                );
                continue;
            }

            // Kelly sizing (if enabled and stats available).
            let size = if cfg.use_kelly_sizing {
                match kelly_stats.and_then(|k| k.get(&s.pair)) {
                    Some(stats) => {
                        let params = KellySizeParams {
                            fallback_min: cfg.per_trade_size_min,
                            fallback_max: cfg.per_trade_size_max,
                            ..KellySizeParams::default()
                        };
                        kelly_size(portfolio.cash, confidence, stats, &params)
                    }
                    None => gates.size_quote,
                }
            } else {
                gates.size_quote
            };

            if size > Decimal::ZERO {
                actions.push(Action::enter(s.pair.clone(), size, confidence));
                obs_map.insert(
                    s.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: s.pair.clone(),
                        composite: s.composite,
                        mark,
                        regime: regime_lbl,
                        decision: "enter".to_string(),
                        reason: format!(
                            "entry: composite {:.3} >= threshold {:.3}",
                            s.composite, self.entry_threshold
                        ),
                        size_quote: size,
                        size_base: Decimal::ZERO,
                    },
                );
            } else {
                obs_map.insert(
                    s.pair.clone(),
                    Observation {
                        timestamp: ts.clone(),
                        pair: s.pair.clone(),
                        composite: s.composite,
                        mark,
                        regime: regime_lbl,
                        decision: "hold".to_string(),
                        reason: "size computed as zero: insufficient cash or sizing limit"
                            .to_string(),
                        size_quote: Decimal::ZERO,
                        size_base: Decimal::ZERO,
                    },
                );
            }
        }

        (actions, obs_map.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::ToPrimitive;
    use tradebot_common::Mode;
    use tradebot_config::models::RiskConfig;
    use tradebot_storage::Side;

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn agg(pair: &str, composite: f64) -> AggregatedScore {
        AggregatedScore {
            pair: pair.to_string(),
            composite,
            sampled_at: Utc::now(),
            scores: Vec::new(),
        }
    }

    fn marks(pairs: &[(&str, &str)]) -> HashMap<String, Money> {
        pairs.iter().map(|(k, v)| (k.to_string(), dec(v))).collect()
    }

    fn slips(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn portfolio(cash: &str) -> Portfolio {
        Portfolio::new(Mode::Demo, dec(cash), Decimal::ZERO)
    }

    #[test]
    fn strong_signal_no_position_emits_enter() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.7)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Enter);
        assert_eq!(actions[0].pair, "SOL/USDC");
        assert!(actions[0].size_quote > Decimal::ZERO);
    }

    #[test]
    fn weak_signal_no_action() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.4)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        assert!(actions.iter().all(|a| a.kind != ActionKind::Enter));
    }

    #[test]
    fn signal_flip_in_profit_emits_exit() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            tp_ladder_fraction: 0.0,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", -0.5)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        let exits: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Exit)
            .collect();
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].reason, Some(ExitReason::SignalFlip));
    }

    #[test]
    fn trailing_stop_emits_exit() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            trailing_stop_pct: 0.02,
            tp_ladder_fraction: 0.0,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let state = RiskState::default();
        engine.update_position_peaks(&marks(&[("SOL/USDC", "110.0")]), &p);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.4)],
            &marks(&[("SOL/USDC", "107.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &state,
            Utc::now(),
            None,
            None,
        );
        let exits: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Exit)
            .collect();
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].reason, Some(ExitReason::TrailingStop));
    }

    #[test]
    fn per_trade_kill_emits_exit() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            per_trade_kill_pct: 0.03,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.4)],
            &marks(&[("SOL/USDC", "95.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        let exits: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Exit && a.reason == Some(ExitReason::PerTradeKill))
            .collect();
        assert_eq!(exits.len(), 1);
    }

    #[test]
    fn kill_switch_blocks_entries_but_allows_exits() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.1"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            per_trade_kill_pct: 0.03,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let state = RiskState {
            kill_switch_active: true,
            kill_switch_reason: "drawdown".to_string(),
            ..RiskState::default()
        };
        let (actions, _obs) = engine.decide(
            &[agg("OTHER/USDC", 0.9), agg("SOL/USDC", 0.7)],
            &marks(&[("SOL/USDC", "95.0"), ("OTHER/USDC", "1.0")]),
            &slips(&[("SOL/USDC", 0.005), ("OTHER/USDC", 0.005)]),
            &p,
            &state,
            Utc::now(),
            None,
            None,
        );
        assert!(actions.iter().any(|a| a.kind == ActionKind::Exit));
        assert!(actions.iter().all(|a| a.kind != ActionKind::Enter));
    }

    #[test]
    fn take_profit_ladder_triggers_partial_exit() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.2"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            tp_ladder_pct: 0.02,
            tp_ladder_fraction: 0.5,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.5)],
            &marks(&[("SOL/USDC", "51.0")]), // entry=50, +2%
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        let tp_exits: Vec<&Action> = actions
            .iter()
            .filter(|a| {
                a.kind == ActionKind::Exit && a.reason == Some(ExitReason::TakeProfitLadder)
            })
            .collect();
        assert_eq!(tp_exits.len(), 1);
        assert!((tp_exits[0].size_base.to_f64().unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn take_profit_ladder_only_fires_once_per_position() {
        let mut p = portfolio("100.0");
        p.apply_fill(
            "SOL/USDC",
            Side::Buy,
            dec("0.2"),
            dec("10.0"),
            Decimal::ZERO,
        )
        .unwrap();
        let cfg = RiskConfig {
            tp_ladder_pct: 0.02,
            tp_ladder_fraction: 0.5,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        // First decide: TP fires.
        engine.decide(
            &[agg("SOL/USDC", 0.5)],
            &marks(&[("SOL/USDC", "51.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        // Apply the partial exit ourselves (in-loop the executor would do
        // this).
        p.apply_fill(
            "SOL/USDC",
            Side::Sell,
            dec("0.1"),
            dec("5.1"),
            Decimal::ZERO,
        )
        .unwrap();
        // Second decide at a higher price: no second TP fires (already
        // laddered out).
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.5)],
            &marks(&[("SOL/USDC", "52.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        let tp_exits: Vec<&Action> = actions
            .iter()
            .filter(|a| a.reason == Some(ExitReason::TakeProfitLadder))
            .collect();
        assert_eq!(tp_exits.len(), 0);
    }

    fn regime_map(pair: &str, r: Regime) -> HashMap<String, Regime> {
        HashMap::from([(pair.to_string(), r)])
    }

    #[test]
    fn chop_regime_blocks_entry() {
        let p = portfolio("100.0");
        let cfg = RiskConfig {
            regime_filter_enabled: true,
            regime_block_chop: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let chop = Regime {
            label: RegimeLabel::Chop,
            adx: 10.0,
            ema_fast_above_slow: false,
        };
        let regimes = regime_map("SOL/USDC", chop);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            Some(&regimes),
            None,
        );
        assert!(actions.iter().all(|a| a.kind != ActionKind::Enter));
    }

    #[test]
    fn trending_up_regime_allows_entry() {
        let p = portfolio("100.0");
        let cfg = RiskConfig {
            regime_filter_enabled: true,
            regime_block_chop: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let trend = Regime {
            label: RegimeLabel::TrendingUp,
            adx: 30.0,
            ema_fast_above_slow: true,
        };
        let regimes = regime_map("SOL/USDC", trend);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            Some(&regimes),
            None,
        );
        let enters: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Enter)
            .collect();
        assert_eq!(enters.len(), 1);
    }

    #[test]
    fn regime_filter_disabled_allows_entry_in_chop() {
        let p = portfolio("100.0");
        let cfg = RiskConfig {
            regime_filter_enabled: false,
            regime_block_chop: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let chop = Regime {
            label: RegimeLabel::Chop,
            adx: 10.0,
            ema_fast_above_slow: false,
        };
        let regimes = regime_map("SOL/USDC", chop);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            Some(&regimes),
            None,
        );
        let enters: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Enter)
            .collect();
        assert_eq!(enters.len(), 1);
    }

    #[test]
    fn missing_regime_allows_entry() {
        let p = portfolio("100.0");
        let cfg = RiskConfig {
            regime_filter_enabled: true,
            regime_block_chop: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let regimes: HashMap<String, Regime> = HashMap::new();
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            Some(&regimes),
            None,
        );
        let enters: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Enter)
            .collect();
        assert_eq!(enters.len(), 1);
    }

    #[test]
    fn kelly_sizing_uses_stats_when_sufficient_history() {
        let p = portfolio("1000.0");
        let cfg = RiskConfig {
            use_kelly_sizing: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let mut returns = vec![0.05; 15];
        returns.extend(vec![-0.02; 10]);
        let stats = crate::sizing::compute_kelly_stats(&returns);
        assert_eq!(stats.n_round_trips, 25);
        let kelly_stats = HashMap::from([("SOL/USDC".to_string(), stats)]);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            Some(&kelly_stats),
        );
        let enters: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Enter)
            .collect();
        assert_eq!(enters.len(), 1);
        assert!(enters[0].size_quote > Decimal::ZERO);
    }

    #[test]
    fn kelly_sizing_fallback_on_no_history() {
        let p = portfolio("100.0");
        let cfg = RiskConfig {
            use_kelly_sizing: true,
            ..RiskConfig::default()
        };
        let mut engine = DecisionEngine::new(RiskManager::new(cfg), 0.6, -0.3);
        let empty_stats = KellyStats {
            n_round_trips: 0,
            win_rate: 0.0,
            avg_win_pct: 0.0,
            avg_loss_pct: 0.0,
            kelly_fraction: 0.0,
        };
        let kelly_stats = HashMap::from([("SOL/USDC".to_string(), empty_stats)]);
        let (actions, _obs) = engine.decide(
            &[agg("SOL/USDC", 0.9)],
            &marks(&[("SOL/USDC", "150.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            Some(&kelly_stats),
        );
        let enters: Vec<&Action> = actions
            .iter()
            .filter(|a| a.kind == ActionKind::Enter)
            .collect();
        assert_eq!(enters.len(), 1);
        let size = enters[0].size_quote.to_f64().unwrap();
        assert!((25.0..=55.0).contains(&size));
    }

    #[test]
    fn observation_emitted_for_blocked_chop() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let chop = Regime {
            label: RegimeLabel::Chop,
            adx: 15.0,
            ema_fast_above_slow: false,
        };
        let regimes = regime_map("SOL/USDC", chop);
        let (actions, obs) = engine.decide(
            &[agg("SOL/USDC", 0.8)],
            &marks(&[("SOL/USDC", "100.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            Some(&regimes),
            None,
        );
        assert!(actions.is_empty());
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].decision, "hold");
        assert!(obs[0].reason.to_lowercase().contains("chop"));
    }

    #[test]
    fn observation_emitted_for_low_composite() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let (actions, obs) = engine.decide(
            &[agg("SOL/USDC", 0.3)],
            &marks(&[("SOL/USDC", "100.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        assert!(actions.is_empty());
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].decision, "hold");
        let lower = obs[0].reason.to_lowercase();
        assert!(lower.contains("composite") || lower.contains("threshold"));
    }

    #[test]
    fn observation_emitted_for_buy() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let (actions, obs) = engine.decide(
            &[agg("SOL/USDC", 0.8)],
            &marks(&[("SOL/USDC", "100.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].decision, "enter");
        assert!(obs[0].size_quote > Decimal::ZERO);
    }

    #[test]
    fn observation_has_correct_fields() {
        let p = portfolio("100.0");
        let mut engine = DecisionEngine::new(RiskManager::new(RiskConfig::default()), 0.6, -0.3);
        let (_actions, obs) = engine.decide(
            &[agg("SOL/USDC", 0.8)],
            &marks(&[("SOL/USDC", "100.0")]),
            &slips(&[("SOL/USDC", 0.005)]),
            &p,
            &RiskState::default(),
            Utc::now(),
            None,
            None,
        );
        assert!(!obs.is_empty());
        let o = &obs[0];
        assert_eq!(o.pair, "SOL/USDC");
        assert!(!o.timestamp.is_empty());
    }
}
