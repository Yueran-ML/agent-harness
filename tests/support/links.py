"""Links between directories, made the way each platform lets a test make them."""

import sys
from pathlib import Path

__all__ = ["link_directory", "unlink_directory"]


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
