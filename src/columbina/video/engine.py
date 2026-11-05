"""The engine that has a project of video open, as the agent reaches it.

**The engine.** ``columbina-engine`` is a program of its own, and the only one that writes a
project: the desktop and the agent are its clients, send it changes, and are told of every
change in one order. Its protocol is ``engine/PROTOCOL.md`` of the repository, JSON-RPC 2.0 over
a named pipe of Windows or a socket of Unix, one message a line. :class:`Engine` is one
connection to it.

**What it is asked.** A transaction holds operations and edits (:mod:`columbina.video.edits`),
and is one step of the history of the project. A timeline is rendered by the engine too, in the
background, :meth:`Engine.render`: what it tells of the render is followed by the
:class:`Rendering` that is returned.

**Reaching it.** The engine that has a project open says where it listens in ``engine.lock`` of
the project. That address is tried first, and once more a moment later, since it may name an
engine that has just ended. Where no engine answers, one is started with ``--headless``: it
belongs to nobody, and ends by itself once no client has been connected for a while (30 seconds
unless it is told otherwise). Two clients that start one at once are sorted out by the engine:
the second is told where the first listens.

**Addresses.** A project may come from anywhere, and so may its ``engine.lock``. An address is
opened only if it is one where an engine listens: on Windows a pipe of this machine named
``columbina-engine-`` and 16 hexadecimal digits, never a pipe of another machine, to which
Windows would sign in; elsewhere a socket of that name in the directory of this user where the
engine makes its sockets. The same holds for an address that an engine says it, or another
engine, listens at.

**The program.** The engine is looked for the way :mod:`columbina.programs` looks for a
program, :func:`program_of`: the path that the config gives, or ``COLUMBINA_ENGINE``, or the
search path, and never in a working directory. It is run in its own directory, where it finds
the libraries of FFmpeg that it reads media with, and not in the directory of the project; a
build for development is also given those of ``FFMPEG_DIR``, :func:`_environment`, while a
packaged one has them beside it. It logs into a file of its own in ``cache/`` of the project,
``engine-<time>-<digits>.log``, which it writes itself, also once this program has ended; the
newest 20 are kept.
"""

import asyncio
import base64
import contextlib
import itertools
import json
import os
import random
import re
import secrets
import subprocess
import sys
import tempfile
import threading
import weakref
from collections import deque
from collections.abc import AsyncIterator, Callable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import IO, Any, Literal, Self, cast

from columbina import programs

__all__ = [
    "NOT_FOUND",
    "ONLY_WINDOWS",
    "PROTOCOL",
    "STUDIO_PLATFORMS",
    "UNEXPECTED",
    "UNREACHABLE",
    "UNSTARTED",
    "AlbumKind",
    "Applied",
    "AudioCodec",
    "Changed",
    "Changes",
    "Channel",
    "Chroma",
    "Engine",
    "EngineError",
    "KeyAlign",
    "Lut",
    "LutKind",
    "LutLibrary",
    "LutRoot",
    "LutStatus",
    "NodeSide",
    "Normalization",
    "NormalizeMode",
    "Progress",
    "RenderAudio",
    "RenderState",
    "Rendering",
    "SoundAnalysis",
    "Statistics",
    "StillFormat",
    "StillKeyframes",
    "StillLevel",
    "StillMode",
    "not_found",
    "program_of",
    "unsupported",
]

PROTOCOL = 2
"""The version of the protocol that is spoken."""

UNREACHABLE = -1
"""The code of an :class:`EngineError` when no engine could be reached, or the connection to it
was lost or closed. A call that failed so may or may not have been done."""

UNSTARTED = -2
"""The code of an :class:`EngineError` when the engine could not be started."""

UNEXPECTED = -3
"""The code of an :class:`EngineError` when the engine answered what the protocol does not say,
or more than can be read, or has another project open."""

_NAME = "columbina-engine"

NOT_FOUND = (
    "no columbina-engine program was found: build it with `cargo build --release -p "
    "columbina-server` in engine/ of the repository, and name it with COLUMBINA_ENGINE or put "
    "it on the search path. A program that lies in the working directory is not taken"
)
"""What is said where :func:`program_of` finds no engine, and none was named."""

_READ_BYTES = 1 << 30
"""The longest line that is read from the engine, without its newline: the engine sends a whole
project in one line, transcripts and all."""

_WRITE_BYTES = 16 * 1024 * 1024
"""The longest request that the engine reads, without its newline."""

_CONNECT_SECONDS = 5.0
"""How long connecting may take: a pipe of Windows that is busy is tried again for ever."""

_LISTENING_SECONDS = 20.0
"""How long an engine that was started may take to say where it listens."""

_TRIES = 8
"""How many times an engine is looked for and started before it is given up."""

_KEPT_LINES = 64
"""How many lines of its log an engine that ended before it listened is judged by."""

_KEPT_LOGS = 20
"""How many logs of engines are kept in the cache of a project."""

_YIELD_SECONDS = 5.0
"""How long an engine that yielded the project may take to be followed by the one it yielded to,
before one is started here."""

_YIELD_LOOK_SECONDS = 0.1
"""How often ``engine.lock`` is read meanwhile."""

_STOP_SECONDS = 5.0
"""How long a render that its caller gave up on while it was being started may take to be told
of, so that it is cancelled."""

_LISTENED = 1024
"""How many things told a reader of :meth:`Engine.changes` may leave unread before the oldest
are dropped."""

_LIBRARIES_MISSING = 0xC0000135
"""How a program of Windows ends when a library that it needs is not found."""

_PIPE = re.compile(r"\\\\\.\\pipe\\columbina-engine-[0-9a-f]{16}")
_SOCKET = re.compile(r"[0-9a-f]{16}\.sock")


class EngineError(Exception):
    """The engine refused, or could not be reached.

    Attributes:
        code: The code of the error of JSON-RPC that the engine answered, such as ``-32001`` for
            a transaction that was refused, or :data:`UNREACHABLE`, :data:`UNSTARTED` or
            :data:`UNEXPECTED` for what went wrong on this side. A request that cannot be sent
            is refused here as the engine would refuse it: ``-32600`` when it is longer than the
            engine reads, ``-32602`` when it cannot be written in JSON.
        data: What the engine said of it, as it said it: the ``index`` and ``reason`` of an
            operation that was refused, the ``path`` of a file that cannot be read.
    """

    def __init__(self, message: str, *, code: int, data: Mapping[str, Any] | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.data = data


@dataclass(frozen=True)
class Applied:
    """A change that the engine made to the project, as it answered it.

    Attributes:
        revision: The revision of the project that the change made.
        done: The operations as they were done, with every id that the engine gave.
        next_id: The ``next_id`` of the project after the change. A copy of the project that
            ``done`` is applied to is given it first, or it would refuse the new ids.
        names: The id that each ``$name`` of the transaction was given.
        clamped: The places in the transaction of the edits that went only as far as they could,
            as a trim that met the end of its media. An undo or a redo clamps nothing.
    """

    revision: int
    done: list[dict[str, Any]]
    next_id: int
    names: dict[str, str] = field(default_factory=dict[str, str])
    clamped: list[int] = field(default_factory=list[int])


@dataclass(frozen=True)
class Changed:
    """What the engine told every client, or what this side found out about the project.

    Attributes:
        method: ``project.changed``, ``engine.error``, ``transport.state``, ``media.proxy`` or
            ``render.progress``, as the engine sends them, or ``engine.yielding``, which an
            engine tells before it ends for the desktop's to take the project over. Or
            ``project.fetched``: the whole project was asked for anew, after a change was missed
            or the connection was made again, and its params are the answer of ``project.get``;
            a copy of the project starts again from it, and the changes after it follow. An
            engine that was started anew counts its revisions from 0 again. Or
            ``changes.missed``: the reader left so much unread that the oldest things told were
            dropped, ``missed`` of them; a copy starts again from :meth:`Engine.project`.
        params: What it said.
    """

    method: str
    params: dict[str, Any]

    @property
    def revision(self) -> int | None:
        """The revision of the project that it tells of, if it tells of one."""
        said = self.params.get("revision")
        return said if isinstance(said, int) and not isinstance(said, bool) else None


SoundAnalysis = Literal["audio.analyze", "audio.normalize", "audio.level_match"]
"""An analysis of sound that the engine runs as a job (:meth:`Engine.analyze_sound`)."""

RenderState = Literal["running", "done", "failed", "cancelled"]
"""Where a render stands: one that is not ``running`` is over."""

_RENDER_STATES: tuple[RenderState, ...] = ("running", "done", "failed", "cancelled")


@dataclass(frozen=True)
class Progress:
    """How far a render of the engine has come, as ``render.progress`` tells it.

    Attributes:
        job: The render that it tells of.
        state: ``running``, or how it ended: ``done``, ``failed`` or ``cancelled``.
        frame: How many frames are written.
        frames: How many frames it writes in all.
        fps: How many frames it writes a second, lately.
        path: The file that it wrote, once it is done.
        encoder: The encoder of FFmpeg that it wrote with (``h264_nvenc``, ``prores_ks``).
        message: Why it failed, or what it could not do (Audio Normalization of a film
            without sound, a standard that it could not meet).
        measuring: While Audio Normalization measures the sound, before the first frame is
            written: ``{"pass", "passes", "frame", "frames"}``, the pass and how many samples of
            it are measured.
        normalization: Once Audio Normalization has measured: ``{"standard", "mode",
            "target", "true_peak_max", "gain_db", "ceiling_db", "before", "after", "met"}``
            (``PROTOCOL.md``, The sound of an export).
    """

    job: str
    state: RenderState
    frame: int = 0
    frames: int = 0
    fps: float = 0.0
    path: str | None = None
    encoder: str | None = None
    message: str | None = None
    measuring: dict[str, Any] | None = None
    normalization: dict[str, Any] | None = None

    @property
    def over(self) -> bool:
        """Whether the render has ended, one way or another."""
        return self.state != "running"

    @classmethod
    def of(cls, params: Mapping[str, Any]) -> "Progress | None":
        """Read what ``render.progress`` says; ``None`` where it is not what the protocol says."""
        job, state = params.get("job"), params.get("state")
        if not isinstance(job, str) or state not in _RENDER_STATES:
            return None

        def whole(key: str) -> int:
            said = params.get(key)
            return said if type(said) is int else 0

        def text(key: str) -> str | None:
            said = params.get(key)
            return said if isinstance(said, str) else None

        def told(key: str) -> dict[str, Any] | None:
            said = params.get(key)
            return dict(cast("Mapping[str, Any]", said)) if isinstance(said, Mapping) else None

        fps = params.get("fps")
        return cls(
            job=job,
            state=state,
            frame=whole("frame"),
            frames=whole("frames"),
            fps=float(fps) if isinstance(fps, (int, float)) and not isinstance(fps, bool) else 0.0,
            path=text("path"),
            encoder=text("encoder"),
            message=text("message"),
            measuring=told("measuring"),
            normalization=told("normalization"),
        )


AudioCodec = Literal["aac", "pcm_s24le", "pcm_f32le"]
"""How the sound of an export is written: AAC (in ``.mp4``, ``.mov``, ``.m4a``), or PCM of 24
bits or of floats (in ``.mov``, ``.wav``)."""

NormalizeMode = Literal["normalize", "optimize"]
"""Audio Normalization of the Deliver page: a gain to whichever of the target and the highest
true peak it meets first, or to the target with a limiter of true peaks after it."""


@dataclass(frozen=True)
class Normalization:
    """Audio Normalization of one export, never of the project.

    See ``PROTOCOL.md``, The sound of an export.

    Attributes:
        standard: An id of the table of loudness standards (``ebu-r128``, ``spotify``).
        mode: How the loudness is brought to the target.
        target: In LUFS, from -60 to 0; the standard's target where it is not given.
    """

    standard: str
    mode: NormalizeMode = "normalize"
    target: float | None = None

    def params(self) -> dict[str, Any]:
        """As ``render.start`` takes it."""
        said: dict[str, Any] = {"standard": self.standard, "mode": self.mode}
        if self.target is not None:
            said["target"] = self.target
        return said


@dataclass(frozen=True)
class RenderAudio:
    """The sound of an export, each setting the engine's default where it is not given.

    See ``PROTOCOL.md``, The sound of an export.

    Attributes:
        bus: The bus rendered, a main output or the bus monitored; by default the bus monitored.
            The file has the channels of its format.
        rate: Samples a second, 7350 to 192000; by default the timeline's.
        codec: By default PCM of 24 bits in a ``.wav``, and AAC else.
        bitrate: Of AAC only: 128000, 192000 (the default), 256000 or 320000.
        normalize: Audio Normalization of this export.
    """

    bus: str | None = None
    rate: int | None = None
    codec: AudioCodec | None = None
    bitrate: int | None = None
    normalize: Normalization | None = None

    def params(self) -> dict[str, Any]:
        """As ``render.start`` takes it: only what is given."""
        said: dict[str, Any] = {}
        for key in ("bus", "rate", "codec", "bitrate"):
            value = getattr(self, key)
            if value is not None:
                said[key] = value
        if self.normalize is not None:
            said["normalize"] = self.normalize.params()
        return said


View = Literal["timeline", "source"]
"""A monitor of the engine: the timeline monitor or the source monitor."""

NodeSide = Literal["input", "output"]
"""The picture going into a node of a grade, or coming out of it."""

StillLevel = Literal["clip", "pre_clip", "post_clip"]
"""Which grade of its clip a still keeps: the Node Editor's mode (``PROTOCOL.md``, The Gallery)."""

StillFormat = Literal["auto", "png", "exr"]
"""How a still's picture is written: ``auto`` is a PNG of 16 bits where every value is within
0..1, else an OpenEXR of half floats."""

StillMode = Literal["apply", "append"]
"""Apply Grade (in place of a grade) or Append Node Graph (after its nodes)."""

StillKeyframes = Literal["none", "source_timecode", "start_frame"]
"""Apply Grades Using: without keys (the setting at the frame grabbed), or with them aligned by
source timecode or by start frames."""

KeyAlign = Literal["source_timecode", "start_frame"]
"""How keys copied to another clip are put on its frames."""

AlbumKind = Literal["stills", "powergrade", "memories"]
"""An album of the project's stills, of the PowerGrades that every project shares, or the
project's one album of the memories A-H, which the Gallery does not list."""

_CHANNELS = ("red", "green", "blue", "luma")


@dataclass(frozen=True)
class Channel:
    """The least, most and mean of a channel over a frame: R', G', B' or luma.

    They are signals from 0 to 1, or beyond where the frame is.
    """

    min: float
    max: float
    mean: float


@dataclass(frozen=True)
class Chroma:
    """The chroma of a frame in Cb and Cr, as the vectorscope shows it unzoomed.

    Cb and Cr are from -0.5 to 0.5 within the gamut.

    Attributes:
        centroid: The means of Cb and Cr: a cast where it is off the centre.
        spread: Their standard deviations.
        radius: The root mean square distance from the centroid.
        saturation: The mean distance from the centre.
        hue: The angle of the centroid in degrees counterclockwise from Cb, 0 to 360 (the skin
            tone line is at 123).
    """

    centroid: tuple[float, float]
    spread: tuple[float, float]
    radius: float
    saturation: float
    hue: float


@dataclass(frozen=True)
class Statistics:
    """The figures of every pixel of the frame that a monitor shows.

    They are what ``scopes.statistics`` answers (``PROTOCOL.md``, Scopes).

    Attributes:
        view: The monitor; ``None`` of a frame drawn off screen.
        frame: The frame shown: of the timeline, or of the media for the source monitor.
        width: Its width in pixels.
        height: Its height.
        samples: How many pixels were measured.
        red: R'.
        green: G'.
        blue: B'.
        luma: Luma, of Rec.709's coefficients until the project's colour settings say otherwise.
        chroma: Its chroma.
        percentiles: For each of ``red``, ``green``, ``blue`` and ``luma``, the signal below which
            ``"1"``, ``"50"`` and ``"99"`` percent of the pixels are.
        histogram: For each channel, how many pixels are in each of ``bins`` levels from 0 to 1.
        bins: How many levels the histogram has.
        timeline: The timeline of a frame drawn off screen or of a node.
        node: Of the picture at a node, ``{"grade", "node", "at"}``.
    """

    view: View | None
    frame: int
    width: int
    height: int
    samples: int
    red: Channel
    green: Channel
    blue: Channel
    luma: Channel
    chroma: Chroma
    percentiles: dict[str, dict[str, float]]
    histogram: dict[str, list[int]]
    bins: int
    timeline: str | None = None
    node: dict[str, Any] | None = None


@dataclass(frozen=True)
class LutRoot:
    """A folder that LUTs are looked for in: the project's, or one of the library.

    Attributes:
        place: ``project`` or ``library``.
        path: The folder, as the engine has it.
        exists: Whether it is there.
    """

    place: Literal["project", "library"]
    path: str
    exists: bool


LutKind = Literal["1d", "3d", "shaper+3d"]
"""What tables a ``.cube`` has, as ``luts.list`` tells it."""

LutStatus = Literal["ok", "error", "unchecked"]
"""Whether a ``.cube`` parses, as ``luts.list`` tells it."""

LutSize = Literal[17, 33, 65]
"""The points on an edge of a cube that ``grade.export_lut`` bakes, as Generate LUT offers."""


@dataclass(frozen=True)
class Lut:
    """A ``.cube`` file under a root, by the name that a corrector's ``lut`` gives it.

    Attributes:
        name: Its path from its root, ``/`` between folders.
        root: The index of its root in :attr:`LutLibrary.roots`.
        size: Its size in bytes.
        modified_us: When it was last written, in microseconds since 1970 (UTC).
        shadowed: Whether a root before it has a file of the same name, which a node of that
            name reads instead.
        kind: Its tables: ``1d``, ``3d``, or ``shaper+3d`` (a 1D shaper before the cube, as
            Resolve writes them); ``None`` where it cannot be told, or the engine does not say.
        status: Whether it parses as the engine reads it when it grades: ``ok``; ``error``
            (``message`` says why), a file that cannot be used; or ``unchecked``, not parsed by
            the list (its time ran out, or the file is very long), and so for an engine from
            before the list said it.
        message: Why it does not parse, where ``status`` is ``error``.
    """

    name: str
    root: int
    size: int
    modified_us: int | None
    shadowed: bool
    kind: LutKind | None = None
    status: LutStatus = "unchecked"
    message: str | None = None


@dataclass(frozen=True)
class LutLibrary:
    """The roots of LUTs and the LUTs in them, as ``luts.list`` answers them.

    The roots are in the order that a name is looked for in them.

    Attributes:
        roots: The folder of the project, then each folder of the library.
        luts: The LUTs, by root, then by name.
        truncated: Whether there were more than the engine lists.
    """

    roots: list[LutRoot]
    luts: list[Lut]
    truncated: bool


class Rendering:
    """A render that the engine does in the background, as :meth:`Engine.render` started it.

    What the engine tells of it is heard from before it was started, so none of it is missed.
    :meth:`progress` passes it on until the render is over, and :meth:`finished` waits for that.
    The render goes on in the engine whether or not it is followed here: :meth:`cancel` stops it.

    Attributes:
        job: What the engine knows the render by.
    """

    def __init__(self, engine: "Engine", job: str, heard: "Changes") -> None:
        self.job = job
        self._engine = engine
        self._heard = heard
        self._last: Progress | None = None

    @property
    def last(self) -> Progress | None:
        """What was told of the render last, if anything was."""
        return self._last

    async def progress(self) -> AsyncIterator[Progress]:
        """Pass on what the engine tells of the render, up to and with how it ended.

        Raises:
            EngineError: The connection to the engine was lost, or it was closed. The render may
                go on in the engine all the same.
        """
        if self._last is not None and self._last.over:
            yield self._last
            return
        try:
            async for told in self._heard:
                if told.method != "render.progress":
                    continue
                progress = Progress.of(told.params)
                if progress is None or progress.job != self.job:
                    continue
                self._last = progress
                yield progress
                if progress.over:
                    return
        finally:
            if self._last is not None and self._last.over:
                await self._heard.aclose()
        raise EngineError(
            f"the connection to the engine was closed before render {self.job} ended",
            code=UNREACHABLE,
        )

    async def finished(self, told: Callable[[Progress], object] | None = None) -> Progress:
        """Wait for the render to end, and return how it ended.

        Args:
            told: What is called with each thing told of it, as it comes, the last included.

        Raises:
            EngineError: The connection to the engine was lost, or it was closed.
        """
        async for progress in self.progress():
            if told is not None:
                told(progress)
        return cast("Progress", self._last)

    async def cancel(self) -> None:
        """Ask the engine to stop the render. Its end is told as ``cancelled``, unless it ended.

        Raises:
            EngineError: The engine refused, as for a render that it does not know.
        """
        await self._engine._call("render.cancel", {"job": self.job})  # pyright: ignore[reportPrivateUsage]

    async def aclose(self) -> None:
        """Hear nothing more of the render. It goes on in the engine."""
        await self._heard.aclose()


def program_of(configured: str | None, search: str, outside: Sequence[Path]) -> str | None:
    """Where the engine is: the configured path, COLUMBINA_ENGINE, or the search path.

    A path that the config or the environment gives is taken as it is, if it is whole and the
    way to it leads through none of the directories ``outside``; it is never looked for
    elsewhere. Without one, ``columbina-engine`` is looked for on the search path, never in the
    working directory or the other ``outside`` places, the same rule as for ffmpeg's
    :func:`~columbina.video.media.located`. This blocks.

    Args:
        configured: The path of the program that the config of the plugin gives, if any.
        search: The search path, as the environment has it.
        outside: The directories that the program must not come from: the directory that this
            program was started in, and the working directory of a session.

    Returns:
        The whole path of the program, or ``None`` if there is none that may be run. Then
        :func:`not_found` says why.
    """
    named = configured or os.environ.get("COLUMBINA_ENGINE")
    if not named:
        return programs.found(_NAME, search, outside)
    program = Path(named).expanduser()
    apart = [Path(os.path.realpath(place)) for place in outside]
    if (
        program.is_absolute()
        and program.is_file()
        and os.access(program, os.X_OK)
        and programs.trusted(program, apart)
    ):
        return str(program)
    return None


STUDIO_PLATFORMS = frozenset({"win32"})
"""Where the editing studio runs, as ``sys.platform`` names it: Windows for now. The engine is
built and tested there only; macOS comes later. The rest of the agent runs everywhere."""

ONLY_WINDOWS = "video editing currently needs Windows; macOS support will come later"
"""What is said instead of editing on any other platform."""


def unsupported() -> str | None:
    """Say why video cannot be edited on this machine, or return None where it can."""
    return None if sys.platform in STUDIO_PLATFORMS else ONLY_WINDOWS


def not_found(configured: str | None) -> str:
    """Say why :func:`program_of` found no engine, given what the config named."""
    named = configured or os.environ.get("COLUMBINA_ENGINE")
    if not named:
        return NOT_FOUND
    given = "the config of the video plugin" if configured else "COLUMBINA_ENGINE"
    return (
        f"{given} names {named} as the columbina-engine program, and it is not taken: it has to "
        "be the whole path of a program, and the way to it may not lead through the working "
        "directory"
    )


def _argv(program: str, directory: Path, idle_seconds: int | None) -> list[str]:
    """What the engine is started with: always headless, as an agent has no window to give it."""
    argv = [program, "--project", str(directory), "--headless"]
    if idle_seconds is not None:
        argv += ["--idle-seconds", str(idle_seconds)]
    return argv


if sys.platform == "win32":  # pragma: windows only

    def _engine_at(address: str) -> bool:
        return _PIPE.fullmatch(address) is not None

else:  # pragma: posix only

    def _engine_at(address: str) -> bool:
        where = Path(address)
        return _SOCKET.fullmatch(where.name) is not None and where.parent == _sockets()

    def _sockets() -> Path:
        """The directory where the engine makes its sockets, as it chooses it."""
        runtime = os.environ.get("XDG_RUNTIME_DIR")
        if runtime and Path(runtime).is_absolute():
            return Path(runtime) / "columbina"
        temporary = os.environ.get("TMPDIR")
        base = Path(temporary) if temporary and Path(temporary).is_absolute() else Path("/tmp")
        return base / f"columbina-{os.geteuid()}"


def _address(said: object) -> str | None:
    """Return what was said, if it is an address where an engine listens, and none other."""
    return said if isinstance(said, str) and _engine_at(said) else None


def _locked(directory: Path) -> str | None:
    """Return the address that ``engine.lock`` of a project names, if it names one.

    This blocks.
    """
    try:
        said: object = json.loads((directory / "engine.lock").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return (
        _address(cast("dict[str, object]", said).get("address")) if isinstance(said, dict) else None
    )


async def _moment() -> None:
    """Wait the moment after which an address that did not answer is looked up again."""
    await asyncio.sleep(random.uniform(0.05, 0.1))


class _Wire(asyncio.Protocol):
    """One connection to the engine: what it sends cut into lines.

    Only what came since the last look is looked through for the end of a line.
    """

    def __init__(
        self,
        heard: Callable[["_Wire", bytes], None],
        lost: Callable[["_Wire", EngineError], None],
    ) -> None:
        self._heard = heard
        self._lost = lost
        self._buffer = bytearray()
        self._looked = 0
        self._ended = False
        self.transport: asyncio.Transport | None = None

    def connection_made(self, transport: asyncio.BaseTransport) -> None:
        self.transport = cast("asyncio.Transport", transport)

    def data_received(self, data: bytes) -> None:
        if self._ended:
            return
        self._buffer += data
        start = 0
        while True:
            end = self._buffer.find(b"\n", max(start, self._looked))
            if end < 0:
                break
            if end - start > _READ_BYTES:
                self._too_long()
                return
            line = bytes(self._buffer[start:end])
            start = end + 1
            self._heard(self, line)
        del self._buffer[:start]
        self._looked = len(self._buffer)
        if self._looked > _READ_BYTES:
            self._too_long()

    def _too_long(self) -> None:
        self._end(
            EngineError(
                f"the engine sent a line longer than {_READ_BYTES / 2**20:g} MiB: the project is "
                "too large to be read here",
                code=UNEXPECTED,
            )
        )
        self.close()

    def connection_lost(self, exc: Exception | None) -> None:
        why = "the engine closed the connection" + (f": {exc}" if exc else "")
        self._end(EngineError(why, code=UNREACHABLE))

    def _end(self, error: EngineError) -> None:
        if not self._ended:
            self._ended = True
            self._buffer.clear()
            self._lost(self, error)

    @staticmethod
    def encoded(message: Mapping[str, Any]) -> bytes:
        """Write a message as a line, without its newline.

        Raises:
            EngineError: It cannot be written in JSON, or is longer than the engine reads.
        """
        try:
            line = json.dumps(message, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
        except (ValueError, TypeError) as error:
            raise EngineError(
                f"the request cannot be written in JSON: {error}", code=-32602
            ) from None
        written = line.encode("utf-8")
        if len(written) > _WRITE_BYTES:
            raise EngineError(
                f"the request is {len(written)} bytes long, and the engine reads none longer "
                "than 16 MiB",
                code=-32600,
                data={"bytes": len(written)},
            )
        return written

    def write(self, line: bytes) -> None:
        """Send a line."""
        cast("asyncio.Transport", self.transport).write(line + b"\n")

    def close(self) -> None:
        """Close the connection. Its loss is told as any other's."""
        cast("asyncio.Transport", self.transport).close()


async def _connected(
    address: str,
    heard: Callable[[_Wire, bytes], None],
    lost: Callable[[_Wire, EngineError], None],
) -> _Wire | None:
    """Connect to an address, or return ``None`` where nothing answers there.

    The connection that is returned has its transport, whether or not its ``connection_made``
    has run yet: a pipe of Windows is returned before it runs, a moment later. Nor is the
    connection made in a task of its own, whose transport would be left open if this were
    cancelled after it was made and before it was taken.
    """
    loop = asyncio.get_running_loop()
    wire = _Wire(heard, lost)
    try:
        async with asyncio.timeout(_CONNECT_SECONDS):
            if sys.platform == "win32":  # pragma: windows only
                proactor = cast("asyncio.ProactorEventLoop", loop)
                # Any protocol is taken, whatever the stubs of typeshed say.
                pipe, _ = await proactor.create_pipe_connection(
                    lambda: wire,  # pyright: ignore[reportArgumentType]
                    address,
                )
                transport = cast("asyncio.Transport", pipe)
            else:  # pragma: posix only
                transport, _ = await loop.create_unix_connection(lambda: wire, address)
    except (OSError, TimeoutError):
        return None
    wire.transport = transport
    return wire


def _log_of(directory: Path) -> IO[bytes]:
    """Make the file that an engine that is started logs into, and take old ones away.

    Where the cache of the project cannot hold it, a file of the system's that is gone once it
    is closed. This blocks.
    """
    cache = directory / "cache"
    try:
        cache.mkdir(parents=True, exist_ok=True)
        for old in sorted(cache.glob("engine-*.log"))[: 1 - _KEPT_LOGS]:
            # One that an engine still writes is not taken away on Windows.
            with contextlib.suppress(OSError):
                old.unlink()
        named = f"engine-{datetime.now(UTC):%Y%m%dT%H%M%S}-{secrets.token_hex(4)}.log"
        return (cache / named).open("x+b")
    except OSError:
        return tempfile.TemporaryFile()


_LIBRARIES = {"win32": ("bin", "PATH"), "darwin": ("lib", "DYLD_FALLBACK_LIBRARY_PATH")}
"""Where a build of FFmpeg keeps its shared libraries, and where a system looks for them, by
system; elsewhere ``lib`` and ``LD_LIBRARY_PATH``."""


def _environment() -> dict[str, str] | None:
    """The environment that an engine is started with: this one's, with the libraries of FFmpeg.

    A build of the engine for development reads media with the libraries of the build of FFmpeg
    that ``FFMPEG_DIR`` names, which need not be on the search path of this program: the
    directory that holds them is put first where the engine looks for libraries, so that no
    other build of libav found earlier, of the GPL perhaps, is loaded in their place. The
    environment of this program is left as it is. A packaged engine has its libraries beside
    it, where the system finds them, and needs nothing of this; ``None`` then, as also where
    ``FFMPEG_DIR`` is not set or holds no such directory.
    """
    built = os.environ.get("FFMPEG_DIR")
    where, variable = _LIBRARIES.get(sys.platform, ("lib", "LD_LIBRARY_PATH"))
    libraries = Path(built) / where if built else None
    if libraries is None or not libraries.is_dir():
        return None
    environment = dict(os.environ)
    before = environment.get(variable)
    environment[variable] = f"{libraries}{os.pathsep}{before}" if before else str(libraries)
    return environment


def _spawned(argv: list[str], cwd: Path, log: IO[bytes]) -> "subprocess.Popen[bytes]":
    """Start an engine apart from the console of this program, and from its job on Windows.

    Its Ctrl+C or closing does not end the engine then, nor does a job of the system that ends
    every process of this one: other clients may still use the engine. This blocks.
    """

    def started(
        creationflags: int = 0, start_new_session: bool = False
    ) -> "subprocess.Popen[bytes]":
        return subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=log,
            cwd=cwd,
            env=_environment(),
            creationflags=creationflags,
            start_new_session=start_new_session,
        )

    if sys.platform == "win32":  # pragma: windows only
        flags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW
        try:
            return started(creationflags=flags | subprocess.CREATE_BREAKAWAY_FROM_JOB)
        except PermissionError:
            # A job that lets no process break away: the engine stays in it.
            return started(creationflags=flags)
    else:  # pragma: posix only
        return started(start_new_session=True)


def _first_line(process: "subprocess.Popen[bytes]") -> bytes:
    """Read what an engine says first on stdout, and close it: nothing else is ever written there.

    This blocks.
    """
    stdout = cast("IO[bytes]", process.stdout)
    try:
        return stdout.readline()
    finally:
        stdout.close()


def _ended_with(process: "subprocess.Popen[bytes]", log: IO[bytes]) -> tuple[int, list[bytes]]:
    """Wait for an engine to end, and return its exit code and the last lines that it logged.

    An engine that ended before it listened leaves no log behind: what it said is told, and
    several clients that start one at once would leave one each. This blocks.
    """
    code = process.wait()
    log.seek(0)
    said = log.read().splitlines()[-_KEPT_LINES:]
    log.close()
    # A file of the system's has no name of a path on every system, and is gone already.
    with contextlib.suppress(OSError, TypeError):
        Path(log.name).unlink()
    return code, said


class Changes:
    """What the engine tells, from the moment :meth:`Engine.changes` was called, in its order.

    The connection lost is raised as an :class:`EngineError` once; the next call of the engine
    connects again, and what is told goes on, beginning with ``project.fetched``. A reader that
    leaves more than 1024 things unread loses the oldest, and is told ``changes.missed`` in their
    place. The changes end when the engine is closed, or when the reader closes them, with
    :meth:`aclose` or as a context.
    """

    def __init__(self, readers: "weakref.WeakSet[Changes]") -> None:
        self._readers = readers
        self._told: deque[Changed | EngineError] = deque()
        self._missed = 0
        self._ended = False
        self._ready = asyncio.Event()

    def tell(self, told: Changed | EngineError) -> None:
        """Pass on something that was told."""
        if len(self._told) >= _LISTENED:
            self._told.popleft()
            self._missed += 1
        self._told.append(told)
        self._ready.set()

    def end(self) -> None:
        """Tell nothing more once what was told is read."""
        self._ended = True
        self._ready.set()

    async def aclose(self) -> None:
        """Hear nothing more."""
        self._readers.discard(self)
        self._told.clear()
        self._missed = 0
        self.end()

    def __aiter__(self) -> Self:
        return self

    async def __anext__(self) -> Changed:
        while not self._told and not self._ended:
            self._ready.clear()
            await self._ready.wait()
        if self._missed:
            missed, self._missed = self._missed, 0
            return Changed("changes.missed", {"missed": missed})
        if not self._told:
            raise StopAsyncIteration
        told = self._told.popleft()
        if isinstance(told, EngineError):
            raise told
        return told

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()


@dataclass
class _Asked:
    """A request that waits for its answer."""

    method: str
    answer: "asyncio.Future[Any]"
    fetching: bool = False


class Engine:
    """A connection to the engine that has a project open, as one client among others.

    Made by :meth:`attach`. Every call is answered by the engine, or raises
    :class:`EngineError`. A connection that was lost is made again by the next call, to the same
    engine or to one that is started anew, which counts the revisions of the project from 0
    again. A call that failed with :data:`UNREACHABLE` may or may not have been done: what was
    done is in the project, and is told as any change.

    Attributes:
        directory: The directory of the project, as a whole path.
        hello: What the engine answered to ``hello``: its version (``engine``), the ``protocol``,
            the ``project`` as it writes the directory, and its ``pid``.
    """

    def __init__(
        self, directory: Path, program: str, client: str, idle_seconds: int | None
    ) -> None:
        self.directory = directory
        self._program = program
        self._client = client
        self._idle_seconds = idle_seconds
        self.hello: dict[str, Any] = {}
        self._wire: _Wire | None = None
        self._asked: dict[int, _Asked] = {}
        self._ids = itertools.count(1)
        self._readers: weakref.WeakSet[Changes] = weakref.WeakSet()
        self._revision = -1
        self._session = 0
        self._next_id = 0
        self._fetching = False
        self._closed = False
        self._yielded = False
        self._reaching = asyncio.Lock()

    @classmethod
    async def attach(
        cls,
        project: Path,
        *,
        program: str,
        client: str = "agent",
        idle_seconds: int | None = None,
    ) -> "Engine":
        """Connect to the engine that has a project open, starting one if none has.

        Args:
            project: The directory of the project. It is made if it is not there.
            program: The engine program, as :func:`program_of` found it.
            client: What this client is: ``agent`` or ``cli``.
            idle_seconds: How long an engine that is started here waits with no client before
                it ends, in whole seconds. The engine's own default, 30, unless it is given.

        Raises:
            EngineError: No engine could be reached or started, or the engine refused.
        """
        made = cls(await asyncio.to_thread(project.resolve), program, client, idle_seconds)
        await made._reach()
        return made

    @property
    def revision(self) -> int:
        """The latest revision of the project that this client was told of."""
        return self._revision

    @property
    def next_id(self) -> int:
        """The ``next_id`` of the project at that revision."""
        return self._next_id

    @property
    def session(self) -> int:
        """How many times this client has greeted an engine.

        An engine that was started anew counts the revisions from 0 again, so a revision alone
        does not say which project it is of: the revision and the session together do.
        """
        return self._session

    @property
    def headless(self) -> bool | None:
        """Whether the engine is one without a window, as ``hello`` says; ``None`` if it does not.

        A headless engine may end to let the desktop's take the project over. The next call then
        reaches the desktop's engine by ``engine.lock``, as it reaches any engine that took over.
        """
        said = self.hello.get("headless")
        return said if isinstance(said, bool) else None

    async def project(self) -> tuple[int, dict[str, Any]]:
        """Return the project as it is now, and its revision."""
        said = await self._call("project.get")
        project = said.get("project")
        if not isinstance(project, dict):
            raise EngineError("the engine answered project.get with no project", code=UNEXPECTED)
        return _whole(said, "revision"), cast("dict[str, Any]", project)

    async def apply(
        self,
        ops: Sequence[Mapping[str, Any]],
        *,
        revision: int | None = None,
        session: int | None = None,
        gesture: Mapping[str, Any] | None = None,
    ) -> Applied:
        """Apply operations and edits to the project, all or none, as one step of its history.

        An operation names itself in ``op``, an edit, such as an insert or a ripple delete, in
        ``edit`` (:mod:`columbina.video.edits` writes both). The engine works each edit out into
        operations as it comes to it, and ``done`` holds those operations only. A thing that an
        operation adds may be given an id of the form ``$name``, which later operations of the
        same call may name; the ids that the engine gave are in ``names``.

        When ``revision`` is given, the engine applies only if that is still the project's
        revision. This prevents ranges computed from a snapshot cutting a concurrently edited
        timeline; a mismatch is refused with ``stale_revision``, without changing history.
        ``session`` is the :attr:`session` that the revision was read in: where this client has
        greeted another engine since, which counts its revisions from 0 again, the transaction
        is refused in the same way without being sent.

        ``gesture`` (``{"id", "end"?}``, PROTOCOL "Gestures") puts the transaction into one step
        of the history with the others of the same gesture, until one with ``end`` true.

        Raises:
            EngineError: The transaction was refused (``-32001``, with the ``index`` of the
                operation or the edit and the ``reason`` in ``data``, as ``locked`` or
                ``ripple_blocked``), or an operation or a name could not be read (``-32602``).
        """
        params: dict[str, Any] = {"ops": [dict(op) for op in ops]}
        if revision is not None:
            params["revision"] = revision
        if gesture is not None:
            params["gesture"] = dict(gesture)
        wire = await self._ensure()
        if session is not None and session != self._session:
            raise EngineError(
                "the engine was started again since the project was read; read it again",
                code=-32001,
                data={"reason": "stale_revision", "session": self._session},
            )
        said = await self._asked_on(wire, "transaction.apply", params)
        return _applied(said, said.get("names"))

    async def undo(self) -> Applied | None:
        """Take back the latest step of the project, whoever made it; ``None`` if there is none."""
        return _stepped(await self._call("history.undo"))

    async def redo(self) -> Applied | None:
        """Do again the step undone last; ``None`` if there is none."""
        return _stepped(await self._call("history.redo"))

    async def imported(
        self, paths: Sequence[Path], *, bin: str | None = None
    ) -> tuple[int, list[str]]:
        """Take files in as media of the project, as one step, and return the revision and the ids.

        Every path is to be absolute: the engine refuses a relative one (``-32602``) and takes
        nothing in. Neither is anything taken in if one file cannot be read (``-32002``, with its
        ``path`` and ``reason``).
        """
        params: dict[str, Any] = {"paths": [str(path) for path in paths]}
        if bin is not None:
            params["bin"] = bin
        said = await self._call("media.import", params)
        added = said.get("added")
        if not isinstance(added, list) or not all(
            isinstance(one, str) for one in cast("list[object]", added)
        ):
            raise EngineError("the engine answered media.import with no ids", code=UNEXPECTED)
        return _whole(said, "revision"), cast("list[str]", added)

    async def duration(self, timeline: str) -> int:
        """Return how many frames a timeline lasts: to the end of the last thing on it.

        Raises:
            EngineError: There is no such timeline (``-32602``).
        """
        return _whole(await self._call("timeline.duration", {"timeline": timeline}), "frames")

    async def inspected(self, paths: Sequence[Path]) -> list[dict[str, Any]]:
        """Read source identities and facts without changing the project or history.

        The returned media have empty ids and can be added alongside a timeline in one
        transaction. This allows a whole legacy migration to be undone in one step.
        """
        said = await self._call("media.inspect", {"paths": [str(path) for path in paths]})
        items = said.get("media")
        if not isinstance(items, list) or not all(
            isinstance(item, dict) for item in cast("list[object]", items)
        ):
            raise EngineError("the engine answered media.inspect with no media", code=UNEXPECTED)
        return cast("list[dict[str, Any]]", items)

    async def statistics(
        self,
        view: View = "timeline",
        *,
        bins: int | None = None,
        timeline: str | None = None,
        frame: int | None = None,
        grade: Mapping[str, Any] | None = None,
        node: str | None = None,
        at: NodeSide | None = None,
        clip: str | None = None,
    ) -> Statistics:
        """Return the figures of a frame, measured on the card.

        Of the frame that the monitor ``view`` shows now, which only an engine with monitors
        (the desktop's) has; or, where ``timeline`` is given, of its ``frame`` drawn off screen
        as an export draws it, in a headless engine too. With ``grade`` (a target of grades) and
        ``node``, of the picture going into the node or coming out of it (``at``, ``output``
        unless given; ``clip`` names the clip of a group's grade). ``bins`` is the number of
        levels of the histogram, 16 to 1024 (256 unless given).

        Raises:
            EngineError: The engine is headless and a monitor is named (``-32601``), the monitor
                shows nothing (``-32004``, ``not_loaded``), or a parameter is out of range or
                names nothing there (``-32602``, with its ``reason``).
        """
        params: dict[str, Any] = (
            {"view": view} if timeline is None else {"timeline": timeline, "frame": frame}
        )
        if grade is not None:
            params["grade"] = dict(grade)
            params["node"] = node
        if at is not None:
            params["at"] = at
        if clip is not None:
            params["clip"] = clip
        if bins is not None:
            params["bins"] = bins
        return _statistics(await self._call("scopes.statistics", params))

    async def luts(self, folder: str | None = None) -> LutLibrary:
        """Return the folders that LUTs are looked for in, and the ``.cube`` files in them.

        ``folder`` lists only that folder of each root: a relative path of plain names.

        Raises:
            EngineError: The folder leads out of the roots (``-32602``, ``outside_roots``).
        """
        said = await self._call("luts.list", {} if folder is None else {"folder": folder})
        return _library(said)

    async def export_lut(
        self,
        grade: Mapping[str, Any],
        path: str | Path,
        *,
        size: LutSize = 33,
        node: str | None = None,
        frame: int | None = None,
        replace: bool = False,
    ) -> dict[str, Any]:
        """Bake a grade, or one corrector of it, into a 3D LUT written as a ``.cube``.

        As Generate LUT of DaVinci Resolve: ``size`` points on an edge, what the grade does to
        each colour on its own (Primaries, curves, Soft Clip, the LUTs of nodes); a node that
        reads the pixels about each one (Midtone Detail) is left out and named in ``ignored``.
        ``path`` is a name in the project folder, or an absolute path. ``frame`` is where keys
        are read. The answer is ``{"path", "name", "size", "nodes", "ignored"}``; ``name`` is
        what a corrector's ``lut`` names it by (``PROTOCOL.md``, Generate LUT).

        Raises:
            EngineError: The size, path, grade or node is refused (``-32602``), or a LUT of a
                node cannot be read, the file is there without ``replace``, or it cannot be
                written (``-32001``).
        """
        method = "grade.export_lut"
        params: dict[str, Any] = {"grade": dict(grade), "size": size, "path": str(path)}
        if node is not None:
            params["node"] = node
        if frame is not None:
            params["frame"] = frame
        if replace:
            params["replace"] = True
        return _object(await self._call(method, params), method)

    async def grab_still(
        self,
        *,
        album: str | None = None,
        clip: str | None = None,
        level: StillLevel = "clip",
        format: StillFormat = "auto",
    ) -> dict[str, Any]:
        """Grab a still of the frame that the timeline monitor shows, with its grades.

        It goes into ``album`` (the project's first where none is named), of ``clip`` (the one on
        top at the playhead where none is named), keeping its grade at ``level``. The answer is
        the still as ``gallery.list`` gives it: ``id``, ``name`` (Track.Shot.Version), ``label``,
        ``image`` and ``source`` (``PROTOCOL.md``, The Gallery).

        Raises:
            EngineError: The engine is headless (``-32601``), no timeline is loaded (``-32004``),
                or the album or clip is not there (``-32602``).
        """
        params: dict[str, Any] = {"view": "timeline"}
        if album is not None:
            params["album"] = album
        if clip is not None:
            params["clip"] = clip
        if level != "clip":
            params["level"] = level
        if format != "auto":
            params["format"] = format
        return _object(await self._call("still.grab", params), "still.grab")

    async def gallery(self, album: str | None = None) -> list[dict[str, Any]]:
        """Return the albums of the Gallery, the project's then the PowerGrades', with stills.

        Each album is ``{"id", "name", "kind", "stills"}``; ``album`` gives that one alone.
        """
        method = "gallery.list"
        said = await self._call(method, {} if album is None else {"album": album})
        return _objects(said.get("albums"), method)

    async def still(self, still: str) -> dict[str, Any]:
        """Return all that a still holds (``PROTOCOL.md``, What a still holds).

        That is its grades with their keys and at the frame grabbed, and where it came from.
        """
        return _object(await self._call("still.get", {"still": still}), "still.get")

    async def still_thumbnail(self, still: str, height: int = 90) -> tuple[int, int, bytes]:
        """Return a PNG of a still ``height`` high (1 to 270): its width, height and bytes."""
        method = "gallery.thumbnail"
        said = await self._call(method, {"still": still, "height": height})
        png = said.get("png")
        if not isinstance(png, str):
            raise _unexpected(method)
        try:
            data = base64.b64decode(png, validate=True)
        except ValueError as error:
            raise _unexpected(method) from error
        return _whole(said, "width"), _whole(said, "height"), data

    async def apply_still(
        self,
        still: str,
        to: Sequence[Mapping[str, Any]],
        *,
        mode: StillMode = "apply",
        keyframes: StillKeyframes = "none",
        preserve_nodes: int = 0,
        revision: int | None = None,
    ) -> Applied:
        """Apply a still's grade to grades of the project, as one step of its history.

        ``to`` are targets of grades (``{"level": "clip", "timeline", "clip"}``, a group's, or a
        timeline's, which takes the still's timeline grade). ``mode`` ``apply`` puts the grade in
        place of each, keeping the first ``preserve_nodes`` nodes of each; ``append`` puts its
        nodes after theirs. ``keyframes`` is Apply Grades Using.

        Raises:
            EngineError: It was refused (``-32001``: ``no_grade``, ``missing``, ``wrong_node``,
                ``out_of_range``, ``stale_revision``), or the still is not there (``-32602``).
        """
        params: dict[str, Any] = {
            "still": still,
            "to": [dict(target) for target in to],
            "mode": mode,
            "keyframes": keyframes,
        }
        if preserve_nodes:
            params["preserve_nodes"] = preserve_nodes
        if revision is not None:
            params["revision"] = revision
        said = await self._call("still.apply", params)
        return _applied(said, said.get("names"))

    async def append_node(
        self,
        grade: Mapping[str, Any],
        node: str,
        to: Sequence[Mapping[str, Any]],
        *,
        keyframes: StillKeyframes = "source_timecode",
        at: int | None = None,
        revision: int | None = None,
    ) -> Applied:
        """Append the corrector ``node`` of ``grade`` to the end of each grade of ``to``.

        It is Append Node to Selected Clips, as one step of the history. With ``keyframes``
        ``none`` each keyed setting is the value of its key at the frame ``at`` of the timeline
        (the playhead) or the nearest before it.

        Raises:
            EngineError: It was refused (``-32001``).
        """
        params: dict[str, Any] = {
            "from": dict(grade),
            "node": node,
            "to": [dict(target) for target in to],
            "keyframes": keyframes,
        }
        if at is not None:
            params["at"] = at
        if revision is not None:
            params["revision"] = revision
        said = await self._call("grade.append_node", params)
        return _applied(said, said.get("names"))

    async def label_still(self, still: str, label: str) -> None:
        """Label a still (at most 100 characters; empty for none)."""
        await self._call("still.label", {"still": still, "label": label})

    async def delete_stills(self, stills: Sequence[str]) -> list[str]:
        """Delete stills from the Gallery, all or none; return their ids.

        ``restore_stills`` puts them back (the last 64 deleted of a gallery).
        """
        method = "still.delete"
        said = await self._call(method, {"stills": list(stills)})
        return _strings(said.get("deleted"), method)

    async def restore_stills(self, stills: Sequence[str]) -> dict[str, str]:
        """Put stills deleted back where they were, under their ids; return the album of each."""
        method = "still.restore"
        return _ids(await self._call(method, {"stills": list(stills)}), "restored", method)

    async def copy_stills(self, stills: Sequence[str], album: str) -> dict[str, str]:
        """Copy stills into an album under new ids; return the id of each copy."""
        method = "still.copy"
        params = {"stills": list(stills), "album": album}
        return _ids(await self._call(method, params), "copied", method)

    async def move_stills(self, stills: Sequence[str], album: str) -> dict[str, str]:
        """Move stills to another album; return the id of each where it is now.

        A still moved between the project's albums and the PowerGrades' is given a new id.
        """
        method = "still.move"
        params = {"stills": list(stills), "album": album}
        return _ids(await self._call(method, params), "moved", method)

    async def create_album(
        self, kind: AlbumKind = "stills", name: str | None = None
    ) -> dict[str, Any]:
        """Add an album of the project's stills or of the PowerGrades; return it.

        It is ``{"id", "name", "kind"}``; without a name, ``Stills N`` or ``PowerGrade N``. Of
        ``memories``, the project's album of the memories, made the first time.
        """
        params: dict[str, Any] = {"kind": kind}
        if name is not None:
            params["name"] = name
        return _object(await self._call("gallery.album.create", params), "gallery.album.create")

    async def rename_album(self, album: str, name: str) -> None:
        """Rename an album (1 to 100 characters)."""
        await self._call("gallery.album.rename", {"album": album, "name": name})

    async def delete_album(self, album: str) -> list[str]:
        """Remove an album and its stills; return the ids of its stills.

        The last album of a gallery stays (``-32001``, ``last_album``).
        """
        method = "gallery.album.delete"
        said = await self._call(method, {"album": album})
        return _strings(said.get("deleted"), method)

    async def export_stills(
        self,
        stills: Sequence[str],
        directory: Path,
        *,
        prefix: str | None = None,
        replace: bool = False,
    ) -> list[dict[str, Any]]:
        """Write each still's picture and its JSON into ``directory`` (absolute).

        They are named ``<prefix>_<number>``; the answer is ``{"image", "json"}`` of each.
        """
        method = "still.export"
        params: dict[str, Any] = {"stills": list(stills), "dir": str(directory)}
        if prefix is not None:
            params["prefix"] = prefix
        if replace:
            params["replace"] = True
        return _objects((await self._call(method, params)).get("written"), method)

    async def import_stills(
        self, paths: Sequence[Path], *, album: str | None = None
    ) -> list[dict[str, Any]]:
        """Take pictures (PNG, TIFF, OpenEXR; absolute paths) in as stills; return them.

        Each takes the grades of a still's JSON beside it, where there is one.
        """
        method = "still.import"
        params: dict[str, Any] = {"paths": [str(path) for path in paths]}
        if album is not None:
            params["album"] = album
        return _objects((await self._call(method, params)).get("added"), method)

    async def render(
        self,
        timeline: str,
        path: Path,
        *,
        codec: Literal["h264", "hevc", "prores"] = "h264",
        quality: Literal["high", "medium", "small"] = "high",
        frames: tuple[int, int] | None = None,
        subtitle_tracks: Sequence[str] = (),
        replace: bool = False,
        revision: int | None = None,
        audio: RenderAudio | None = None,
        video: bool = True,
    ) -> Rendering:
        """Have the engine render a timeline into a file, in the background.

        The engine encodes with the hardware of this machine where it can, and writes ProRes into
        a ``.mov`` where it cannot. What it tells of the render is followed from before it is
        asked for, by the :class:`Rendering` that is returned.

        Args:
            timeline: The timeline.
            path: The file to write, an absolute path.
            codec: The codec of the picture.
            quality: How much is kept of the picture, against the size of the file.
            frames: The frames to render, the first and the one after the last; all of them if
                it is not given.
            subtitle_tracks: The subtitle tracks of the timeline whose subtitles are written
                beside the film, each as an SRT, whether it is shown or hidden; none if empty.
            replace: Whether a file that is there already is written over.
            revision: The revision of the project that the render was prepared at, such as the
                names of its SRT files; the engine refuses it where the project is not at that
                revision any more (``stale_revision``). Not held to any if it is not given.
            audio: The sound of the export: its bus, rate, codec, rate of bits and Audio
                Normalization; the engine's defaults where it is not given.
            video: Whether the picture is rendered; without it the sound is rendered alone,
                into a ``.wav`` or an ``.m4a``, and ``codec`` and ``quality`` are not sent.

        Raises:
            EngineError: The engine refused: no such timeline (``missing``), a relative path, a
                file that is there already, nothing to render, a subtitle track that is not one of
                the timeline's (``subtitle_track``), a revision that is not the project's, or a
                bus, rate, codec, rate of bits, standard or target of the sound that cannot be
                (``bus``, ``rate``, ``audio_codec``, ``bitrate``, ``standard``, ``target``).
        """
        params: dict[str, Any] = {
            "timeline": timeline,
            "path": str(path),
            "subtitle_tracks": list(subtitle_tracks),
            "replace": replace,
        }
        if video:
            params["codec"] = codec
            params["quality"] = quality
        else:
            params["video"] = None
        if audio is not None:
            params["audio"] = audio.params()
        if frames is not None:
            params["range"] = {"from": frames[0], "to": frames[1]}
        if revision is not None:
            params["revision"] = revision
        heard = self.changes()
        try:
            wire = await self._ensure()
            number, asked = self._sent(wire, "render.start", params)
            try:
                # The answer is shielded: once the engine was asked, a caller that gives up still
                # learns which render it began, and stops it instead of leaving it running.
                said: object = await asyncio.shield(asked.answer)
            except asyncio.CancelledError:
                await self._stopped(asked.answer)
                raise
            finally:
                self._asked.pop(number, None)
            job = cast("dict[str, Any]", said).get("job") if isinstance(said, dict) else None
            if not isinstance(job, str):
                raise EngineError("the engine answered render.start with no job", code=UNEXPECTED)
        except BaseException:
            await heard.aclose()
            raise
        return Rendering(self, job, heard)

    async def analyze_sound(
        self, method: SoundAnalysis, params: Mapping[str, Any]
    ) -> dict[str, Any]:
        """Have the engine measure sound, and wait for what it found.

        The engine runs ``audio.analyze``, ``audio.normalize`` or ``audio.level_match`` as a job
        off the threads of its sound, as fast as the clips can be read (PROTOCOL.md, "Meters,
        loudness and the analysis of sound"); what it tells of the job is followed from before it
        is asked for. A caller that gives up has the job cancelled.

        Args:
            method: Which analysis: the loudness and peaks of clips, tracks or buses
                (``audio.analyze``), the gains of Normalize Audio Levels (``audio.normalize``), or
                of Level Matcher (``audio.level_match``).
            params: Its params, as the protocol says (``timeline``, ``targets``, ``clips``,
                ``apply``, ...).

        Returns:
            What ``audio.analysis`` told as the job was done: the ``results`` of
            ``audio.analyze``; the ``clips`` and their gains of the others, and ``applied`` (the
            transaction that applied them, where ``apply`` was asked for and a clip had a gain).

        Raises:
            EngineError: The engine refused the params (``missing``, ``standard``, ...) or is
                busy; the job failed or was cancelled (``data`` is what the engine told of it); or
                the connection was lost or closed.
        """
        heard = self.changes()
        try:
            wire = await self._ensure()
            number, asked = self._sent(wire, method, params)
            try:
                # Shielded as render's: a caller that gives up before the job is named still has
                # it cancelled.
                answer: object = await asyncio.shield(asked.answer)
            except asyncio.CancelledError:
                await self._stopped(asked.answer, "audio.cancel")
                raise
            finally:
                self._asked.pop(number, None)
            job = cast("dict[str, Any]", answer).get("job") if isinstance(answer, dict) else None
            if not isinstance(job, str):
                raise EngineError(f"the engine answered {method} with no job", code=UNEXPECTED)
            try:
                async for told in heard:
                    if told.method != "audio.analysis" or told.params.get("job") != job:
                        continue
                    state = told.params.get("state")
                    if state == "running":
                        continue
                    if state == "done":
                        return told.params
                    why = told.params.get("message")
                    raise EngineError(
                        f"the analysis {job} {state}"
                        + (f": {why}" if isinstance(why, str) else ""),
                        code=UNEXPECTED,
                        data=told.params,
                    )
            except asyncio.CancelledError:
                with contextlib.suppress(EngineError, TimeoutError):
                    async with asyncio.timeout(_STOP_SECONDS):
                        await self._call("audio.cancel", {"job": job})
                raise
            raise EngineError(
                f"the connection to the engine was closed before the analysis {job} ended",
                code=UNREACHABLE,
            )
        finally:
            await heard.aclose()

    async def _stopped(
        self, answer: "asyncio.Future[object]", cancel: str = "render.cancel"
    ) -> None:
        """Cancel the job that a request given up on began, if the engine began one.

        ``cancel`` is the method that cancels it: ``render.cancel`` for a render,
        ``audio.cancel`` for an analysis of sound.

        This waits for the answer a while, and fails silently: the caller is cancelled already.
        """
        with contextlib.suppress(EngineError, TimeoutError):
            async with asyncio.timeout(_STOP_SECONDS):
                said = await answer
                job = cast("dict[str, Any]", said).get("job") if isinstance(said, dict) else None
                if isinstance(job, str):
                    await self._call(cancel, {"job": job})

    def changes(self) -> Changes:
        """What the engine tells from now on: every change of the project, and its other news."""
        reader = Changes(self._readers)
        if self._closed:
            reader.end()
        else:
            self._readers.add(reader)
        return reader

    async def aclose(self) -> None:
        """Close the connection. An engine that was started here ends once it has no clients."""
        self._closed = True
        self._drop()
        self._failed(EngineError("the connection to the engine was closed", code=UNREACHABLE))
        for reader in list(self._readers):
            reader.end()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()

    # Reaching the engine

    async def _reach(self) -> None:
        """Connect to the engine of the project, or start one, and say hello to it.

        A connection that is made and not greeted, as when this is cancelled, is closed again.
        """
        if sys.platform == "win32" and not isinstance(  # pragma: windows only
            asyncio.get_running_loop(), asyncio.ProactorEventLoop
        ):
            raise EngineError(
                "the engine is reached by a named pipe, which only the proactor event loop of "
                "asyncio opens, and this one is another",
                code=UNREACHABLE,
            )
        try:
            await self._greeted_somewhere()
        except BaseException:
            self._drop()
            raise

    async def _greeted_somewhere(self) -> None:
        why = "no engine answered"
        for _ in range(_TRIES):
            self._open()
            wire = await self._locked_wire()
            if wire is None and self._yielded:
                wire = await self._handed_over()
            if wire is None:
                kind, address = await self._started()
                wire = None if address is None else await self._wire_at(address)
                if wire is None and kind == "held":
                    await _moment()
                    wire = await self._locked_wire(again=False)
                if wire is None:
                    at = f" at {address}" if address else ""
                    why = {
                        "listening": f"the engine that was started did not answer{at}",
                        "held": f"the engine that has it open did not answer{at}",
                        "contended": "another engine is starting for it",
                    }[kind]
                    await _moment()
                    continue
            try:
                await self._greeted(wire)
            except EngineError as error:
                if error.code != UNREACHABLE or self._closed:
                    raise
                why = str(error)
                # Lost, or refused in the words of no engine and still open.
                self._drop()
                continue
            # It may have been closed while it was greeted.
            self._open()
            return
        raise EngineError(
            f"no engine for the project {self.directory} could be reached: {why}",
            code=UNREACHABLE,
        )

    def _open(self) -> None:
        """Raise if the engine was closed."""
        if self._closed:
            raise EngineError("the connection to the engine was closed", code=UNREACHABLE)

    async def _locked_wire(self, *, again: bool = True) -> _Wire | None:
        """Connect to the address that ``engine.lock`` names, or to what it names a moment later."""
        address = await asyncio.to_thread(_locked, self.directory)
        wire = None if address is None else await self._wire_at(address)
        if wire is None and again:
            await _moment()
            address = await asyncio.to_thread(_locked, self.directory)
            wire = None if address is None else await self._wire_at(address)
        return wire

    async def _handed_over(self) -> _Wire | None:
        """Connect to the engine that the last one yielded the project to, once it says where.

        An engine that yields, so that the desktop's may take the project over, ends before the
        other has taken its lock: ``engine.lock`` is read again for a while, and no engine is
        started here meanwhile, which would take the lock first.
        """
        self._yielded = False
        loop = asyncio.get_running_loop()
        until = loop.time() + _YIELD_SECONDS
        while loop.time() < until:
            await asyncio.sleep(_YIELD_LOOK_SECONDS)
            address = await asyncio.to_thread(_locked, self.directory)
            wire = None if address is None else await self._wire_at(address)
            if wire is not None:
                return wire
        return None

    async def _wire_at(self, address: str) -> _Wire | None:
        wire = await _connected(address, self._heard, self._lost)
        if wire is not None:
            self._wire = wire
        return wire

    async def _started(self) -> tuple[Literal["listening", "held", "contended"], str | None]:
        """Start an engine for the project, and return where it, or the one that holds it, listens.

        Raises:
            EngineError: It could not be started, or ended for another reason than another
                engine that has the project open.
        """
        argv = _argv(self._program, self.directory, self._idle_seconds)
        log = await asyncio.to_thread(_log_of, self.directory)
        try:
            try:
                process = await asyncio.to_thread(_spawned, argv, Path(self._program).parent, log)
            except OSError as error:
                raise EngineError(
                    f"the engine program {self._program} could not be started: {error}",
                    code=UNSTARTED,
                ) from None
            # Its end is waited for, so that it is not left unwaited for, whoever ends first.
            threading.Thread(target=process.wait, daemon=True, name="engine-wait").start()
            try:
                first = await asyncio.wait_for(
                    asyncio.to_thread(_first_line, process), _LISTENING_SECONDS
                )
            except TimeoutError:
                process.kill()
                raise EngineError(
                    f"the engine program {self._program} did not say where it listens within "
                    f"{_LISTENING_SECONDS:g} seconds",
                    code=UNSTARTED,
                ) from None
            if not first:
                code, said = await asyncio.to_thread(_ended_with, process, log)
                return _ended(self._program, code, said)
            try:
                return "listening", _listening(first)
            except EngineError:
                process.kill()
                raise
        finally:
            log.close()

    async def _greeted(self, wire: _Wire) -> None:
        """Say hello, check that the engine has this project open, and ask for the project.

        What the engine tells before it answers with the project is in that answer, and is not
        told again.

        Raises:
            EngineError: The engine refused, has another project open, or the connection was lost.
        """
        self._fetching = True
        hello = await self._asked_on(wire, "hello", {"client": self._client, "protocol": PROTOCOL})
        project = hello.get("project")
        if not isinstance(project, str) or not await asyncio.to_thread(
            _same, Path(project), self.directory
        ):
            raise EngineError(
                f"the engine at that address has another project open: {project}",
                code=UNEXPECTED,
                data=hello,
            )
        # An engine that was started anew counts from 0 again.
        self._revision = _whole(hello, "revision")
        self._session += 1
        self.hello = hello
        await self._asked_on(wire, "project.get", None, fetching=True)

    async def _ensure(self) -> _Wire:
        """The connection, made again if it was lost.

        Raises:
            EngineError: The engine was closed, or cannot be reached again.
        """
        async with self._reaching:
            self._open()
            if self._wire is None:
                await self._reach()
            return cast("_Wire", self._wire)

    def _drop(self) -> None:
        """Close the connection, if there is one."""
        wire, self._wire = self._wire, None
        if wire is not None:
            wire.close()

    # Requests and what is heard

    async def _call(self, method: str, params: Mapping[str, Any] | None = None) -> dict[str, Any]:
        wire = await self._ensure()
        return await self._asked_on(wire, method, params)

    async def _asked_on(
        self,
        wire: _Wire,
        method: str,
        params: Mapping[str, Any] | None,
        *,
        fetching: bool = False,
    ) -> dict[str, Any]:
        number, asked = self._sent(wire, method, params, fetching=fetching)
        try:
            said = await asked.answer
        finally:
            self._asked.pop(number, None)
        if not isinstance(said, dict):
            raise EngineError(f"the engine answered {method} with no object", code=UNEXPECTED)
        return cast("dict[str, Any]", said)

    def _sent(
        self, wire: _Wire, method: str, params: Mapping[str, Any] | None, *, fetching: bool = False
    ) -> tuple[int, _Asked]:
        """Send a request, once it can be written, and wait for its answer from then on.

        Raises:
            EngineError: It cannot be written, or is too long.
        """
        number = next(self._ids)
        message: dict[str, Any] = {"jsonrpc": "2.0", "id": number, "method": method}
        if params is not None:
            message["params"] = params
        line = _Wire.encoded(message)
        asked = _Asked(method, asyncio.get_running_loop().create_future(), fetching)
        self._asked[number] = asked
        wire.write(line)
        return number, asked

    def _heard(self, wire: _Wire, line: bytes) -> None:
        try:
            message: object = json.loads(line)
        except ValueError:
            return
        if not isinstance(message, dict):
            return
        said = cast("dict[str, Any]", message)
        if "id" in said:
            number = said["id"]
            if number is None and "error" in said:
                # An error that the engine could not tie to its request, as a line that it could
                # not read. It answers in order, so it is the oldest request's.
                number = next(iter(self._asked), None)
            asked = self._asked.pop(number, None) if type(number) is int else None
            # A request whose caller gave up on it is no longer waited for.
            if asked is not None and not asked.answer.done():
                self._answered(asked, said)
            return
        method, params = said.get("method"), said.get("params")
        if isinstance(method, str):
            self._told(method, cast("dict[str, Any]", params) if isinstance(params, dict) else {})

    def _answered(self, asked: _Asked, said: dict[str, Any]) -> None:
        if asked.fetching:
            self._fetching = False
        error = said.get("error")
        if isinstance(error, dict):
            failed = cast("dict[str, Any]", error)
            code, data = failed.get("code"), failed.get("data")
            refused = EngineError(
                str(failed.get("message", "the engine refused")),
                code=code if isinstance(code, int) else UNEXPECTED,
                data=cast("dict[str, Any]", data) if isinstance(data, dict) else None,
            )
            asked.answer.set_exception(refused)
            return
        result = said.get("result")
        if isinstance(result, dict):
            told = cast("dict[str, Any]", result)
            revision, next_id = told.get("revision"), told.get("next_id")
            if asked.method == "project.get" and isinstance(revision, int):
                # Changes told before it are in it; an answer is never behind them.
                self._revision = max(self._revision, revision)
            if isinstance(next_id, int) and (revision == self._revision or asked.fetching):
                self._next_id = next_id
            if asked.fetching:
                # Told here, before what is heard after it.
                self._tell(Changed("project.fetched", told))
        asked.answer.set_result(result)

    def _told(self, method: str, params: dict[str, Any]) -> None:
        if method == "engine.yielding":
            # It ends next, for another engine to take the project over.
            self._yielded = True
        elif method == "project.changed":
            revision, next_id = params.get("revision"), params.get("next_id")
            if not isinstance(revision, int) or self._fetching or revision <= self._revision:
                # What it tells is in the project that is being asked for, or was told before.
                return
            if revision > self._revision + 1:
                self._fetch()
                return
            self._revision = revision
            if isinstance(next_id, int):
                self._next_id = next_id
        self._tell(Changed(method, params))

    def _fetch(self) -> None:
        """Ask for the whole project anew, as a change was missed."""
        self._fetching = True
        _, asked = self._sent(cast("_Wire", self._wire), "project.get", None, fetching=True)
        # Nobody waits for it: what it answers is told to the readers.
        asked.answer.add_done_callback(lambda done: done.exception())

    def _lost(self, wire: _Wire, error: EngineError) -> None:
        if wire is not self._wire:
            # One that was closed here: what waited on it was failed then.
            return
        self._wire = None
        self._fetching = False
        self._failed(error)
        self._tell(error)

    def _failed(self, error: EngineError) -> None:
        asked, self._asked = self._asked, {}
        for one in asked.values():
            if not one.answer.done():
                one.answer.set_exception(EngineError(str(error), code=error.code))

    def _tell(self, told: Changed | EngineError) -> None:
        for reader in list(self._readers):
            reader.tell(told)


def _whole(said: dict[str, Any], key: str) -> int:
    """Read a whole number of an answer.

    Raises:
        EngineError: It is not there, or no whole number.
    """
    number = said.get(key)
    if type(number) is not int:
        raise EngineError(f"the engine answered with no {key}", code=UNEXPECTED)
    return number


def _unexpected(method: str) -> EngineError:
    return EngineError(
        f"the engine answered {method} with what the protocol does not say", code=UNEXPECTED
    )


def _real(value: object, method: str) -> float:
    """Read a number of an answer.

    ``null`` is read as NaN: the engine writes it for a figure that is not a number.
    """
    if value is None:
        return float("nan")
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise _unexpected(method)
    return float(value)


def _object(value: object, method: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise _unexpected(method)
    return cast("dict[str, Any]", value)


def _objects(value: object, method: str) -> list[dict[str, Any]]:
    """Read a list of objects of an answer."""
    if not isinstance(value, list) or not all(
        isinstance(one, dict) for one in cast("list[object]", value)
    ):
        raise _unexpected(method)
    return cast("list[dict[str, Any]]", value)


def _strings(value: object, method: str) -> list[str]:
    """Read a list of strings of an answer."""
    if not isinstance(value, list) or not all(
        isinstance(one, str) for one in cast("list[object]", value)
    ):
        raise _unexpected(method)
    return cast("list[str]", value)


def _ids(said: object, field: str, method: str) -> dict[str, str]:
    """Read an object of ids by id, ``field`` of an answer (``moved``, ``copied``, ``restored``)."""
    ids = _object(_object(said, method).get(field), method)
    if not all(isinstance(id, str) for id in ids.values()):
        raise _unexpected(method)
    return cast("dict[str, str]", ids)


def _pair(value: object, method: str) -> tuple[float, float]:
    if not isinstance(value, list) or len(cast("list[object]", value)) != 2:
        raise _unexpected(method)
    first, second = cast("list[object]", value)
    return _real(first, method), _real(second, method)


def _statistics(said: dict[str, Any]) -> Statistics:
    """Read the answer of ``scopes.statistics``.

    Raises:
        EngineError: It is not what the protocol says.
    """
    method = "scopes.statistics"
    view = said.get("view")
    timeline = said.get("timeline")
    node = said.get("node")
    if view not in ("timeline", "source") and not (view is None and isinstance(timeline, str)):
        raise _unexpected(method)
    if timeline is not None and not isinstance(timeline, str):
        raise _unexpected(method)

    def channel(name: str) -> Channel:
        figures = _object(said.get(name), method)
        return Channel(*(_real(figures.get(key), method) for key in ("min", "max", "mean")))

    chroma = _object(said.get("chroma"), method)
    percentiles = _object(said.get("percentiles"), method)
    histogram = _object(said.get("histogram"), method)
    counts: dict[str, list[int]] = {}
    shares: dict[str, dict[str, float]] = {}
    for name in _CHANNELS:
        each = histogram.get(name)
        if not isinstance(each, list) or not all(
            type(count) is int for count in cast("list[object]", each)
        ):
            raise _unexpected(method)
        counts[name] = cast("list[int]", each)
        shares[name] = {
            share: _real(value, method)
            for share, value in _object(percentiles.get(name), method).items()
        }
    return Statistics(
        view=view,
        frame=_whole(said, "frame"),
        width=_whole(said, "width"),
        height=_whole(said, "height"),
        samples=_whole(said, "samples"),
        red=channel("red"),
        green=channel("green"),
        blue=channel("blue"),
        luma=channel("luma"),
        chroma=Chroma(
            centroid=_pair(chroma.get("centroid"), method),
            spread=_pair(chroma.get("spread"), method),
            radius=_real(chroma.get("radius"), method),
            saturation=_real(chroma.get("saturation"), method),
            hue=_real(chroma.get("hue"), method),
        ),
        percentiles=shares,
        histogram=counts,
        bins=_whole(histogram, "bins"),
        timeline=timeline,
        node=None if node is None else _object(node, method),
    )


def _library(said: dict[str, Any]) -> LutLibrary:
    """Read the answer of ``luts.list``.

    Raises:
        EngineError: It is not what the protocol says.
    """
    method = "luts.list"
    roots, luts, truncated = said.get("roots"), said.get("luts"), said.get("truncated")
    if not isinstance(roots, list) or not isinstance(luts, list) or not isinstance(truncated, bool):
        raise _unexpected(method)
    read_roots: list[LutRoot] = []
    for root in cast("list[object]", roots):
        each = _object(root, method)
        place, path, exists = each.get("place"), each.get("path"), each.get("exists")
        if place not in ("project", "library") or not isinstance(path, str):
            raise _unexpected(method)
        if not isinstance(exists, bool):
            raise _unexpected(method)
        read_roots.append(LutRoot(place=place, path=path, exists=exists))
    read_luts: list[Lut] = []
    for lut in cast("list[object]", luts):
        each = _object(lut, method)
        name, shadowed, modified = each.get("name"), each.get("shadowed"), each.get("modified_us")
        root = _whole(each, "root")
        if (
            not isinstance(name, str)
            or not isinstance(shadowed, bool)
            or not (modified is None or type(modified) is int)
            or not 0 <= root < len(read_roots)
        ):
            raise _unexpected(method)
        # An engine from before the kind and the status of each file leaves them out.
        kind, status = each.get("kind"), each.get("status", "unchecked")
        message = each.get("message") if status == "error" else None
        if (
            kind not in (None, "1d", "3d", "shaper+3d")
            or status not in ("ok", "error", "unchecked")
            or (status == "error" and not isinstance(message, str))
        ):
            raise _unexpected(method)
        read_luts.append(
            Lut(
                name=name,
                root=root,
                size=_whole(each, "size"),
                modified_us=modified,
                shadowed=shadowed,
                kind=kind,
                status=status,
                message=message,
            )
        )
    return LutLibrary(roots=read_roots, luts=read_luts, truncated=truncated)


def _same(written: Path, directory: Path) -> bool:
    """Whether the directory that the engine names is the one of the project. This blocks."""
    try:
        return written.samefile(directory)
    except OSError:
        return False


def _applied(said: dict[str, Any], names: object) -> Applied:
    """Read the answer of a transaction, an undo or a redo.

    Raises:
        EngineError: It is not what the protocol says.
    """
    done, clamped = said.get("done"), said.get("clamped", [])
    if (
        not isinstance(done, list)
        or not isinstance(names, dict)
        or not isinstance(clamped, list)
        or not all(type(one) is int for one in cast("list[object]", clamped))
    ):
        raise EngineError("the engine answered with no change of the project", code=UNEXPECTED)
    return Applied(
        revision=_whole(said, "revision"),
        done=cast("list[dict[str, Any]]", done),
        next_id=_whole(said, "next_id"),
        names=cast("dict[str, str]", names),
        clamped=cast("list[int]", clamped),
    )


def _stepped(said: dict[str, Any]) -> Applied | None:
    """Read the answer of an undo or a redo: ``None`` where there was nothing to do."""
    if "done" in said and said["done"] is None:
        _whole(said, "revision")
        return None
    return _applied(said, {})


def _listening(first: bytes) -> str:
    """Read the address from the first line of an engine.

    Raises:
        EngineError: It is not such a line, of this protocol, at an address where an engine
            listens.
    """
    try:
        said: object = json.loads(first)
    except ValueError:
        said = None
    told = cast("dict[str, Any]", said) if isinstance(said, dict) else {}
    address = _address(told.get("listening"))
    if address is None or told.get("protocol") != PROTOCOL:
        raise EngineError(
            f"the engine did not say where it listens in protocol {PROTOCOL}, at an address "
            f"where an engine listens: {first[:200]!r}",
            code=UNSTARTED,
        )
    return address


def _ended(
    program: str, code: int, logged: list[bytes]
) -> tuple[Literal["held", "contended"], str | None]:
    """Tell what an engine that ended before it listened said.

    Raises:
        EngineError: It could not start, for another reason than another engine.
    """
    for line in reversed(logged):
        try:
            said: object = json.loads(line)
        except ValueError:
            continue
        if not isinstance(said, dict):
            continue
        told = cast("dict[str, Any]", said)
        held = told.get("held")
        if code == 3 and isinstance(held, dict):
            return "held", _address(cast("dict[str, Any]", held).get("address"))
        if code == 3 and told.get("contended") is True:
            return "contended", None
        if code == 2 and isinstance(told.get("error"), str):
            raise EngineError(str(told["error"]), code=UNSTARTED, data={"exit_code": code})
    shown = f"0x{code:08X}" if code > 0xFFFF else str(code)
    why = f"the engine program {program} ended with exit code {shown} before it listened"
    if sys.platform == "win32" and code == _LIBRARIES_MISSING:  # pragma: windows only
        why += (
            ": the libraries of FFmpeg that it reads media with (avcodec, avformat, avutil, "
            "swscale) are neither beside it, nor on the search path, nor in the bin of "
            "FFMPEG_DIR"
        )
    raise EngineError(why, code=UNSTARTED, data={"exit_code": code})
