from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import pytest

from tradebot.core.risk import RiskState
from tradebot.storage.repo import (
    JsonStorage,
    OHLCVCandle,
    PortfolioState,
    PositionRecord,
    Trade,
)


@pytest.fixture
def storage(tmp_path: Path) -> JsonStorage:
    return JsonStorage(root=tmp_path)


def _trade(side: str = "buy", pair: str = "SOL/USDC") -> Trade:
    return Trade(
        mode="demo",
        pair=pair,
        side=side,
        base_amount=0.1,
        quote_amount=10.0,
        price=100.0,
        fee_quote=0.01,
        slippage_pct=0.001,
        tx_signature=None,
        opened_at=datetime(2026, 5, 3, 12, tzinfo=UTC),
        confidence=0.7,
        notes=None,
    )


def test_append_and_list_trades(storage):
    storage.append_trade(_trade())
    storage.append_trade(_trade(side="sell"))
    trades = storage.list_trades(mode="demo", limit=10)
    assert len(trades) == 2
    # Newest first
    assert trades[0].side == "sell"


def test_list_trades_filters_by_mode(storage):
    storage.append_trade(_trade())
    real = _trade()
    real.mode = "real"
    storage.append_trade(real)
    assert len(storage.list_trades(mode="demo", limit=10)) == 1
    assert len(storage.list_trades(mode="real", limit=10)) == 1


def test_save_and_load_portfolio_state(storage):
    state = PortfolioState(
        mode="demo",
        cash=42.5,
        realized_pnl_total=2.5,
        equity_high=50.0,
        positions=[
            PositionRecord(
                pair="SOL/USDC", base_amount=0.1, avg_entry_price=100.0, fees_paid_quote=0.01
            ),
        ],
    )
    storage.save_portfolio_state(state)
    loaded = storage.load_portfolio_state(mode="demo")
    assert loaded is not None
    assert loaded.cash == 42.5
    assert len(loaded.positions) == 1
    assert loaded.positions[0].pair == "SOL/USDC"


def test_load_portfolio_state_missing_returns_none(storage):
    assert storage.load_portfolio_state(mode="demo") is None


def test_append_and_list_equity_snapshots(storage):
    base = datetime(2026, 5, 3, 12, tzinfo=UTC)
    for i in range(5):
        storage.append_equity_snapshot(
            mode="demo",
            snapshot_at=base + timedelta(minutes=i),
            equity=50.0 + i,
            cash=50.0 + i,
            positions_value=0.0,
        )
    out = storage.list_equity_snapshots(mode="demo", limit=10)
    assert len(out) == 5
    # Oldest first (chronological)
    assert out[0]["equity"] == 50.0
    assert out[-1]["equity"] == 54.0


def test_append_and_load_ohlcv(storage):
    base = datetime(2026, 5, 3, 12, 0, tzinfo=UTC)
    for i in range(3):
        storage.upsert_ohlcv(
            OHLCVCandle(
                pair="SOL/USDC",
                timeframe="1m",
                bucket_start=base + timedelta(minutes=i),
                open=100.0,
                high=101.0,
                low=99.0,
                close=100.5,
                volume_quote=1000.0,
            )
        )
    df = storage.load_ohlcv(pair="SOL/USDC", timeframe="1m", limit=10)
    assert len(df) == 3
    assert df["close"].iloc[-1] == 100.5


def test_upsert_ohlcv_replaces_same_bucket(storage):
    base = datetime(2026, 5, 3, 12, 0, tzinfo=UTC)
    storage.upsert_ohlcv(OHLCVCandle("SOL/USDC", "1m", base, 100.0, 101.0, 99.0, 100.5, 1000.0))
    storage.upsert_ohlcv(OHLCVCandle("SOL/USDC", "1m", base, 100.0, 105.0, 99.0, 104.0, 2000.0))
    df = storage.load_ohlcv(pair="SOL/USDC", timeframe="1m", limit=10)
    assert len(df) == 1
    assert df["close"].iloc[-1] == 104.0
    assert df["high"].iloc[-1] == 105.0


def test_filename_safe_for_pair_with_slash(storage):
    base = datetime(2026, 5, 3, 12, tzinfo=UTC)
    storage.upsert_ohlcv(OHLCVCandle("SOL/USDC", "1m", base, 1, 1, 1, 1, 0))
    files = list((storage._root / "ohlcv").glob("*.json"))  # noqa: SLF001
    assert any("SOL--USDC" in f.name for f in files)


def test_save_and_load_risk_state(storage: JsonStorage):
    state = RiskState(
        trades_per_day={date(2026, 5, 3): 4},
        daily_start_equity={date(2026, 5, 3): 50.0},
        weekly_start_equity={date(2026, 4, 27): 50.0},
        day_paused_until=date(2026, 5, 4),
        kill_switch_active=True,
        kill_switch_reason="drawdown 16% >= 15%",
    )
    storage.save_risk_state(mode="demo", state=state)
    loaded = storage.load_risk_state(mode="demo")
    assert loaded is not None
    assert loaded.trades_per_day == {date(2026, 5, 3): 4}
    assert loaded.day_paused_until == date(2026, 5, 4)
    assert loaded.kill_switch_active is True
    assert loaded.kill_switch_reason.startswith("drawdown")


def test_load_risk_state_missing_returns_none(storage: JsonStorage):
    assert storage.load_risk_state(mode="demo") is None


def test_save_and_load_mark_history(storage: JsonStorage):
    base = datetime(2026, 5, 3, 12, 0, tzinfo=UTC)
    history = {
        "SOL/USDC": [(base, 150.0), (base.replace(minute=1), 151.0)],
        "JUP/USDC": [(base, 0.85)],
    }
    storage.save_mark_history(mode="demo", history=history)
    loaded = storage.load_mark_history(mode="demo")
    assert "SOL/USDC" in loaded
    assert len(loaded["SOL/USDC"]) == 2
    assert loaded["SOL/USDC"][0][1] == 150.0
    assert loaded["JUP/USDC"][0][0] == base


def test_load_mark_history_missing_returns_empty(storage: JsonStorage):
    assert storage.load_mark_history(mode="demo") == {}


def test_round_trip_returns_basic(storage: JsonStorage):
    """A buy then sell should produce one round-trip return entry."""
    buy = _trade(side="buy")  # quote_amount=10.0, fee_quote=0.01 → cost=10.01
    sell = Trade(
        mode="demo",
        pair="SOL/USDC",
        side="sell",
        base_amount=0.1,
        quote_amount=10.5,
        price=105.0,
        fee_quote=0.01,
        slippage_pct=0.001,
        tx_signature=None,
        opened_at=buy.opened_at,
        confidence=0.7,
        notes=None,
    )
    storage.append_trade(buy)
    storage.append_trade(sell)
    returns = storage.round_trip_returns(mode="demo", pair="SOL/USDC")
    assert len(returns) == 1
    # sell_quote_net = 10.5 - 0.01 = 10.49; cost = 10.01; ret = (10.49-10.01)/10.01
    expected = (10.49 - 10.01) / 10.01
    assert returns[0] == pytest.approx(expected, abs=1e-6)


def test_round_trip_returns_filters_by_pair(storage: JsonStorage):
    """round_trip_returns respects the pair filter."""
    buy_sol = _trade(side="buy", pair="SOL/USDC")
    sell_sol = _trade(side="sell", pair="SOL/USDC")
    buy_jup = _trade(side="buy", pair="JUP/USDC")
    for t in [buy_sol, sell_sol, buy_jup]:
        storage.append_trade(t)
    returns_sol = storage.round_trip_returns(mode="demo", pair="SOL/USDC")
    returns_jup = storage.round_trip_returns(mode="demo", pair="JUP/USDC")
    assert len(returns_sol) == 1
    assert len(returns_jup) == 0  # no sell for JUP


def test_round_trip_returns_empty_when_no_trades(storage: JsonStorage):
    returns = storage.round_trip_returns(mode="demo")
    assert returns == []
