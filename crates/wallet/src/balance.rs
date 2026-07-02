//! Port of `tradebot/wallet/balance.py`: SOL and SPL token balance lookups
//! via the Solana RPC client.

use tradebot_data::{RpcError, SolanaRpcClient};

/// Returns the SOL balance of `address`, in SOL (not lamports).
pub async fn get_sol_balance(rpc: &SolanaRpcClient, address: &str) -> Result<f64, RpcError> {
    rpc.get_balance_sol(address).await
}

/// Returns the total UI-amount balance of `mint` held by `owner`, summed
/// across all matching token accounts. Accounts whose parsed response is
/// missing or malformed contribute 0, matching Python's
/// `except (KeyError, TypeError, ValueError): continue`.
pub async fn get_token_balance(
    rpc: &SolanaRpcClient,
    owner: &str,
    mint: &str,
) -> Result<f64, RpcError> {
    let accounts = rpc.get_token_accounts_by_owner(owner, mint).await?;
    let mut total = 0.0;
    for acct in accounts {
        let ui_amount = acct
            .get("account")
            .and_then(|v| v.get("data"))
            .and_then(|v| v.get("parsed"))
            .and_then(|v| v.get("info"))
            .and_then(|v| v.get("tokenAmount"))
            .and_then(|v| v.get("uiAmount"))
            .and_then(|v| v.as_f64());
        total += ui_amount.unwrap_or(0.0);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
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

    #[tokio::test]
    async fn get_sol_balance_reads_lamports_as_sol() {
        let payload = load_fixture("rpc_get_balance.json");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
            .mount(&server)
            .await;

        let client = SolanaRpcClient::new(server.uri());
        let sol = get_sol_balance(&client, "11111111111111111111111111111111")
            .await
            .unwrap();
        assert!((sol - 1.23456789).abs() < 1e-9, "sol={sol}");
    }

    #[tokio::test]
    async fn get_token_balance_sums_existing_accounts() {
        let payload = load_fixture("rpc_get_token_accounts_by_owner.json");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
            .mount(&server)
            .await;

        let client = SolanaRpcClient::new(server.uri());
        let bal = get_token_balance(
            &client,
            "11111111111111111111111111111111",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        )
        .await
        .unwrap();
        assert!((bal - 1.5).abs() < 1e-9, "bal={bal}");
    }

    #[tokio::test]
    async fn get_token_balance_no_account_returns_zero() {
        let empty = serde_json::json!({
            "jsonrpc": "2.0",
            "result": {"context": {"slot": 0}, "value": []},
            "id": 1,
        });
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&empty))
            .mount(&server)
            .await;

        let client = SolanaRpcClient::new(server.uri());
        let bal = get_token_balance(
            &client,
            "11111111111111111111111111111111",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        )
        .await
        .unwrap();
        assert_eq!(bal, 0.0);
    }
}
