from datetime import UTC, datetime

from tradebot.core.manual_actions import ManualActionQueue, ManualExitRequest


def test_queue_starts_empty():
    q = ManualActionQueue()
    assert len(q) == 0
    assert q.drain() == []


def test_request_exit_appends_and_drain_clears():
    q = ManualActionQueue()
    req = q.request_exit(pair="SOL/USDC", reason="took profit")
    assert isinstance(req, ManualExitRequest)
    assert req.pair == "SOL/USDC"
    assert req.reason == "took profit"
    assert isinstance(req.requested_at, datetime)
    assert req.requested_at.tzinfo == UTC
    assert len(q) == 1
    drained = q.drain()
    assert len(drained) == 1
    assert drained[0].pair == "SOL/USDC"
    assert len(q) == 0


def test_empty_reason_defaults_to_manual_sell():
    q = ManualActionQueue()
    req = q.request_exit(pair="BONK/USDC", reason="")
    assert req.reason == "manual sell"


def test_multiple_requests_drain_in_order():
    q = ManualActionQueue()
    q.request_exit(pair="A/USDC", reason="r1")
    q.request_exit(pair="B/USDC", reason="r2")
    q.request_exit(pair="C/USDC", reason="r3")
    drained = q.drain()
    assert [r.pair for r in drained] == ["A/USDC", "B/USDC", "C/USDC"]
