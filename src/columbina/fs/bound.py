"""Opening and replacing a file so that what is touched is the file that was meant.

A path is checked before it is used, and between the two a directory on the
way can be replaced by a link that leads elsewhere. So the check is repeated
on what was opened. An open file knows where it is: Windows, Linux and macOS
each have a way to ask. If the answer is not the place that was checked, the
file is closed again and nobody reads it.

The answer of the system and the place that was checked may be spelled
differently and still be one place: in another case, or with an accent stored
another way. Whether they are is not decided by lowering the case, which no
two systems do alike. Two spellings are one place if they name the same entry
of the same directory at every level, and that is asked of the disk. A link
and what it leads to are two entries, so a path that goes through a link is
not the place it ends in.

Where the system has no way to say where an open file is, the open file is
compared with the file that the path names now. That catches a replacement
that stays in place, though not one that is undone at just the right moment.

Only regular files are opened. A pipe or a device is refused, and it is
opened in a way that does not wait for whatever is at its other end.

A file is replaced through its directory, which is held open while that is
done, and checked the same way. The new content is written to a file beside
the old one, which then takes its name, so that a reader finds the old
content or the new and never a part of either. On POSIX every name is used
relative to the open directory. Windows has no such calls, but there a
directory that is held open without sharing its deletion cannot be renamed or
removed, and neither can any directory above it. This was tried on NTFS: the
rename of the held directory fails with error 32, and that of its parent with
error 5, while files in it are still made and replaced.
"""

import contextlib
import itertools
import os
import secrets
import stat
import sys
import time
from collections.abc import Generator
from pathlib import Path
from typing import BinaryIO

__all__ = [
    "ChangedError",
    "MovedError",
    "NotAFileError",
    "NotShownError",
    "open_for_reading",
    "replace",
]


class MovedError(OSError):
    """What was opened is not at the place that was checked."""


class NotAFileError(OSError):
    """The path names something that is no regular file."""


class NotShownError(Exception):
    """The file is there, and the one who wants to replace it never saw it."""


class ChangedError(Exception):
    """The file is no longer in the state in which it was seen."""


def _one_entry(left: Path, right: Path) -> bool:
    try:
        return os.path.samestat(left.lstat(), right.lstat())
    except OSError:
        return False


def _same_entries(one: str, other: str) -> bool:
    """Tell whether two spellings of a path name the same entries, level by level.

    A link is an entry of its own, so it is not the same as what it leads to.
    """
    first, second = Path(one), Path(other)
    if len(first.parts) != len(second.parts):
        return False
    levels = zip((first, *first.parents), (second, *second.parents), strict=True)
    # From the file upwards, as far as the two are spelled differently.
    apart = itertools.takewhile(lambda level: str(level[0]) != str(level[1]), levels)
    return all(_one_entry(left, right) for left, right in apart)


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
    _kernel32.CreateFileW.argtypes = (
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    )
    _kernel32.CreateFileW.restype = wintypes.HANDLE
    _kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
    _kernel32.CloseHandle.restype = wintypes.BOOL

    _OPEN_FLAGS = os.O_RDONLY | os.O_BINARY | os.O_NOINHERIT
    _CREATE_FLAGS = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_BINARY | os.O_NOINHERIT
    _EXACT = True
    _LONGEST_PATH = 32768
    _VERBATIM = "\\\\?\\"
    _VERBATIM_SHARE = _VERBATIM + "UNC\\"
    _NO_HANDLE = wintypes.HANDLE(-1).value
    _LIST_DIRECTORY = 0x0001
    _SHARE_READ_AND_WRITE = 0x0001 | 0x0002
    _OPEN_EXISTING = 3
    _BACKUP_SEMANTICS = 0x02000000
    _RENAME_ATTEMPTS = 6
    _RENAME_PAUSE = 0.05

    def _plain(path: str) -> str:
        """Return a path as people write it, from the form in which the system reports it."""
        if path.startswith(_VERBATIM_SHARE):
            return "\\\\" + path.removeprefix(_VERBATIM_SHARE)
        return path.removeprefix(_VERBATIM)

    def _verbatim(path: Path) -> str:
        """Return a path in the form that the system takes whatever its length."""
        text = str(path)
        if text.startswith(_VERBATIM):
            return text
        if text.startswith("\\\\"):
            return _VERBATIM_SHARE + text[2:]
        return _VERBATIM + text

    def _where_of(handle: int) -> str | None:
        buffer = ctypes.create_unicode_buffer(_LONGEST_PATH)
        length = _kernel32.GetFinalPathNameByHandleW(handle, buffer, _LONGEST_PATH, 0)
        return _plain(buffer.value) if 0 < length < _LONGEST_PATH else None

    def _where(descriptor: int) -> str | None:
        """Ask the system where an open file is."""
        return _where_of(msvcrt.get_osfhandle(descriptor))

    def _spelled_alike(one: str, other: str) -> bool:
        return _plain(one) == _plain(other) or _same_entries(_plain(one), _plain(other))

    def _open(path: Path) -> int:
        try:
            return os.open(path, _OPEN_FLAGS)
        except PermissionError:
            # This is how Windows answers when a directory is opened as a file.
            if path.is_dir():
                raise NotAFileError(f"{path} is a directory") from None
            raise

    class _Held:
        """A directory that is held open, so that it stays where it is.

        Attributes:
            path: Where the directory is.
        """

        __slots__ = ("_handle", "path")

        def __init__(self, path: Path) -> None:
            handle = _kernel32.CreateFileW(
                _verbatim(path),
                _LIST_DIRECTORY,
                _SHARE_READ_AND_WRITE,
                None,
                _OPEN_EXISTING,
                _BACKUP_SEMANTICS,
                None,
            )
            if handle is None or handle == _NO_HANDLE:
                raise ctypes.WinError(ctypes.get_last_error())
            self._handle: int = handle
            self.path = path
            actual = _where_of(handle)
            if actual is None or not _spelled_alike(actual, str(path)):
                self.close()
                raise MovedError(f"{path} was somewhere else when it was opened")

        def enter(self, name: str) -> "_Held":
            """Make the directory ``name`` in this one if it is missing, and hold it instead."""
            with contextlib.suppress(FileExistsError):
                (self.path / name).mkdir()
            try:
                return _Held(self.path / name)
            finally:
                self.close()

        def status(self, name: str) -> os.stat_result:
            return (self.path / name).lstat()

        def create(self, name: str) -> int:
            return os.open(self.path / name, _CREATE_FLAGS, 0o666)

        def keep_mode(self, descriptor: int, existing: os.stat_result) -> None:
            """Do nothing: a file has no permission bits here."""

        def rename(self, old: str, new: str) -> None:
            # A program that looks at files as they change may have the old one open for a moment.
            attempt = 1
            while True:
                try:
                    (self.path / old).replace(self.path / new)
                except PermissionError:
                    if attempt == _RENAME_ATTEMPTS:
                        raise
                    attempt += 1
                    time.sleep(_RENAME_PAUSE)
                else:
                    return

        def remove(self, name: str) -> None:
            with contextlib.suppress(OSError):
                (self.path / name).unlink()

        def sync(self) -> None:
            """Do nothing: a directory cannot be flushed here."""

        def close(self) -> None:
            _kernel32.CloseHandle(self._handle)

else:  # pragma: posix only
    _OPEN_FLAGS = os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC
    _CREATE_FLAGS = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    _DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    _EXACT = sys.platform in ("linux", "darwin")

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
        if one == other:
            return True
        # Linux tells every spelling from every other. Elsewhere the disk decides.
        return sys.platform != "linux" and _same_entries(one, other)

    def _open(path: Path) -> int:
        return os.open(path, _OPEN_FLAGS)

    class _Held:
        """A directory that is held open, so that names are used in it and nowhere else.

        Attributes:
            path: Where the directory is.
        """

        __slots__ = ("_descriptor", "path")

        def __init__(self, path: Path, descriptor: int | None = None) -> None:
            self.path = path
            if descriptor is not None:
                self._descriptor = descriptor
                return
            self._descriptor = os.open(path, _DIRECTORY_FLAGS)
            if not _is_at(self._descriptor, path):
                self.close()
                raise MovedError(f"{path} was somewhere else when it was opened")

        def enter(self, name: str) -> "_Held":
            """Make the directory ``name`` in this one if it is missing, and hold it instead."""
            try:
                with contextlib.suppress(FileExistsError):
                    os.mkdir(name, dir_fd=self._descriptor)
                # A link that was put there in the meantime is not followed.
                inner = os.open(name, _DIRECTORY_FLAGS | os.O_NOFOLLOW, dir_fd=self._descriptor)
                return _Held(self.path / name, inner)
            finally:
                self.close()

        def status(self, name: str) -> os.stat_result:
            return os.stat(name, dir_fd=self._descriptor, follow_symlinks=False)

        def create(self, name: str) -> int:
            return os.open(name, _CREATE_FLAGS, 0o666, dir_fd=self._descriptor)

        def keep_mode(self, descriptor: int, existing: os.stat_result) -> None:
            os.fchmod(descriptor, stat.S_IMODE(existing.st_mode))

        def rename(self, old: str, new: str) -> None:
            os.replace(old, new, src_dir_fd=self._descriptor, dst_dir_fd=self._descriptor)

        def remove(self, name: str) -> None:
            with contextlib.suppress(OSError):
                os.unlink(name, dir_fd=self._descriptor)

        def sync(self) -> None:
            os.fsync(self._descriptor)

        def close(self) -> None:
            os.close(self._descriptor)


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


@contextlib.contextmanager
def _held(directory: Path) -> Generator[_Held, None, None]:
    """Hold a directory open, having made it and what is missing above it."""
    present = directory
    missing: list[str] = []
    while not present.is_dir() and present.parent != present:
        if present.exists():
            raise NotADirectoryError(f"{present} is in the way, and it is no directory")
        missing.append(present.name)
        present = present.parent
    # If nothing of the path is there, not even its top, the system says so here.
    held = _Held(present)
    for name in reversed(missing):
        # Entering lets go of the directory it leaves, whether or not it gets into the next.
        held = held.enter(name)
    try:
        yield held
    finally:
        held.close()


def replace(path: Path, data: bytes, seen: tuple[int, int] | None) -> os.stat_result:
    """Put ``data`` in the place of the file at ``path``, or make the file.

    Whoever replaces a file has to have seen what it replaces. So a file that
    is there is replaced only in the state in which it was seen.

    Args:
        path: Where the file is or will be, as :func:`columbina.fs.locate`
            gave it. Directories that are missing on the way are made.
        data: The new content.
        seen: The size and the time of the last change of the file as it was
            seen, or ``None`` if it was never seen.

    Returns:
        What the system says about the file after it was replaced.

    Raises:
        MovedError: A directory on the way is somewhere else than it was.
        NotAFileError: The path names a directory, a link or something else
            that is no regular file.
        NotShownError: The file is there and was never seen.
        ChangedError: The file is not in the state in which it was seen.
        OSError: The file cannot be written.
    """
    name = path.name
    with _held(path.parent) as directory:
        try:
            existing = directory.status(name)
        except FileNotFoundError:
            existing = None
        if existing is not None:
            # A link that has taken the place of the file is no regular file either.
            if not stat.S_ISREG(existing.st_mode):
                raise NotAFileError(f"{path} is not a regular file")
            if seen is None:
                raise NotShownError(f"{path} was never seen")
            if (existing.st_size, existing.st_mtime_ns) != seen:
                raise ChangedError(f"{path} is not as it was seen")
        beside = f".columbina-{secrets.token_hex(8)}.tmp"
        descriptor = directory.create(beside)
        try:
            with os.fdopen(descriptor, "wb") as file:
                if existing is not None:
                    directory.keep_mode(descriptor, existing)
                file.write(data)
                file.flush()
                os.fsync(descriptor)
            directory.rename(beside, name)
        except BaseException:
            directory.remove(beside)
            raise
        directory.sync()
        return directory.status(name)
