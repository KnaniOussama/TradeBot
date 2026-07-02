import json
import re
from pathlib import Path

import pytest

from tradebot.data.jupiter import JupiterClient, JupiterQuote, JupiterSwap

QUOTE = json.loads(Path("tests/fixtures/jupiter_quote_sol_usdc.json").read_text())
SWAP = json.loads(Path("tests/fixtures/jupiter_swap_response.json").read_text())


def _quote_obj() -> JupiterQuote:
    return JupiterQuote(
        input_mint=QUOTE["inputMint"],
        output_mint=QUOTE["outputMint"],
        in_amount=int(QUOTE["inAmount"]),
        out_amount=int(QUOTE["outAmount"]),
        other_amount_threshold=int(QUOTE["otherAmountThreshold"]),
        slippage_bps=int(QUOTE["slippageBps"]),
        price_impact_pct=float(QUOTE["priceImpactPct"]),
        route_labels=["Raydium"],
        raw=QUOTE,
    )


@pytest.mark.asyncio
async def test_build_swap_returns_serialized_tx(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*/swap$"), json=SWAP)
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as c:
        out = await c.build_swap(
            quote=_quote_obj(),
            user_pubkey="11111111111111111111111111111111",
            priority_fee_microlamports=10_000,
            wrap_and_unwrap_sol=True,
        )
    assert isinstance(out, JupiterSwap)
    assert out.serialized_tx_b64 == "AQABAgM="
    assert out.last_valid_block_height == 350000999
    assert out.prioritization_fee_lamports == 5000


@pytest.mark.asyncio
async def test_build_swap_http_error(httpx_mock):
    httpx_mock.add_response(url=re.compile(r".*/swap$"), status_code=500, json={"error": "boom"})
    async with JupiterClient(base_url="https://quote-api.jup.ag/v6") as c:
        with pytest.raises(Exception):  # noqa: B017
            await c.build_swap(
                quote=_quote_obj(),
                user_pubkey="11111111111111111111111111111111",
                priority_fee_microlamports=0,
                wrap_and_unwrap_sol=True,
            )
