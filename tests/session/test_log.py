"""The session log: append-only, typed, and the single source of truth."""

import itertools
from typing import Any, ClassVar, cast

import pytest

from columbina.kernel import Context
from columbina.llm import TextBlock, ToolMessage
from columbina.session import (
    CORE_EVENTS,
    SESSION_EVENT,
    Committed,
    EventData,
    InvalidEventError,
    Session,
    SessionClosedError,
    SessionEvent,
    StepStartEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}


def open_session(ctx: Context, session_id: str = "s-1") -> Session:
    clock = itertools.count(1000, 10)
    return Session(ctx, session_id, KINDS, clock=lambda: next(clock))


def result(meta: dict[str, Any] | None = None) -> ToolResultEvent:
    return ToolResultEvent(
        message=ToolMessage(call_id="call-1", content=(TextBlock(text="ok"),)), meta=meta
    )


async def test_an_event_gets_its_place_and_its_time(ctx: Context) -> None:
    session = open_session(ctx)

    first = session.append(TurnStartEvent(turn=1))
    second = session.append(StepStartEvent(turn=1, step=1))

    assert first == SessionEvent(seq=0, time=1000, data=TurnStartEvent(turn=1))
    assert second == SessionEvent(seq=1, time=1010, data=StepStartEvent(turn=1, step=1))
    assert list(session.events) == [first, second]
    assert len(session) == 2
    assert session.id == "s-1"
    assert repr(session) == "<Session 's-1' at seq 2>"


async def test_the_default_clock_tells_the_time_in_milliseconds(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)

    event = session.append(TurnStartEvent(turn=1))

    assert 1_700_000_000_000 < event.time < 4_000_000_000_000
    # A session that is not told when it was created was created when it was made.
    assert 1_700_000_000_000 < session.created_at <= event.time


async def test_a_session_knows_when_it_was_created_and_asks_no_clock_of_its_own_for_it(
    ctx: Context,
) -> None:
    clock = itertools.count(1000, 10)

    session = Session(ctx, "s-1", KINDS, clock=lambda: next(clock), created_at=77)

    # The clock is asked for the time of an event, and it has not been asked yet.
    assert session.created_at == 77
    assert session.append(TurnStartEvent(turn=1)).time == 1000


async def test_the_log_keeps_its_own_copy_of_the_data(ctx: Context) -> None:
    session = open_session(ctx)
    data = result({"lines": [1, 2]})

    event = session.append(data)
    cast("dict[str, Any]", data.meta)["lines"].append(3)

    assert data == result({"lines": [1, 2, 3]})
    assert event.data == result({"lines": [1, 2]})


async def test_an_event_of_an_unknown_type_is_rejected(ctx: Context) -> None:
    class Stray(EventData):
        event_type: ClassVar[str] = "test/stray"

    session = open_session(ctx)

    with pytest.raises(InvalidEventError, match="the event type 'test/stray' is not registered"):
        session.append(Stray())

    assert len(session) == 0


async def test_an_event_of_a_class_that_only_shares_the_name_is_rejected(ctx: Context) -> None:
    class Impostor(EventData):
        event_type: ClassVar[str] = "turn/start"

        turn: int

    session = open_session(ctx)

    with pytest.raises(InvalidEventError, match="is registered as TurnStartEvent, not as"):
        session.append(Impostor(turn=1))

    assert len(session) == 0


async def test_data_that_changes_when_written_as_json_is_rejected(ctx: Context) -> None:
    session = open_session(ctx)

    with pytest.raises(InvalidEventError, match="does not survive being written as JSON"):
        session.append(result({"score": float("nan")}))

    assert len(session) == 0


async def test_data_that_cannot_be_written_as_json_is_rejected(ctx: Context) -> None:
    session = open_session(ctx)
    data = result({})
    cast("dict[str, Any]", data.meta)["handle"] = object()

    with pytest.raises(InvalidEventError, match="cannot be written as JSON"):
        session.append(data)

    assert len(session) == 0


async def test_every_committed_event_is_announced(ctx: Context) -> None:
    session = open_session(ctx)
    seen: list[Committed] = []
    ctx.on(SESSION_EVENT, seen.append)

    event = session.append(TurnStartEvent(turn=1))

    assert seen == [Committed(session, event)]


async def test_a_rejected_event_is_not_announced(ctx: Context) -> None:
    session = open_session(ctx)
    seen: list[Committed] = []
    ctx.on(SESSION_EVENT, seen.append)

    with pytest.raises(InvalidEventError):
        session.append(result({"score": float("nan")}))

    assert seen == []


async def test_an_observer_that_fails_does_not_undo_the_event(ctx: Context) -> None:
    session = open_session(ctx)
    seen: list[int] = []

    def broken(committed: Committed) -> None:
        raise RuntimeError("observer bug")

    ctx.on(SESSION_EVENT, broken)
    ctx.on(SESSION_EVENT, lambda committed: seen.append(committed.event.seq))

    event = session.append(TurnStartEvent(turn=1))

    assert list(session.events) == [event]
    assert seen == [0]


async def test_events_appended_by_an_observer_are_announced_in_order(ctx: Context) -> None:
    session = open_session(ctx)
    seen: list[int] = []

    def close_the_turn(committed: Committed) -> None:
        if isinstance(committed.event.data, TurnStartEvent):
            session.append(TurnEndEvent(turn=committed.event.data.turn, reason="completed"))

    ctx.on(SESSION_EVENT, close_the_turn)
    ctx.on(SESSION_EVENT, lambda committed: seen.append(committed.event.seq))

    session.append(TurnStartEvent(turn=1))

    assert [event.type for event in session.events] == ["turn/start", "turn/end"]
    assert seen == [0, 1]


async def test_an_observer_in_another_scope_does_not_hear_the_session(ctx: Context) -> None:
    here = ctx.scope("agent-a")
    there = ctx.scope("agent-b")
    session = open_session(here)
    heard: list[str] = []
    here.on(SESSION_EVENT, lambda committed: heard.append("here"))
    there.on(SESSION_EVENT, lambda committed: heard.append("there"))
    ctx.on(SESSION_EVENT, lambda committed: heard.append("everywhere"))

    session.append(TurnStartEvent(turn=1))

    assert heard == ["here", "everywhere"]


async def test_a_closed_session_takes_no_more_events(ctx: Context) -> None:
    session = open_session(ctx)
    session.append(TurnStartEvent(turn=1))

    session.close()

    assert session.closed
    with pytest.raises(SessionClosedError, match="the session 's-1' is closed"):
        session.append(TurnEndEvent(turn=1, reason="completed"))
    assert len(session) == 1
