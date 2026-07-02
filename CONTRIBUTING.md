# Contributing to TradeBot

Thanks for your interest in improving TradeBot. This is a hobby/learning project
for autonomous Solana DEX trading — contributions of all sizes are welcome.

> ⚠️ **This software trades real money in real mode.** Bugs can cost users funds.
> Correctness and safety matter more than features. When in doubt, favor the more
> conservative behavior and add a test.

## Getting set up

You'll need **Python 3.12+**.

```bash
python -m venv .venv
# Windows:  .venv\Scripts\activate
# Unix:     source .venv/bin/activate
pip install -e .[dev]
```

Run the test suite to confirm everything works:

```bash
pytest
```

## Before you open a pull request

Please make sure all of the following pass locally — CI runs the same checks:

```bash
ruff check .        # lint
mypy tradebot       # type check (strict mode)
pytest              # tests
```

New behavior needs a test. Bug fixes should come with a regression test that fails
before the fix and passes after.

## Coding conventions

- **Line length:** 100 (enforced by ruff).
- **Typing:** the codebase is `mypy --strict`. Keep it that way — no new `Any`
  leaks or missing annotations.
- **Async:** the trading loop and all I/O clients are `asyncio`. Don't block the
  event loop; use the existing rate-limited clients for network calls.
- **Signals:** a new signal is a module in `tradebot/signals/` exposing an async
  `score(ctx) -> SignalScore` with a value in `[-1, +1]`. Wire it into the
  aggregator in `tradebot/main.py` and add its weight to `weights.signals`.
- Match the style of the surrounding code — comment density, naming, structure.

## Security

Never commit secrets: keystores, passphrases, API keys, or a real
`tradebot.config.json`. These are already in `.gitignore` — don't override it.
If you find a security issue, see [SECURITY.md](SECURITY.md).

## Reporting bugs / requesting features

Open an issue describing what you expected, what happened, and (for bugs) the
smallest steps to reproduce. Redact any addresses, keys, or personal data.
