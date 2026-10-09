"""Finding the file that a path means.

A model writes a path the way it likes: relative, absolute, through ``..``,
through a link. Before anything is decided about the path, it is brought to
one form, so that the decision is about the file and not about a spelling of
it. Links are followed. On Windows the names that exist are also given the
case they have on the disk. Elsewhere a name keeps the case it was written
in, even on a disk that does not tell upper case from lower, so whatever
compares names there has to allow for that.

Whether a file is in the working directory is told by comparing the names
letter by letter, after the system has given both paths its own spelling.
Lowering the case first would take two directories for one where Python and
the disk disagree about what a lower case letter is.

Some spellings are refused before the file system is asked anything. On
Windows a path can name another machine, and the system opens a connection to
that machine as soon as it resolves the path, which hands it the credentials
of the user. So such a path is refused by its spelling, and the links on the
way are looked at one by one before the system is let near them. Windows also
reads several names as the same file, or as no file at all: a name with a
trailing dot or space, a name with a colon, the name of a device. A rule
about a file could be got around with them, so they are refused too.

The look at the links and the opening of the file are two looks. A program
that changes a link between them can still make the system go to another
machine. What is read is safe from such a change, because what was opened is
checked again by its handle. Where the system goes for it is not.
"""

import os
import sys
from dataclasses import dataclass
from pathlib import Path

from columbina.tools import ToolFailure

__all__ = ["Located", "below", "locate", "usable"]

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
        root: Where the working directory was when the file was located:
            absolute, with every link followed.
    """

    path: Path
    written: str
    inside: bool
    relative: str
    root: Path


def _refused(written: str, why: str) -> ToolFailure:
    return ToolFailure(f"The path {written!r} is not used here: {why}", code="INVALID_PATH")


if sys.platform == "win32":  # pragma: windows only
    import stat

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

    def _odd(name: str) -> str | None:
        """Say why a name is not used, if Windows reads it as something else than it says."""
        if ":" in name:
            return "a colon inside a name is not allowed."
        if name[-1] in ". ":
            return "a name may not end with a dot or a space."
        if name.split(".")[0].rstrip(" ").upper() in _DEVICES:
            return f"{name} is the name of a device."
        return None

    def _check_spelling(written: str) -> None:
        """Refuse a spelling that Windows reads as another machine, a device or another name."""
        if _begins_with_two_slashes(written):
            raise _refused(written, "a path that begins with two slashes names another machine.")
        drive, rest = os.path.splitdrive(written)
        if drive and rest[:1] not in ("\\", "/"):
            raise _refused(written, "a drive has to be followed by a slash.")
        for part in _parts(rest):
            why = None if part in (".", "..") else _odd(part)
            if why is not None:
                raise _refused(written, why)

    def _on_a_drive(rest: str) -> bool:
        return len(rest) >= 2 and rest[0].isascii() and rest[0].isalpha() and rest[1] == ":"

    def _leads_away(target: str) -> bool:
        """Tell whether the target of a link is anything but a place on a disk of this machine.

        Behind the prefix that switches the usual reading of a path off, the
        system finds much more than disks: other machines, pipes, devices. Of
        all that, only a drive and a volume without a letter are let through.
        """
        if target.startswith(_VERBATIM):
            rest = target[len(_VERBATIM) :]
            return not (_on_a_drive(rest) or rest.upper().startswith("VOLUME{"))
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

        The path is walked one name at a time, and a link is followed only
        after its target was looked at. Resolving the whole path at once
        would have the system follow the links first.

        Windows takes every step back out of the text of a path before it
        follows any link, so ``docs/../notes`` is ``notes`` whatever ``docs``
        leads to. The walk does the same, here and wherever a link puts a new
        path in the place of the old one, so that the links it looks at are
        the ones that the system will follow.

        The working directory and what it lies under are not looked at: where
        that is, the user chose.
        """
        pending = os.path.normpath(joined)
        for _ in range(_MAX_LINKS + 1):
            drive, rest = os.path.splitdrive(pending)
            reached = drive + "\\"
            parts = _parts(rest)
            for index, part in enumerate(parts):
                reached = os.path.join(reached, part)
                if root == reached or root.startswith(reached + "\\"):
                    continue
                target = _link_target(reached)
                if target is None:
                    continue
                if _leads_away(target):
                    raise _refused(written, "a link on the way leads to another machine.")
                if _on_a_drive(target.removeprefix(_VERBATIM)):
                    target = target.removeprefix(_VERBATIM)
                # A target that names no drive starts where the link lies, or at the top
                # of its drive, and that is what joining it to the directory comes to.
                onward = os.path.join(os.path.dirname(reached), target, *parts[index + 1 :])
                pending = os.path.normpath(onward)
                break
            else:
                return
        raise _refused(written, "there are too many links on the way.")

else:  # pragma: posix only

    def _odd(name: str) -> str | None:
        """Say nothing: every name means what it says here."""
        return None

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
    return _placed(Path(root), Path(os.path.realpath(joined)), written)


def _placed(root: Path, path: Path, written: str | None) -> Located:
    """Say where a file is, seen from a working directory. Neither path goes through a link."""
    top = root.parts
    inside = path.parts[: len(top)] == top
    relative = ("/".join(path.parts[len(top) :]) or ".") if inside else path.as_posix()
    return Located(path, relative if written is None else written, inside, relative, root)


def usable(name: str) -> bool:
    """Tell whether the system reads the name of a file as that name and nothing else.

    On Windows a name that ends with a dot or a space, that holds a colon, or
    that is the name of a device can lie on a disk, but a path that is
    written with it leads to another file or to none. :func:`locate` refuses
    such a path, and whoever lists a directory passes such a name over.
    """
    return _odd(name) is None


def below(directory: Located, name: str) -> Located:
    """Return the place of an entry of a directory that was located already.

    The entry is spelled the way :func:`locate` would spell it, so that a
    rule speaks about a file the same way whether a call named the file or a
    search came upon it. No link is followed: the entry is taken as it lies
    in its directory.
    """
    return _placed(directory.root, directory.path / name, None)
