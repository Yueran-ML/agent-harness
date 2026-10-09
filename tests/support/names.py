"""The second names that a disk keeps for some files."""

import sys
from pathlib import Path

__all__ = ["short_name"]

if sys.platform == "win32":
    import ctypes
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.GetShortPathNameW.argtypes = (wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD)
    _kernel32.GetShortPathNameW.restype = wintypes.DWORD

    def short_name(path: Path) -> str | None:
        """Return the short name that the disk keeps for the last part of ``path``.

        NTFS can keep a second name of eight letters and three for a file
        whose own name is longer or begins with a dot: ``GIT~1`` for
        ``.git``. Whether it does is a setting of the volume. The answer is
        ``None`` if it keeps none for this file.
        """
        buffer = ctypes.create_unicode_buffer(32768)
        if not _kernel32.GetShortPathNameW(str(path), buffer, len(buffer)):
            return None
        short = Path(buffer.value).name
        return None if short.casefold() == path.name.casefold() else short

else:

    def short_name(path: Path) -> str | None:
        """Return ``None``: only NTFS keeps short names."""
        return None
