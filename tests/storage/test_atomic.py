import json
from pathlib import Path

from tradebot.storage.atomic import atomic_write_json, read_json_or_default


def test_atomic_write_creates_file(tmp_path: Path):
    p = tmp_path / "x.json"
    atomic_write_json(p, {"a": 1})
    assert p.exists()
    assert json.loads(p.read_text()) == {"a": 1}


def test_atomic_write_overwrites_safely(tmp_path: Path):
    p = tmp_path / "x.json"
    atomic_write_json(p, {"a": 1})
    atomic_write_json(p, {"a": 2})
    assert json.loads(p.read_text()) == {"a": 2}


def test_atomic_write_creates_parent_dirs(tmp_path: Path):
    p = tmp_path / "deep" / "nested" / "x.json"
    atomic_write_json(p, [1, 2, 3])
    assert p.exists()


def test_read_json_or_default_returns_default_for_missing(tmp_path: Path):
    out = read_json_or_default(tmp_path / "missing.json", default=[])
    assert out == []


def test_read_json_or_default_returns_parsed_for_existing(tmp_path: Path):
    p = tmp_path / "x.json"
    p.write_text(json.dumps([1, 2, 3]))
    assert read_json_or_default(p, default=[]) == [1, 2, 3]


def test_read_json_or_default_corrupted_returns_default(tmp_path: Path):
    p = tmp_path / "bad.json"
    p.write_text("not json")
    assert read_json_or_default(p, default={"k": "v"}) == {"k": "v"}
