"""Where the hook files lie, and how they are read from the disk.

Two things read them. The plugin does when a turn begins, to run what they
hold. ``columbina hooks`` does when the user wants to see what would run, or
says that the file of a project may. The two have to mean the same file by
the same place, and read it in the same way: what the user trusted has to be
what runs. So both go by this module.

The file of the user lies where the config says. The file of a project lies
in ``.columbina`` at the root of the project that the working directory
belongs to, and its place is told with every link on the way to that root
followed. Somebody else wrote it, so more care is taken with it: a link in
its place is not followed, and a directory or a device is not read.

No more of a file is read than a hook file may hold, and one byte beyond
that, by which the reader of its content knows that it is too large. A file
of any size can lie in a repository.
"""

from dataclasses import dataclass
from pathlib import Path

from columbina.fs import bound, locate
from columbina.hooks.config import MAX_FILE_BYTES
from columbina.project import root_of

__all__ = [
    "PROJECT_FILE",
    "Found",
    "file_in",
    "of_the_project",
    "of_the_user",
    "project_of",
    "record_at",
]

PROJECT_FILE = ".columbina/hooks.json"
"""Where the hooks of a project are, from its root."""


@dataclass(frozen=True, slots=True)
class Found:
    """What there is at the place of a hook file.

    Attributes:
        path: The place.
        content: What the file holds, as far as it is read. ``None`` if no
            file is there, or if it cannot be read.
        problem: Why the file cannot be read, if something is there.
    """

    path: Path
    content: bytes | None = None
    problem: str | None = None


def project_of(cwd: Path) -> Path:
    """Return the root of the project that a working directory belongs to.

    Every link on the way is followed, so the same project has the same root
    by whatever way it was entered. This reads the disk.
    """
    return root_of(locate(cwd, ".").path)


def file_in(project: Path) -> Path:
    """Return where the hook file of the project at a root lies."""
    return project / PROJECT_FILE


def of_the_user(path: Path) -> Found:
    """Read the hook file of the user. This reads the disk, and fails for nobody."""
    try:
        with path.open("rb") as file:
            return Found(path, file.read(MAX_FILE_BYTES + 1))
    except FileNotFoundError:
        return Found(path)
    except OSError as error:
        return Found(path, problem=str(error))


def of_the_project(project: Path) -> Found:
    """Read the hook file of the project at a root. This reads the disk, and fails for nobody.

    Args:
        project: The root, as :func:`project_of` gave it.
    """
    path = file_in(project)
    try:
        with bound.open_for_reading(path)[0] as file:
            return Found(path, file.read(MAX_FILE_BYTES + 1))
    except (FileNotFoundError, NotADirectoryError):
        # No such file, or no directory for it to be in.
        return Found(path)
    except OSError as error:
        return Found(path, problem=str(error))


def record_at(path: Path | None) -> bytes | None:
    """Read the record of the hook files that are trusted, if there is one to read.

    A record that is not there or cannot be read is none, and trusts no file.
    """
    if path is None:
        return None
    try:
        return path.read_bytes()
    except OSError:
        return None
