"""Walking a directory tree for a tool that searches.

A search names one directory and then comes upon many files. Each of them is
an access of its own: the walk asks the seam about every directory before it
goes in and about every file before it hands the file on, so that a rule
which keeps a tool from a file also keeps a search from it. A file is handed
on with the permit that the seam gave for it, and a tool that wants to look
into the file opens it through that. What the seam refuses is counted, and
the tool says how much it was not allowed to see.

A directory is listed while it is held open and after the system has said
that it is where the walk thinks it is, the same way a file is opened. Links
are not followed, whatever they lead to: a tree is walked as it lies on the
disk. Directories that hold the history of a repository are passed over.

The walk looks at one directory at a time, in a worker thread, and decides
about what it found in the event loop. So it can be cancelled between two
directories, and it never asks a restriction from another thread.
"""

import asyncio
from collections import deque
from collections.abc import AsyncGenerator
from dataclasses import dataclass
from pathlib import Path

from columbina.fs import Access, Fs, Located, Permit, bound
from columbina.fs.paths import below, usable
from columbina.session import Session
from columbina.tools import ToolFailure

__all__ = ["SKIPPED", "Found", "Tally", "counted", "remarks", "walk"]

SKIPPED = frozenset({".git"})
"""The names of directories that a walk does not go into."""


@dataclass(frozen=True, slots=True)
class Found:
    """A file that a walk came upon.

    Attributes:
        where: The place of the file.
        below: The names that lead to the file from the directory that is
            walked, the name of the file last.
        size: Its size in bytes.
        modified_ns: When it was last changed, in nanoseconds.
        permit: The permit to read it.
    """

    where: Located
    below: tuple[str, ...]
    size: int
    modified_ns: int
    permit: Permit


@dataclass(slots=True)
class Tally:
    """What a walk passed without handing it on.

    Attributes:
        entries: How many files and directories the walk has looked at.
        refused: How many of them the seam did not let the tool see.
        unreadable: How many directories could not be listed.
        cut_short: Whether the walk stopped because it had looked at as many
            entries as it may.
    """

    entries: int = 0
    refused: int = 0
    unreadable: int = 0
    cut_short: bool = False


def counted(number: int, one: str, many: str) -> str:
    """Return a number with what it counts, in the singular or the plural."""
    return f"1 {one}" if number == 1 else f"{number} {many}"


def remarks(tally: Tally) -> list[str]:
    """Return what a search tells the model about what its walk passed.

    A model takes what a search does not show for what is not there. So a
    search says what it did not look at, one remark to a line.
    """
    said: list[str] = []
    if tally.cut_short:
        said.append(
            f"[The search stopped after {tally.entries} files and directories, so there may "
            "be more. Search a narrower directory.]"
        )
    if tally.refused:
        skipped = counted(tally.refused, "file or directory was", "files and directories were")
        said.append(f"[{skipped} not searched, because this tool may not see them.]")
    if tally.unreadable:
        unlisted = counted(tally.unreadable, "directory", "directories")
        said.append(f"[{unlisted} could not be listed.]")
    return said


async def walk(
    fs: Fs,
    session: Session,
    cwd: Path,
    root: Located,
    tool: str,
    *,
    limit: int,
    tally: Tally,
) -> AsyncGenerator[Found, None]:
    """Hand on the files below a directory that the tool may see, level by level.

    Args:
        fs: The seam, which is asked about every entry.
        session: The session that the search belongs to.
        cwd: The working directory of the agent.
        root: The directory to walk. The caller has a permit for it.
        tool: The name of the tool that searches.
        limit: How many entries the walk looks at before it stops.
        tally: Where the walk writes down what it passed.
    """
    waiting: deque[tuple[Located, tuple[str, ...]]] = deque([(root, ())])
    while waiting:
        directory, names = waiting.popleft()
        try:
            # One entry more than may be looked at, so that it shows if there are more.
            listed, more = await asyncio.to_thread(
                bound.listing, directory.path, limit - tally.entries + 1
            )
        except OSError:
            # It was removed meanwhile, or this user may not list it, or it has moved.
            tally.unreadable += 1
            continue
        for entry in listed:
            if entry.kind not in ("file", "directory"):
                continue
            if entry.kind == "directory" and entry.name in SKIPPED:
                continue
            if tally.entries >= limit:
                tally.cut_short = True
                return
            tally.entries += 1
            if not usable(entry.name):
                # A path that is written with this name leads to another file.
                tally.refused += 1
                continue
            where = below(directory, entry.name)
            led = (*names, entry.name)
            try:
                permit = fs.permit(Access(session, cwd, where, "read", tool, named=False))
            except ToolFailure:
                tally.refused += 1
                continue
            if entry.kind == "directory":
                waiting.append((where, led))
            else:
                yield Found(where, led, entry.size, entry.modified_ns, permit)
        if more:
            # The directory holds entries that were not asked for.
            tally.cut_short = True
            return
