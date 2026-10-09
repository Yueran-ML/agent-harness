"""An exclusive lock on a file, which keeps a session to one writer.

The lock is held by the operating system on behalf of this process, so it is
gone when the process ends, however it ends. A second attempt to take a lock
fails even inside the process that holds it.
"""

import os
import sys
from pathlib import Path

if sys.platform == "win32":  # pragma: windows only
    import msvcrt

    def _lock(descriptor: int) -> None:
        msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)

else:  # pragma: posix only
    import fcntl

    def _lock(descriptor: int) -> None:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)


class FileLock:
    """A lock that is held. Release it to let another writer in."""

    __slots__ = ("_descriptor",)

    def __init__(self, descriptor: int) -> None:
        self._descriptor: int | None = descriptor

    def release(self) -> None:
        """Give the lock up. Releasing a second time does nothing."""
        if self._descriptor is not None:
            os.close(self._descriptor)
            self._descriptor = None


def acquire(path: Path) -> FileLock | None:
    """Take the lock on the file at ``path``, creating the file if needed.

    Returns:
        The lock, or ``None`` if it is held already.
    """
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT)
    try:
        _lock(descriptor)
    except OSError:
        os.close(descriptor)
        return None
    return FileLock(descriptor)
