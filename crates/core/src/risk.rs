//! Port of `tradebot/core/risk.py`: per-trade sizing, entry gating, and the
//! daily/weekly/drawdown circuit breakers.
//!
//! Ratios and config thresholds (confidence, slippage_pct, all `_pct`
//! fields, daily/weekly pnl pct) are `f64`; cash, equity, and computed order
//! sizes stay `Money` (Decimal). Converting an equity ratio via
//! `Money::to_f64` for the daily/weekly pnl checks introduces at most
//! float-epsilon noise (~1e-15 relative) versus a pure-float Python
//! computation on the same clean decimal inputs; this is far below the
//! `_pct` thresholds being compared against, so no extra tolerance is
//! needed for correctness of the gate decisions themselves.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tradebot_config::models::RiskConfig;
use tradebot_storage::RiskStateRecord;

use crate::clamp01;
use crate::portfolio::Portfolio;
use tradebot_common::Money;

/// Per-mode risk manager state: trade counters, day/week start equity,
/// pause timers, and the kill switch. Mirrors `RiskState` in risk.py.
#[derive(Debug, Clone, Default)]
pub struct RiskState {
    pub trades_per_day: std::collections::HashMap<NaiveDate, i64>,
    pub daily_start_equity: std::collections::HashMap<NaiveDate, Money>,
    pub weekly_start_equity: std::collections::HashMap<NaiveDate, Money>,
    pub day_paused_until: Option<NaiveDate>,
    pub week_paused_until: Option<NaiveDate>,
    pub kill_switch_active: bool,
    pub kill_switch_reason: String,
}

impl RiskState {
    pub fn to_record(&self) -> RiskStateRecord {
        RiskStateRecord {
            trades_per_day: self.trades_per_day.iter().map(|(k, v)| (*k, *v)).collect(),
            daily_start_equity: self
                .daily_start_equity
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect(),
            weekly_start_equity: self
                .weekly_start_equity
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect(),
            day_paused_until: self.day_paused_until,
            week_paused_until: self.week_paused_until,
            kill_switch_active: self.kill_switch_active,
            kill_switch_reason: self.kill_switch_reason.clone(),
        }
    }

    pub fn from_record(r: &RiskStateRecord) -> Self {
        Self {
            trades_per_day: r.trades_per_day.iter().map(|(k, v)| (*k, *v)).collect(),
            daily_start_equity: r.daily_start_equity.iter().map(|(k, v)| (*k, *v)).collect(),
            weekly_start_equity: r
                .weekly_start_equity
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect(),
            day_paused_until: r.day_paused_until,
            week_paused_until: r.week_paused_until,
            kill_switch_active: r.kill_switch_active,
            kill_switch_reason: r.kill_switch_reason.clone(),
        }
    }
}

/// Outcome of `RiskManager::evaluate_entry`. Mirrors `RiskDecision` in
/// risk.py.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskDecision {
    pub allowed: bool,
    pub size_quote: Money,
    pub reason: String,
}

/// Outcome of `RiskManager::check_per_trade_kill`. Mirrors the
/// `Literal["ok", "kill"]` return type in risk.py.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillAction {
    Ok,
    Kill,
}

/// Monday of the ISO week containing `d`. Mirrors `_week_start` in
/// risk.py (`d - timedelta(days=d.weekday())`).
fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// Port of `RiskManager` in risk.py.
pub struct RiskManager {
    cfg: RiskConfig,
}

impl RiskManager {
    pub fn new(cfg: RiskConfig) -> Self {
        Self { cfg }
    }

    /// Confidence in `[0, 1]` maps linearly up to `size_max`: at c=0.6 ->
    /// 30% (size_min), at c=1.0 -> 50% (size_max). `frac = c * size_max`,
    /// clamped to `[size_min, size_max]` when positive.
    pub fn size_for(&self, confidence: f64, available_cash: Money) -> Money {
        let c = clamp01(confidence);
        let mut frac = c * self.cfg.per_trade_size_max;
        if frac > 0.0 {
            frac = frac
                .max(self.cfg.per_trade_size_min)
                .min(self.cfg.per_trade_size_max);
        }
        available_cash * Decimal::from_f64_retain(frac).unwrap_or(Decimal::ZERO)
    }

    /// Ordered entry gates: kill switch, day/week pause, slippage, daily
    /// trade cap, existing position for the pair, max concurrent
    /// positions, then sizing. Mirrors `evaluate_entry` in risk.py.
    pub fn evaluate_entry(
        &self,
        pair: &str,
        confidence: f64,
        portfolio: &Portfolio,
        state: &RiskState,
        slippage_pct: f64,
        now: DateTime<Utc>,
    ) -> RiskDecision {
        let today = now.date_naive();
        let deny = |reason: String| RiskDecision {
            allowed: false,
            size_quote: Decimal::ZERO,
            reason,
        };

        if state.kill_switch_active {
            return deny(format!("kill switch active: {}", state.kill_switch_reason));
        }
        if let Some(paused) = state.day_paused_until {
            if today < paused {
                return deny(format!("day paused until {paused}"));
            }
        }
        if let Some(paused) = state.week_paused_until {
            if today < paused {
                return deny(format!("week paused until {paused}"));
            }
        }
        if slippage_pct > self.cfg.max_slippage_pct {
            return deny(format!(
                "slippage {:.4} > max {}",
                slippage_pct, self.cfg.max_slippage_pct
            ));
        }
        if *state.trades_per_day.get(&today).unwrap_or(&0) >= self.cfg.max_trades_per_day as i64 {
            return deny("max trades per day reached".to_string());
        }
        if portfolio.position_for(pair).is_some() {
            return deny(format!("position already open for {pair}"));
        }
        if portfolio.open_positions().len() >= self.cfg.max_concurrent_positions as usize {
            return deny("max concurrent positions reached".to_string());
        }

        let size = self.size_for(confidence, portfolio.cash);
        if size <= Decimal::ZERO {
            return deny("sized to zero (no cash)".to_string());
        }

        RiskDecision {
            allowed: true,
            size_quote: size,
            reason: "ok".to_string(),
        }
    }

    /// Updates day/week start-equity bookkeeping, the equity high, and
    /// trips the drawdown/daily/weekly circuit breakers. Mirrors
    /// `update_state` in risk.py.
    pub fn update_state(
        &self,
        portfolio: &mut Portfolio,
        state: &mut RiskState,
        current_equity: Money,
        now: DateTime<Utc>,
    ) {
        let today = now.date_naive();
        state
            .daily_start_equity
            .entry(today)
            .or_insert(current_equity);
        let wk_start = week_start(today);
        state
            .weekly_start_equity
            .entry(wk_start)
            .or_insert(current_equity);

        portfolio.update_equity_high(current_equity);

        let dd = portfolio.drawdown_pct(current_equity);
        if dd >= self.cfg.drawdown_circuit_pct {
            state.kill_switch_active = true;
            state.kill_switch_reason = format!(
                "drawdown {:.2}% >= {:.2}%",
                dd * 100.0,
                self.cfg.drawdown_circuit_pct * 100.0
            );
            return;
        }

        let day_start = *state.daily_start_equity.get(&today).unwrap();
        if day_start > Decimal::ZERO {
            let daily_pnl_pct = money_pct_change(day_start, current_equity);
            if daily_pnl_pct <= -self.cfg.daily_loss_limit_pct {
                state.day_paused_until = Some(today + Duration::days(1));
            }
        }

        let week_start_eq = *state.weekly_start_equity.get(&wk_start).unwrap();
        if week_start_eq > Decimal::ZERO {
            let weekly_pnl_pct = money_pct_change(week_start_eq, current_equity);
            if weekly_pnl_pct <= -self.cfg.weekly_loss_limit_pct {
                state.week_paused_until = Some(wk_start + Duration::days(7));
            }
        }
    }

    pub fn record_trade(&self, state: &mut RiskState, now: DateTime<Utc>) {
        let today = now.date_naive();
        *state.trades_per_day.entry(today).or_insert(0) += 1;
    }

    pub fn check_per_trade_kill(&self, unrealized_loss_pct: f64) -> KillAction {
        if unrealized_loss_pct >= self.cfg.per_trade_kill_pct {
            KillAction::Kill
        } else {
            KillAction::Ok
        }
    }
}

/// `(current - start) / start` as a ratio, matching the plain-float
/// division in risk.py's daily/weekly pnl checks.
fn money_pct_change(start: Money, current: Money) -> f64 {
    let ratio = (current - start) / start;
    ratio.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_common::Mode;
    use tradebot_storage::Side;

    fn dec(s: &str) -> Money {
        s.parse().unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dt(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        date(y, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc()
    }

    #[test]
    fn size_scales_with_confidence() {
        let rm = RiskManager::new(RiskConfig::default());
        let p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let low = rm.size_for(0.6, p.cash);
        let high = rm.size_for(1.0, p.cash);
        assert!(low < high);
        assert!((low.to_f64().unwrap() - 30.0).abs() < 1e-6);
        assert!((high.to_f64().unwrap() - 50.0).abs() < 1e-6);
    }

    #[test]
    fn evaluate_buy_accepts_basic() {
        let rm = RiskManager::new(RiskConfig::default());
        let state = RiskState::default();
        let p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let decision = rm.evaluate_entry("SOL/USDC", 0.8, &p, &state, 0.005, dt(2026, 5, 3, 0));
        assert!(decision.allowed);
        assert!(decision.size_quote > Decimal::ZERO);
    }

    #[test]
    fn evaluate_buy_rejects_high_slippage() {
        let rm = RiskManager::new(RiskConfig::default());
        let p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let decision = rm.evaluate_entry("X", 0.9, &p, &RiskState::default(), 0.02, Utc::now());
        assert!(!decision.allowed);
        assert!(decision.reason.to_lowercase().contains("slippage"));
    }

    #[test]
    fn evaluate_buy_rejects_when_daily_trade_cap_hit() {
        let cfg = RiskConfig {
            max_trades_per_day: 3,
            ..RiskConfig::default()
        };
        let rm = RiskManager::new(cfg);
        let p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let today = date(2026, 5, 3);
        let mut state = RiskState::default();
        state.trades_per_day.insert(today, 3);
        let decision = rm.evaluate_entry("X", 0.9, &p, &state, 0.005, dt(2026, 5, 3, 12));
        assert!(!decision.allowed);
        assert!(decision.reason.to_lowercase().contains("trades per day"));
    }

    #[test]
    fn evaluate_buy_rejects_when_max_concurrent_positions() {
        let cfg = RiskConfig {
            max_concurrent_positions: 1,
            ..RiskConfig::default()
        };
        let rm = RiskManager::new(cfg);
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill("A/USDC", Side::Buy, dec("1"), dec("10"), Decimal::ZERO)
            .unwrap();
        let decision =
            rm.evaluate_entry("B/USDC", 0.9, &p, &RiskState::default(), 0.005, Utc::now());
        assert!(!decision.allowed);
        assert!(decision.reason.to_lowercase().contains("concurrent"));
    }

    #[test]
    fn evaluate_buy_rejects_existing_position_for_same_pair() {
        let rm = RiskManager::new(RiskConfig::default());
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.apply_fill("A/USDC", Side::Buy, dec("1"), dec("10"), Decimal::ZERO)
            .unwrap();
        let decision =
            rm.evaluate_entry("A/USDC", 0.9, &p, &RiskState::default(), 0.005, Utc::now());
        assert!(!decision.allowed);
    }

    #[test]
    fn drawdown_circuit_trips_kill_switch() {
        let cfg = RiskConfig {
            drawdown_circuit_pct: 0.15,
            ..RiskConfig::default()
        };
        let rm = RiskManager::new(cfg);
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        p.equity_high = dec("100.0");
        let mut state = RiskState::default();
        rm.update_state(&mut p, &mut state, dec("80.0"), dt(2026, 5, 3, 0));
        assert!(state.kill_switch_active);
        assert!(state.kill_switch_reason.to_lowercase().contains("drawdown"));
    }

    #[test]
    fn daily_loss_limit_pauses_until_next_day() {
        let cfg = RiskConfig {
            daily_loss_limit_pct: 0.08,
            ..RiskConfig::default()
        };
        let rm = RiskManager::new(cfg);
        let mut p = Portfolio::new(Mode::Demo, dec("100.0"), Decimal::ZERO);
        let mut state = RiskState::default();
        state
            .daily_start_equity
            .insert(date(2026, 5, 3), dec("100.0"));
        rm.update_state(&mut p, &mut state, dec("91.0"), dt(2026, 5, 3, 12));
        assert_eq!(state.day_paused_until, Some(date(2026, 5, 4)));
    }

    #[test]
    fn record_trade_increments_counter() {
        let rm = RiskManager::new(RiskConfig::default());
        let mut state = RiskState::default();
        rm.record_trade(&mut state, dt(2026, 5, 3, 0));
        assert_eq!(state.trades_per_day.get(&date(2026, 5, 3)), Some(&1));
    }

    #[test]
    fn risk_state_record_roundtrip() {
        let mut state = RiskState::default();
        state.trades_per_day.insert(date(2026, 5, 3), 2);
        state
            .daily_start_equity
            .insert(date(2026, 5, 3), dec("100.0"));
        state
            .weekly_start_equity
            .insert(date(2026, 5, 4), dec("100.0"));
        state.day_paused_until = Some(date(2026, 5, 4));
        state.week_paused_until = None;
        state.kill_switch_active = true;
        state.kill_switch_reason = "drawdown 20.00% >= 15.00%".to_string();

        let record = state.to_record();
        let back = RiskState::from_record(&record);

        assert_eq!(back.trades_per_day, state.trades_per_day);
        assert_eq!(back.daily_start_equity, state.daily_start_equity);
        assert_eq!(back.weekly_start_equity, state.weekly_start_equity);
        assert_eq!(back.day_paused_until, state.day_paused_until);
        assert_eq!(back.week_paused_until, state.week_paused_until);
        assert_eq!(back.kill_switch_active, state.kill_switch_active);
        assert_eq!(back.kill_switch_reason, state.kill_switch_reason);
    }

    #[test]
    fn per_trade_kill_returns_close_action() {
        let cfg = RiskConfig {
            per_trade_kill_pct: 0.03,
            ..RiskConfig::default()
        };
        let rm = RiskManager::new(cfg);
        assert_eq!(rm.check_per_trade_kill(0.04), KillAction::Kill);
        assert_eq!(rm.check_per_trade_kill(0.02), KillAction::Ok);
    }
}
