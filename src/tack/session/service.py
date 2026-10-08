"""The session service: where sessions are created and reopened."""

import os
from collections.abc import Callable
from pathlib import Path

from tack.kernel import Context, Registry, ServiceKey
from tack.session.codec import SessionHeader
from tack.session.errors import SessionError
from tack.session.events import EventData
from tack.session.log import Session, now_ms
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
        return self._adopt(ctx, Session(ctx, identity, self.kinds, clock=self._now, store=store))

    def resume(self, ctx: Context, cwd: str | Path, session_id: str) -> Session:
        """Reopen a stored session to go on writing it. It belongs to the fiber of ``ctx``.

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
        return self._adopt(ctx, session)

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

    def _adopt(self, ctx: Context, session: Session) -> Session:
        ctx.effect(lambda: session.close)
        return session


SESSIONS = ServiceKey[Sessions]("sessions")
"""The session service."""
