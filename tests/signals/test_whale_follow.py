from datetime import UTC, datetime, timedelta

import pytest

from tradebot.core.whale_activity import WhaleActivityTracker
from tradebot.data.helius import HeliusClient, WhaleSwap
from tradebot.signals.base import MarketContext
from tradebot.signals.whale_follow import WhaleFollowSignal

SOL = "So11111111111111111111111111111111111111112"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
WALLET_A = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
NOW = datetime(2026, 5, 3, 12, 0, tzinfo=UTC)


def _swap(*, in_mint: str, out_mint: str, age_seconds: int) -> WhaleSwap:
    return WhaleSwap(
        wallet=WALLET_A,
        timestamp=NOW - timedelta(seconds=age_seconds),
        signature="sig",
        in_mint=in_mint,
        out_mint=out_mint,
        in_amount_raw=1_000_000,
        out_amount_raw=1_000_000,
    )


def _make_signal(wallets=None, **kwargs):
    helius = HeliusClient(api_key="test")
    tracker = WhaleActivityTracker(
        helius=helius, wallets=wallets if wallets is not None else [WALLET_A]
    )
    defaults = dict(
        pair="SOL/USDC",
        base_mint=SOL,
        quote_mint=USDC,
        tracker=tracker,
        lookback_seconds=1800,
        decay_half_life_s=600.0,
    )
    defaults.update(kwargs)
    return WhaleFollowSignal(**defaults)


def test_score_buy_swap_is_positive():
    sig = _make_signal()
    swap = _swap(in_mint=USDC, out_mint=SOL, age_seconds=0)
    score = sig._score_from_swaps([swap], now=NOW)
    assert score == pytest.approx(1.0)


def test_score_sell_swap_is_negative():
    sig = _make_signal()
    swap = _swap(in_mint=SOL, out_mint=USDC, age_seconds=0)
    score = sig._score_from_swaps([swap], now=NOW)
    assert score == pytest.approx(-1.0)


def test_score_balanced_buy_and_sell_returns_zero():
    sig = _make_signal()
    swaps = [
        _swap(in_mint=USDC, out_mint=SOL, age_seconds=0),
        _swap(in_mint=SOL, out_mint=USDC, age_seconds=0),
    ]
    assert sig._score_from_swaps(swaps, now=NOW) == pytest.approx(0.0)


def test_unrelated_swap_does_not_contribute():
    sig = _make_signal()
    other_mint = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN"
    swaps = [_swap(in_mint=USDC, out_mint=other_mint, age_seconds=0)]
    assert sig._score_from_swaps(swaps, now=NOW) == 0.0


def test_old_swap_outside_lookback_ignored():
    sig = _make_signal(lookback_seconds=600)
    # 1 hour old > 10 minute lookback
    old = _swap(in_mint=USDC, out_mint=SOL, age_seconds=3600)
    assert sig._score_from_swaps([old], now=NOW) == 0.0


def test_decay_makes_old_swap_count_less():
    sig = _make_signal(lookback_seconds=3600, decay_half_life_s=300.0)
    fresh_buy = _swap(in_mint=USDC, out_mint=SOL, age_seconds=0)
    old_sell = _swap(in_mint=SOL, out_mint=USDC, age_seconds=900)  # 3 half-lives → ~12.5%
    score = sig._score_from_swaps([fresh_buy, old_sell], now=NOW)
    # Fresh buy dominates; net positive but not 1.0.
    assert 0.5 < score < 1.0


def test_no_swaps_returns_zero():
    sig = _make_signal()
    assert sig._score_from_swaps([], now=NOW) == 0.0


@pytest.mark.asyncio
async def test_score_protocol_returns_zero_for_other_pair():
    sig = _make_signal()
    ctx = MarketContext(pair="BONK/USDC", now=NOW, ohlcv={})
    out = await sig.score(ctx)
    assert out.score == 0.0
    assert out.signal == "whale_follow"
