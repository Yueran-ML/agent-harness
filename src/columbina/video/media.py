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
import contextlib
import json
import os
import re
import shutil
import tempfile
from collections.abc import Generator, Sequence
from dataclasses import dataclass
from decimal import ROUND_HALF_EVEN, Decimal, InvalidOperation
from fractions import Fraction
from pathlib import Path
from typing import Any, Literal, cast

from columbina import process, programs
from columbina.data import Data

__all__ = [
    "DEMUXERS",
    "GUARDS",
    "AudioFacts",
    "Facts",
    "MediaUnavailableError",
    "Silence",
    "Tools",
    "UnreadableMediaError",
    "VideoFacts",
    "apart",
    "audio",
    "checked",
    "facts",
    "located",
    "probe_argv",
    "probed",
    "ran",
    "silences",
    "silences_in",
    "source",
    "versions",
]

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


# What a file holds


class VideoFacts(Data):
    """The picture of a source.

    Attributes:
        width: How wide it is shown: the stored width by the shape of a pixel, turned.
        height: How high it is shown.
        rotation: How far it is turned to be shown, counterclockwise: 0, 90, 180 or 270.
        rate: The rate of its frames, as a fraction such as ``30000/1001``. Of a source whose
            frames come at changing times, the standard rate nearest to their mean.
        variable: Whether its frames come at changing times.
        pixel_format: How a pixel is stored, as ffmpeg names it.
    """

    width: int
    height: int
    rotation: int = 0
    rate: str
    variable: bool = False
    pixel_format: str = ""


class AudioFacts(Data):
    """The sound of a source.

    Attributes:
        sample_rate: Samples in a second.
        channels: How many channels.
    """

    sample_rate: int
    channels: int


class Facts(Data):
    """What ffprobe says that a file holds.

    Attributes:
        kind: ``video``, ``audio`` or ``image``.
        format: The format, as ffprobe names it.
        duration_us: How long it lasts, in microseconds; 0 for a picture.
        video: The picture, if it has one that is no cover.
        audio: The sound, if it has any.
        warnings: What this version does not do for the file, for a person.
    """

    kind: Literal["video", "audio", "image"]
    format: str
    duration_us: int
    video: VideoFacts | None = None
    audio: AudioFacts | None = None
    warnings: tuple[str, ...] = ()


class UnreadableMediaError(Exception):
    """A file is no source that can be edited. The words say why."""


DEMUXERS = (
    "mov",
    "matroska",
    "avi",
    "mpegts",
    "wav",
    "mp3",
    "flac",
    "ogg",
    "aac",
    "png_pipe",
    "jpeg_pipe",
    "webp_pipe",
)
"""The formats that a source may be in. A list of other files, such as an HLS list or a concat
list, is none of them, so ffmpeg opens nothing that such a list names."""

_PICTURES = ("png_pipe", "jpeg_pipe", "webp_pipe")

GUARDS = ("-protocol_whitelist", "file", "-format_whitelist", ",".join(DEMUXERS))
"""What every run on a file from outside is given: files alone, of the formats above."""

_STANDARD_RATES = (
    "24000/1001",
    "24/1",
    "25/1",
    "30000/1001",
    "30/1",
    "48/1",
    "50/1",
    "60000/1001",
    "60/1",
)

_HDR = ("smpte2084", "arib-std-b67")

_RUN_SECONDS = 600.0
"""How long one run of ffmpeg on a source may take, other than a render."""


def source(path: Path) -> str:
    """Name a file to ffmpeg by the protocol of files.

    No part of its name is read as another protocol then, or as an option.
    """
    return f"file:{path}"


def probe_argv(tools: Tools, path: Path) -> list[str]:
    """Return what ffprobe is run with to say what a file holds."""
    return [
        tools.ffprobe,
        "-hide_banner",
        "-loglevel",
        "error",
        *GUARDS,
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        source(path),
    ]


def _fraction(said: object) -> Fraction | None:
    if not isinstance(said, str) or "/" not in said:
        return None
    top, _, bottom = said.partition("/")
    try:
        value = Fraction(int(top), int(bottom))
    except (ValueError, ZeroDivisionError):
        return None
    return value if value > 0 else None


def _microseconds(said: object) -> int | None:
    """Turn seconds that ffprobe writes in decimals into microseconds, without floats."""
    if not isinstance(said, str):
        return None
    try:
        seconds = Decimal(said)
    except InvalidOperation:
        return None
    if not seconds.is_finite() or seconds < 0:
        return None
    return int((seconds * 1_000_000).to_integral_value(rounding=ROUND_HALF_EVEN))


def _rotation(stream: dict[str, Any]) -> int:
    """How far the picture of a stream is turned to be shown, by its display matrix."""
    for side in cast("list[object]", stream.get("side_data_list") or []):
        turned = cast("dict[str, object]", side).get("rotation") if isinstance(side, dict) else None
        if isinstance(turned, (int, float)):
            return round(turned / 90) * 90 % 360
    return 0


def _rate(stream: dict[str, Any]) -> tuple[str, bool]:
    """Return the rate of the frames of a stream, and whether they come at changing times."""
    real = _fraction(stream.get("r_frame_rate"))
    mean = _fraction(stream.get("avg_frame_rate"))
    if real is None and mean is None:
        raise UnreadableMediaError("ffprobe says no rate of its frames")
    if real is not None and (mean is None or abs(mean - real) <= real / 100):
        return f"{real.numerator}/{real.denominator}", False
    seen = cast("Fraction", mean)
    nearest = min(_STANDARD_RATES, key=lambda rate: abs(cast("Fraction", _fraction(rate)) - seen))
    return nearest, real is not None


def _picture(stream: dict[str, Any]) -> VideoFacts:
    width, height = stream.get("width"), stream.get("height")
    if not isinstance(width, int) or not isinstance(height, int) or width <= 0 or height <= 0:
        raise UnreadableMediaError("ffprobe says no size of its picture")
    shape = _fraction(str(stream.get("sample_aspect_ratio", "1:1")).replace(":", "/"))
    if shape is not None and shape != 1:
        width = round(width * shape)
    rotation = _rotation(stream)
    if rotation in (90, 270):
        width, height = height, width
    rate, variable = _rate(stream)
    return VideoFacts(
        width=width,
        height=height,
        rotation=rotation,
        rate=rate,
        variable=variable,
        pixel_format=str(stream.get("pix_fmt", "")),
    )


def facts(answer: dict[str, Any]) -> Facts:
    """Say what a file holds, from what ffprobe answered of it.

    The first stream of pictures that is no cover gives the picture, and the first stream of
    sound gives the sound.

    Raises:
        UnreadableMediaError: It is in no format that is taken, holds neither picture nor
            sound, or ffprobe says too little of it.
    """
    shape = cast("dict[str, Any]", answer.get("format") or {})
    named = str(shape.get("format_name", ""))
    if not set(named.split(",")) & set(DEMUXERS):
        raise UnreadableMediaError(f"its format {named or 'that is not known'} is not taken")
    given = cast("list[object]", answer.get("streams") or [])
    streams = [cast("dict[str, Any]", one) for one in given if isinstance(one, dict)]
    # The cover of a song is a stream of pictures too, and no picture of the source.
    pictures = [
        one
        for one in streams
        if one.get("codec_type") == "video"
        and not cast("dict[str, Any]", one.get("disposition") or {}).get("attached_pic")
    ]
    sounds = [one for one in streams if one.get("codec_type") == "audio"]
    video = _picture(pictures[0]) if pictures else None
    audio = None
    if sounds:
        rate, channels = sounds[0].get("sample_rate"), sounds[0].get("channels")
        if isinstance(rate, str) and rate.isdigit() and isinstance(channels, int) and channels > 0:
            audio = AudioFacts(sample_rate=int(rate), channels=channels)
    if video is None and audio is None:
        raise UnreadableMediaError("it holds neither picture nor sound")
    kind: Literal["video", "audio", "image"] = (
        "image" if named in _PICTURES else "video" if video is not None else "audio"
    )
    duration = _microseconds(shape.get("duration"))
    if duration is None:
        if kind != "image":
            raise UnreadableMediaError("ffprobe says not how long it lasts")
        duration = 0
    warnings: list[str] = []
    if pictures:
        depth = str(pictures[0].get("pix_fmt", ""))
        if re.search(r"1[02]", depth):
            warnings.append(
                f"its picture has more than 8 bits a colour ({depth}), which is not converted"
            )
        if pictures[0].get("color_transfer") in _HDR:
            warnings.append("its picture is of a high dynamic range, which is not converted")
        if video is not None and video.variable:
            warnings.append(f"its frames come at changing times, and are taken at {video.rate}")
    return Facts(
        kind=kind,
        format=named,
        duration_us=0 if kind == "image" else duration,
        video=video,
        audio=audio,
        warnings=tuple(warnings),
    )


@contextlib.contextmanager
def apart(place: Path) -> Generator[Path]:
    """Make a directory for one run of ffmpeg in ``place``, and take it away afterwards.

    ffmpeg is never run in the working directory: a program on Windows looks for its libraries
    in the directory that it is run in.
    """
    place.mkdir(parents=True, exist_ok=True)
    made = Path(tempfile.mkdtemp(prefix=".run-", dir=place))
    try:
        yield made
    finally:
        shutil.rmtree(made, ignore_errors=True)


async def ran(
    tools: Tools, argv: Sequence[str], *, cwd: Path, seconds: float = _RUN_SECONDS
) -> str:
    """Run ffmpeg or ffprobe to its end, and return what it wrote.

    Raises:
        MediaUnavailableError: The program is no longer there to be run, or could not be started.
        UnreadableMediaError: It failed, or its time was up.
    """
    await asyncio.to_thread(checked, tools)
    try:
        done = await process.run(argv, cwd=cwd, timeout_seconds=seconds, streams="apart")
    except OSError as error:
        raise MediaUnavailableError(f"{argv[0]} could not be started: {error}") from error
    if done.timed_out:
        raise UnreadableMediaError(f"{Path(argv[0]).stem} took more than {seconds:g} seconds")
    if done.exit_code != 0:
        said = (done.error_output or done.output).strip().splitlines()
        raise UnreadableMediaError(
            f"{Path(argv[0]).stem} failed: {said[-1] if said else done.exit_code}"
        )
    return done.output


async def probed(tools: Tools, path: Path, *, place: Path) -> Facts:
    """Ask ffprobe what a file holds.

    Args:
        tools: The programs.
        path: The file.
        place: Where a directory for the run is made.

    Raises:
        MediaUnavailableError: ffprobe cannot be run.
        UnreadableMediaError: The file is no source that can be edited.
    """
    with apart(place) as cwd:
        said = await ran(tools, probe_argv(tools, path), cwd=cwd)
    try:
        answer: object = json.loads(said)
    except ValueError as error:
        raise UnreadableMediaError(f"ffprobe answered with no document: {error}") from None
    if not isinstance(answer, dict):
        raise UnreadableMediaError("ffprobe answered with no document")
    return facts(cast("dict[str, Any]", answer))


def _ffmpeg(tools: Tools, path: Path) -> list[str]:
    return [
        tools.ffmpeg,
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        *GUARDS,
        "-i",
        source(path),
    ]


async def audio(tools: Tools, path: Path, target: Path) -> None:
    """Take the sound of a file out, as 16 kHz, one channel and 16 bits: what a transcriber reads.

    It is written in a directory of its own beside ``target`` first, and moved there when done.

    Raises:
        MediaUnavailableError: ffmpeg cannot be run.
        UnreadableMediaError: The sound cannot be taken out.
    """
    with apart(target.parent) as cwd:
        await ran(
            tools,
            [
                *_ffmpeg(tools, path),
                "-vn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "sound.wav",
            ],
            cwd=cwd,
        )
        (cwd / "sound.wav").replace(target)


@dataclass(frozen=True)
class Silence:
    """A stretch of a source in which nothing louder than the threshold is heard.

    Attributes:
        start_us: Where it begins.
        end_us: Where it ends.
    """

    start_us: int
    end_us: int


def silences_in(written: str, duration_us: int) -> tuple[Silence, ...]:
    """Read what ``silencedetect`` wrote by way of ``ametadata``.

    A source that ends in silence has no end written for the last stretch: it ends with the source.
    """
    found: list[Silence] = []
    began: int | None = None
    for line in written.splitlines():
        key, _, value = line.strip().partition("=")
        at = _microseconds(value)
        if at is None:
            continue
        if key == "lavfi.silence_start":
            began = at
        elif key == "lavfi.silence_end" and began is not None:
            found.append(Silence(began, at))
            began = None
    if began is not None and began < duration_us:
        found.append(Silence(began, duration_us))
    return tuple(found)


async def silences(
    tools: Tools,
    path: Path,
    *,
    place: Path,
    duration_us: int,
    noise_db: float = -35.0,
    shortest_us: int = 300_000,
) -> tuple[Silence, ...]:
    """Find the stretches of silence in the sound of a file.

    They are written to a file of the run, by a bare name: an hour of talk has more pauses than
    the output of a run keeps.

    Raises:
        MediaUnavailableError: ffmpeg cannot be run.
        UnreadableMediaError: The sound cannot be read.
    """
    detect = (
        f"silencedetect=noise={noise_db:g}dB:d={shortest_us / 1_000_000:g},"
        "ametadata=mode=print:file=silences.txt"
    )
    with apart(place) as cwd:
        await ran(tools, [*_ffmpeg(tools, path), "-vn", "-af", detect, "-f", "null", "-"], cwd=cwd)
        written = (
            (cwd / "silences.txt").read_text(encoding="utf-8")
            if (cwd / "silences.txt").exists()
            else ""
        )
    return silences_in(written, duration_us)


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
