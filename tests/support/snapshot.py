"""Text that is kept in a file and compared with what the code gives today."""

import os
from pathlib import Path

import pytest

UPDATE = "TACK_UPDATE_SNAPSHOTS"
"""The environment variable that makes the tests write their snapshots anew."""


def stored(path: Path, text: str) -> str:
    """Return the snapshot kept at ``path``, for ``text`` to be compared with.

    A test compares the two itself, so that a difference is shown line by
    line. When the environment variable :data:`UPDATE` is 1, ``text`` is
    stored first. That is how a snapshot is made, and how it is renewed after
    a change that was meant. The difference then shows in the next commit.
    """
    if os.environ.get(UPDATE) == "1":
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        pytest.fail(f"there is no snapshot at {path}: run the test with {UPDATE}=1 to make it")
