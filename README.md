# TradeBot

A small autonomous trading bot for Solana DEXs (Raydium, Orca, etc. via Jupiter).
Runs on your computer, decides what to trade based on signals, and shows everything
in a live web dashboard at `http://127.0.0.1:8765`.

Two modes:

- **Demo:** pretend money. Uses real prices but never sends a transaction. Safe to leave running for days while you watch.
- **Real:** your money, on-chain. Trades through a dedicated bot wallet you fund from your Phantom wallet. Your Phantom seed is **never** touched.

---

## Table of contents

1. [How the bot works (in plain English)](#how-the-bot-works)
2. [How decisions are made](#how-decisions-are-made)
3. [Install](#install)
4. [Demo mode: try it in 2 minutes](#demo-mode)
5. [The dashboard](#the-dashboard)
6. [Real mode: connecting your Phantom wallet](#real-mode)
7. [Configuration cheat-sheet](#configuration)
8. [Safety + things that can go wrong](#safety)
9. [FAQ](#faq)

---

## How the bot works

Every 30 seconds (configurable), the bot does this:

```
1. Look at each coin in your watchlist.
2. Fetch the current price + recent price history.
3. Run "signals" on each coin (see next section) → produces a score from -1 to +1.
4. If a score is high enough AND risk rules allow → BUY.
5. If a coin you already hold drops too much OR signal flips → SELL.
6. Update the dashboard.
7. Wait 30s, repeat.
```

That's the whole loop. Nothing magic.

---

## How decisions are made

The bot doesn't trade on a single indicator. It combines several, weighted, into one **composite score** between **-1 (strongly bearish)** and **+1 (strongly bullish)**.

### The signal layer

| Signal | What it looks at | Source |
|---|---|---|
| **TA** (Technical Analysis) | RSI, MACD, EMA crossovers, Bollinger Bands, ATR-normalized momentum | Recent price history (OHLCV) |
| **Microstructure** | Order book depth imbalance, VWAP deviation, volume z-score | Live Jupiter quotes |
| **On-chain** | Whale transfer flow, DEX volume surges | Solana on-chain data (via Helius) |

Each signal outputs a number from -1 to +1. They're combined using the **weights** in your config:

```json
"weights": {
    "timeframes": {"5s": 0.10, "1m": 0.20, "15m": 0.30, "1h": 0.40},
    "signals":    {"ta": 0.40, "microstructure": 0.30, "onchain": 0.30}
}
```

Higher-timeframe signals dominate (a 1-hour trend matters more than a 5-second blip), and TA gets the biggest signal weight by default.

### The decision rule

```
ENTER LONG (buy) if:
    composite_score > entry_threshold (default: 0.6)
    AND no existing position in this pair
    AND we're under max_concurrent_positions (default: 3)
    AND current slippage < max_slippage_pct (default: 1%)
    AND no kill-switch active
    AND not yet at max_trades_per_day (default: 10)

EXIT (sell) if any of:
    Price dropped > 3% from entry         (per-trade kill switch)
    Price dropped > 2% from peak          (trailing stop)
    Composite score flipped < -0.3 while in profit  (signal flip)
```

### Risk caps (the safety net)

| Rule | Default | What happens |
|---|---|---|
| Per-trade kill | -3% | Position closed immediately + bot pauses until manual restart |
| Trailing stop | -2% from peak | Position closed |
| Daily loss limit | -8% | Bot stops trading until next UTC day |
| Weekly loss limit | -10% | Bot stops, requires restart |
| Drawdown circuit | -15% from all-time-high equity | Bot stops, requires restart |
| Max slippage | 1% per trade | Trade refused |
| Max trades/day | 10 | Trade refused after limit |

### Position sizing

Each trade size scales with signal confidence between **30% and 50%** of available cash:

- A weak signal (composite ≈ 0.6) → ~30% of cash
- A strong signal (composite ≈ 1.0) → ~50% of cash

You can change all of this in the **Settings** tab of the dashboard.

---

## Install

You'll need **Python 3.12+**. Then in PowerShell from the project folder:

```powershell
python -m venv .venv
.venv\Scripts\activate
pip install -e .[dev]
```

Sanity check:

```powershell
pytest
```

You should see `286 passed`.

---

## Demo mode

Two commands and you're running:

```powershell
tradebot init       # creates tradebot.config.json with sensible defaults
tradebot start      # starts engine + dashboard (demo mode)
```

Open **http://127.0.0.1:8765** in your browser. Within a minute you'll see live prices populating the **Markets** card.

The bot starts with a virtual **$50** of pretend USDC and trades against live Solana market data. Nothing is sent on-chain. Press **Ctrl-C** in the terminal to stop.

---

## The dashboard

| Section | What it shows |
|---|---|
| **Top bar** | Mode (DEMO/REAL), connection status (live/reconnecting) |
| **Stats strip** | Equity, cash, realized P&L, current drawdown, kill-switch status |
| **Equity Curve** | Your portfolio value over time |
| **Markets** | Mini price chart per watchlist pair with current price + % change |
| **Open Positions** | What you currently hold + unrealized P&L |
| **Signal Composite** | The combined signal score per pair, shown as a bar |
| **Recent Trades** | Last 25 buys/sells |
| **Settings tab** | Edit `tradebot.config.json` directly in the browser. Save then restart the bot to apply most changes. |

---

## Real mode

> **Warning:** Real mode trades real money. Test on **devnet** first (the default RPC is devnet, free fake SOL). Only switch to **mainnet** once you're confident the bot behaves the way you expect.

### Step 1: Set a strong passphrase

The bot's wallet is encrypted with a passphrase. Set it as an environment variable
**before** running anything (so it doesn't end up in your shell history):

```powershell
$env:TRADEBOT_PASSPHRASE = "use-a-long-random-passphrase-here"
```

This stays set for the current PowerShell session. For each new terminal, set it again.

### Step 2: Generate a dedicated bot wallet

```powershell
tradebot wallet generate --keystore keystore\bot.keystore.json
```

You'll see something like:

```
Generated keystore at keystore\bot.keystore.json
Bot address: 7xKw...HsPq
Fund this address from your Phantom wallet to begin trading.
```

**Copy that address.** It's a brand-new Solana wallet that exists only for the bot. Your Phantom wallet stays completely separate. The bot never knows your Phantom seed.

### Step 3: Fund the bot from Phantom

Open Phantom → click **Send** → paste the bot address. Send:

- **A small amount of SOL** for transaction fees (~0.05 SOL is plenty to start).
- **The USDC you want the bot to trade with** (e.g. $50 worth).

> Tip: If you only have SOL in Phantom, swap a portion to USDC inside Phantom first
> (Swap tab → SOL → USDC).

Wait 5-10 seconds for the transfer to confirm. Verify on [solscan.io](https://solscan.io)
by pasting the bot address.

### Step 4: Start in real mode (still on devnet by default)

```powershell
tradebot start --mode real --confirm-real --keystore keystore\bot.keystore.json
```

The `--confirm-real` flag is a deliberate safety gate: without it the bot refuses
to start in real mode. It also requires `TRADEBOT_PASSPHRASE` to be set.

You'll see in the logs:

```
{"event": "REAL_MODE_ACTIVE", "address": "7xKw...HsPq", "rpc": "https://api.devnet.solana.com"}
```

Open the dashboard. The mode pill in the top-right will be a **pulsing red REAL**.

### Step 5: Switch to mainnet (when you're ready)

Open the dashboard's **Settings** tab. Find this in the JSON:

```json
"app": {
    "rpc_url": "https://api.devnet.solana.com",
```

Change it to a mainnet RPC. Two free options:

- **Public (rate-limited, often unreliable for bots):**
  `"rpc_url": "https://api.mainnet-beta.solana.com"`
- **Helius free tier (recommended):** sign up at [helius.dev](https://helius.dev),
  copy your API key, then:
  `"rpc_url": "https://mainnet.helius-rpc.com/?api-key=YOUR_KEY_HERE"`

Click **Save**, **Ctrl-C** the bot, fund the bot wallet with **mainnet** SOL+USDC
from Phantom (mainnet has different addresses for tokens; Phantom handles this
automatically when you select the network), and run `tradebot start --mode real --confirm-real ...` again.

### Stopping the bot

Press **Ctrl-C** in the terminal. Open positions stay open (on-chain holdings don't
disappear when the bot stops). Restart the bot and it picks up where it left off.

To liquidate manually: open Phantom on the bot wallet (you'd need to import the
keystore, which is intentionally awkward; instead, just use Jupiter directly via
the bot's address using a swap UI, or write a one-shot script).

---

## Configuration

Everything lives in `tradebot.config.json`. The dashboard's **Settings** tab is the
easiest way to edit it. Most-likely-to-tune fields:

| Field | What | Default | Tip |
|---|---|---|---|
| `app.starting_capital_usd` | Demo virtual capital, or expected real capital | `50.0` | Match what you actually fund the bot with |
| `app.decision_interval_s` | How often the bot makes a decision | `30.0` | Lower = more reactive, more API calls |
| `app.entry_threshold` | Minimum signal score to enter | `0.6` | Higher = fewer, more selective trades |
| `risk.per_trade_size_min/max` | Range of % cash per trade | `0.30 / 0.50` | Lower for more cautious sizing |
| `risk.trailing_stop_pct` | Trailing stop distance | `0.02` (2%) | Tighter = exits sooner, may exit on noise |
| `risk.max_slippage_pct` | Refuse trades above this slippage | `0.01` (1%) | Lower for major pairs, higher for thin pairs |
| `watchlist.entries` | Tokens the bot trades | SOL, JUP, JTO | Add any Solana token by its mint address |
| `weights.signals` | How TA / microstructure / onchain are blended | `0.4 / 0.3 / 0.3` | Must sum to 1.0 |

### Adding a token to the watchlist

Find the token's **mint address** on [birdeye.so](https://birdeye.so) or [solscan.io](https://solscan.io). Then in Settings tab:

```json
"watchlist": {
    "entries": [
        {"symbol": "SOL", "mint": "So11111111111111111111111111111111111111112", "decimals": 9},
        {"symbol": "BONK", "mint": "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", "decimals": 5},
    ]
}
```

`decimals` is a per-token property. Find it on solscan under the token's metadata.

---

## Resume + caching

The bot saves its full state every 30 seconds. If you stop and restart, it picks up where it left off:

| What persists | Where |
|---|---|
| Cash + open positions | `data/portfolio.<mode>.json` |
| Realized P&L + equity high (drawdown anchor) | same |
| Risk state (kill switch, daily/weekly counters, day-paused-until) | `data/risk_state.<mode>.json` |
| Price chart history (per-pair sparklines) | `data/mark_history.<mode>.json` |
| Trade log + equity snapshots + OHLCV | `data/{trades,equity.<mode>,ohlcv}.json` |

To start completely fresh (wipe portfolio + risk + chart cache, but keep the trade log for posterity):

    tradebot start --reset

In real mode, on startup the bot also queries your bot wallet's on-chain balances and warns if they don't match the saved portfolio (e.g. you sent funds in/out via Phantom while the bot was off). It does not auto-correct: you decide whether to `--reset` or trust the saved state.

---

## Safety

**Things that protect you:**

- Demo mode never sends a transaction.
- Real mode requires `--confirm-real` AND a passphrase env var.
- The bot wallet is separate from Phantom: your seed phrase is never involved.
- Risk circuit breakers stop trading on big losses.
- Slippage guard refuses bad fills.
- Default RPC is devnet: switching to mainnet is a deliberate edit.

**Things that can still go wrong:**

- **You leak the keystore + passphrase** → an attacker can drain the bot wallet.
  Never put the keystore in cloud sync, screenshots, or chat. Treat
  `tradebot.config.json` and the `keystore/` folder as secrets.
- **You set `TRADEBOT_PASSPHRASE` in a script that gets committed.**
- **You give the bot too much money.** Treat the bot wallet as expendable. Don't
  fund it with anything you can't afford to lose.
- **A signal misfires during a flash crash.** The kill switches help, but they
  can't catch an instantaneous gap. Smaller positions = smaller blast radius.
- **The Solana network gets congested.** Transactions may fail or land at
  unfavorable prices. The slippage guard rejects the worst cases but not all.

The repo's `.gitignore` already excludes `tradebot.config.json`, `data/`, and
`keystore/`. Don't second-guess that.

---

## FAQ

**Q: Does the bot trade with my Phantom wallet?**
No. It generates its own dedicated wallet and uses that. You fund it from Phantom
once, like topping up a sub-account.

**Q: Can I withdraw money from the bot wallet?**
The keystore + passphrase together control it. You can write a small script that
loads the keystore and sends a transaction, or import the keystore into another
Solana wallet that supports raw private keys.

**Q: Where does my data live?**
Locally, in `data/` as plain JSON files. No cloud, no database server.
`data/trades.json`, `data/portfolio.<mode>.json`, `data/equity.<mode>.json`.
You can read them with any text editor.

**Q: Can I run this on a server / VPS?**
Yes, but change `dashboard_host` to `0.0.0.0` and **put it behind a reverse proxy
with HTTPS + auth**. The dashboard has no built-in authentication.

**Q: Will the bot make me money?**
Probably not. Most retail trading bots lose money to fees and slippage over time.
Treat this as a learning project / experiment. Run it in demo mode for at least
a few days before risking anything real.

**Q: What if I want to add a new signal?**
Drop a new module in `tradebot/signals/` that exposes a `score(ctx)` async method
returning a `SignalScore` with a value in [-1, +1]. Add it to the aggregator wiring
in `tradebot/main.py`. Add its weight to `weights.signals` in the config.

**Q: I get `getaddrinfo failed` errors, what do I do?**
DNS or network issue. Try `nslookup lite-api.jup.ag` from the same shell. If that
fails, you have no internet or your DNS is broken. If it works but the bot still
errors, check VPN / firewall.

**Q: The bot didn't make any trades for an hour. Is it broken?**
Probably not. With `entry_threshold: 0.6`, it only trades when the combined signal
is genuinely strong. In flat markets that's rare. Lower the threshold (e.g. `0.4`)
if you want more trade activity, but expect more losers.

---

## Project layout (for the curious)

```
tradebot/
├── config/        # JSON config schema + loader
├── core/          # portfolio, aggregator, decision engine, risk manager, main loop
├── dashboard/     # FastAPI server + WebSocket + static frontend
├── data/          # Jupiter quote client, Solana RPC, OHLCV aggregator
├── execution/     # DemoExecutor (paper trades), RealExecutor (on-chain swaps)
├── signals/       # TA + microstructure + on-chain signal modules
├── storage/       # JSON file repositories (trades, portfolio, equity, ohlcv)
├── wallet/        # Encrypted keypair (Argon2id + AES-256-GCM)
└── main.py        # CLI entry point: tradebot init / start / wallet
```

That's it. Have fun. **Don't trade what you can't afford to lose.**

---

## Backtest

Open the dashboard → **Backtest** tab. Upload an OHLCV CSV (columns: `timestamp, open, high, low, close, volume`, UTC ISO timestamps), set params, click **Run backtest**.

Results: total return, Sharpe ratio, max drawdown, win/loss, trade list, equity curve. Last 20 runs are kept in memory and shown in the History panel. Click any row to reload that result.

Sources for OHLCV CSVs:
- Birdeye: download token historical price as CSV
- CoinGecko: free historical API
- The bot's own `data/ohlcv/<pair>__<timeframe>.json`: convert to CSV with a one-liner

Backtests use **TA only** in v1. Microstructure and on-chain signals require live data feeds and aren't replayable from OHLCV alone.
