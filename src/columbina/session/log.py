"""The session log: an append-only list of typed events."""

from __future__ import annotations

import time
from collections import deque
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import Protocol, TypeVar

from columbina.kernel import Context, EmitEvent
from columbina.llm.types import Message
from columbina.session.errors import InvalidEventError, SessionClosedError
from columbina.session.events import EventData, SessionEvent
from columbina.session.surface import Surface

DataT = TypeVar("DataT", bound=EventData)


def now_ms() -> int:
    """Return the wall-clock time in milliseconds since the epoch."""
    return time.time_ns() // 1_000_000


@dataclass(frozen=True, slots=True)
class Committed:
    """The payload of :data:`SESSION_EVENT`: an event that has just been appended."""

    session: Session
    event: SessionEvent[EventData]


SESSION_EVENT: EmitEvent[Committed] = EmitEvent("session/event")
"""Emitted for every event once it is in the log."""


class SessionStore(Protocol):
    """Where a session keeps its events beyond the life of the process."""

    def write(self, event: SessionEvent[EventData]) -> None:
        """Store ``event``, or raise ``SessionStoreError`` if that cannot be done."""
        ...

    async def flush(self) -> None:
        """Wait until everything written so far is safe from a crash of the machine."""
        ...

    def close(self) -> None:
        """Let go of whatever the store holds."""
        ...


class Session:
    """An append-only log of typed events, the single source of truth of a conversation.

    Everything else about a conversation is derived from the log: the messages
    a model sees, the state a frontend shows, and what a resumed run starts
    from.

    Args:
        ctx: The context through which committed events are announced. Only
            listeners that this context reaches hear the session.
        session_id: The id of the session.
        kinds: The event types that the log accepts, by name.
        clock: Tells the time in milliseconds since the epoch.
        store: Where the events are kept. Without one the session lives in
            memory only.
        events: The events that the session starts with, as read from a store.
            They are not announced.

    Raises:
        InvalidEventError: ``events`` are not numbered from zero without a
            gap, or their messages do not add up.

    Attributes:
        id: The id of the session.
    """

    __slots__ = (
        "__weakref__",
        "_announcing",
        "_clock",
        "_closed",
        "_ctx",
        "_events",
        "_kinds",
        "_outbox",
        "_store",
        "_surface",
        "id",
    )

    def __init__(
        self,
        ctx: Context,
        session_id: str,
        kinds: Mapping[str, type[EventData]],
        *,
        clock: Callable[[], int] = now_ms,
        store: SessionStore | None = None,
        events: Iterable[SessionEvent[EventData]] = (),
    ) -> None:
        self.id = session_id
        self._ctx = ctx
        self._kinds = kinds
        self._clock = clock
        self._store = store
        self._events: list[SessionEvent[EventData]] = []
        self._surface = Surface()
        for event in events:
            if event.seq != len(self._events):
                raise InvalidEventError(
                    f"event {event.seq} stands where event {len(self._events)} belongs"
                )
            self._surface.apply(event)
            self._events.append(event)
        self._outbox: deque[SessionEvent[EventData]] = deque()
        self._announcing = False
        self._closed = False

    def __len__(self) -> int:
        return len(self._events)

    def __repr__(self) -> str:
        return f"<Session {self.id!r} at seq {len(self._events)}>"

    @property
    def events(self) -> Sequence[SessionEvent[EventData]]:
        """The events of the log, in order."""
        return self._events

    @property
    def messages(self) -> tuple[Message, ...]:
        """The messages that the model sees, derived from the log."""
        return self._surface.messages

    @property
    def surface(self) -> tuple[int, ...]:
        """The positions in the log of the events behind :attr:`messages`, in the same order."""
        return self._surface.seqs

    @property
    def replaced(self) -> int:
        """How many times a message has taken the place of others in what the model sees.

        Whoever keeps something in step with the messages can tell by this
        number that some of them are gone, without looking through them. A
        log that is read again comes to the same number.
        """
        return self._surface.replaced

    @property
    def closed(self) -> bool:
        """Whether the session has stopped taking events."""
        return self._closed

    def close(self) -> None:
        """Stop taking events and close the store. The events in the log stay readable."""
        if self._closed:
            return
        self._closed = True
        if self._store is not None:
            self._store.close()

    async def flush(self) -> None:
        """Wait until every event appended so far is safe from a crash of the machine.

        Raises:
            SessionStoreError: The store could not make them safe.
        """
        if self._store is not None:
            await self._store.flush()

    def append(self, data: DataT, *, replaces: Iterable[int] = ()) -> SessionEvent[DataT]:
        """Append an event and announce it.

        The log keeps its own copy of ``data``, read back from the JSON it
        would be stored as. Later changes to ``data`` do not reach the log.
        The copy must be left as it is: it is the log.

        An event that carries a message becomes the last message the model
        sees. With ``replaces`` it instead takes the place of the messages of
        the listed events, which must be neighbours in what the model sees
        now and must be listed in that order. Nothing is removed from the
        log.

        The event is written to the store first. It is in the log, and is
        announced, only if the store took it.

        Observers of :data:`SESSION_EVENT` are told in the order of the log,
        before this method returns. The one exception is an event that an
        observer appends: it is announced after the current one, by which
        time the call of the observer has returned.

        Raises:
            SessionClosedError: The session is closed.
            InvalidEventError: The event type is not registered with this
                session, the data is not the same after a round trip through
                JSON, or ``replaces`` does not fit. The log is unchanged.
            SessionStoreError: The store did not take the event. The log is
                unchanged.
        """
        if self._closed:
            raise SessionClosedError(f"the session {self.id!r} is closed")
        kind = type(data)
        name = kind.event_type
        registered = self._kinds.get(name)
        if registered is None:
            raise InvalidEventError(f"the event type {name!r} is not registered")
        if registered is not kind:
            raise InvalidEventError(
                f"the event type {name!r} is registered as {registered.__qualname__}, "
                f"not as {kind.__qualname__}"
            )
        try:
            snapshot = kind.model_validate_json(data.model_dump_json())
        except ValueError as error:
            raise InvalidEventError(f"a {name} event cannot be written as JSON: {error}") from error
        if snapshot != data:
            raise InvalidEventError(f"a {name} event does not survive being written as JSON")
        replaced = tuple(replaces)
        on_surface = snapshot.for_model() is not None
        if replaced:
            if not on_surface:
                raise InvalidEventError(f"a {name} event is no message and cannot replace any")
            self._surface.locate(replaced)
        event = SessionEvent(
            seq=len(self._events),
            time=self._clock(),
            data=snapshot,
            on_surface=on_surface,
            replaces=replaced,
        )
        if self._store is not None:
            self._store.write(event)
        self._surface.apply(event)
        self._events.append(event)
        self._outbox.append(event)
        self._announce()
        return event

    def _announce(self) -> None:
        """Tell the observers about the events in the outbox, one at a time."""
        if self._announcing:
            return
        self._announcing = True
        try:
            while self._outbox:
                self._ctx.emit(SESSION_EVENT, Committed(self, self._outbox.popleft()))
        finally:
            self._announcing = False
