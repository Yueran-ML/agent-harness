"""Programs that this one runs by their names, and where they may come from.

The system would find a program by its name, and on Windows it looks in the
directory that this program was started in before it looks anywhere else.
That directory is often a repository. So the looking is done here,
:func:`found`: only the entries of the search path that are whole paths
count, and none that lies in a working directory. A program is run by its
name many times, and a name may be a link, so the whole way from the name to
the file is walked, :func:`trusted`: a link or a directory on that way that
lies in a working directory could be changed by a command that runs there,
which would then choose what this machine runs next.
"""

import os
import sys
from collections.abc import Sequence
from pathlib import Path

__all__ = ["found", "trusted"]

_HOPS = 40
"""How many links are followed on the way from the name of a program to its file."""

_STEPS_BACK_FIRST = sys.platform == "win32"
"""Whether the system takes the steps back out of a path before it looks at anything.

Windows does: a path that goes into a link and one step back names the directory that the
link lies in, whatever the link leads to. The other systems go where the link leads and take
the directory above that.
"""


def _plain(target: str) -> str:
    """Return what a link leads to as a plain path.

    Windows tells of some links in the long form of a path, which begins
    with a mark of its own. It names the same place.
    """
    if target.startswith("\\\\?\\UNC\\"):
        return "\\\\" + target[8:]
    if target.startswith("\\\\?\\"):
        return target[4:]
    return target


def _leads(here: Path) -> str | None:
    """Return what a link says it leads to, or ``None`` for a name that is no link."""
    try:
        return _plain(str(here.readlink()))
    except (OSError, ValueError):
        # A file or a directory, or nothing. Windows also has names that are neither a
        # plain file nor a link, such as a file that is fetched only when it is opened, and
        # of those Python says so in a way of its own.
        return None


def _itself(place: Path) -> tuple[int, int] | None:
    """Return the numbers by which the system tells what a path leads to, or ``None``.

    They are its device and its inode, the same under every name that it goes
    by. ``None`` stands for nothing there, and for a file system that numbers
    nothing.
    """
    try:
        told = place.stat()
    except OSError:
        return None
    return (told.st_dev, told.st_ino) if told.st_ino else None


def trusted(program: Path, outside: Sequence[Path]) -> bool:
    """Tell whether a program can be run by its name without going through a working directory.

    The name is walked the way the system walks it, one part after the
    other, and a link is followed to where it leads. At no step may the way
    be in one of the directories ``outside``: what lies there, a link or a
    directory or the file itself, can be changed by whoever can write there,
    also after this look. Whether a program lies at the end of the way is
    another question.

    A directory is known by its name and by what it is. A system that takes
    no notice of case may leave a name the way it was written, and then one
    directory has more names than can be compared.

    Args:
        program: The whole path of the program, as it will be run.
        outside: Directories that the way must not touch, each with every
            link on its own path resolved.

    Returns:
        Whether the way stays outside all of them, to its end.
    """
    if _STEPS_BACK_FIRST:
        program = Path(os.path.normpath(program))
    apart = {_itself(place) for place in outside} - {None}
    done = Path(program.anchor)
    left = list(program.parts[1:])
    hops = 0
    while left:
        part = left.pop(0)
        if part == "..":
            done = done.parent
            continue
        here = done / part
        leads = _leads(here)
        if leads is None:
            # Its name is taken as the system spells it, which may be another way than it
            # was written.
            here = Path(os.path.realpath(here))
            if any(here == place or place in here.parents for place in outside):
                return False
            if _itself(here) in apart:
                return False
            done = here
            continue
        hops += 1
        if hops > _HOPS:
            return False
        # The link lies in a directory that is outside all of them. Where it leads is
        # walked from the start, and counted from the directory of the link.
        way = done / leads
        done, left = Path(way.anchor), [*way.parts[1:], *left]
    return True


def found(name: str, search: str, outside: Sequence[Path] = ()) -> str | None:
    """Return the path of a program that a search path leads to, or ``None``.

    Args:
        name: The name of the program, without the ``.exe`` that Windows
            gives it.
        search: The search path, as the environment has it.
        outside: Directories that the program must not come from, and that
            no link on the way to it may lie in: the directory that this
            program was started in, and the working directory of a session.
    """
    file = f"{name}.exe" if sys.platform == "win32" else name
    apart = [Path(os.path.realpath(directory)) for directory in outside]
    for written in search.split(os.pathsep):
        entry = Path(written.strip('"'))
        program = entry / file
        # An entry that is no whole path means a place below the current directory.
        if not entry.is_absolute():
            continue
        if program.is_file() and os.access(program, os.X_OK) and trusted(program, apart):
            return str(program)
    return None
