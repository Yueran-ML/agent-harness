"""Finding the file that a path means.

A model writes a path the way it likes: relative, absolute, through ``..``,
through a link. Before anything is decided about the path, it is brought to
one form, so that the decision is about the file and not about a spelling of
it. Links are followed, and on a system that does not tell upper case from
lower, the names that exist are given the case they have on the disk.

Some spellings are refused before the file system is asked anything. On
Windows a path can name another machine, and the system opens a connection to
that machine as soon as it resolves the path, which hands it the credentials
of the user. So such a path is refused by its spelling, and a link is not
followed if it leads there. Windows also reads several names as the same file,
or as no file at all: a name with a trailing dot or space, a name with a
colon, the name of a device. A rule about a file could be got around with
them, so they are refused too.
"""

import os
import sys
from dataclasses import dataclass
from pathlib import Path

from columbina.tools import ToolFailure

__all__ = ["Located", "locate"]

_MAX_LINKS = 40


@dataclass(frozen=True, slots=True)
class Located:
    """The file that a path means.

    Attributes:
        path: Where the file is: absolute, with every link followed.
        written: The path as the model wrote it.
        inside: Whether the file is in the working directory or below it.
        relative: The path that rules are matched against, with forward
            slashes: from the working directory if the file is inside it,
            and absolute otherwise. The working directory itself is ``.``.
    """

    path: Path
    written: str
    inside: bool
    relative: str


def _refused(written: str, why: str) -> ToolFailure:
    return ToolFailure(f"The path {written!r} is not used here: {why}", code="INVALID_PATH")


if sys.platform == "win32":  # pragma: windows only
    import stat
    from collections import deque

    _SUPERSCRIPTS = chr(0xB9) + chr(0xB2) + chr(0xB3)
    _DEVICES = frozenset(
        ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        + [f"{port}{digit}" for port in ("COM", "LPT") for digit in "0123456789" + _SUPERSCRIPTS]
    )
    _VERBATIM = "\\\\?\\"

    def _parts(path: str) -> list[str]:
        return [part for part in path.replace("/", "\\").split("\\") if part]

    def _begins_with_two_slashes(path: str) -> bool:
        return path[:2].replace("/", "\\") == "\\\\"

    def _check_spelling(written: str) -> None:
        """Refuse a spelling that Windows reads as another machine, a device or another name."""
        if _begins_with_two_slashes(written):
            raise _refused(written, "a path that begins with two slashes names another machine.")
        drive, rest = os.path.splitdrive(written)
        if drive and rest[:1] not in ("\\", "/"):
            raise _refused(written, "a drive has to be followed by a slash.")
        for part in _parts(rest):
            if part in (".", ".."):
                continue
            if ":" in part:
                raise _refused(written, "a colon inside a name is not allowed.")
            if part[-1] in ". ":
                raise _refused(written, "a name may not end with a dot or a space.")
            if part.split(".")[0].rstrip(" ").upper() in _DEVICES:
                raise _refused(written, f"{part} is the name of a device.")

    def _leads_away(target: str) -> bool:
        """Tell whether the target of a link is on another machine or is a device."""
        if target.startswith(_VERBATIM):
            target = target[len(_VERBATIM) :]
            if target.upper().startswith("UNC\\"):
                return True
        return _begins_with_two_slashes(target)

    def _link_target(path: str) -> str | None:
        """Return where a link points, or ``None`` if ``path`` is no link."""
        try:
            status = os.lstat(path)
        except OSError:
            return None
        junction = (
            status.st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT
            and status.st_reparse_tag == stat.IO_REPARSE_TAG_MOUNT_POINT
        )
        if not (stat.S_ISLNK(status.st_mode) or junction):
            return None
        try:
            return os.readlink(path)
        except OSError:
            return None

    def _check_links(root: str, joined: str, written: str) -> None:
        """Refuse a path that reaches another machine through a link.

        The path is walked one name at a time, the way the system would walk
        it, and a link is followed only after its target was looked at.
        Resolving the whole path at once would have the system follow the
        links first. The working directory and what it lies under are not
        looked at: where that is, the user chose.
        """
        trusted = os.path.normcase(root)
        drive, rest = os.path.splitdrive(joined)
        reached = drive + "\\"
        ahead = deque(_parts(rest))
        links = 0
        while ahead:
            part = ahead.popleft()
            if part == ".":
                continue
            if part == "..":
                reached = os.path.dirname(reached)
                continue
            reached = os.path.join(reached, part)
            here = os.path.normcase(reached)
            if trusted == here or trusted.startswith(here.rstrip("\\") + "\\"):
                continue
            target = _link_target(reached)
            if target is None:
                continue
            if _leads_away(target):
                raise _refused(written, "a link on the way leads to another machine.")
            links += 1
            if links > _MAX_LINKS:
                raise _refused(written, "there are too many links on the way.")
            target = target.removeprefix(_VERBATIM)
            drive, rest = os.path.splitdrive(target)
            if drive:
                reached = drive + "\\"
            elif rest[:1] in ("\\", "/"):
                # From the top of the drive that the link lies on.
                reached = os.path.splitdrive(reached)[0] + "\\"
            else:
                # From the directory that the link lies in.
                reached = os.path.dirname(reached)
            ahead.extendleft(reversed(_parts(rest)))

else:  # pragma: posix only

    def _check_spelling(written: str) -> None:
        """Refuse nothing: every spelling means one file here."""

    def _check_links(root: str, joined: str, written: str) -> None:
        """Refuse nothing: a link cannot reach another machine here."""


def locate(cwd: Path, written: str) -> Located:
    """Find the file that a path means, as a model wrote it.

    This asks the file system, so it takes the time that takes.

    Args:
        cwd: The working directory. A relative path starts there.
        written: The path as the model wrote it.

    Raises:
        ToolFailure: The path is empty, or it is spelled in a way that is not
            used here. The code is ``INVALID_PATH``.
    """
    if not written.strip():
        raise ToolFailure("No path was given.", code="INVALID_PATH")
    if chr(0) in written:
        raise _refused(written, "it contains a character that no path can contain.")
    _check_spelling(written)
    root = os.path.realpath(cwd)
    joined = os.path.join(root, written)
    _check_links(root, joined, written)
    path = Path(os.path.realpath(joined))
    base = Path(root)
    if path.is_relative_to(base):
        return Located(path, written, True, path.relative_to(base).as_posix())
    return Located(path, written, False, path.as_posix())
