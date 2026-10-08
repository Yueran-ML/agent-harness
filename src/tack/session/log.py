"""The session log: an append-only list of typed events."""

from __future__ import annotations

import time
from collections import deque
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import TypeVar

from tack.kernel import Context, EmitEvent
from tack.session.errors import InvalidEventError, SessionClosedError
from tack.session.events import EventData, SessionEvent

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

    Attributes:
        id: The id of the session.
    """

    __slots__ = ("_announcing", "_clock", "_closed", "_ctx", "_events", "_kinds", "_outbox", "id")

    def __init__(
        self,
        ctx: Context,
        session_id: str,
        kinds: Mapping[str, type[EventData]],
        *,
        clock: Callable[[], int] = now_ms,
    ) -> None:
        self.id = session_id
        self._ctx = ctx
        self._kinds = kinds
        self._clock = clock
        self._events: list[SessionEvent[EventData]] = []
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
    def closed(self) -> bool:
        """Whether the session has stopped taking events."""
        return self._closed

    def close(self) -> None:
        """Stop taking events. The events that are in the log stay readable."""
        self._closed = True

    def append(self, data: DataT) -> SessionEvent[DataT]:
        """Append an event and announce it.

        The log keeps its own copy of ``data``, read back from the JSON it
        would be stored as. Later changes to ``data`` do not reach the log.

        Observers of :data:`SESSION_EVENT` are told before this method
        returns, in the order of the log. An event that an observer appends
        is announced after the current one.

        Raises:
            SessionClosedError: The session is closed.
            InvalidEventError: The event type is not registered with this
                session, or the data is not the same after a round trip
                through JSON. The log is unchanged.
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
        event = SessionEvent(seq=len(self._events), time=self._clock(), data=snapshot)
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
