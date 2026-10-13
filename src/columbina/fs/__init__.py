"""The seam between tools and the files of the machine.

This module is a plugin. Loading it provides :data:`FS`.

A tool that touches a file asks here first, and then touches the file through
the permit that it is given. So one place decides what may be touched, and one
place makes sure that what is touched is what was decided about.

The decision has three parts, taken in this order.

1. Restrictions. Each one may refuse an access or say nothing. They are asked
   one after another, and the first refusal ends the asking. Nothing but a
   refusal can keep a restriction from being asked, and nothing that follows
   can undo a refusal.
2. The rule of the seam itself: a file in the working directory may be read.
3. Grants. Each one may allow an access or say nothing. One of them is enough.

What neither the rule nor a grant allows is refused. So nothing is written
unless something registered here allows it, and a program that loads no such
thing writes nothing.

Restrictions and grants are plain functions that do not wait. They go by what
is in memory, so that a search can ask them about every file it comes upon.

The working directory of a session is where it was when the session first
asked about a file. A path that is checked and a path that is used are both
taken from there, and nothing else ties the two together but that place. If
the directory leads somewhere else later, because it is a link that was
turned or because another directory has taken its name, the session touches
nothing any more: what it would touch is not what it was started in.

The seam also remembers which files the model was shown, and in what state. A
file that is there is replaced only if the model was shown it as it is now:
what the model has not read, or what has changed since, it would overwrite
blindly. The memory belongs to a session and lasts as long as the process, so
a session that is taken up again reads before it writes.

Shown means: before the model when it wrote the call. A model may ask for a
file to be read and to be written in one reply, and it wrote the second call
without having seen the result of the first. So what a call reads counts from
the next step on, which is when its result goes to the model. What the agent
itself writes counts at once: the model knows what is in a file that it has
just written.

Files are written in worker threads, and a thread cannot be cancelled. A call
that writes therefore stays with its thread: see :func:`carry_out`.
"""

import asyncio
import contextlib
import logging
import threading
import weakref
from collections.abc import Callable, Collection
from dataclasses import dataclass, replace
from pathlib import Path
from types import TracebackType
from typing import Any, BinaryIO, Literal, NamedTuple, TypeVar, cast

from columbina.fs import bound
from columbina.fs.paths import Located, locate
from columbina.kernel import Context, Registry, ServiceKey
from columbina.session import SESSION_EVENT, Committed, Session, StepStartEvent
from columbina.tools import ToolFailure

__all__ = [
    "FS",
    "Access",
    "Fingerprint",
    "Fs",
    "Grant",
    "Kind",
    "Located",
    "Opened",
    "Permit",
    "Refuse",
    "Restriction",
    "apply",
    "carry_out",
    "locate",
    "name",
]

name = "fs"

logger = logging.getLogger(__name__)

Kind = Literal["read", "write"]
"""What an access does to a file."""

ResultT = TypeVar("ResultT")

_LET_GO_SECONDS = 5.0
"""How long a caller that was cancelled stays with work that does not end."""


def _forget(working: "asyncio.Future[Any]") -> None:
    """Take note of how work ended that nobody waits for any more."""
    with contextlib.suppress(asyncio.CancelledError):
        working.exception()


async def carry_out(work: Callable[[threading.Event], ResultT]) -> ResultT:
    """Do work on files in a worker thread, and stay until the thread is done.

    A thread cannot be cancelled. If the caller is cancelled, the work is
    told so through the event that it was handed, and the caller stays until
    the thread has ended. So nothing happens to a file after the call that
    asked for it is over: what is written by then is written, and what is
    not will not be.

    Work that does not end within a few seconds is let go of. It has been
    told to give up, and it will, unless it is stuck in the very step that
    writes.

    Args:
        work: What to do. It is handed an event that is set when what it
            does is no longer wanted.
    """
    given_up = threading.Event()
    working = asyncio.ensure_future(asyncio.to_thread(work, given_up))
    try:
        return await asyncio.shield(working)
    except asyncio.CancelledError:
        given_up.set()
        loop = asyncio.get_running_loop()
        until = loop.time() + _LET_GO_SECONDS
        while not working.done() and loop.time() < until:
            try:
                await asyncio.wait({working}, timeout=until - loop.time())
            except asyncio.CancelledError:
                # Cancelled once more. The thread is no further for that.
                continue
        working.add_done_callback(_forget)
        raise


@dataclass(frozen=True, slots=True)
class Access:
    """One access to one file, before it happens.

    Attributes:
        session: The session that the access belongs to.
        cwd: The working directory of the agent.
        where: The file.
        kind: What is done to it.
        tool: The name of the tool that asks.
        named: Whether the call of the tool named this file. It is ``False``
            for a file that a tool comes upon while it searches.
        call_id: The id of the call that the access is made for, if it is
            made for one. What the call shows the model of the file is
            remembered under it, see :meth:`Fs.unshown`.
    """

    session: Session
    cwd: Path
    where: Located
    kind: Kind
    tool: str
    named: bool = True
    call_id: str | None = None


@dataclass(frozen=True, slots=True)
class Refuse:
    """A refusal of an access.

    Attributes:
        reason: What the model reads. It should say what to do instead.
        code: A stable code for programs.
    """

    reason: str
    code: str


Grant = Callable[[Access], bool]
"""Allows accesses that the seam does not allow by itself. It answers ``True`` or says nothing."""

Restriction = Callable[[Access], Refuse | None]
"""Refuses accesses. It answers with a refusal or says nothing, and cannot allow."""


@dataclass(frozen=True, slots=True)
class Fingerprint:
    """What tells one state of a file from another, as far as it can be told cheaply.

    Attributes:
        size: The size of the file in bytes.
        modified_ns: When the file was last changed, in nanoseconds.
    """

    size: int
    modified_ns: int


class Opened:
    """An open file, and what it looked like when it was opened.

    It closes the file when a ``with`` statement is left.

    Attributes:
        file: The file, open for reading bytes.
        fingerprint: The state of the file when it was opened.
    """

    __slots__ = ("file", "fingerprint")

    def __init__(self, file: BinaryIO, fingerprint: Fingerprint) -> None:
        self.file = file
        self.fingerprint = fingerprint

    def __enter__(self) -> "Opened":
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.file.close()


class Permit:
    """An access that was allowed. Only :meth:`Fs.permit` gives one out.

    The file is opened or replaced through the permit and in no other way,
    so that what is touched is the file that the decision was about.

    Attributes:
        access: The access that was allowed.
    """

    __slots__ = ("_fs", "access")

    def __init__(self, access: Access, fs: "Fs") -> None:
        self.access = access
        self._fs = fs

    def open(self) -> Opened:
        """Open the file for reading. This blocks, so it belongs in a worker thread.

        Raises:
            ToolFailure: The file was not where it was when the access was
                decided (code ``MOVED``), or it is no regular file (code
                ``NOT_A_FILE``).
            OSError: The file cannot be opened. The caller says so in words
                that fit what it was doing.
            ValueError: The permit is not for reading.
        """
        if self.access.kind != "read":
            raise ValueError("this permit is not for reading")
        where = self.access.where
        try:
            file, status = bound.open_for_reading(where.path)
        except bound.MovedError:
            raise ToolFailure(
                f"{where.written} was not where it had been a moment before, "
                "so it was not opened. Try again.",
                code="MOVED",
            ) from None
        except bound.NotAFileError:
            raise ToolFailure(f"{where.written} is not a file.", code="NOT_A_FILE") from None
        return Opened(file, Fingerprint(status.st_size, status.st_mtime_ns))

    def replace(
        self,
        data: bytes,
        *,
        made_from: Fingerprint | None = None,
        given_up: threading.Event | None = None,
    ) -> Fingerprint:
        """Put ``data`` in the place of the file, or make the file with it.

        This blocks, so it belongs in a worker thread. The new content takes
        the place of the old in one step, and directories that are missing
        are made. What was written counts as shown to the model.

        Args:
            data: The new content.
            made_from: The state of the file that the new content was made
                from, for a tool that changes a part of a file. The file is
                replaced only if that is the state the model was shown and
                the state it is still in.
            given_up: Set by the caller to say that the file is not to be
                replaced after all. See :func:`carry_out`.

        Returns:
            The state of the content that was written.

        Raises:
            ToolFailure: The file is there and the model was never shown it
                (code ``NOT_READ``), or it has changed since (code
                ``CHANGED_SINCE_READ``). A directory on the way was not where
                it was when the access was decided (code ``MOVED``). The path
                names something that is no file (code ``NOT_A_FILE``).
            bound.GivenUpError: ``given_up`` was set in time. Nothing has
                changed.
            OSError: The file cannot be written. The caller says so in words
                that fit what it was doing.
            ValueError: The permit is not for writing.
        """
        if self.access.kind != "write":
            raise ValueError("this permit is not for writing")
        where = self.access.where
        shown = self._fs.seen(self.access.session, where.path)
        if made_from is not None and shown != made_from:
            # Another call has written the file since this one read it.
            raise self._fs.unread(self.access) if shown is None else _changed(where)
        try:
            status = bound.replace(
                where.path,
                data,
                None if shown is None else (shown.size, shown.modified_ns),
                given_up,
            )
        except bound.MovedError:
            raise ToolFailure(
                f"A directory on the way to {where.written} was not where it had been a "
                "moment before, so nothing was written. Try again.",
                code="MOVED",
            ) from None
        except bound.NotAFileError:
            raise ToolFailure(f"{where.written} is not a file.", code="NOT_A_FILE") from None
        except bound.NotShownError:
            raise self._fs.unread(self.access) from None
        except bound.ChangedError:
            raise _changed(where) from None
        fingerprint = Fingerprint(status.st_size, status.st_mtime_ns)
        self._fs.shown(self, fingerprint)
        return fingerprint


def _not_read(where: Located) -> ToolFailure:
    return ToolFailure(
        f"{where.written} exists and has not been read. Read it before writing it.",
        code="NOT_READ",
    )


def _no_longer_shown(where: Located) -> ToolFailure:
    return ToolFailure(
        f"{where.written} was read, but what was read of it has since been taken out of the "
        "conversation to make it shorter. Read it again before writing it.",
        code="NOT_READ",
    )


def _changed(where: Located) -> ToolFailure:
    return ToolFailure(
        f"{where.written} has changed since it was read. Read it again before writing it.",
        code="CHANGED_SINCE_READ",
    )


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


def _key(path: Path) -> str:
    # The paths come from locate(), which has given them the spelling of the system.
    return str(path)


class _Showing(NamedTuple):
    """A state in which a model was shown a file, and the call that showed it, if one did."""

    fingerprint: Fingerprint
    call_id: str | None


class Fs:
    """Decides what tools may do to files, and remembers what the model was shown.

    Attributes:
        restrictions: The restrictions, by name. They are asked about every
            access, in the order they were added, until one refuses.
        grants: The grants, by name. They are asked about an access that the
            seam does not allow by itself.
        names: What tells the name that the working directory of a session
            has in a place where its commands run, or ``None`` if it has
            its own path there. A path under such a name is read as the
            path of the same file on this machine.
    """

    __slots__ = (
        "_coming",
        "_gone",
        "_lock",
        "_roots",
        "_shown",
        "grants",
        "names",
        "restrictions",
    )

    def __init__(self) -> None:
        self.restrictions: Registry[Restriction] = Registry("restriction of file access")
        self.grants: Registry[Grant] = Registry("grant of file access")
        self.names: Registry[Callable[[Session], str | None]] = Registry(
            "name of the working directory elsewhere"
        )
        self._roots: weakref.WeakKeyDictionary[Session, Path] = weakref.WeakKeyDictionary()
        self._shown: weakref.WeakKeyDictionary[Session, dict[str, _Showing]] = (
            weakref.WeakKeyDictionary()
        )
        # What calls of the step that is under way have read, and the model has yet to see.
        self._coming: weakref.WeakKeyDictionary[Session, dict[str, _Showing]] = (
            weakref.WeakKeyDictionary()
        )
        # The files that the model was shown and that were forgotten since. A file stays
        # among them when it is shown again: nobody asks while it is.
        self._gone: weakref.WeakKeyDictionary[Session, set[str]] = weakref.WeakKeyDictionary()
        # A file is replaced in a worker thread, which reads and writes what was shown.
        self._lock = threading.Lock()

    def locate(self, session: Session, cwd: Path, written: str) -> Located:
        """Find the file that a path means, for a session.

        This is :func:`columbina.fs.locate`, and it also holds the session to
        the place where its working directory was at first.

        Raises:
            ToolFailure: The path is not used here (code ``INVALID_PATH``),
                or the working directory leads somewhere else than it did
                (code ``WORKING_DIRECTORY_MOVED``).
        """
        here = self.here(session, written)
        where = locate(cwd, here)
        moved = self._moved(session, where)
        if moved is not None:
            raise ToolFailure(moved.reason, code=moved.code)
        # What is told back of a path is the path as the model wrote it.
        return where if here == written else replace(where, written=written)

    def here(self, session: Session, written: str) -> str:
        """Return a path as it is written for this machine.

        Where the commands of a session run in another place, that place may
        have the working directory under a name of its own, and a model
        reads paths with that name in what its commands write. Such a path
        becomes the path of the same file from the working directory. Every
        other path is returned as it is, and so is every path where no such
        name is known.
        """
        for _, there in list(self.names.items()):
            name = there(session)
            if name is not None and (written == name or written.startswith(f"{name}/")):
                return written[len(name) :].lstrip("/") or "."
        return written

    def _moved(self, session: Session, where: Located) -> Refuse | None:
        """Refuse a place that was found from another working directory than the first."""
        with self._lock:
            first = self._roots.setdefault(session, where.root)
        if first == where.root:
            return None
        return Refuse(
            f"The working directory was {first} when this session first touched a file, "
            f"and it leads to {where.root} now. Nothing is touched there. Tell the user that "
            "the session has to be started again.",
            "WORKING_DIRECTORY_MOVED",
        )

    def judge(self, access: Access) -> Refuse | None:
        """Decide about an access. Return the refusal, or ``None`` if it may happen.

        A restriction that fails refuses: an access that cannot be checked
        does not happen. A grant that fails allows nothing.
        """
        moved = self._moved(access.session, access.where)
        if moved is not None:
            return moved
        for name, restriction in list(self.restrictions.items()):
            try:
                refusal = cast("object", restriction(access))
            except Exception as error:
                logger.error("the restriction %r of file access failed", name, exc_info=error)
                return Refuse(
                    f"The access could not be checked and did not happen: {_describe(error)}",
                    "CHECK_FAILED",
                )
            if isinstance(refusal, Refuse):
                return refusal
            if refusal is not None:
                logger.error("the restriction %r of file access gave no refusal", name)
                return Refuse(
                    "The access could not be checked and did not happen: "
                    f"the restriction {name!r} gave an answer that is no refusal",
                    "CHECK_FAILED",
                )
        where = access.where
        if access.kind == "read" and where.inside:
            return None
        for name, grant in list(self.grants.items()):
            try:
                if cast("object", grant(access)) is True:
                    return None
            except Exception as error:
                logger.error("the grant %r of file access failed", name, exc_info=error)
        if not where.inside:
            deed = "read" if access.kind == "read" else "written"
            return Refuse(
                f"The path {where.written} is outside the working directory. "
                f"Only files inside it can be {deed}.",
                "OUTSIDE_WORKING_DIRECTORY",
            )
        return Refuse(
            f"The path {where.written} cannot be written: nothing here allows writing files.",
            "NOT_WRITABLE",
        )

    def permit(self, access: Access) -> Permit:
        """Decide about an access, and return the permit to carry it out.

        Raises:
            ToolFailure: The access is refused. The message and the code are
                those of the refusal.
        """
        refusal = self.judge(access)
        if refusal is not None:
            raise ToolFailure(refusal.reason, code=refusal.code)
        return Permit(access, self)

    def shown(self, permit: Permit, fingerprint: Fingerprint) -> None:
        """Remember that the model knows a file in this state, from now on.

        This is for what the agent itself has written. What a call has read
        is told with :meth:`showing`.
        """
        access = permit.access
        key = _key(access.where.path)
        with self._lock:
            self._shown.setdefault(access.session, {})[key] = _Showing(fingerprint, access.call_id)
            # An older state that is on its way to the model is of no use any more.
            self._coming.get(access.session, {}).pop(key, None)

    def showing(self, permit: Permit, fingerprint: Fingerprint) -> None:
        """Remember that a call has read a file in this state for the model.

        It counts as shown once :meth:`delivered` says that the results of
        the step have gone to the model.
        """
        access = permit.access
        with self._lock:
            self._coming.setdefault(access.session, {})[_key(access.where.path)] = _Showing(
                fingerprint, access.call_id
            )

    def delivered(self, session: Session) -> None:
        """Take note that the model has before it what the calls of a session have read so far.

        The seam does this by itself whenever a step of the session begins.
        Whoever calls tools without a loop says it here.
        """
        with self._lock:
            coming = self._coming.pop(session, None)
            if coming:
                self._shown.setdefault(session, {}).update(coming)

    def unshown(self, session: Session, call_ids: Collection[str]) -> int:
        """Forget what some calls showed the model of files, and return how many files that is.

        Whoever takes a call or its result out of what the model sees says so
        here: a result that is cut down, a part of a conversation that a
        summary takes the place of. The model no longer has before it what
        those files hold. They count as files that it has not read, so it
        reads them again before it changes them.

        A file that a later call has shown again stays shown. So does what
        was shown for no call in particular.

        Args:
            session: The session of the calls.
            call_ids: The ids of the calls.
        """
        with self._lock:
            forgotten: set[str] = set()
            for kept in (self._shown.get(session), self._coming.get(session)):
                for key in [key for key, by in (kept or {}).items() if by.call_id in call_ids]:
                    del cast("dict[str, _Showing]", kept)[key]
                    forgotten.add(key)
            # One that is still shown from another call is not gone.
            forgotten -= set(self._shown.get(session, {}))
            if forgotten:
                self._gone.setdefault(session, set()).update(forgotten)
            return len(forgotten)

    def unread(self, access: Access) -> ToolFailure:
        """Return the refusal for a file that the model does not have before it.

        The words tell a file that was never read from one whose reading
        has gone out of the conversation: a model remembers that it read
        the second kind, and would not know what is asked of it.
        """
        with self._lock:
            gone = _key(access.where.path) in self._gone.get(access.session, set())
        return _no_longer_shown(access.where) if gone else _not_read(access.where)

    def seen(self, session: Session, path: Path) -> Fingerprint | None:
        """Return the state in which the model was last shown a file, if it was."""
        with self._lock:
            shown = self._shown.get(session, {}).get(_key(path))
        return None if shown is None else shown.fingerprint

    def must_have_shown(self, permit: Permit, fingerprint: Fingerprint) -> None:
        """Make sure that the model was shown a file in the state that it is in.

        A tool that changes a part of a file asks this as soon as it has
        opened the file. So it says nothing about the content of a file that
        the model has not seen, not even that a text does not occur in it.

        Args:
            permit: The permit that the file was opened through.
            fingerprint: The state of the file when it was opened.

        Raises:
            ToolFailure: The model was never shown the file (code
                ``NOT_READ``), or it was shown it in another state (code
                ``CHANGED_SINCE_READ``).
        """
        access = permit.access
        shown = self.seen(access.session, access.where.path)
        if shown is None:
            raise self.unread(access)
        if shown != fingerprint:
            raise _changed(access.where)


FS = ServiceKey[Fs]("fs")
"""The seam."""


def apply(ctx: Context, config: object) -> None:
    """Provide the seam, and have it told when a step begins."""
    seam = Fs()

    def step_begins(committed: Committed) -> None:
        # The request of this step carries the results of the one before it.
        if isinstance(committed.event.data, StepStartEvent):
            seam.delivered(committed.session)

    ctx.on(SESSION_EVENT, step_begins)
    ctx.provide(FS, seam)
