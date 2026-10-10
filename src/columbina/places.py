"""Places where what people wrote for a model is looked for.

Skills are looked for in several directories, and so are the kinds of
subagent. Such a directory is a root. Some roots lie in a project and
others are the user's own, and the difference is who wrote what lies there.

A root of a project is taken as it lies there. A link is not followed, be
it the root itself or something in it: what a repository holds was written
by somebody else, and a link could lead to a place that is nobody's
business. A root of the user is the user's own, and links in it are
followed.

This module knows these two ways of going into a root, and nothing of what
is found there.
"""

import os
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO

from columbina.fs import bound

__all__ = ["LINK", "NOWHERE", "Root", "RootError", "followed", "listed", "opened", "target"]

LINK = "it lies behind a link, and links are not followed in a project"
"""Why something in a root of a project is not gone into."""

NOWHERE = "it is a link that leads nowhere"
"""Why a link in a root of the user is not gone into."""


class RootError(Exception):
    """Something in a root is not gone into. The text says why, for whoever put it there."""


@dataclass(frozen=True, slots=True)
class Root:
    """A place where something is looked for.

    Attributes:
        path: The directory.
        label: What the place is called before a person.
        own: Whether the place is the user's own. Links are followed there,
            and nowhere else.
    """

    path: Path
    label: str
    own: bool


def followed(path: Path) -> Path:
    """Return where a path leads, with every link followed.

    Raises:
        OSError: Nothing is there, or the links on the way go in a circle.
    """
    # Not ``Path.resolve``: before Python 3.13 it answers a circle of links with an error
    # that is no ``OSError``, and nobody who reads files is ready for that one.
    return Path(os.path.realpath(path, strict=True))


def listed(root: Root, at_most: int) -> tuple[Path, list[bound.Listed], bool] | None:
    """Look into a root.

    Args:
        root: The root.
        at_most: How many of its entries are taken.

    Returns:
        Where the root lies, with every link followed if it is the user's
        own; its entries, by name; and whether it holds more than these.
        ``None`` if the root is not there.

    Raises:
        RootError: The root lies in a project, behind a link.
        OSError: The root is there and cannot be looked into.
    """
    base = root.path
    try:
        if root.own:
            base = followed(base)
        entries, more = bound.listing(base, at_most)
    except (FileNotFoundError, NotADirectoryError):
        return None
    except bound.MovedError:
        if root.own:
            # Its links were followed a moment before, so somebody is at work there.
            raise
        raise RootError(LINK) from None
    return base, entries, more


def target(root: Root, there: Path, entry: bound.Listed) -> Path:
    """Return where an entry of a root lies, which is somewhere else if the entry is a link.

    Args:
        root: The root.
        there: Where the entry lies in the root.
        entry: The entry.

    Raises:
        RootError: The entry is a link, and the root lies in a project or
            the link leads nowhere.
        OSError: The links on the way go in a circle.
    """
    if entry.kind != "link":
        return there
    if not root.own:
        raise RootError(LINK)
    try:
        return followed(there)
    except FileNotFoundError:
        raise RootError(NOWHERE) from None


def opened(root: Root, file: Path) -> BinaryIO | None:
    """Open a file that lies in a root, to read it.

    Args:
        root: The root.
        file: Where the file lies. In a root of the user the file may be a
            link, and is read where it leads.

    Returns:
        The open file, or ``None`` if no file is there.

    Raises:
        RootError: The file lies in a project, behind a link.
        bound.NotAFileError: A directory, a pipe or a device lies there.
        OSError: The file cannot be opened.
    """
    try:
        if root.own:
            file = followed(file)
        return bound.open_for_reading(file)[0]
    except (FileNotFoundError, NotADirectoryError):
        # No such file, or the link that was followed leads to a file and not to a directory.
        return None
    except bound.MovedError:
        if root.own:
            # Its links were followed a moment before, so somebody is at work on it.
            raise
        raise RootError(LINK) from None
