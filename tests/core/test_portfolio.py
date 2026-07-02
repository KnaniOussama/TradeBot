import pytest

from tradebot.core.portfolio import Portfolio, PortfolioError


def test_new_portfolio_all_cash():
    p = Portfolio(mode="demo", starting_cash=50.0)
    assert p.cash == 50.0
    assert p.equity({}) == 50.0
    assert p.open_positions() == []


def test_apply_buy_reduces_cash_creates_position():
    p = Portfolio(mode="demo", starting_cash=50.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=15.0, fee_quote=0.015)
    assert p.cash == pytest.approx(50.0 - 15.0 - 0.015)
    pos = p.position_for("SOL/USDC")
    assert pos is not None
    assert pos.base_amount == pytest.approx(0.1)
    assert pos.avg_entry_price == pytest.approx(150.0)


def test_apply_buy_then_buy_averages_entry():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=20.0, fee_quote=0.0)
    pos = p.position_for("SOL/USDC")
    assert pos.base_amount == pytest.approx(0.2)
    assert pos.avg_entry_price == pytest.approx(150.0)


def test_apply_sell_realizes_pnl_and_closes():
    p = Portfolio(mode="demo", starting_cash=50.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    realized = p.apply_fill(
        pair="SOL/USDC", side="sell", base_amount=0.1, quote_amount=15.0, fee_quote=0.0
    )
    assert realized == pytest.approx(5.0)
    assert p.position_for("SOL/USDC") is None
    assert p.cash == pytest.approx(50.0 - 10.0 + 15.0)
    assert p.realized_pnl_total == pytest.approx(5.0)


def test_partial_sell_keeps_position_and_realizes_pnl():
    p = Portfolio(mode="demo", starting_cash=50.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.2, quote_amount=20.0, fee_quote=0.0)
    realized = p.apply_fill(
        pair="SOL/USDC", side="sell", base_amount=0.1, quote_amount=15.0, fee_quote=0.0
    )
    assert realized == pytest.approx(5.0)
    pos = p.position_for("SOL/USDC")
    assert pos is not None
    assert pos.base_amount == pytest.approx(0.1)
    assert pos.avg_entry_price == pytest.approx(100.0)


def test_sell_without_position_raises():
    p = Portfolio(mode="demo", starting_cash=50.0)
    with pytest.raises(PortfolioError):
        p.apply_fill(
            pair="SOL/USDC", side="sell", base_amount=0.1, quote_amount=10.0, fee_quote=0.0
        )


def test_oversell_raises():
    p = Portfolio(mode="demo", starting_cash=50.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    with pytest.raises(PortfolioError):
        p.apply_fill(
            pair="SOL/USDC", side="sell", base_amount=0.5, quote_amount=50.0, fee_quote=0.0
        )


def test_equity_uses_marks():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    # cash 90, position 0.1 SOL marked at 200 => 20 USD
    eq = p.equity({"SOL/USDC": 200.0})
    assert eq == pytest.approx(110.0)


def test_equity_marks_missing_uses_avg_entry():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.apply_fill(pair="SOL/USDC", side="buy", base_amount=0.1, quote_amount=10.0, fee_quote=0.0)
    eq = p.equity({})  # no marks; falls back to avg entry
    assert eq == pytest.approx(100.0)


def test_drawdown_tracked_from_high_water_mark():
    p = Portfolio(mode="demo", starting_cash=100.0)
    p.update_equity_high(120.0)
    dd = p.drawdown_pct(current_equity=108.0)
    assert dd == pytest.approx(0.10)
    p.update_equity_high(108.0)  # noop, since 108 < 120
    assert p.equity_high == 120.0


def test_portfolio_seed_from_state():
    from tradebot.core.portfolio import Portfolio
    from tradebot.storage.repo import PortfolioState, PositionRecord

    state = PortfolioState(
        mode="demo",
        cash=42.0,
        realized_pnl_total=8.0,
        equity_high=55.0,
        positions=[
            PositionRecord(
                pair="SOL/USDC", base_amount=0.1, avg_entry_price=150.0, fees_paid_quote=0.01
            ),
        ],
    )
    p = Portfolio.from_state(state)
    assert p.mode == "demo"
    assert p.cash == 42.0
    assert p.realized_pnl_total == 8.0
    assert p.equity_high == 55.0
    pos = p.position_for("SOL/USDC")
    assert pos is not None
    assert pos.base_amount == 0.1
    assert pos.avg_entry_price == 150.0


# --- SOL gas tracking ---


def test_sol_balance_seeded_from_constructor():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.05)
    assert p.sol_balance == 0.05
    assert p.sol_gas_paid_total == 0.0


def test_charge_gas_deducts_and_tracks_total():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.01)
    p.charge_gas(0.000005)
    p.charge_gas(0.000005)
    assert p.sol_balance == pytest.approx(0.01 - 0.00001)
    assert p.sol_gas_paid_total == pytest.approx(0.00001)


def test_charge_gas_zero_is_noop():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.01)
    p.charge_gas(0.0)
    assert p.sol_balance == 0.01


def test_charge_gas_insufficient_raises():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.000001)
    with pytest.raises(PortfolioError, match="insufficient SOL"):
        p.charge_gas(0.001)


def test_charge_gas_negative_raises():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.01)
    with pytest.raises(PortfolioError, match="negative"):
        p.charge_gas(-0.001)


def test_equity_includes_sol_balance_at_mark():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.05)
    eq = p.equity({"SOL/USDC": 140.0})
    assert eq == pytest.approx(50.0 + 0.05 * 140.0)


def test_equity_uses_fallback_sol_price_when_mark_missing():
    p = Portfolio(mode="demo", starting_cash=50.0, starting_sol_balance=0.05)
    # No SOL/USDC mark — fallback constant kicks in (140.0 in module).
    eq = p.equity({})
    assert eq > 50.0  # SOL contribution non-zero


def test_portfolio_state_roundtrip_preserves_sol():
    from tradebot.storage.repo import PortfolioState

    state = PortfolioState(
        mode="demo",
        cash=42.0,
        realized_pnl_total=0.0,
        equity_high=50.0,
        sol_balance=0.0473,
        sol_gas_paid_total=0.0027,
        positions=[],
    )
    p = Portfolio.from_state(state)
    assert p.sol_balance == pytest.approx(0.0473)
    assert p.sol_gas_paid_total == pytest.approx(0.0027)
