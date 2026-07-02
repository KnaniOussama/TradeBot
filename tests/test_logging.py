import json

from tradebot.logging_setup import configure_logging, get_logger


def test_get_logger_emits_json(capsys):
    configure_logging(level="INFO", json_output=True)
    log = get_logger("test")
    log.info("hello", pair="SOL/USDC", price=150.5)
    captured = capsys.readouterr()
    line = captured.out.strip().splitlines()[-1]
    payload = json.loads(line)
    assert payload["event"] == "hello"
    assert payload["pair"] == "SOL/USDC"
    assert payload["price"] == 150.5
    assert payload["level"] == "info"


def test_configure_logging_respects_level(capsys):
    configure_logging(level="WARNING", json_output=True)
    log = get_logger("test")
    log.info("should_not_appear")
    log.warning("should_appear")
    captured = capsys.readouterr()
    assert "should_not_appear" not in captured.out
    assert "should_appear" in captured.out
