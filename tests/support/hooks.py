"""Hooks that really run in a test: a small program, and the command that starts it."""

import sys
from pathlib import Path

import pytest

from columbina import shells

__all__ = ["SHELL", "needs_shell", "python_hook", "quoted"]

SHELL = shells.find()
"""The shell that runs the hooks on this system, if it is installed."""

needs_shell = pytest.mark.skipif(SHELL is None, reason="the shell of this system is not installed")
"""Marks a test that runs a hook, which cannot be done without a shell."""


def quoted(text: str | Path) -> str:
    """Write a text so that the shell of this system takes it as one word, as it is."""
    return "'" + str(text).replace("'", "''" if sys.platform == "win32" else "'\\''") + "'"


def python_hook(place: Path, body: str, name: str = "hook.py") -> str:
    """Write a program of Python into a directory, and return the command that runs it as a hook."""
    script = place / name
    script.write_text(body, encoding="utf-8")
    call = "& " if sys.platform == "win32" else ""
    return f"{call}{quoted(sys.executable)} {quoted(script)}"
