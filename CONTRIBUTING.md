# Contributing to TradeBot

Thanks for your interest in improving TradeBot. This is a hobby/learning project
for autonomous Solana DEX trading, and contributions of all sizes are welcome.

> **Warning: This software trades real money in real mode.** Bugs can cost users funds.
> Correctness and safety matter more than features. When in doubt, favor the more
> conservative behavior and add a test.

## Getting set up

You'll need **Rust (stable)**. Install it via [rustup](https://rustup.rs) if you
don't have it already; the repo pins the toolchain via `rust-toolchain.toml`.

```bash
cargo build --release
```

The binary lands at `target/release/tradebot`. Either `cargo install --path
crates/tradebot` to get `tradebot` on your PATH, or run it directly with `cargo
run -p tradebot -- <args>`.

Run the test suite to confirm everything works:

```bash
cargo test --all
```

## Before you open a pull request

Please make sure all of the following pass locally (CI runs the same checks):

```bash
cargo fmt --all --check                        # format
cargo clippy --all-targets -- -D warnings      # lint
cargo test --all                               # tests
```

New behavior needs a test. Bug fixes should come with a regression test that fails
before the fix and passes after.

## Coding conventions

- **Line length:** rustfmt default. Don't fight the formatter, just run `cargo fmt`.
- **Typing:** Rust's type system does the work here. Avoid `unwrap()`/`expect()`
  outside of tests and startup code; propagate errors with `Result` instead.
- **Async:** the trading loop and all I/O clients run on `tokio`. Don't block the
  async runtime; use the existing rate-limited clients for network calls.
- **Signals:** a new signal implements the `Signal` trait in `crates/signals`
  with a `score(ctx) -> SignalScore` method returning a value in `[-1, +1]`. Wire
  it into the aggregator in `crates/tradebot/src/wiring.rs` and add its weight to
  `weights.signals`.
- Match the style of the surrounding code: comment density, naming, structure.

## Security

Never commit secrets: keystores, passphrases, API keys, or a real
`tradebot.config.json`. These are already in `.gitignore`, so don't override it.
If you find a security issue, see [SECURITY.md](SECURITY.md).

## Reporting bugs / requesting features

Open an issue describing what you expected, what happened, and (for bugs) the
smallest steps to reproduce. Redact any addresses, keys, or personal data.
