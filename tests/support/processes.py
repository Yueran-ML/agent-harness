"""Looking at processes from a test: whether one still runs, and ending one that does."""

import asyncio
import contextlib
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

__all__ = ["alive", "end", "gone", "python"]


def python(code: str, *arguments: str) -> list[str]:
    """The command that runs ``code`` in a Python of its own."""
    return [sys.executable, "-c", code, *arguments]


if sys.platform == "win32":
    import ctypes
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.GetExitCodeProcess.argtypes = (wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD))
    _kernel32.GetExitCodeProcess.restype = wintypes.BOOL
    _kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
    _kernel32.CloseHandle.restype = wintypes.BOOL

    _QUERY_LIMITED_INFORMATION = 0x1000
    _STILL_ACTIVE = 259

    def alive(pid: int) -> bool:
        """Tell whether a process still runs."""
        handle = _kernel32.OpenProcess(_QUERY_LIMITED_INFORMATION, False, pid)
        if not handle:
            return False
        try:
            code = wintypes.DWORD()
            _kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
            return code.value == _STILL_ACTIVE
        finally:
            _kernel32.CloseHandle(handle)

else:
    _PS = next((place for place in ("/bin/ps", "/usr/bin/ps") if Path(place).exists()), "ps")
    """The program that lists processes, by its whole path where it has one of the usual two.

    A test may leave little on the search path, and whoever asks here may be a program that
    a test started with that search path.
    """

    def alive(pid: int) -> bool:
        """Tell whether a process still runs. One that has ended and waits to be reaped does not."""
        state = subprocess.run(
            [_PS, "-o", "stat=", "-p", str(pid)], capture_output=True, text=True, check=False
        )
        return state.returncode == 0 and not state.stdout.strip().startswith("Z")


async def gone(pid: int, within: float = 10.0) -> bool:
    """Wait until a process has ended, and tell whether it did in time."""
    deadline = time.monotonic() + within
    while time.monotonic() < deadline:
        if not alive(pid):
            return True
        await asyncio.sleep(0.05)
    return False


def end(pid: int) -> None:
    """End a process that a test left behind on purpose."""
    with contextlib.suppress(OSError):
        os.kill(pid, signal.SIGTERM)
