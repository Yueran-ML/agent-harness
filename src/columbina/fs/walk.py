"""Walking a directory tree for a tool that searches.

A search names one directory and then comes upon many files. Each of them is
an access of its own: the walk asks the seam about every directory before it
goes in and about every file before it hands the file on, so that a rule
which keeps a tool from a file also keeps a search from it. What the seam
refuses is counted, and the tool says how much it was not allowed to see.

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
from collections.abc import AsyncIterator
from dataclasses import dataclass
from pathlib import Path

from columbina.fs import Access, Fs, Located, bound
from columbina.session import Session

__all__ = ["SKIPPED", "Found", "Tally", "walk"]

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
    """

    where: Located
    below: tuple[str, ...]
    size: int
    modified_ns: int


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


def _below(parent: Located, name: str) -> Located:
    """Return the place of an entry of a directory that was located already."""
    relative = name if parent.relative == "." else f"{parent.relative}/{name}"
    return Located(parent.path / name, relative, parent.inside, relative)


async def walk(
    fs: Fs,
    session: Session,
    cwd: Path,
    root: Located,
    tool: str,
    *,
    limit: int,
    tally: Tally,
) -> AsyncIterator[Found]:
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
            listed = await asyncio.to_thread(bound.listing, directory.path)
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
            where = _below(directory, entry.name)
            below = (*names, entry.name)
            if fs.judge(Access(session, cwd, where, "read", tool, named=False)) is not None:
                tally.refused += 1
            elif entry.kind == "directory":
                waiting.append((where, below))
            else:
                yield Found(where, below, entry.size, entry.modified_ns)
