# Security Policy

TradeBot can move real funds on Solana mainnet. Please treat security issues
seriously.

## Reporting a vulnerability

**Do not open a public issue for security vulnerabilities.**

Instead, report privately via GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
("Report a vulnerability" button on the Security tab), or email the maintainer.

Please include:

- A description of the issue and its impact.
- Steps to reproduce, if possible.
- Any suggested fix.

## Scope

Especially interested in issues affecting:

- The encrypted keystore (`tradebot/wallet/` — Argon2id + AES-256-GCM).
- Anything that could leak a private key, passphrase, or API key.
- Transaction construction / slippage handling in `tradebot/execution/real.py`.
- The dashboard (`tradebot/dashboard/`) — note it ships **without authentication**
  and is intended to bind to `127.0.0.1`. Exposing it publicly is out of scope
  unless there's an issue beyond "the user exposed an unauthenticated service."

## User responsibilities

TradeBot is provided "as is" (see [LICENSE](LICENSE)). Users are responsible for:

- Keeping their keystore and `TRADEBOT_PASSPHRASE` secret.
- Not funding the bot wallet with more than they can afford to lose.
- Running demo mode / devnet before risking real funds on mainnet.
