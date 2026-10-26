"""How the tasks of stored sessions went."""

import json

import pytest

from columbina.llm import AssistantMessage, CallConfig, ToolMessage, Usage
from columbina.prices import Prices
from columbina.session import (
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.stats import Tally, counted, told
from columbina.verdicts import VerdictEvent

DAY = 24 * 60 * 60 * 1000
NOW = 1_800_000_000_000


def log(*happened: EventData, at: int = NOW) -> list[SessionEvent[EventData]]:
    return [SessionEvent(seq, at + seq, data) for seq, data in enumerate(happened)]


def called(call_id: str, name: str, **arguments: object) -> ToolCallEvent:
    return ToolCallEvent(call_id=call_id, name=name, arguments=json.dumps(arguments))


def result(call_id: str, *, error: str | None = None, wrote: str | None = None) -> ToolResultEvent:
    return ToolResultEvent(
        message=ToolMessage(call_id=call_id, content=()),
        error=None if error is None else ToolError(code=error, reason="no"),
        meta=None if wrote is None else {"wrote": wrote},
    )


def ended(turn: int, reason: str = "completed") -> TurnEndEvent:
    return TurnEndEvent(turn=turn, reason=reason)  # type: ignore[arg-type]


def paid(output: int) -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=AssistantMessage(content=()),
        usage=Usage(output_tokens=output),
        finish_reason="stop",
    )


def tallied(*sessions: tuple[int, list[SessionEvent[EventData]]], days: int = 30) -> Tally:
    return counted(sessions, since_ms=NOW - days * DAY, prices=Prices())


def test_turns_verdicts_and_subagents_are_counted() -> None:
    conversation = log(
        TurnStartEvent(turn=1),
        ended(1),
        VerdictEvent(turn=1, right=False),
        VerdictEvent(turn=1, right=True),
        TurnStartEvent(turn=2),
        ended(2, "error"),
        VerdictEvent(turn=2, right=False),
        TurnStartEvent(turn=3),
        ended(3, "aborted"),
    )
    subagent = log(TurnStartEvent(turn=1), ended(1, "blocked"))

    tally = tallied((0, conversation), (1, subagent))

    assert tally.sessions == 1
    assert tally.subagent_sessions == 1
    assert dict(tally.turns) == {"completed": 1, "error": 1, "aborted": 1}
    assert dict(tally.verdicts) == {"right": 1, "wrong": 1}
    assert dict(tally.subagent_turns) == {"blocked": 1}


def test_waste_is_counted_turn_by_turn() -> None:
    conversation = log(
        TurnStartEvent(turn=1),
        called("1", "read", path="a.py"),
        result("1"),
        called("2", "read", path="a.py"),
        result("2"),
        called("3", "grep", pattern="x"),
        result("3", error="NO_MATCH"),
        called("4", "grep", pattern="x"),
        result("4", error="REPEATED_CALL"),
        called("5", "edit", path="a.py"),
        result("5", wrote="a.py"),
        called("6", "read", path="a.py"),
        result("6"),
        called("7", "read", path="b.py"),
        result("7"),
        called("8", "write", path="c.py"),
        result("8", wrote="other/c.py"),
        called("9", "read", path="b.py"),
        result("9"),
        ToolCallEvent(call_id="10", name="read", arguments="not json"),
        ToolCallEvent(call_id="11", name="read", arguments="[1]"),
        ToolCallEvent(call_id="12", name="read", arguments='{"path": 3}'),
        result("13", wrote="nowhere"),
        ended(1),
        TurnStartEvent(turn=2),
        called("14", "read", path="a.py"),
        result("14"),
        ended(2),
    )

    tally = tallied((0, conversation))

    assert tally.calls == 11
    assert dict(tally.failed) == {"NO_MATCH": 1, "REPEATED_CALL": 1}
    # 2 and 4 are the same as before. After the edit nothing is, and a.py read again counts
    # as the first read since. b.py was read again after a write of another file.
    assert tally.repeated == 2
    assert tally.read_again == 2


def test_only_turns_that_began_in_the_time_asked_about_are_counted() -> None:
    old = log(TurnStartEvent(turn=1), ended(1), at=NOW - 40 * DAY)
    header = RequestHeaderEvent(
        config=CallConfig(provider="anthropic", model="claude-opus-5-5"), tools=()
    )
    before = log(header, TurnStartEvent(turn=1), paid(1_000_000), ended(1), at=NOW - 40 * DAY)
    mixed = [
        *before,
        SessionEvent(4, NOW, TurnStartEvent(turn=2)),
        SessionEvent(5, NOW, paid(1_000_000)),
        SessionEvent(6, NOW, ended(2)),
    ]

    tally = tallied((0, old), (0, mixed))

    assert tally.sessions == 1
    assert dict(tally.turns) == {"completed": 1}
    assert tally.spent.money == {"USD": pytest.approx(20)}


def test_what_was_counted_is_told_to_a_person() -> None:
    conversation = log(
        RequestHeaderEvent(config=CallConfig(provider="openai", model="gpt-5"), tools=()),
        TurnStartEvent(turn=1),
        paid(10),
        called("1", "read", path="a.py"),
        result("1", error="NOT_FOUND"),
        ended(1),
        VerdictEvent(turn=1, right=True),
        TurnStartEvent(turn=2),
        ended(2, "max-tokens"),
        VerdictEvent(turn=2, right=False),
    )
    subagent = log(TurnStartEvent(turn=1), ended(1))

    text = told(tallied((0, conversation), (1, subagent)))

    assert text.split("\n") == [
        "Conversations: 1. Turns: 2 (1 done, 1 cut off by the length of a reply).",
        "Said to be right: 1 of the 2 turns that were judged (50%).",
        "Subagents: 1, whose turns came back done 1 times of 1.",
        "Spent: nothing that is known.",
        "No price is known for openai/gpt-5.",
        "Calls of tools: 1.",
        "Failed: 1 (NOT_FOUND 1).",
        "Made again with nothing changed: 0. Files read again unchanged: 0.",
    ]


def test_nothing_counted_is_told_too() -> None:
    assert told(Tally()).split("\n") == [
        "Conversations: 0. Turns: 0.",
        "No turn was judged: say /verdict right or /verdict wrong after a turn.",
        "Spent: nothing that is known.",
        "Calls of tools: 0.",
        "Made again with nothing changed: 0. Files read again unchanged: 0.",
    ]


def test_money_is_told_and_an_unknown_end_by_its_name() -> None:
    tally = Tally()
    tally.spent.money["USD"] = 1.5
    tally.turns["odd"] += 1

    assert "Spent: about $1.50." in told(tally)
    assert "Turns: 1 (1 odd)." in told(tally)


def test_what_was_counted_is_given_to_a_program() -> None:
    conversation = log(
        TurnStartEvent(turn=1), called("1", "read"), result("1", error="X"), ended(1)
    )

    assert tallied((0, conversation)).as_json() == {
        "sessions": 1,
        "subagent_sessions": 0,
        "turns": {"completed": 1},
        "subagent_turns": {},
        "verdicts": {},
        "calls": 1,
        "failed": {"X": 1},
        "repeated": 0,
        "read_again": 0,
        "spent": {},
        "unpriced": [],
    }
