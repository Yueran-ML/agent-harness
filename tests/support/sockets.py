"""Sockets of the system in the directories of a test."""

import os
import socket
import sys
from pathlib import Path

__all__ = ["listening_at"]


def listening_at(path: Path) -> socket.socket:
    """Make a socket of the system at a path, and return it for the caller to close.

    The name of such a socket may be about a hundred bytes long, fewer on
    macOS than on Linux, and on some machines the directory of a test is
    longer than that. So the socket is bound from its own directory, by its
    name alone.
    """
    assert sys.platform != "win32", "Windows has no such socket in a directory"
    made = socket.socket(socket.AF_UNIX)
    back = Path.cwd()
    try:
        os.chdir(path.parent)
        try:
            made.bind(path.name)
        finally:
            os.chdir(back)
    except BaseException:
        # Left open, it would be found by whatever test the collector of garbage runs in.
        made.close()
        raise
    return made
