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

__all__ = ["made", "real", "real_engine"]

_REPOSITORY = Path(__file__).resolve().parents[2]


def real_engine() -> str:
    """Return the path of the engine program: ``COLUMBINA_ENGINE``, or a build of the repository.

    A build is looked for in ``CARGO_TARGET_DIR`` and in ``engine/target``, the newest of its
    debug and release builds. The libraries of FFmpeg that the engine reads media with are
    found beside it, on the search path, or in ``FFMPEG_DIR``, the build that it was made with,
    which the engine is started with. Without an engine the test that asks is passed over, or
    fails where ``COLUMBINA_REQUIRE_ENGINE`` is ``1``.
    """
    named = os.environ.get("COLUMBINA_ENGINE")
    program = "columbina-engine.exe" if os.name == "nt" else "columbina-engine"
    places = [Path(named)] if named else []
    for target in (os.environ.get("CARGO_TARGET_DIR"), _REPOSITORY / "engine" / "target"):
        if target:
            built = [Path(target) / kind / program for kind in ("debug", "release")]
            places += sorted(
                (one for one in built if one.is_file()),
                key=lambda one: one.stat().st_mtime,
                reverse=True,
            )
    found = next((one for one in places if one.is_file()), None)
    if found is None:
        why = "no engine program was found"
        if os.environ.get("COLUMBINA_REQUIRE_ENGINE") == "1":
            pytest.fail(f"COLUMBINA_REQUIRE_ENGINE is set, and {why}")
        pytest.skip(why)
    return str(found)


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
