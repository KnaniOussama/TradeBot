from __future__ import annotations

from tradebot.data.rpc import SolanaRpcClient


async def get_sol_balance(rpc: SolanaRpcClient, address: str) -> float:
    return await rpc.get_balance_sol(address)


async def get_token_balance(rpc: SolanaRpcClient, owner: str, mint: str) -> float:
    accounts = await rpc.get_token_accounts_by_owner(owner=owner, mint=mint)
    total = 0.0
    for acct in accounts:
        try:
            ui = acct["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmount"]
            total += float(ui or 0.0)
        except (KeyError, TypeError, ValueError):
            continue
    return total
