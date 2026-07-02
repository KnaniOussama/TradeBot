//! Port of `tradebot/wallet/reconcile.py`: compares the saved portfolio
//! state against on-chain balances and reports mismatches.
//!
//! Unlike the Python version, which takes `rpc: Any` and an injected
//! `token_balance_fn` so tests can stub the RPC layer with plain mocks, this
//! port calls [`SolanaRpcClient`] and [`get_token_balance`] directly. Rust
//! tests instead stand up a real HTTP mock (`wiremock`) at the transport
//! layer, which exercises the same code path production uses.

use std::collections::HashMap;

use rust_decimal::prelude::ToPrimitive;
use tradebot_core::Portfolio;
use tradebot_data::{RpcError, SolanaRpcClient};

use crate::balance::get_token_balance;

/// Default minimum SOL balance the bot wallet should hold for transaction
/// fees, matching `min_sol_for_fees: float = 0.01` in reconcile.py.
pub const DEFAULT_MIN_SOL_FOR_FEES: f64 = 0.01;

/// Comparison tolerance for base-amount mismatches, matching Python's
/// `_MISMATCH_TOLERANCE = 1e-6`.
const MISMATCH_TOLERANCE: f64 = 1e-6;

/// The kind of discrepancy a [`ReconcileFinding`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    PositionMismatch,
    UnexpectedBalance,
    LowSol,
}

/// One discrepancy found by [`reconcile`] between the portfolio's recorded
/// state and the on-chain wallet.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileFinding {
    pub kind: FindingKind,
    pub pair: Option<String>,
    pub message: String,
    pub expected: Option<f64>,
    pub observed: Option<f64>,
}

/// Compares `portfolio`'s recorded SOL and per-pair base balances against
/// what the chain actually reports for `bot_address`, over the pairs listed
/// in `base_mints` (pair -> (mint, decimals); decimals is unused, kept for
/// parity with the Python signature).
pub async fn reconcile(
    portfolio: &Portfolio,
    rpc: &SolanaRpcClient,
    bot_address: &str,
    base_mints: &HashMap<String, (String, u32)>,
    min_sol_for_fees: f64,
) -> Result<Vec<ReconcileFinding>, RpcError> {
    let mut findings = Vec::new();

    let sol = rpc.get_balance_sol(bot_address).await?;
    if sol < min_sol_for_fees {
        findings.push(ReconcileFinding {
            kind: FindingKind::LowSol,
            pair: None,
            message: format!(
                "bot wallet has only {sol:.6} SOL; need >= {min_sol_for_fees} for tx fees"
            ),
            observed: Some(sol),
            expected: Some(min_sol_for_fees),
        });
    }

    for (pair, (mint, _decimals)) in base_mints {
        let observed = get_token_balance(rpc, bot_address, mint).await?;
        let pos = portfolio.position_for(pair);
        let expected = pos.and_then(|p| p.base_amount.to_f64()).unwrap_or(0.0);

        if pos.is_some() {
            if (observed - expected).abs() > MISMATCH_TOLERANCE {
                findings.push(ReconcileFinding {
                    kind: FindingKind::PositionMismatch,
                    pair: Some(pair.clone()),
                    message: format!(
                        "{pair}: portfolio expects {expected:.6}, wallet has {observed:.6}"
                    ),
                    expected: Some(expected),
                    observed: Some(observed),
                });
            }
        } else if observed > MISMATCH_TOLERANCE {
            findings.push(ReconcileFinding {
                kind: FindingKind::UnexpectedBalance,
                pair: Some(pair.clone()),
                message: format!(
                    "{pair}: wallet has {observed:.6} but portfolio holds no position"
                ),
                expected: Some(0.0),
                observed: Some(observed),
            });
        }
    }

    Ok(findings)
}

/// Logs `findings` via `tracing`, matching `log_findings` in reconcile.py.
pub fn log_findings(findings: &[ReconcileFinding]) {
    if findings.is_empty() {
        tracing::info!(findings = 0, "reconcile_clean");
        return;
    }
    for f in findings {
        tracing::warn!(
            kind = ?f.kind,
            pair = ?f.pair,
            message = %f.message,
            expected = ?f.expected,
            observed = ?f.observed,
            "reconcile_finding",
        );
    }
    tracing::warn!(findings = findings.len(), "reconcile_summary");
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::path::{Path, PathBuf};
    use tradebot_common::Mode;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn load_fixture(name: &str) -> serde_json::Value {
        let text = std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
    }

    async fn mock_server_for(
        balance: serde_json::Value,
        token_accounts: serde_json::Value,
    ) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "method": "getBalance"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&balance))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "method": "getTokenAccountsByOwner"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_accounts))
            .mount(&server)
            .await;
        server
    }

    fn low_sol_balance() -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "result": {"context": {"slot": 1}, "value": 50_000_000u64},
            "id": 1,
        })
    }

    fn empty_token_accounts() -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "result": {"context": {"slot": 1}, "value": []},
            "id": 1,
        })
    }

    fn base_mints() -> HashMap<String, (String, u32)> {
        let mut m = HashMap::new();
        m.insert("SOL/USDC".to_string(), ("So111".to_string(), 9));
        m
    }

    #[tokio::test]
    async fn matches_returns_no_findings() {
        // Wallet has 0.05 SOL (above the 0.01 threshold), portfolio expects
        // nothing and the wallet has nothing: no findings.
        let server = mock_server_for(low_sol_balance(), empty_token_accounts()).await;
        let client = SolanaRpcClient::new(server.uri());
        let portfolio = Portfolio::new(Mode::Real, Decimal::ZERO, Decimal::ZERO);

        let findings = reconcile(
            &portfolio,
            &client,
            "X",
            &base_mints(),
            DEFAULT_MIN_SOL_FOR_FEES,
        )
        .await
        .unwrap();
        assert!(findings.is_empty());
    }

    #[tokio::test]
    async fn flags_missing_position() {
        let token_accounts = serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "context": {"slot": 1},
                "value": [{
                    "account": {"data": {"parsed": {"info": {"tokenAmount": {"uiAmount": 0.1}}}}},
                }],
            },
            "id": 1,
        });
        let server = mock_server_for(low_sol_balance(), token_accounts).await;
        let client = SolanaRpcClient::new(server.uri());

        let mut portfolio = Portfolio::new(Mode::Real, Decimal::new(100, 0), Decimal::ZERO);
        portfolio
            .apply_fill(
                "SOL/USDC",
                tradebot_storage::Side::Buy,
                Decimal::new(5, 1),  // 0.5
                Decimal::new(75, 0), // 75.0
                Decimal::ZERO,
            )
            .unwrap();

        let findings = reconcile(
            &portfolio,
            &client,
            "X",
            &base_mints(),
            DEFAULT_MIN_SOL_FOR_FEES,
        )
        .await
        .unwrap();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::PositionMismatch);
        assert!(findings[0].message.contains("SOL/USDC"));
    }

    #[tokio::test]
    async fn flags_unexpected_balance() {
        let token_accounts = serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "context": {"slot": 1},
                "value": [{
                    "account": {"data": {"parsed": {"info": {"tokenAmount": {"uiAmount": 1.0}}}}},
                }],
            },
            "id": 1,
        });
        let server = mock_server_for(low_sol_balance(), token_accounts).await;
        let client = SolanaRpcClient::new(server.uri());
        let portfolio = Portfolio::new(Mode::Real, Decimal::new(10, 0), Decimal::ZERO);

        let findings = reconcile(
            &portfolio,
            &client,
            "X",
            &base_mints(),
            DEFAULT_MIN_SOL_FOR_FEES,
        )
        .await
        .unwrap();

        assert!(findings
            .iter()
            .any(|f| f.kind == FindingKind::UnexpectedBalance));
    }

    #[tokio::test]
    async fn low_sol_balance_is_flagged() {
        let server = mock_server_for(low_sol_balance(), empty_token_accounts()).await;
        let client = SolanaRpcClient::new(server.uri());
        let portfolio = Portfolio::new(Mode::Real, Decimal::ZERO, Decimal::ZERO);

        let findings = reconcile(&portfolio, &client, "X", &HashMap::new(), 0.1)
            .await
            .unwrap();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::LowSol);
    }

    #[tokio::test]
    async fn balance_fixtures_load_end_to_end() {
        // Exercises the real fixtures used by balance.rs's own tests, via
        // the reconcile path (getBalance uses rpc_get_balance.json).
        let balance = load_fixture("rpc_get_balance.json");
        let token_accounts = load_fixture("rpc_get_token_accounts_by_owner.json");
        let server = mock_server_for(balance, token_accounts).await;
        let client = SolanaRpcClient::new(server.uri());
        let portfolio = Portfolio::new(Mode::Real, Decimal::ZERO, Decimal::ZERO);

        let mut mints = HashMap::new();
        mints.insert(
            "USDC/USDC".to_string(),
            (
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                6,
            ),
        );

        let findings = reconcile(&portfolio, &client, "X", &mints, DEFAULT_MIN_SOL_FOR_FEES)
            .await
            .unwrap();
        // 1.23 SOL is above the fee threshold; 1.5 tokens with no recorded
        // position is an unexpected balance.
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::UnexpectedBalance);
    }
}
