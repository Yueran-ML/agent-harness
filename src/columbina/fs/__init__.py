"""The seam between tools and the files of the machine.

This module is a plugin. Loading it provides :data:`FS`.

A tool that touches a file asks here first, and then touches the file through
the permit that it is given. So one place decides what may be touched, and one
place makes sure that what is touched is what was decided about.

The decision has three parts, taken in this order.

1. Restrictions. Each one may refuse an access or say nothing. All of them are
   asked, and one refusal is enough. Nothing can keep a restriction from being
   asked, and nothing that follows can undo its refusal.
2. The rule of the seam itself: a file in the working directory may be read.
3. Grants. Each one may allow an access or say nothing. One of them is enough.

What neither the rule nor a grant allows is refused. So nothing is written
unless something registered here allows it, and a program that loads no such
thing writes nothing.

Restrictions and grants are plain functions that do not wait. They go by what
is in memory, so that a search can ask them about every file it comes upon.
"""

import logging
import os
import weakref
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import BinaryIO, Literal, cast

from columbina.fs import bound
from columbina.fs.paths import Located, locate
from columbina.kernel import Context, Registry, ServiceKey
from columbina.session import Session
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
    "locate",
    "name",
]

name = "fs"

logger = logging.getLogger(__name__)

Kind = Literal["read", "write"]
"""What an access does to a file."""


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
    """

    session: Session
    cwd: Path
    where: Located
    kind: Kind
    tool: str
    named: bool = True


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

    The file is opened through the permit and in no other way, so that what
    is opened is the file that the decision was about.

    Attributes:
        access: The access that was allowed.
    """

    __slots__ = ("access",)

    def __init__(self, access: Access) -> None:
        self.access = access

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


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


def _key(path: Path) -> str:
    return os.path.normcase(path)


class Fs:
    """Decides what tools may do to files, and remembers what the model was shown.

    Attributes:
        restrictions: The restrictions, by name. Every one is asked about
            every access.
        grants: The grants, by name. They are asked about an access that the
            seam does not allow by itself.
    """

    __slots__ = ("_shown", "grants", "restrictions")

    def __init__(self) -> None:
        self.restrictions: Registry[Restriction] = Registry("restriction of file access")
        self.grants: Registry[Grant] = Registry("grant of file access")
        self._shown: weakref.WeakKeyDictionary[Session, dict[str, Fingerprint]] = (
            weakref.WeakKeyDictionary()
        )

    def judge(self, access: Access) -> Refuse | None:
        """Decide about an access. Return the refusal, or ``None`` if it may happen.

        A restriction that fails refuses: an access that cannot be checked
        does not happen. A grant that fails allows nothing.
        """
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
        return Permit(access)

    def shown(self, permit: Permit, fingerprint: Fingerprint) -> None:
        """Remember that the model was shown a file in this state."""
        access = permit.access
        self._shown.setdefault(access.session, {})[_key(access.where.path)] = fingerprint

    def seen(self, session: Session, path: Path) -> Fingerprint | None:
        """Return the state in which the model was last shown a file, if it was."""
        return self._shown.get(session, {}).get(_key(path))


FS = ServiceKey[Fs]("fs")
"""The seam."""


def apply(ctx: Context, config: object) -> None:
    """Provide the seam."""
    ctx.provide(FS, Fs())
