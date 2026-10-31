"""The measure of what a service reads from its cache across levels, without a service."""

import os
import re
from pathlib import Path

import pytest

from bench import cache_levels
from bench.cache_levels import Row, Serving, measure, plan, row_of, table
from support.launch import profile


def test_every_level_is_asked_for_twice_and_the_first_again_at_the_end() -> None:
    assert plan(["default", "low", "ultra"]) == [
        "default",
        "default",
        "low",
        "low",
        "ultra",
        "ultra",
        "default",
        "default",
    ]
    assert plan(["default", "ultra"], ["ultra", "default"]) == ["ultra", "default"]


def test_a_turn_costs_what_its_requests_cost_and_its_attempts() -> None:
    events = [
        {"type": "turn/start", "data": {"turn": 3}},
        {
            "type": "assistant/attempt",
            "data": {"usage": {"input_tokens": 5, "cache_read_tokens": 0}},
        },
        {
            "type": "assistant/message",
            "data": {
                "usage": {
                    "input_tokens": 20,
                    "output_tokens": 4,
                    "cache_read_tokens": 900,
                    "cache_write_tokens": 80,
                }
            },
        },
        {"type": "turn/end", "data": {"turn": 3, "reason": "completed"}},
    ]

    row = row_of(3, "high", events)

    assert row == Row(turn=3, level="high", requests=2, read=900, written=80, fresh=25, output=4)
    assert row.share == pytest.approx(900 / 1005)
    assert Row(1, "default", 0, 0, 0, 0, 0).share == 0.0


def test_the_table_has_a_line_for_each_turn() -> None:
    rows = [Row(1, "default", 1, 0, 1000, 10, 2), Row(2, "ultra", 1, 1000, 40, 10, 2)]

    lines = table(rows).splitlines()

    assert lines[0].split() == [
        "turn",
        "level",
        "requests",
        "read",
        "written",
        "fresh",
        "output",
        "cached",
    ]
    assert lines[2].split() == ["2", "ultra", "1", "1000", "40", "10", "2", "95.2%"]


@pytest.mark.timeout(180)
def test_a_conversation_of_a_service_without_a_cache_is_measured_turn_by_turn(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # No key of any service goes to a program that asks no service for anything.
    for name in list(os.environ):
        if re.search("KEY|TOKEN|SECRET|PASSWORD", name.upper()):
            monkeypatch.delenv(name)
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    work = tmp_path / "work"
    work.mkdir()
    chosen = profile(tmp_path, *({"text": "Blue."} for _ in range(6)), extends="serve")

    serving = Serving(["--profile", chosen], str(work), seconds=60)
    try:
        rows = measure(serving)
    finally:
        code = serving.close()

    # The scripted model offers the default and ultra, and tells of no tokens.
    assert [(row.turn, row.level, row.requests) for row in rows] == [
        (1, "default", 1),
        (2, "default", 1),
        (3, "ultra", 1),
        (4, "ultra", 1),
        (5, "default", 1),
        (6, "default", 1),
    ]
    assert code == 0


def test_a_level_that_is_not_on_offer_ends_the_measure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    for name in list(os.environ):
        if re.search("KEY|TOKEN|SECRET|PASSWORD", name.upper()):
            monkeypatch.delenv(name)
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    chosen = profile(tmp_path, extends="serve")

    assert cache_levels.main(["--profile", chosen, "--levels", "max"]) == 1

    assert "This service does not take max." in capsys.readouterr().err
