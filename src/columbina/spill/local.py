"""Text that is too large for a conversation, kept in files under the home of the program.

This module is a plugin. Loading it provides :data:`columbina.spill.SPILL`.

A file lies under the directory of its working directory and of its session,
with a name that begins with a few letters drawn at random. It is made anew
and never written over, and where the system knows such a thing, only its
owner gets at it: what a command printed may be a secret.

The files are outside the working directory, where the tools read nothing.
So the plugin tells the seam of the file system, if the program has one,
that what was saved for a working directory may be read from there. It may
not be written, and what was saved for another working directory stays out
of reach.

What is saved is read with the tool that reads files, which shows a line up
to a length and counts the rest. A text is therefore saved with its longer
lines broken at that length. What comes in pieces is saved as it comes.

Files that have not been touched for a month are removed when the program
starts. Nothing else of what the directory holds is touched, and no link is
followed.
"""

import asyncio
import contextlib
import logging
import os
import re
import secrets
import time
from pathlib import Path

from pydantic import BaseModel, Field

from columbina.fs import FS, Access
from columbina.kernel import Context
from columbina.session import Session
from columbina.session.store import workspace_key
from columbina.spill import SPILL, Saved

__all__ = ["FOLD_AT", "Config", "Local", "apply", "name", "sweep"]

name = "spill"

logger = logging.getLogger(__name__)

FOLD_AT = 2000
"""How long a line of a saved text may be. It is what the read tool shows of one line."""

_HINT = "Read a part of it with the read tool, by offset and limit, or search it with grep."
_FOLDED = f" Lines of more than {FOLD_AT} characters are broken there."
_UNFIT = re.compile(r"[^A-Za-z0-9._-]")
_NAME_CHARACTERS = 80
_DRAWS = 5
_DAY = 24 * 60 * 60


def _drawn() -> str:
    """Return the letters that a name of a file begins with, drawn at random."""
    return secrets.token_hex(4)


def _put(descriptor: int, data: memoryview) -> int:
    """Hand data to the system for a file, and return how much of it the system took."""
    return os.write(descriptor, data)


def _fit(name: str) -> str:
    """Return a name that is one name of a file, whatever was asked for."""
    fit = _UNFIT.sub("_", name)[:_NAME_CHARACTERS]
    # A name of nothing but dots and dashes names nothing.
    return fit if re.search("[A-Za-z0-9]", fit) else "output"


def _folded(text: str) -> tuple[str, bool]:
    """Return a text with its long lines broken, and whether any was."""
    lines = text.split("\n")
    if all(len(line) <= FOLD_AT for line in lines):
        return text, False
    pieces = (line[at : at + FOLD_AT] for line in lines for at in range(0, len(line) or 1, FOLD_AT))
    return "\n".join(pieces), True


class _File:
    """A file that is being written as its content comes."""

    __slots__ = ("_descriptor", "_saved", "_written", "locator")

    def __init__(self, path: Path) -> None:
        # Made anew, or not at all. What is there is somebody else's.
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
        self._descriptor = os.open(path, flags, 0o600)
        self.locator = str(path)
        self._written = 0
        self._saved: Saved | None = None

    def write(self, data: bytes) -> None:
        if self._saved is not None:
            raise ValueError(f"{self.locator} is closed")
        view = memoryview(data)
        while view:
            view = view[_put(self._descriptor, view) :]
        self._written += len(data)

    def close(self, hint: str = _HINT) -> Saved:
        if self._saved is None:
            os.close(self._descriptor)
            self._saved = Saved(self.locator, self._written, hint)
        return self._saved


class Local:
    """Keeps what is too large for a conversation in files.

    Args:
        root: The directory under which everything is kept.
    """

    __slots__ = ("root",)

    def __init__(self, root: Path) -> None:
        self.root = root

    def directory(self, cwd: Path) -> Path:
        """Return the directory that holds what was saved for a working directory."""
        return self.root / workspace_key(cwd)

    def save_text(self, session: Session, cwd: Path, name: str, text: str) -> Saved:
        """Save a text, with its long lines broken. This blocks."""
        folded, broken = _folded(text)
        file = self.open(session, cwd, name)
        try:
            file.write(folded.encode("utf-8", errors="replace"))
        except BaseException:
            # Half a text is of no use to anybody, and nobody would be told of it.
            file.close()
            with contextlib.suppress(OSError):
                Path(file.locator).unlink()
            raise
        return file.close(_HINT + _FOLDED if broken else _HINT)

    def open(self, session: Session, cwd: Path, name: str) -> _File:
        """Make a file for something that comes in pieces."""
        directory = self.directory(cwd) / session.id
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        for _ in range(_DRAWS):
            with contextlib.suppress(FileExistsError):
                return _File(directory / f"{_drawn()}-{_fit(name)}")
        raise OSError(f"no free name was found for a file in {directory}")

    def grants(self, access: Access) -> bool:
        """Tell whether an access reads what was saved for its working directory."""
        if access.kind != "read":
            return False
        # The place is asked for anew each time: it may not have been there a moment ago.
        directory = Path(os.path.realpath(self.directory(access.cwd)))
        return directory == access.where.path or directory in access.where.path.parents


def sweep(root: Path, *, keep_days: float) -> int:
    """Remove the saved files that were last written longer ago than they are kept.

    A file counts if it lies where saved text lies: in the directory of a
    session, in that of a working directory, under the root. Nothing else is
    touched and no link is followed. A directory that is left empty is
    removed as well, except the root.

    Returns:
        How many files were removed.
    """
    before = time.time() - keep_days * _DAY
    removed = 0
    for workspace in _directories(root):
        for session in _directories(workspace):
            for entry in _entries(session):
                with contextlib.suppress(OSError):
                    status = entry.stat(follow_symlinks=False)
                    if entry.is_file(follow_symlinks=False) and status.st_mtime < before:
                        Path(entry.path).unlink()
                        removed += 1
            with contextlib.suppress(OSError):
                Path(session).rmdir()
        with contextlib.suppress(OSError):
            Path(workspace).rmdir()
    return removed


def _entries(directory: str | Path) -> list["os.DirEntry[str]"]:
    try:
        with os.scandir(directory) as found:
            return list(found)
    except OSError:
        return []


def _directories(directory: str | Path) -> list[str]:
    return [entry.path for entry in _entries(directory) if entry.is_dir(follow_symlinks=False)]


class Config(BaseModel):
    """The config of the files that keep what is too large for a conversation.

    Attributes:
        root: The directory under which everything is kept.
        keep_days: How many days a file is kept after it was written.
    """

    root: Path
    keep_days: float = Field(default=30, gt=0)


async def _tidy(root: Path, keep_days: float) -> None:
    try:
        removed = await asyncio.to_thread(sweep, root, keep_days=keep_days)
    except Exception:
        logger.exception("the old files under %s could not be looked through", root)
    else:
        if removed:
            logger.info("removed %d saved files that were older than %g days", removed, keep_days)


def apply(ctx: Context, config: Config) -> None:
    """Provide the place, let the model read from it, and remove what is old."""
    store = Local(config.root)
    seam = ctx.get(FS)
    if seam is not None:
        seam.grants.add(ctx, "spill", store.grants)
    tidying = asyncio.ensure_future(_tidy(config.root, config.keep_days))

    def leave_it() -> None:
        tidying.cancel()

    ctx.effect(lambda: leave_it)
    ctx.provide(SPILL, store)
