"""The session service: where sessions are created and reopened."""

import os
from collections.abc import Callable, Sequence
from pathlib import Path

from tack.kernel import Context, Registry, ServiceKey
from tack.session.codec import SessionHeader
from tack.session.errors import SessionError
from tack.session.events import EndSeedEvent, EventData, SessionEvent
from tack.session.log import Session, now_ms
from tack.session.repair import CloseReason, closing_events
from tack.session.store import (
    SessionFile,
    StoredSession,
    check_session_id,
    new_session_id,
    read_session,
    workspace_key,
)


class Sessions:
    """Creates sessions and reopens stored ones.

    Attributes:
        kinds: The event types that sessions accept, by name.
        root: The directory under which sessions are stored, or ``None`` when
            sessions live in memory only.
        clock: Tells the time in milliseconds since the epoch.
    """

    __slots__ = ("clock", "kinds", "root")

    def __init__(self, root: Path | None) -> None:
        self.kinds: Registry[type[EventData]] = Registry("event type")
        self.root = root
        self.clock: Callable[[], int] = now_ms

    def declare(self, ctx: Context, *kinds: type[EventData]) -> None:
        """Register event types. They leave when the fiber of ``ctx`` unloads.

        Raises:
            RegistryConflictError: A type of the same name is registered.
        """
        for kind in kinds:
            self.kinds.add(ctx, kind.event_type, kind)

    def directory(self, cwd: str | Path, session_id: str) -> Path:
        """Return the directory that holds, or would hold, the session ``session_id``.

        Raises:
            SessionError: Sessions are kept in memory.
            ValueError: The id cannot name a directory.
        """
        if self.root is None:
            raise SessionError(
                "sessions are kept in memory and cannot be resumed or read back, "
                "because the session plugin was given no root"
            )
        return self.root / workspace_key(cwd) / check_session_id(session_id)

    def create(
        self,
        ctx: Context,
        cwd: str | Path,
        *,
        session_id: str | None = None,
        parent: str | None = None,
    ) -> Session:
        """Start a session that belongs to the fiber of ``ctx``.

        The session announces its events through ``ctx`` and closes when that
        fiber unloads. With a root, it is stored under ``cwd`` from its first
        event on.

        Args:
            ctx: The context of the owner.
            cwd: The working directory that the session belongs to.
            session_id: The id of the session. One is made up by default.
            parent: The id of the session that this one continues from.

        Raises:
            ValueError: The id cannot name a directory.
        """
        return self._start(ctx, cwd, session_id, parent, ())

    def fork(
        self,
        ctx: Context,
        cwd: str | Path,
        source: Session | str,
        *,
        upto: int | None = None,
        session_id: str | None = None,
    ) -> Session:
        """Start a session that continues from a copy of another one.

        The new session begins with the events of ``source``, which are copied
        as they are and not announced. After them it gets a
        ``session/end-seed`` event, and a turn that the copy leaves open is
        closed with the reason ``forked``. The source is not changed.

        Args:
            ctx: The context of the owner of the new session.
            cwd: The working directory that both sessions belong to.
            source: The session to copy: one that is open, or the id of a
                stored one.
            upto: The position of the last event to copy. By default every
                event is copied.
            session_id: The id of the new session. One is made up by default.

        Raises:
            ValueError: ``upto`` is not a position in the source, or an id
                cannot name a directory.
            UnreadableSessionError: The stored source cannot be loaded.
        """
        events: Sequence[SessionEvent[EventData]]
        if isinstance(source, Session):
            parent, events = source.id, source.events
        else:
            parent, events = source, self.read(cwd, source).events
        if upto is not None:
            if not 0 <= upto < len(events):
                raise ValueError(f"the session {parent!r} has no event {upto}")
            events = events[: upto + 1]
        session = self._start(ctx, cwd, session_id, parent, events)
        self._take_over(session, inherited=True, reason="forked")
        return session

    def resume(self, ctx: Context, cwd: str | Path, session_id: str) -> Session:
        """Reopen a stored session to go on writing it. It belongs to the fiber of ``ctx``.

        The session gets a ``session/end-seed`` event, which marks where this
        lifetime begins. A turn that a crash left open is closed with the
        reason ``interrupted``.

        Raises:
            SessionError: Sessions are kept in memory.
            UnreadableSessionError: There is no such session, or it cannot be
                loaded without losing something.
            SessionLockedError: The session is open for writing already.
            ValueError: The id cannot name a directory.
        """
        store, events = SessionFile.open(self.directory(cwd, session_id), self.kinds)
        session = Session(
            ctx, store.header.id, self.kinds, clock=self._now, store=store, events=events
        )
        self._adopt(ctx, session)
        self._take_over(session, inherited=False, reason="interrupted")
        return session

    def read(self, cwd: str | Path, session_id: str) -> StoredSession:
        """Read a stored session without opening it for writing.

        Raises:
            SessionError: Sessions are kept in memory.
            UnreadableSessionError: There is no such session, or it cannot be
                loaded without losing something.
            ValueError: The id cannot name a directory.
        """
        return read_session(self.directory(cwd, session_id), self.kinds)

    def _now(self) -> int:
        return self.clock()

    def _start(
        self,
        ctx: Context,
        cwd: str | Path,
        session_id: str | None,
        parent: str | None,
        events: Sequence[SessionEvent[EventData]],
    ) -> Session:
        """Start a session that begins with ``events``, and store those first."""
        created = self.clock()
        identity = new_session_id(created) if session_id is None else check_session_id(session_id)
        store: SessionFile | None = None
        if self.root is not None:
            header = SessionHeader(
                id=identity,
                created_at=created,
                cwd=os.path.abspath(cwd),  # noqa: PTH100
                parent_session=parent,
            )
            store = SessionFile.create(self.directory(cwd, identity), header)
            for event in events:
                store.write(event)
        session = Session(ctx, identity, self.kinds, clock=self._now, store=store, events=events)
        return self._adopt(ctx, session)

    def _take_over(self, session: Session, *, inherited: bool, reason: CloseReason) -> None:
        """Mark what ``session`` holds as taken over, and close a turn that it leaves open."""
        session.append(EndSeedEvent(inherited=inherited))
        for data in closing_events(session.events, reason):
            session.append(data)

    def _adopt(self, ctx: Context, session: Session) -> Session:
        ctx.effect(lambda: session.close)
        return session


SESSIONS = ServiceKey[Sessions]("sessions")
"""The session service."""
