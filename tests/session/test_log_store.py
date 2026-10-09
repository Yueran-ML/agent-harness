"""A session and its store: every event is written before it counts."""

from pathlib import Path

import pytest

from columbina.kernel import Context
from columbina.llm import TextBlock, UserMessage
from columbina.session import (
    CORE_EVENTS,
    SESSION_EVENT,
    EventData,
    InvalidEventError,
    Session,
    SessionEvent,
    SessionHeader,
    SessionStoreError,
    StepStartEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.session.store import SessionFile, read_session

KINDS: dict[str, type[EventData]] = {kind.event_type: kind for kind in CORE_EVENTS}


def said(text: str) -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)))


class Recorder:
    """A store that remembers what it was asked to do."""

    def __init__(self, journal: list[str]) -> None:
        self.journal = journal
        self.refuse = False

    def write(self, event: SessionEvent[EventData]) -> None:
        if self.refuse:
            raise SessionStoreError("the disk is full")
        self.journal.append(f"write {event.seq}")

    async def flush(self) -> None:
        self.journal.append("flush")

    def close(self) -> None:
        self.journal.append("close")


async def test_an_event_is_written_before_it_is_announced(ctx: Context) -> None:
    journal: list[str] = []
    session = Session(ctx, "s-1", KINDS, store=Recorder(journal))
    ctx.on(SESSION_EVENT, lambda committed: journal.append(f"announce {committed.event.seq}"))

    session.append(TurnStartEvent(turn=1))
    session.append(StepStartEvent(turn=1, step=1))

    assert journal == ["write 0", "announce 0", "write 1", "announce 1"]


async def test_an_event_that_cannot_be_written_does_not_count(ctx: Context) -> None:
    journal: list[str] = []
    store = Recorder(journal)
    session = Session(ctx, "s-1", KINDS, store=store)
    ctx.on(SESSION_EVENT, lambda committed: journal.append(f"announce {committed.event.seq}"))
    session.append(said("first"))
    store.refuse = True

    with pytest.raises(SessionStoreError, match="the disk is full"):
        session.append(said("second"))

    assert len(session) == 1
    assert session.surface == (0,)
    assert journal == ["write 0", "announce 0"]


async def test_a_rejected_event_does_not_reach_the_store(ctx: Context) -> None:
    journal: list[str] = []
    session = Session(ctx, "s-1", KINDS, store=Recorder(journal))

    with pytest.raises(InvalidEventError):
        session.append(said("summary"), replaces=[5])

    assert journal == []


async def test_a_barrier_is_passed_on_to_the_store(ctx: Context) -> None:
    journal: list[str] = []
    session = Session(ctx, "s-1", KINDS, store=Recorder(journal))

    await session.flush()

    assert journal == ["flush"]


async def test_a_session_without_a_store_has_nothing_to_sync(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)

    await session.flush()

    assert not session.closed


async def test_closing_a_session_closes_its_store_once(ctx: Context) -> None:
    journal: list[str] = []
    session = Session(ctx, "s-1", KINDS, store=Recorder(journal))

    session.close()
    session.close()

    assert journal == ["close"]


async def test_a_session_can_start_from_events_that_were_read(ctx: Context) -> None:
    journal: list[str] = []
    read = [
        SessionEvent(seq=0, time=1, data=TurnStartEvent(turn=1)),
        SessionEvent(seq=1, time=2, data=said("hello"), on_surface=True),
        SessionEvent(seq=2, time=3, data=said("how are you"), on_surface=True),
        SessionEvent(seq=3, time=4, data=said("a greeting"), on_surface=True, replaces=(1, 2)),
    ]
    ctx.on(SESSION_EVENT, lambda committed: journal.append(f"announce {committed.event.seq}"))

    session = Session(ctx, "s-1", KINDS, store=Recorder(journal), events=read)

    assert list(session.events) == read
    assert session.surface == (3,)
    assert journal == []

    event = session.append(TurnEndEvent(turn=1, reason="completed"))

    assert event.seq == 4
    assert journal == ["write 4", "announce 4"]


async def test_events_that_were_read_must_be_numbered_from_zero(ctx: Context) -> None:
    read = [SessionEvent(seq=1, time=1, data=TurnStartEvent(turn=1))]

    with pytest.raises(InvalidEventError, match="event 1 stands where event 0 belongs"):
        Session(ctx, "s-1", KINDS, events=read)


async def test_a_session_with_a_file_can_be_read_back(ctx: Context, tmp_path: Path) -> None:
    header = SessionHeader(id="s-1", created_at=1, cwd="/work")
    session = Session(ctx, "s-1", KINDS, store=SessionFile.create(tmp_path / "s-1", header))

    session.append(TurnStartEvent(turn=1))
    session.append(said("hello"))
    session.append(said("how are you"))
    session.append(said("a greeting"), replaces=[1, 2])
    await session.flush()
    session.close()

    stored = read_session(tmp_path / "s-1", KINDS)
    assert stored.events == list(session.events)
    again = Session(ctx, "s-1", KINDS, events=stored.events)
    assert again.messages == session.messages
    assert again.surface == session.surface
