"""The session service: where sessions are created and reopened."""

import itertools
from pathlib import Path
from typing import ClassVar

import pytest

import columbina.session
from columbina.kernel import Context, Plugin
from columbina.llm import TextBlock, UserMessage
from columbina.session import (
    CORE_EVENTS,
    SESSIONS,
    EventData,
    InvalidEventError,
    SessionClosedError,
    SessionError,
    SessionLockedError,
    Sessions,
    TurnEndEvent,
    TurnStartEvent,
    UnreadableSessionError,
    UserMessageEvent,
)
from columbina.session.store import SESSION_FILE, read_session, workspace_key


class NoteEvent(EventData):
    event_type: ClassVar[str] = "test/note"

    text: str


def said(text: str) -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)))


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    """The session service, storing under a temporary directory, with a clock that ticks."""
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    service = ctx.use(SESSIONS)
    ticks = itertools.count(1_791_234_567_890, 1000)
    service.clock = lambda: next(ticks)
    return service


async def test_the_plugin_provides_the_service_with_the_core_events(ctx: Context) -> None:
    await ctx.plugin(columbina.session)

    sessions = ctx.use(SESSIONS)

    assert list(sessions.kinds.values()) == list(CORE_EVENTS)
    assert sessions.root is None


async def test_without_a_root_a_session_lives_in_memory(ctx: Context, tmp_path: Path) -> None:
    await ctx.plugin(columbina.session)
    sessions = ctx.use(SESSIONS)

    session = sessions.create(ctx, tmp_path)
    session.append(TurnStartEvent(turn=1))
    await session.flush()

    assert len(session) == 1
    assert list(tmp_path.iterdir()) == []
    # It knows when it began all the same.
    assert session.created_at <= session.events[0].time
    with pytest.raises(SessionError, match="sessions are kept in memory and cannot be resumed"):
        sessions.resume(ctx, tmp_path, session.id)


async def test_a_session_is_stored_under_its_working_directory(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    work = tmp_path / "work"

    session = sessions.create(ctx, work, session_id="s-1")
    session.append(TurnStartEvent(turn=1))

    path = tmp_path / "sessions" / workspace_key(work) / "s-1" / SESSION_FILE
    assert path.exists()
    assert sessions.directory(work, "s-1") == path.parent


async def test_a_new_session_gets_an_id_a_time_and_its_working_directory(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    work = tmp_path / "work"

    first = sessions.create(ctx, work)
    second = sessions.create(ctx, work, parent=first.id)
    first.append(TurnStartEvent(turn=1))
    second.append(TurnStartEvent(turn=1))

    assert first.id.startswith("20261005T")
    assert first.id != second.id
    header = sessions.read(work, first.id).header
    assert header.created_at == 1_791_234_567_890
    # The session says the same of itself, each its own time.
    assert (first.created_at, second.created_at) == (1_791_234_567_890, 1_791_234_568_890)
    assert header.cwd == str(work)
    assert header.parent_session is None
    assert sessions.read(work, second.id).header.parent_session == first.id
    assert first.events[0].time == 1_791_234_569_890


async def test_an_id_that_could_leave_the_root_is_refused(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    with pytest.raises(ValueError, match="is not a usable session id"):
        sessions.create(ctx, tmp_path, session_id="../escape")
    with pytest.raises(ValueError, match="is not a usable session id"):
        sessions.resume(ctx, tmp_path, "../escape")
    with pytest.raises(ValueError, match="is not a usable session id"):
        sessions.read(tmp_path, "../escape")


async def test_a_session_can_be_resumed_and_continued(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    first = sessions.create(ctx, tmp_path, session_id="s-1")
    first.append(TurnStartEvent(turn=1))
    first.append(said("hello"))
    first.append(TurnEndEvent(turn=1, reason="completed"))
    first.close()

    second = sessions.resume(ctx, tmp_path, "s-1")
    event = second.append(TurnStartEvent(turn=2))

    assert second.id == "s-1"
    # It was created when it was created, and not when it was taken up.
    assert second.created_at == first.created_at == 1_791_234_567_890
    assert list(second.events[:3]) == list(first.events)
    assert second.messages == first.messages
    assert event.seq == 4
    second.close()
    stored = read_session(sessions.directory(tmp_path, "s-1"), sessions.kinds)
    assert stored.events == list(second.events)


async def test_a_session_that_is_open_cannot_be_resumed(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    session = sessions.create(ctx, tmp_path, session_id="s-1")
    session.append(TurnStartEvent(turn=1))

    with pytest.raises(SessionLockedError):
        sessions.resume(ctx, tmp_path, "s-1")


async def test_a_session_that_is_not_stored_cannot_be_resumed(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    with pytest.raises(UnreadableSessionError, match="there is no session file at"):
        sessions.resume(ctx, tmp_path, "s-1")


async def test_a_session_closes_when_its_owner_unloads(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    opened: list[columbina.session.Session] = []

    def apply(inner: Context, config: object) -> None:
        opened.append(inner.use(SESSIONS).create(inner, tmp_path, session_id="s-1"))

    fiber = await ctx.plugin(Plugin("owner", apply, inject=(SESSIONS,)))
    session = opened[0]
    session.append(TurnStartEvent(turn=1))

    await fiber.dispose()

    assert session.closed
    with pytest.raises(SessionClosedError):
        session.append(TurnEndEvent(turn=1, reason="completed"))
    resumed = sessions.resume(ctx, tmp_path, "s-1")
    assert resumed.events[0].data == TurnStartEvent(turn=1)


async def test_a_plugin_can_bring_event_types_that_leave_with_it(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(SESSIONS).declare(inner, NoteEvent)

    session = sessions.create(ctx, tmp_path, session_id="s-1")
    fiber = await ctx.plugin(Plugin("notes", apply, inject=(SESSIONS,)))

    session.append(NoteEvent(text="kept"))
    await fiber.dispose()

    with pytest.raises(InvalidEventError, match="the event type 'test/note' is not registered"):
        session.append(NoteEvent(text="lost"))
    assert [event.type for event in session.events] == ["test/note"]


async def test_a_session_with_an_event_type_that_is_gone_cannot_be_resumed(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(SESSIONS).declare(inner, NoteEvent)

    fiber = await ctx.plugin(Plugin("notes", apply, inject=(SESSIONS,)))
    session = sessions.create(ctx, tmp_path, session_id="s-1")
    session.append(NoteEvent(text="kept"))
    session.close()
    await fiber.dispose()

    with pytest.raises(UnreadableSessionError, match="the event type 'test/note' is not known"):
        sessions.resume(ctx, tmp_path, "s-1")
