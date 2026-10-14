"""What the editing of video asks of ffmpeg and ffprobe, and how it asks.

**The programs.** ffmpeg and ffprobe are programs of the system, not of this
package. ffmpeg is looked for on the search path the way
:mod:`columbina.programs` looks for a program: not in the directory that this
program was started in, and on no way that leads through a working
directory. ffprobe is the one beside it, and no other: the two come from one
build, and the options of the one are those of the other. The way to both is
walked again before every run, :func:`checked`.

**The version.** A nightly build calls itself ``N-12345-g…``, which says
nothing that can be compared. The libraries say more, and every build names
them: libavformat 60 is ffmpeg 6.0, the oldest that is taken, and libavfilter
10 is ffmpeg 7.0, the first that reads a filter graph from a file with
``-/filter_complex``.
"""

import asyncio
import os
import re
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from columbina import process, programs

__all__ = ["MediaUnavailableError", "Tools", "checked", "located", "versions"]

_OLDEST_FORMAT = 60
"""The major version of libavformat in ffmpeg 6.0, the oldest that is taken."""

_ASKING_SECONDS = 30.0
"""How long ffmpeg may take to say what it is."""

_INSTALL = (
    "Install ffmpeg: on Windows with `winget install Gyan.FFmpeg`, on macOS with "
    "`brew install ffmpeg`, on Linux with the ffmpeg package of the distribution"
)


class MediaUnavailableError(Exception):
    """There is no ffmpeg that can be used. The words say what to do about it."""


@dataclass(frozen=True)
class Tools:
    """The ffmpeg and ffprobe of this machine, and what they were built with.

    Attributes:
        ffmpeg: The whole path of ffmpeg.
        ffprobe: The whole path of ffprobe, in the same directory.
        avformat: The major version of libavformat.
        avfilter: The major version of libavfilter.
        outside: The directories that no step of the way to either program may
            lie in, each with every link on its own path resolved.
    """

    ffmpeg: str
    ffprobe: str
    avformat: int
    avfilter: int
    outside: tuple[Path, ...] = ()


def checked(tools: Tools) -> None:
    """See that both programs are still there, and that no working directory is on their way.

    The way is walked again before every run: a link on it may have been put
    there since the programs were found. This blocks.

    Raises:
        MediaUnavailableError: One of them is gone, or the way to it now leads
            through a directory that it must not.
    """
    for program in (tools.ffmpeg, tools.ffprobe):
        if not Path(program).is_file() or not programs.trusted(Path(program), tools.outside):
            raise MediaUnavailableError(
                f"the program {program} is no longer there, or the way to it now leads "
                "through the working directory. It is not run"
            )


def versions(said: str) -> tuple[int, int] | None:
    """Return the major versions of libavformat and libavfilter that ``ffmpeg -version`` names.

    ``None`` if it does not name both.
    """
    found = [
        re.search(rf"^{library}\s+(\d+)\.", said, re.MULTILINE)
        for library in ("libavformat", "libavfilter")
    ]
    if found[0] is None or found[1] is None:
        return None
    return int(found[0][1]), int(found[1][1])


async def located(search: str, outside: Sequence[Path], *, cwd: Path) -> Tools:
    """Find ffmpeg on a search path, and ffprobe beside it, and ask what they are.

    Args:
        search: The search path, as the environment has it.
        outside: Directories that the programs must not come from, and that no
            link on the way to them may lie in: the directory that this program
            was started in, and the working directory of a session.
        cwd: Where ffmpeg is run to say what it is.

    Raises:
        MediaUnavailableError: There is no ffmpeg, no ffprobe beside it, or
            none that says it is new enough.
    """
    ffmpeg, beside, apart = await asyncio.to_thread(_found, search, outside)
    try:
        told = await process.run(
            [ffmpeg, "-hide_banner", "-version"], cwd=cwd, timeout_seconds=_ASKING_SECONDS
        )
    except OSError as error:
        raise MediaUnavailableError(
            f"the ffmpeg program {ffmpeg} could not be started: {error}"
        ) from error
    both = versions(told.output) if told.exit_code == 0 else None
    if both is None:
        raise MediaUnavailableError(
            f"the ffmpeg program {ffmpeg} did not say which version it is. {_INSTALL}"
        )
    avformat, avfilter = both
    if avformat < _OLDEST_FORMAT:
        raise MediaUnavailableError(
            f"the ffmpeg program {ffmpeg} is too old: it was built with libavformat "
            f"{avformat}, and ffmpeg 6.0 or newer is needed, with libavformat "
            f"{_OLDEST_FORMAT}. {_INSTALL}"
        )
    return Tools(ffmpeg=ffmpeg, ffprobe=beside, avformat=avformat, avfilter=avfilter, outside=apart)


def _found(search: str, outside: Sequence[Path]) -> tuple[str, str, tuple[Path, ...]]:
    """Return ffmpeg, the ffprobe beside it, and the directories that their ways must not touch.

    This blocks.

    Raises:
        MediaUnavailableError: There is no ffmpeg, or no ffprobe beside it.
    """
    ffmpeg = programs.found("ffmpeg", search, outside)
    if ffmpeg is None:
        raise MediaUnavailableError(
            f"no ffmpeg program was found on the search path. {_INSTALL}. A program that "
            "lies in the working directory is not taken"
        )
    beside = programs.found("ffprobe", str(Path(ffmpeg).parent), outside)
    if beside is None:
        raise MediaUnavailableError(
            f"there is no ffprobe beside the ffmpeg program {ffmpeg}. The two come from one "
            f"build, and neither is used without the other. {_INSTALL}"
        )
    return ffmpeg, beside, tuple(Path(os.path.realpath(place)) for place in outside)
