from __future__ import annotations

from datetime import UTC, datetime

from tradebot.backtest.metrics import compute_metrics


def _ts(i: int) -> datetime:
    return datetime(2026, 5, 1, 0, i % 60, i // 60, tzinfo=UTC)


def test_empty_equity_curve_returns_defaults():
    result = compute_metrics(equity_curve=[], trades=[], starting_cash=100.0)
    assert result["final_equity"] == 100.0
    assert result["n_wins"] == 0
    assert result["n_losses"] == 0
    assert result["max_drawdown_pct"] == 0.0
    assert result["sharpe"] == 0.0
    assert result["total_return_pct"] == 0.0


def test_max_drawdown_computed_correctly():
    # equity goes up then down
    curve = [
        (_ts(0), 100.0),
        (_ts(1), 120.0),
        (_ts(2), 90.0),  # drawdown from peak 120 -> 90 = 25%
        (_ts(3), 100.0),
    ]
    result = compute_metrics(equity_curve=curve, trades=[], starting_cash=100.0, bar_seconds=60)
    assert abs(result["max_drawdown_pct"] - 0.25) < 1e-6


def test_total_return_positive():
    curve = [(_ts(0), 100.0), (_ts(1), 110.0)]
    result = compute_metrics(equity_curve=curve, trades=[], starting_cash=100.0)
    assert abs(result["total_return_pct"] - 0.1) < 1e-9
    assert result["final_equity"] == 110.0


def test_win_loss_counted_from_round_trips():
    # Simple trade list: buy at 100, sell at 120 (win), buy at 130, sell at 120 (loss)
    trades = [
        {"side": "buy", "price": 100.0},
        {"side": "sell", "price": 120.0},
        {"side": "buy", "price": 130.0},
        {"side": "sell", "price": 120.0},
    ]
    curve = [(_ts(0), 100.0), (_ts(1), 110.0)]
    result = compute_metrics(equity_curve=curve, trades=trades, starting_cash=100.0)
    assert result["n_wins"] == 1
    assert result["n_losses"] == 1


def test_sharpe_nonzero_with_returns():
    # Flat equity → std=0 → sharpe=0
    flat = [(_ts(i), 100.0) for i in range(10)]
    result_flat = compute_metrics(equity_curve=flat, trades=[], starting_cash=100.0)
    assert result_flat["sharpe"] == 0.0

    # Monotonically increasing equity → positive sharpe
    growing = [(_ts(i), 100.0 + i * 0.5) for i in range(50)]
    result_growing = compute_metrics(equity_curve=growing, trades=[], starting_cash=100.0)
    assert result_growing["sharpe"] > 0.0
