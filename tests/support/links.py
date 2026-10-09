"""Links between directories, made the way each platform lets a test make them."""

import sys
from pathlib import Path

import pytest

__all__ = ["link_directory", "link_file", "unlink_directory"]

_NOT_ALLOWED = 1314
"""What Windows answers a user who may not make symbolic links."""


def link_directory(link: Path, target: Path) -> None:
    """Make ``link`` lead to the directory ``target``.

    On Windows this is a junction, which needs no privilege. Elsewhere it is a
    symbolic link.
    """
    if sys.platform == "win32":
        import _winapi

        _winapi.CreateJunction(str(target), str(link))
    else:
        link.symlink_to(target, target_is_directory=True)


def unlink_directory(link: Path) -> None:
    """Take a link away that :func:`link_directory` made, and leave its target alone."""
    if sys.platform == "win32":
        link.rmdir()
    else:
        link.unlink()


def link_file(link: Path, target: str | Path) -> None:
    """Make ``link`` a symbolic link to a file, or skip the test where that cannot be done.

    On Windows a user may make such a link only in developer mode or as an
    administrator. The target is stored as it is given, so a relative one is
    relative to the directory of the link.
    """
    try:
        link.symlink_to(target)
    except OSError as error:
        if getattr(error, "winerror", None) != _NOT_ALLOWED:
            raise
        pytest.skip("this user may not make symbolic links")
