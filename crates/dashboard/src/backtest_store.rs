//! Port of the `BacktestStore` class in `tradebot/dashboard/backtest_api.py`:
//! an in-memory ring buffer of the last N backtest runs, used by the
//! History panel.

use std::collections::VecDeque;

use serde_json::{json, Value};
use tradebot_backtest::BacktestResult;

const DEFAULT_MAX_RESULTS: usize = 20;

pub struct BacktestStore {
    results: VecDeque<BacktestResult>,
    max_results: usize,
}

impl BacktestStore {
    pub fn new(max_results: usize) -> Self {
        Self {
            results: VecDeque::new(),
            max_results: max_results.max(1),
        }
    }

    pub fn add(&mut self, result: BacktestResult) {
        if self.results.len() >= self.max_results {
            self.results.pop_front();
        }
        self.results.push_back(result);
    }

    /// Newest-first summary list. Mirrors `list_summary()` in
    /// backtest_api.py.
    pub fn list_summary(&self) -> Vec<Value> {
        self.results
            .iter()
            .rev()
            .map(|r| {
                json!({
                    "id": r.id,
                    "pair": r.pair,
                    "completed_at": r.completed_at,
                    "total_return_pct": r.total_return_pct,
                    "sharpe": r.sharpe,
                    "n_trades": r.n_trades,
                    "max_drawdown_pct": r.max_drawdown_pct,
                })
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&BacktestResult> {
        self.results.iter().find(|r| r.id == id)
    }
}

impl Default for BacktestStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RESULTS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradebot_backtest::BacktestParams;

    fn result(id: &str) -> BacktestResult {
        BacktestResult {
            id: id.to_string(),
            pair: "SOL/USDC".to_string(),
            starting_cash: Default::default(),
            final_equity: 100.0,
            total_return_pct: 0.1,
            realized_pnl: Default::default(),
            n_trades: 2,
            n_wins: 1,
            n_losses: 1,
            max_drawdown_pct: 0.05,
            sharpe: 1.0,
            bars_processed: 70,
            equity_curve: Vec::new(),
            trades: Vec::new(),
            params: BacktestParams::new("SOL/USDC"),
            completed_at: "2026-05-03T12:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn store_starts_empty() {
        let store = BacktestStore::default();
        assert!(store.list_summary().is_empty());
    }

    #[test]
    fn add_then_list_is_newest_first() {
        let mut store = BacktestStore::default();
        store.add(result("a"));
        store.add(result("b"));
        let list = store.list_summary();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "b");
        assert_eq!(list[1]["id"], "a");
    }

    #[test]
    fn get_by_id_finds_result() {
        let mut store = BacktestStore::default();
        store.add(result("a"));
        assert!(store.get("a").is_some());
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn drops_oldest_beyond_max() {
        let mut store = BacktestStore::new(2);
        store.add(result("a"));
        store.add(result("b"));
        store.add(result("c"));
        assert!(store.get("a").is_none());
        assert!(store.get("b").is_some());
        assert!(store.get("c").is_some());
    }
}
