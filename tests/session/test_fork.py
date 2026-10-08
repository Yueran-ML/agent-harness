"""Resuming a session after a crash, and forking a session."""

import itertools
from pathlib import Path
from typing import ClassVar

import pytest

import tack.session
from tack.kernel import Context, Plugin
from tack.llm import (
    AssistantMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from tack.session import (
    SESSION_EVENT,
    SESSIONS,
    AssistantMessageEvent,
    Committed,
    EndSeedEvent,
    EventData,
    Sessions,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UnknownEvent,
    UserMessageEvent,
)
from tack.session.store import SESSION_FILE

ASKING = AssistantMessageEvent(
    message=AssistantMessage(
        content=(ToolCallBlock(call_id="call-1", name="read", arguments="{}"),)
    ),
    usage=Usage(),
    finish_reason="tool-calls",
)
ANSWERED = ToolResultEvent(
    message=ToolMessage(call_id="call-1", content=(TextBlock(text="hello"),))
)


class HintEvent(EventData):
    event_type: ClassVar[str] = "test/hint"
    ignorable: ClassVar[bool] = True

    text: str


def said(text: str) -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)))


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    """The session service, storing under a temporary directory, with a clock that ticks."""
    await ctx.plugin(tack.session, {"root": tmp_path / "sessions"})
    service = ctx.use(SESSIONS)
    ticks = itertools.count(1_791_234_567_890, 1000)
    service.clock = lambda: next(ticks)
    return service


def one_turn(sessions: Sessions, ctx: Context, cwd: Path, session_id: str = "s-1") -> None:
    """Store a session with one complete turn: seqs 0 to 7."""
    session = sessions.create(ctx, cwd, session_id=session_id)
    for data in (
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        said("Read a.txt."),
        ASKING,
        ToolCallEvent(call_id="call-1", name="read", arguments="{}"),
        ANSWERED,
        StepEndEvent(turn=1, step=1),
        TurnEndEvent(turn=1, reason="completed"),
    ):
        session.append(data)
    session.close()


# Resuming


async def test_resuming_marks_where_this_lifetime_begins(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)

    session = sessions.resume(ctx, tmp_path, "s-1")

    assert [event.type for event in session.events[8:]] == ["session/end-seed"]
    assert session.events[8].data == EndSeedEvent(inherited=False)


async def test_resuming_closes_a_turn_that_a_crash_left_open(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    crashed = sessions.create(ctx, tmp_path, session_id="s-1")
    for data in (
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        said("Read a.txt."),
        ASKING,
        ToolCallEvent(call_id="call-1", name="read", arguments="{}"),
    ):
        crashed.append(data)
    crashed.close()

    session = sessions.resume(ctx, tmp_path, "s-1")

    assert [event.type for event in session.events[5:]] == [
        "session/end-seed",
        "tool/result",
        "step/end",
        "turn/end",
    ]
    result = session.events[6].data
    assert isinstance(result, ToolResultEvent)
    assert result.error is not None
    assert result.error.code == "TOOL_OUTCOME_UNKNOWN"
    assert session.events[8].data == TurnEndEvent(turn=1, reason="interrupted")
    assert session.messages[-1] == result.message
    stored = sessions.read(tmp_path, "s-1")
    assert stored.events == list(session.events)


async def test_what_resuming_appends_is_announced(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    crashed = sessions.create(ctx, tmp_path, session_id="s-1")
    crashed.append(TurnStartEvent(turn=1))
    crashed.close()
    heard: list[str] = []
    ctx.on(SESSION_EVENT, lambda committed: heard.append(committed.event.type))

    sessions.resume(ctx, tmp_path, "s-1")

    assert heard == ["session/end-seed", "turn/end"]


# Forking


async def test_a_fork_starts_with_the_events_of_its_parent(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)
    parent = sessions.read(tmp_path, "s-1")

    fork = sessions.fork(ctx, tmp_path, "s-1", session_id="s-2")

    assert fork.id == "s-2"
    assert list(fork.events[:8]) == parent.events
    assert [event.type for event in fork.events[8:]] == ["session/end-seed"]
    assert fork.events[8].data == EndSeedEvent(inherited=True)
    assert fork.messages == tuple(
        message for event in parent.events if (message := event.data.for_model()) is not None
    )


async def test_a_fork_is_stored_as_a_session_of_its_own(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)
    before = (sessions.directory(tmp_path, "s-1") / SESSION_FILE).read_bytes()

    fork = sessions.fork(ctx, tmp_path, "s-1", session_id="s-2")
    fork.append(TurnStartEvent(turn=2))
    fork.close()

    stored = sessions.read(tmp_path, "s-2")
    assert stored.header.id == "s-2"
    assert stored.header.parent_session == "s-1"
    assert stored.events == list(fork.events)
    assert (sessions.directory(tmp_path, "s-1") / SESSION_FILE).read_bytes() == before


async def test_a_fork_can_stop_at_an_earlier_event(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)

    fork = sessions.fork(ctx, tmp_path, "s-1", upto=4)

    assert [event.type for event in fork.events] == [
        "turn/start",
        "step/start",
        "user/message",
        "assistant/message",
        "tool/call",
        "session/end-seed",
        "tool/result",
        "step/end",
        "turn/end",
    ]
    result = fork.events[6].data
    assert isinstance(result, ToolResultEvent)
    assert result.error is not None
    assert result.error.code == "TOOL_OUTCOME_UNKNOWN"
    assert result.message.content[0].text.startswith("The conversation was forked")
    assert fork.events[8].data == TurnEndEvent(turn=1, reason="forked")


@pytest.mark.parametrize("upto", [-1, 8, 100])
async def test_a_fork_cannot_stop_at_an_event_that_is_not_there(
    ctx: Context, sessions: Sessions, tmp_path: Path, upto: int
) -> None:
    one_turn(sessions, ctx, tmp_path)

    with pytest.raises(ValueError, match=f"the session 's-1' has no event {upto}"):
        sessions.fork(ctx, tmp_path, "s-1", upto=upto)

    stored = sessions.directory(tmp_path, "s-1").parent
    assert [directory.name for directory in stored.iterdir()] == ["s-1"]


async def test_a_session_that_is_open_can_be_forked(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    parent = sessions.create(ctx, tmp_path, session_id="s-1")
    parent.append(TurnStartEvent(turn=1))
    parent.append(said("hello"))

    fork = sessions.fork(ctx, tmp_path, parent)
    parent.append(TurnEndEvent(turn=1, reason="completed"))

    assert fork.id != parent.id
    assert [event.type for event in fork.events] == [
        "turn/start",
        "user/message",
        "session/end-seed",
        "turn/end",
    ]
    assert fork.events[3].data == TurnEndEvent(turn=1, reason="forked")
    assert sessions.read(tmp_path, fork.id).header.parent_session == "s-1"
    assert len(parent) == 3


async def test_only_what_a_fork_adds_is_announced(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)
    heard: list[tuple[str, int]] = []

    def listen(committed: Committed) -> None:
        heard.append((committed.session.id, committed.event.seq))

    ctx.on(SESSION_EVENT, listen)

    sessions.fork(ctx, tmp_path, "s-1", session_id="s-2")

    assert heard == [("s-2", 8)]


async def test_a_fork_works_without_a_store(ctx: Context, tmp_path: Path) -> None:
    await ctx.plugin(tack.session)
    sessions = ctx.use(SESSIONS)
    parent = sessions.create(ctx, tmp_path)
    parent.append(TurnStartEvent(turn=1))

    fork = sessions.fork(ctx, tmp_path, parent)

    assert [event.type for event in fork.events] == ["turn/start", "session/end-seed", "turn/end"]


async def test_a_fork_keeps_events_that_nothing_here_understands(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(SESSIONS).declare(inner, HintEvent)

    hints = await ctx.plugin(Plugin("hints", apply, inject=(SESSIONS,)))
    parent = sessions.create(ctx, tmp_path, session_id="s-1")
    parent.append(HintEvent(text="remember this"))
    parent.close()
    await hints.dispose()

    fork = sessions.fork(ctx, tmp_path, "s-1", session_id="s-2")
    fork.close()

    assert isinstance(fork.events[0].data, UnknownEvent)
    lines = [
        (sessions.directory(tmp_path, name) / SESSION_FILE).read_bytes().split(b"\n")[1]
        for name in ("s-1", "s-2")
    ]
    assert lines[0] == lines[1]
    assert b'"ignorable":true' in lines[0]


async def test_a_fork_closes_with_its_owner(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)
    made: list[tack.session.Session] = []

    def apply(inner: Context, config: object) -> None:
        made.append(inner.use(SESSIONS).fork(inner, tmp_path, "s-1", session_id="s-2"))

    owner = await ctx.plugin(Plugin("owner", apply, inject=(SESSIONS,)))

    await owner.dispose()

    assert made[0].closed


async def test_a_fork_keeps_a_line_it_does_not_understand_byte_for_byte(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    one_turn(sessions, ctx, tmp_path)
    path = sessions.directory(tmp_path, "s-1") / SESSION_FILE
    foreign = (
        b'{"type":"later/idea", "seq":8,"time":5,'
        b'"data":{"word":"\\u0061","twice":1,"twice":2},"ignorable":true}'
    )
    path.write_bytes(path.read_bytes() + foreign + b"\n")

    fork = sessions.fork(ctx, tmp_path, "s-1", session_id="s-2")
    fork.close()

    copied = (sessions.directory(tmp_path, "s-2") / SESSION_FILE).read_bytes().split(b"\n")
    assert copied[9] == foreign
