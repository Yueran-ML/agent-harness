"""The engine that has a project of video open, as the agent reaches it.

**The engine.** ``columbina-engine`` is a program of its own, and the only one that writes a
project: the desktop and the agent are its clients, send it changes, and are told of every
change in one order. Its protocol is ``engine/PROTOCOL.md`` of the repository, JSON-RPC 2.0 over
a named pipe of Windows or a socket of Unix, one message a line. :class:`Engine` is one
connection to it.

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
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import IO, Any, Literal, Self, cast

from columbina import programs

__all__ = [
    "NOT_FOUND",
    "PROTOCOL",
    "UNEXPECTED",
    "UNREACHABLE",
    "UNSTARTED",
    "Applied",
    "Changed",
    "Changes",
    "Engine",
    "EngineError",
    "not_found",
    "program_of",
]

PROTOCOL = 1
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
    """

    revision: int
    done: list[dict[str, Any]]
    next_id: int
    names: dict[str, str] = field(default_factory=dict[str, str])


@dataclass(frozen=True)
class Changed:
    """What the engine told every client, or what this side found out about the project.

    Attributes:
        method: ``project.changed``, ``engine.error``, ``transport.state`` or ``media.proxy``, as
            the engine sends them, or ``engine.yielding``, which an engine tells before it ends
            for the desktop's to take the project over. Or ``project.fetched``: the whole project
            was asked for anew, after a change was missed or the connection was made again, and
            its params are the answer of ``project.get``; a copy of the project starts again from
            it, and the changes after it follow. An engine that was started anew counts its
            revisions from 0 again. Or ``changes.missed``: the reader left so much unread that
            the oldest things told were dropped, ``missed`` of them; a copy starts again from
            :meth:`Engine.project`.
        params: What it said.
    """

    method: str
    params: dict[str, Any]

    @property
    def revision(self) -> int | None:
        """The revision of the project that it tells of, if it tells of one."""
        said = self.params.get("revision")
        return said if isinstance(said, int) and not isinstance(said, bool) else None


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
    """Connect to an address, or return ``None`` where nothing answers there."""
    loop = asyncio.get_running_loop()
    wire = _Wire(heard, lost)
    try:
        if sys.platform == "win32":  # pragma: windows only
            proactor = cast("asyncio.ProactorEventLoop", loop)
            # Any protocol is taken, whatever the stubs of typeshed say.
            connecting = proactor.create_pipe_connection(
                lambda: wire,  # pyright: ignore[reportArgumentType]
                address,
            )
        else:  # pragma: posix only
            connecting = loop.create_unix_connection(lambda: wire, address)
        await asyncio.wait_for(connecting, _CONNECT_SECONDS)
    except (OSError, TimeoutError):
        return None
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
    directory that holds them is put at the end of where the engine looks for libraries. The
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
    environment[variable] = f"{before}{os.pathsep}{libraries}" if before else str(libraries)
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

    async def apply(self, ops: Sequence[Mapping[str, Any]]) -> Applied:
        """Apply operations to the project, all or none, as one step of its history.

        A thing that an operation adds may be given an id of the form ``$name``, which later
        operations of the same call may name; the ids that the engine gave are in ``names``.

        Raises:
            EngineError: The transaction was refused (``-32001``, with the ``index`` of the
                operation and the ``reason`` in ``data``), or an operation or a name could not
                be read (``-32602``).
        """
        said = await self._call("transaction.apply", {"ops": [dict(op) for op in ops]})
        return _applied(said, said.get("names"))

    async def undo(self) -> Applied | None:
        """Take back the latest step of the project, whoever made it; ``None`` if there is none."""
        return _stepped(await self._call("history.undo"))

    async def redo(self) -> Applied | None:
        """Do again the step undone last; ``None`` if there is none."""
        return _stepped(await self._call("history.redo"))

    async def imported(self, paths: Sequence[Path]) -> tuple[int, list[str]]:
        """Take files in as media of the project, as one step, and return the revision and the ids.

        Every path is to be absolute: the engine refuses a relative one (``-32602``) and takes
        nothing in. Neither is anything taken in if one file cannot be read (``-32002``, with its
        ``path`` and ``reason``).
        """
        said = await self._call("media.import", {"paths": [str(path) for path in paths]})
        added = said.get("added")
        if not isinstance(added, list) or not all(
            isinstance(one, str) for one in cast("list[object]", added)
        ):
            raise EngineError("the engine answered media.import with no ids", code=UNEXPECTED)
        return _whole(said, "revision"), cast("list[str]", added)

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
    done = said.get("done")
    if not isinstance(done, list) or not isinstance(names, dict):
        raise EngineError("the engine answered with no change of the project", code=UNEXPECTED)
    return Applied(
        revision=_whole(said, "revision"),
        done=cast("list[dict[str, Any]]", done),
        next_id=_whole(said, "next_id"),
        names=cast("dict[str, str]", names),
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
