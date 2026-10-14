"""ffmpeg for the tests that need it, and the sources that they make with it.

Such tests run ffmpeg. They are passed over where there is none, unless
``COLUMBINA_REQUIRE_FFMPEG`` is ``1``: then that is a failure, so that a
machine which is meant to run them cannot pass without them. Every source is
made by the test from nothing, with the ``lavfi`` input of ffmpeg.
"""

import os
import subprocess
from pathlib import Path
from typing import NoReturn

import pytest

from columbina import programs

__all__ = ["made", "real"]


def real() -> str:
    """Return the path of ffmpeg, if there is one beside ffprobe on the search path.

    Otherwise the test that asks is passed over, or fails where ffmpeg is required.
    """

    def without(why: str) -> NoReturn:
        if os.environ.get("COLUMBINA_REQUIRE_FFMPEG") == "1":
            pytest.fail(f"COLUMBINA_REQUIRE_FFMPEG is set, and {why}")
        pytest.skip(why)

    found = programs.found("ffmpeg", os.environ.get("PATH", ""))
    if found is None:
        without("no ffmpeg program was found")
    if programs.found("ffprobe", str(Path(found).parent)) is None:
        without("there is no ffprobe beside ffmpeg")
    return found


def made(directory: Path, name: str, *arguments: str) -> Path:
    """Have ffmpeg make a file in a directory, and return its path."""
    directory.mkdir(parents=True, exist_ok=True)
    done = subprocess.run(
        [real(), "-hide_banner", "-loglevel", "error", "-nostdin", "-y", *arguments, name],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return directory / name
