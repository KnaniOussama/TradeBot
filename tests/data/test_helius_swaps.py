import pytest

from tradebot.data.helius import HeliusClient, _parse_swap_tx, get_recent_swaps_for_wallet

WALLET = "WhaleAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
SOL = "So11111111111111111111111111111111111111112"


def _swap_tx(*, sent_mint, sent_raw, received_mint, received_raw, ts=1714742400):
    return {
        "signature": "S" * 88,
        "timestamp": ts,
        "tokenTransfers": [
            {
                "mint": sent_mint,
                "fromUserAccount": WALLET,
                "toUserAccount": "Pool111",
                "rawTokenAmount": {"tokenAmount": str(sent_raw), "decimals": 6},
                "tokenAmount": sent_raw / 1_000_000,
            },
            {
                "mint": received_mint,
                "fromUserAccount": "Pool111",
                "toUserAccount": WALLET,
                "rawTokenAmount": {"tokenAmount": str(received_raw), "decimals": 9},
                "tokenAmount": received_raw / 1_000_000_000,
            },
        ],
    }


def test_parse_swap_tx_extracts_legs():
    tx = _swap_tx(sent_mint=USDC, sent_raw=10_000_000, received_mint=SOL, received_raw=70_000_000)
    out = _parse_swap_tx(WALLET, tx)
    assert out is not None
    assert out.in_mint == USDC
    assert out.out_mint == SOL
    assert out.in_amount_raw == 10_000_000
    assert out.out_amount_raw == 70_000_000
    assert out.wallet == WALLET


def test_parse_swap_tx_returns_none_when_wallet_not_in_transfers():
    tx = {
        "signature": "S",
        "timestamp": 1,
        "tokenTransfers": [
            {"mint": USDC, "fromUserAccount": "OtherA", "toUserAccount": "OtherB",
             "rawTokenAmount": {"tokenAmount": "1"}, "tokenAmount": 1.0},
        ],
    }
    assert _parse_swap_tx(WALLET, tx) is None


def test_parse_swap_tx_returns_none_for_zero_amount():
    tx = _swap_tx(sent_mint=USDC, sent_raw=0, received_mint=SOL, received_raw=10)
    assert _parse_swap_tx(WALLET, tx) is None


@pytest.mark.asyncio
async def test_get_recent_swaps_parses_response(httpx_mock):
    payload = [
        _swap_tx(sent_mint=USDC, sent_raw=10_000_000, received_mint=SOL, received_raw=70_000_000),
        _swap_tx(sent_mint=SOL, sent_raw=70_000_000, received_mint=USDC, received_raw=10_500_000,
                 ts=1714742500),
    ]
    import re
    httpx_mock.add_response(url=re.compile(r".*/v0/addresses/.*"), json=payload)
    async with HeliusClient(api_key="test-key") as c:
        swaps = await get_recent_swaps_for_wallet(c, address=WALLET, limit=10)
    assert len(swaps) == 2
    assert swaps[0].in_mint == USDC and swaps[0].out_mint == SOL
    assert swaps[1].in_mint == SOL and swaps[1].out_mint == USDC


@pytest.mark.asyncio
async def test_get_recent_swaps_returns_empty_on_http_error(httpx_mock):
    import re
    httpx_mock.add_response(url=re.compile(r".*/v0/addresses/.*"), status_code=500)
    async with HeliusClient(api_key="test-key") as c:
        swaps = await get_recent_swaps_for_wallet(c, address=WALLET, limit=10)
    assert swaps == []
