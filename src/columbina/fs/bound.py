"""Opening a file so that what is open is the file that was meant.

A path is checked before it is opened, and between the two a directory on the
way can be replaced by a link that leads elsewhere. So the check is repeated
on what was opened. An open file knows where it is: Windows, Linux and macOS
each have a way to ask. If the answer is not the place that was checked, the
file is closed again and nobody reads it.

Where the system has no such way, or its answer cannot be compared letter by
letter, the open file is compared with the file that the path names now. That
catches a replacement that stays in place, though not one that is undone at
just the right moment.

Only regular files are opened. A pipe or a device is refused, and it is
opened in a way that does not wait for whatever is at its other end.
"""

import os
import stat
import sys
import unicodedata
from pathlib import Path
from typing import BinaryIO

__all__ = ["MovedError", "NotAFileError", "open_for_reading"]


class MovedError(OSError):
    """What was opened is not at the place that was checked."""


class NotAFileError(OSError):
    """The path names something that is no regular file."""


if sys.platform == "win32":  # pragma: windows only
    import ctypes
    import msvcrt
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.GetFinalPathNameByHandleW.argtypes = (
        wintypes.HANDLE,
        wintypes.LPWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
    )
    _kernel32.GetFinalPathNameByHandleW.restype = wintypes.DWORD

    _OPEN_FLAGS = os.O_RDONLY | os.O_BINARY | os.O_NOINHERIT
    _EXACT = True
    _LONGEST_PATH = 32768
    _VERBATIM = "\\\\?\\"
    _VERBATIM_SHARE = _VERBATIM + "UNC\\"

    def _plain(path: str) -> str:
        """Return a path as people write it, from the form in which the system reports it."""
        if path.startswith(_VERBATIM_SHARE):
            return "\\\\" + path.removeprefix(_VERBATIM_SHARE)
        return path.removeprefix(_VERBATIM)

    def _where(descriptor: int) -> str | None:
        """Ask the system where an open file is."""
        buffer = ctypes.create_unicode_buffer(_LONGEST_PATH)
        length = _kernel32.GetFinalPathNameByHandleW(
            msvcrt.get_osfhandle(descriptor), buffer, _LONGEST_PATH, 0
        )
        return _plain(buffer.value) if 0 < length < _LONGEST_PATH else None

    def _spelled_alike(one: str, other: str) -> bool:
        return os.path.normcase(one) == os.path.normcase(other)

    def _open(path: Path) -> int:
        try:
            return os.open(path, _OPEN_FLAGS)
        except PermissionError:
            # This is how Windows answers when a directory is opened as a file.
            if path.is_dir():
                raise NotAFileError(f"{path} is a directory") from None
            raise

else:  # pragma: posix only
    _OPEN_FLAGS = os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC
    _EXACT = sys.platform == "linux"

    def _where(descriptor: int) -> str | None:
        """Ask the system where an open file is."""
        try:
            if sys.platform == "linux":
                return str(Path(f"/proc/self/fd/{descriptor}").readlink())
            if sys.platform == "darwin":
                import fcntl

                answer = fcntl.fcntl(descriptor, fcntl.F_GETPATH, bytes(1024))
                return os.fsdecode(answer.split(bytes(1), 1)[0])
        except OSError:
            return None
        return None

    def _spelled_alike(one: str, other: str) -> bool:
        if sys.platform == "linux":
            return one == other
        # Names are stored in a form of their own there, and their case does not count.
        return (
            unicodedata.normalize("NFC", one).casefold()
            == unicodedata.normalize("NFC", other).casefold()
        )

    def _open(path: Path) -> int:
        return os.open(path, _OPEN_FLAGS)


def _names_it_now(descriptor: int, expected: Path) -> bool:
    """Tell whether ``expected`` names the open file now, and still leads where it led."""
    try:
        named = expected.stat()
        still = os.path.realpath(expected)
    except OSError:
        return False
    return os.path.samestat(os.fstat(descriptor), named) and _spelled_alike(still, str(expected))


def _is_at(descriptor: int, expected: Path) -> bool:
    """Tell whether an open file is the file at ``expected``, which has no link on its way."""
    actual = _where(descriptor)
    if actual is not None and _spelled_alike(actual, str(expected)):
        return True
    if actual is not None and _EXACT:
        return False
    # The system gave no answer that settles it.
    return _names_it_now(descriptor, expected)


def open_for_reading(path: Path) -> tuple[BinaryIO, os.stat_result]:
    """Open the regular file at a path that has no link on its way.

    Args:
        path: Where the file is, as :func:`columbina.fs.locate` gave it.

    Returns:
        The open file, and what the system said about it when it was opened.

    Raises:
        MovedError: What the path led to when it was opened is somewhere else.
        NotAFileError: The path names a directory, a pipe or a device.
        OSError: The file cannot be opened.
    """
    descriptor = _open(path)
    try:
        status = os.fstat(descriptor)
        if not stat.S_ISREG(status.st_mode):
            raise NotAFileError(f"{path} is not a regular file")
        if not _is_at(descriptor, path):
            raise MovedError(f"{path} was somewhere else when it was opened")
        return os.fdopen(descriptor, "rb"), status
    except BaseException:
        os.close(descriptor)
        raise
