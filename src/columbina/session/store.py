"""Session files: the log of one session as JSON Lines, written by one process at a time."""

from __future__ import annotations

import asyncio
import contextlib
import hashlib
import logging
import os
import re
import secrets
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO

from columbina.session import lock
from columbina.session.codec import (
    FORMAT_VERSION,
    SessionHeader,
    decode_event,
    decode_header,
    encode_event,
    encode_header,
)
from columbina.session.errors import (
    InvalidEventError,
    SessionLockedError,
    SessionStoreError,
    UnreadableSessionError,
)
from columbina.session.events import EventData, SessionEvent
from columbina.session.surface import Surface

logger = logging.getLogger("columbina.session")

SESSION_FILE = f"session.v{FORMAT_VERSION}.jsonl"
"""The name of the file that holds the log, inside the directory of a session."""

LOCK_FILE = "lock"
"""The name of the file whose lock marks a session as open for writing."""

_SESSION_ID = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9._-]{0,98}[A-Za-z0-9])?")


def check_session_id(session_id: str) -> str:
    """Return ``session_id`` if it can safely name a directory.

    Raises:
        ValueError: The id is empty, too long, or holds a character that
            could lead outside the directory of the sessions.
    """
    if _SESSION_ID.fullmatch(session_id) is None:
        raise ValueError(
            f"{session_id!r} is not a usable session id: it may hold up to 100 letters, digits, "
            "dots, dashes and underscores, and must begin and end with a letter or a digit"
        )
    return session_id


def new_session_id(now_ms: int) -> str:
    """Return a fresh session id. Ids sort by the second in which they were made."""
    stamp = time.strftime("%Y%m%dT%H%M%S", time.gmtime(now_ms / 1000))
    return f"{stamp}-{secrets.token_hex(4)}"


def workspace_key(cwd: str | Path) -> str:
    """Return the name of the directory that holds the sessions of one working directory.

    The name ends with a digest of the whole path, so two working directories
    never share a name. Before the digest stands the end of the path, in a form
    that people can read.
    """
    # Not Path.resolve(): the key must not depend on what the file system holds.
    absolute = os.path.normcase(os.path.abspath(cwd))  # noqa: PTH100
    readable = re.sub(r"[^A-Za-z0-9]+", "-", absolute).strip("-")[-71:].lstrip("-")
    digest = hashlib.sha256(absolute.encode()).hexdigest()[:8]
    return f"{readable or 'root'}-{digest}"


@dataclass(frozen=True, slots=True)
class StoredSession:
    """What a session file holds.

    Attributes:
        header: The first line of the file.
        events: The events, in order.
        valid_length: How many bytes of the file hold complete lines. What
            follows them is a line that was never finished.
    """

    header: SessionHeader
    events: list[SessionEvent[EventData]]
    valid_length: int


def read_session(directory: Path, kinds: Mapping[str, type[EventData]]) -> StoredSession:
    """Read the session stored in ``directory``, without opening it for writing.

    A last line without a line break was cut short by a crash. It is not read.

    Raises:
        UnreadableSessionError: There is no session, or reading it would lose
            or distort something: the header is of another format version, a
            line is malformed, an event is of an unknown type that may not be
            skipped, the numbering has a gap, or the messages do not add up.
    """
    path = directory / SESSION_FILE
    try:
        content = path.read_bytes()
    except FileNotFoundError:
        raise UnreadableSessionError(f"there is no session file at {path}") from None
    complete, line_break, _ = content.rpartition(b"\n")
    lines = complete.split(b"\n") if line_break else []
    if not lines:
        raise UnreadableSessionError(f"{path}, line 1: the first line is not a session header")
    try:
        header = decode_header(lines[0])
    except UnreadableSessionError as error:
        raise UnreadableSessionError(f"{path}, line 1: {error}") from error
    events: list[SessionEvent[EventData]] = []
    surface = Surface()
    for number, line in enumerate(lines[1:], start=2):
        try:
            event = decode_event(line, kinds)
            if event.seq != len(events):
                raise UnreadableSessionError(
                    f"the event has seq {event.seq} where {len(events)} belongs"
                )
            surface.apply(event)
        except (UnreadableSessionError, InvalidEventError) as error:
            raise UnreadableSessionError(f"{path}, line {number}: {error}") from error
        events.append(event)
    return StoredSession(header, events, len(complete) + len(line_break))


class SessionFile:
    """The file of one session, open for writing by this process alone.

    A new session leaves nothing on the disk until its first event is written.
    Once a write or a sync has failed, the file takes nothing more: a log with
    a hole in it would be worse than a log that stops.

    Attributes:
        header: The first line of the file.
    """

    __slots__ = ("_closed", "_directory", "_failure", "_file", "_lock", "header")

    def __init__(
        self,
        directory: Path,
        header: SessionHeader,
        file: BinaryIO | None,
        held: lock.FileLock | None,
    ) -> None:
        self.header = header
        self._directory = directory
        self._file = file
        self._lock = held
        self._failure: Exception | None = None
        self._closed = False

    @classmethod
    def create(cls, directory: Path, header: SessionHeader) -> SessionFile:
        """Prepare the file of a new session in ``directory``. Nothing is written yet."""
        return cls(directory, header, None, None)

    @classmethod
    def open(
        cls, directory: Path, kinds: Mapping[str, type[EventData]]
    ) -> tuple[SessionFile, list[SessionEvent[EventData]]]:
        """Open the session stored in ``directory`` to go on writing it.

        A last line that was never finished is cut off the file.

        Returns:
            The file, and the events it holds.

        Raises:
            UnreadableSessionError: See :func:`read_session`.
            SessionLockedError: The session is open for writing already.
        """
        path = directory / SESSION_FILE
        if not path.exists():
            raise UnreadableSessionError(f"there is no session file at {path}")
        held = lock.acquire(directory / LOCK_FILE)
        if held is None:
            raise SessionLockedError(
                f"the session {directory.name!r} is open for writing somewhere else"
            )
        try:
            content = read_session(directory, kinds)
            unfinished = path.stat().st_size - content.valid_length
            if unfinished:
                logger.warning(
                    "cut an unfinished line of %d bytes off the end of %s", unfinished, path
                )
                os.truncate(path, content.valid_length)
            file = path.open("ab")
        except BaseException:
            held.release()
            raise
        return cls(directory, content.header, file, held), content.events

    def write(self, event: SessionEvent[EventData]) -> None:
        """Add the line of ``event`` to the file and hand it to the operating system.

        Raises:
            SessionLockedError: The session is new, and another writer holds
                a session of the same id.
            SessionStoreError: The line could not be written, or an earlier
                failure has spoiled the file.
        """
        self._check()
        try:
            file = self._file or self._begin()
            file.write(encode_event(event).encode() + b"\n")
            file.flush()
        except SessionStoreError as error:
            self._fail(error)
            raise
        except OSError as error:
            self._fail(error)
            raise SessionStoreError(
                f"the session {self.header.id!r} cannot be written: {error}"
            ) from error

    async def flush(self) -> None:
        """Wait until everything written so far is on the disk.

        Raises:
            SessionStoreError: The file could not be synced, or an earlier
                failure has spoiled it.
        """
        self._check()
        if self._file is None:
            return
        try:
            await asyncio.to_thread(os.fsync, self._file.fileno())
        except OSError as error:
            self._fail(error)
            raise SessionStoreError(
                f"the session {self.header.id!r} cannot be synced to the disk: {error}"
            ) from error

    def close(self) -> None:
        """Close the file and let another writer in. Closing a second time does nothing."""
        self._closed = True
        self._release()

    def _check(self) -> None:
        if self._closed:
            raise SessionStoreError(f"the file of the session {self.header.id!r} is closed")
        if self._failure is not None:
            raise SessionStoreError(
                f"the session {self.header.id!r} can no longer be written: {self._failure}"
            ) from self._failure

    def _begin(self) -> BinaryIO:
        """Put a new session on the disk: its directory, its lock and its header."""
        self._directory.mkdir(parents=True, exist_ok=True)
        self._lock = lock.acquire(self._directory / LOCK_FILE)
        if self._lock is None:
            raise SessionLockedError(
                f"the session {self.header.id!r} is open for writing somewhere else"
            )
        path = self._directory / SESSION_FILE
        if path.exists():
            raise SessionStoreError(f"a session {self.header.id!r} is stored already")
        self._file = file = path.open("xb")
        file.write(encode_header(self.header).encode() + b"\n")
        return file

    def _fail(self, error: Exception) -> None:
        self._failure = error
        self._release()

    def _release(self) -> None:
        if self._file is not None:
            with contextlib.suppress(OSError):
                self._file.close()
            self._file = None
        if self._lock is not None:
            self._lock.release()
            self._lock = None
