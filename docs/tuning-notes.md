# Regime Filter & Kelly Sizing: Design Notes

Background on two of the more opinionated risk knobs: the **regime filter** and
**Kelly position sizing**. Both default to on.

## Configurations worth comparing

If you want to measure the effect of these features on your own data, these are
the four combinations to backtest:

| regime_filter_enabled | regime_block_chop | use_kelly_sizing | Description |
|-----------------------|-------------------|------------------|-------------|
| false | false | false | Legacy linear sizing, no regime gate |
| true  | true  | false | Regime blocks chop entries only |
| false | false | true  | Kelly sizing, no regime gate |
| true  | true  | true  | Both on (the shipped defaults) |

> No benchmark numbers are published here: results depend entirely on the pairs,
> period, and market conditions you test against. Run your own backtests (see the
> **Backtest** tab in the dashboard) before trusting any configuration with real funds.

## Shipped defaults

```python
regime_filter_enabled = True   # block entries during chop
regime_block_chop     = True   # only block chop (not neutral/trending)
use_kelly_sizing      = True   # falls back to linear when history is sparse
```

Why these are safe starting points:

- `regime_filter_enabled` + `regime_block_chop` block entries only in **confirmed
  chop** (ADX ≤ 20). Neutral and trending markets trade normally. The rationale:
  chop tends to whipsaw momentum-style entries into small losses.
- `use_kelly_sizing` automatically falls back to the linear
  (`per_trade_size_min`..`per_trade_size_max`) sizing until at least 20 round
  trips have been recorded per pair, so there is no behaviour cliff on a fresh
  install.

**To disable for comparison:** set `regime_filter_enabled` and `use_kelly_sizing`
to `false` under the `risk` block in `tradebot.config.json` (or via the dashboard
Settings tab).

## Known limitations

- The backtest runner does **not** compute regimes per-bar (see
  `tests/backtest/test_regime_backtest.py`). Regime-filtered results in a backtest
  will not match live behaviour until per-bar regime classification is wired in.
- Kelly's `round_trip_returns` treats partial sells as full closes, an acceptable
  approximation, but it can overstate returns if the take-profit ladder fires often.
- ADX warmup needs `max(adx_period, ema_slow) + 5` bars. On short histories the
  regime defaults to `neutral` (entries allowed), so sparse OHLCV effectively
  disables the chop gate.
