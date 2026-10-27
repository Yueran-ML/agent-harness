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

**The program.** The engine is looked for the way :mod:`columbina.programs` looks for a
program, :func:`program_of`: the path that the config gives, or ``COLUMBINA_ENGINE``, or the
search path, and never in a working directory. It is run in its own directory, where it finds
the libraries of FFmpeg that it reads media with, and not in the directory of the project. What
it logs is copied to ``cache/engine-<pid>.log`` of the project.
"""

import asyncio
import contextlib
import itertools
import json
import os
import random
import subprocess
import sys
import threading
import weakref
from collections import deque
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
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
    "program_of",
]

PROTOCOL = 1
"""The version of the protocol that is spoken."""

UNREACHABLE = -1
"""The code of an :class:`EngineError` when no engine could be reached, or the connection to it
was lost or closed."""

UNSTARTED = -2
"""The code of an :class:`EngineError` when the engine could not be started."""

UNEXPECTED = -3
"""The code of an :class:`EngineError` when the engine answered what the protocol does not say,
or has another project open."""

_NAME = "columbina-engine"

NOT_FOUND = (
    "no columbina-engine program was found: build it with `cargo build --release -p "
    "columbina-server` in engine/ of the repository, and name it with COLUMBINA_ENGINE or put "
    "it on the search path. A program that lies in the working directory is not taken"
)
"""What is said where :func:`program_of` finds no engine."""

_LINE_BYTES = 16 * 1024 * 1024
"""The longest line that is read from the engine, without its newline."""

_CONNECT_SECONDS = 5.0
"""How long connecting may take: a pipe of Windows that is busy is tried again for ever."""

_LISTENING_SECONDS = 20.0
"""How long an engine that was started may take to say where it listens."""

_TRIES = 8
"""How many times an engine is looked for and started before it is given up."""

_KEPT_LINES = 64
"""How many lines of its log an engine that ended before it listened is judged by."""

_LIBRARIES_MISSING = 0xC0000135
"""How a program of Windows ends when a library that it needs is not found."""


class EngineError(Exception):
    """The engine refused, or could not be reached.

    Attributes:
        code: The code of the error of JSON-RPC that the engine answered, such as ``-32001`` for
            a transaction that was refused, or :data:`UNREACHABLE`, :data:`UNSTARTED` or
            :data:`UNEXPECTED` for what went wrong on this side.
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
            the engine sends them; or ``project.fetched`` when the whole project was asked for
            anew, after a change was missed or the connection was made again. Its params are
            the answer of ``project.get``, and the changes after it follow.
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
        The whole path of the program, or ``None`` if there is none that may be run.
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


def _argv(program: str, directory: Path, idle_seconds: float | None) -> list[str]:
    """What the engine is started with: always headless, as an agent has no window to give it."""
    argv = [program, "--project", str(directory), "--headless"]
    if idle_seconds is not None:
        argv += ["--idle-seconds", f"{idle_seconds:g}"]
    return argv


def _locked(directory: Path) -> str | None:
    """Return the address that ``engine.lock`` of a project names, if it names one."""
    try:
        said: object = json.loads((directory / "engine.lock").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    address = cast("dict[str, object]", said).get("address") if isinstance(said, dict) else None
    return address if isinstance(address, str) and address else None


async def _moment() -> None:
    """Wait the moment after which an address that did not answer is looked up again."""
    await asyncio.sleep(random.uniform(0.05, 0.1))


class _Wire(asyncio.Protocol):
    """One connection to the engine: what it sends cut into lines.

    Only what came since the last look is looked through for the end of a line.
    """

    def __init__(
        self, heard: Callable[["_Wire", bytes], None], lost: Callable[["_Wire", str], None]
    ):
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
            if end - start > _LINE_BYTES:
                self._too_long()
                return
            line = bytes(self._buffer[start:end])
            start = end + 1
            self._heard(self, line)
        del self._buffer[:start]
        self._looked = len(self._buffer)
        if self._looked > _LINE_BYTES:
            self._too_long()

    def _too_long(self) -> None:
        self._end("the engine sent a line longer than 16 MiB")
        cast("asyncio.Transport", self.transport).close()

    def connection_lost(self, exc: Exception | None) -> None:
        self._end("the engine closed the connection" + (f": {exc}" if exc else ""))

    def _end(self, why: str) -> None:
        if not self._ended:
            self._ended = True
            self._buffer.clear()
            self._lost(self, why)

    def send(self, message: Mapping[str, Any]) -> None:
        """Send a message as a line."""
        line = json.dumps(message, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
        cast("asyncio.Transport", self.transport).write(line.encode("utf-8") + b"\n")

    def close(self) -> None:
        """Close the connection, and hear nothing more of it."""
        self._ended = True
        cast("asyncio.Transport", self.transport).close()


async def _connected(
    address: str, heard: Callable[[_Wire, bytes], None], lost: Callable[[_Wire, str], None]
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


class _Log:
    """What an engine that was started writes on stderr, read as it comes so that it never waits.

    Once the engine says where it listens, its log is copied to ``cache/engine-<pid>.log`` of the
    project; before that, its last lines are kept, to tell why it ended if it did.
    """

    def __init__(self, process: "subprocess.Popen[bytes]", directory: Path) -> None:
        self._process = process
        self._directory = directory
        self._kept: deque[bytes] = deque(maxlen=_KEPT_LINES)
        self._file: IO[bytes] | None = None
        self._lock = threading.Lock()
        self._thread = threading.Thread(target=self._drained, daemon=True, name="engine-log")
        self._thread.start()

    def _drained(self) -> None:
        stderr = cast("IO[bytes]", self._process.stderr)
        try:
            for line in stderr:
                with self._lock:
                    if self._file is not None:
                        self._file.write(line)
                        self._file.flush()
                    else:
                        self._kept.append(line)
        finally:
            stderr.close()
            with self._lock:
                if self._file is not None:
                    self._file.close()
            self._process.wait()

    def keep(self, pid: int) -> None:
        """Copy the log to the cache of the project from now on, with what was written before.

        Args:
            pid: The engine's own, as it said where it listens.
        """
        with self._lock:
            try:
                cache = self._directory / "cache"
                cache.mkdir(parents=True, exist_ok=True)
                self._file = (cache / f"engine-{pid}.log").open("ab")
                self._file.writelines(self._kept)
                self._file.flush()
            except OSError:
                self._file = None
            self._kept.clear()

    def ended(self) -> tuple[int, list[dict[str, Any]]]:
        """Wait for the engine to end, and return its exit code and the lines of JSON it logged.

        This blocks.
        """
        self._thread.join()
        said: list[dict[str, Any]] = []
        for line in self._kept:
            with contextlib.suppress(ValueError):
                one: object = json.loads(line)
                if isinstance(one, dict):
                    said.append(cast("dict[str, Any]", one))
        return cast("int", self._process.returncode), said


def _first_line(process: "subprocess.Popen[bytes]") -> bytes:
    """Read what an engine says first on stdout, and close it: nothing else is ever written there.

    This blocks.
    """
    stdout = cast("IO[bytes]", process.stdout)
    try:
        return stdout.readline()
    finally:
        stdout.close()


def _spawned(argv: list[str], cwd: Path) -> "subprocess.Popen[bytes]":
    """Start an engine apart from the console of this program.

    Its Ctrl+C or closing does not end the engine then: other clients may still use it.

    This blocks.
    """
    if sys.platform == "win32":  # pragma: windows only
        return subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=cwd,
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW,
        )
    else:  # pragma: posix only
        return subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=cwd,
            start_new_session=True,
        )


class Changes:
    """What the engine tells, from the moment :meth:`Engine.changes` was called, in its order.

    The connection lost is raised as an :class:`EngineError` once; the next call of the engine
    connects again, and what is told goes on, beginning with ``project.fetched``. The changes end
    when the engine is closed.
    """

    def __init__(self) -> None:
        self._told: asyncio.Queue[Changed | EngineError | None] = asyncio.Queue()

    def tell(self, told: Changed | EngineError | None) -> None:
        """Pass on something that was told."""
        self._told.put_nowait(told)

    def __aiter__(self) -> Self:
        return self

    async def __anext__(self) -> Changed:
        told = await self._told.get()
        if told is None:
            self._told.put_nowait(None)
            raise StopAsyncIteration
        if isinstance(told, EngineError):
            raise told
        return told


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
    engine or to one that is started anew.

    Attributes:
        directory: The directory of the project, as a whole path.
        hello: What the engine answered to ``hello``: its version (``engine``), the ``protocol``,
            the ``project`` as it writes the directory, and its ``pid``.
    """

    def __init__(
        self, directory: Path, program: str, client: str, idle_seconds: float | None
    ) -> None:
        self.directory = directory
        self._program = program
        self._client = client
        self._idle_seconds = idle_seconds
        self.hello: dict[str, Any] = {}
        self._wire: _Wire | None = None
        self._asked: dict[int, _Asked] = {}
        self._ids = itertools.count(1)
        self._listeners: weakref.WeakSet[Changes] = weakref.WeakSet()
        self._revision = -1
        self._next_id = 0
        self._fetching = False
        self._closed = False
        self._reaching = asyncio.Lock()

    @classmethod
    async def attach(
        cls,
        project: Path,
        *,
        program: str,
        client: str = "agent",
        idle_seconds: float | None = None,
    ) -> "Engine":
        """Connect to the engine that has a project open, starting one if none has.

        Args:
            project: The directory of the project. It is made if it is not there.
            program: The engine program, as :func:`program_of` found it.
            client: What this client is: ``agent`` or ``cli``.
            idle_seconds: How long an engine that is started here waits with no client before
                it ends. The engine's own default, 30 seconds, unless it is given.

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
        listener = Changes()
        self._listeners.add(listener)
        return listener

    async def aclose(self) -> None:
        """Close the connection. An engine that was started here ends once it has no clients."""
        self._closed = True
        wire, self._wire = self._wire, None
        if wire is not None:
            wire.close()
        self._failed("the connection to the engine was closed")
        self._tell(None)

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()

    # Reaching the engine

    async def _reach(self) -> None:
        """Connect to the engine of the project, or start one, and say hello to it."""
        if sys.platform == "win32" and not isinstance(  # pragma: windows only
            asyncio.get_running_loop(), asyncio.ProactorEventLoop
        ):
            raise EngineError(
                "the engine is reached by a named pipe, which only the proactor event loop of "
                "asyncio opens, and this one is another",
                code=UNREACHABLE,
            )
        why = "no engine answered"
        for _ in range(_TRIES):
            wire = await self._locked_wire()
            if wire is None:
                kind, address = await self._started()
                wire = None if address is None else await self._wire_at(address)
                if wire is None and kind == "held":
                    await _moment()
                    wire = await self._locked_wire(again=False)
                if wire is None:
                    why = {
                        "listening": f"the engine that was started, at {address}, did not answer",
                        "held": f"the engine that has it open, at {address}, did not answer",
                        "contended": "another engine is starting for it",
                    }[kind]
                    await _moment()
                    continue
            try:
                await self._greeted(wire)
            except EngineError as error:
                if error.code != UNREACHABLE:
                    wire.close()
                    self._wire = None
                    raise
                why = str(error)
                continue
            return
        raise EngineError(
            f"no engine for the project {self.directory} could be reached: {why}",
            code=UNREACHABLE,
        )

    async def _locked_wire(self, *, again: bool = True) -> _Wire | None:
        """Connect to the address that ``engine.lock`` names, or to what it names a moment later."""
        address = _locked(self.directory)
        wire = None if address is None else await self._wire_at(address)
        if wire is None and again:
            await _moment()
            address = _locked(self.directory)
            wire = None if address is None else await self._wire_at(address)
        return wire

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
        try:
            process = await asyncio.to_thread(_spawned, argv, Path(self._program).parent)
        except OSError as error:
            raise EngineError(
                f"the engine program {self._program} could not be started: {error}",
                code=UNSTARTED,
            ) from None
        log = _Log(process, self.directory)
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
        if first:
            address, pid = _listening(first)
            log.keep(pid if pid is not None else process.pid)
            return "listening", address
        code, said = await asyncio.to_thread(log.ended)
        return _ended(self._program, code, said)

    async def _greeted(self, wire: _Wire) -> None:
        """Say hello, check that the engine has this project open, and ask for the project.

        Raises:
            EngineError: The engine refused, has another project open, or the connection was lost.
        """
        hello = await self._asked_on(wire, "hello", {"client": self._client, "protocol": PROTOCOL})
        project = hello.get("project")
        if not isinstance(project, str) or not _same(Path(project), self.directory):
            raise EngineError(
                f"the engine at that address has another project open: {project}",
                code=UNEXPECTED,
                data=hello,
            )
        self._revision = _whole(hello, "revision")
        self.hello = hello
        said = await self._asked_on(wire, "project.get", None)
        if self._listeners:
            self._tell(Changed("project.fetched", said))

    async def _ensure(self) -> _Wire:
        """The connection, made again if it was lost.

        Raises:
            EngineError: The engine was closed, or cannot be reached again.
        """
        async with self._reaching:
            if self._closed:
                raise EngineError("the connection to the engine was closed", code=UNREACHABLE)
            if self._wire is None:
                await self._reach()
            return cast("_Wire", self._wire)

    # Requests and what is heard

    async def _call(self, method: str, params: Mapping[str, Any] | None = None) -> dict[str, Any]:
        wire = await self._ensure()
        return await self._asked_on(wire, method, params)

    async def _asked_on(
        self, wire: _Wire, method: str, params: Mapping[str, Any] | None
    ) -> dict[str, Any]:
        number, asked = self._sent(wire, method, params)
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
        number = next(self._ids)
        asked = _Asked(method, asyncio.get_running_loop().create_future(), fetching)
        self._asked[number] = asked
        message: dict[str, Any] = {"jsonrpc": "2.0", "id": number, "method": method}
        if params is not None:
            message["params"] = params
        wire.send(message)
        return number, asked

    def _heard(self, wire: _Wire, line: bytes) -> None:
        try:
            message: object = json.loads(line)
        except ValueError:
            return
        if not isinstance(message, dict):
            return
        said = cast("dict[str, Any]", message)
        number = said.get("id")
        if number is not None:
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
                self._tell(Changed("project.fetched", told))
        asked.answer.set_result(result)

    def _told(self, method: str, params: dict[str, Any]) -> None:
        if method == "project.changed":
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
        # Nobody waits for it: what it answers is told to the listeners.
        asked.answer.add_done_callback(lambda done: done.exception())

    def _lost(self, wire: _Wire, why: str) -> None:
        self._wire = None
        self._fetching = False
        self._failed(why)
        self._tell(EngineError(why, code=UNREACHABLE))

    def _failed(self, why: str) -> None:
        asked, self._asked = self._asked, {}
        for one in asked.values():
            if not one.answer.done():
                one.answer.set_exception(EngineError(why, code=UNREACHABLE))

    def _tell(self, told: Changed | EngineError | None) -> None:
        for listener in self._listeners:
            listener.tell(told)


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
    """Whether the directory that the engine names is the one of the project."""
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


def _listening(first: bytes) -> tuple[str, int | None]:
    """Read the address, and the pid, from the first line of an engine.

    Raises:
        EngineError: It is not such a line, or of another protocol.
    """
    try:
        said: object = json.loads(first)
    except ValueError:
        said = None
    told = cast("dict[str, Any]", said) if isinstance(said, dict) else {}
    address, protocol = told.get("listening"), told.get("protocol")
    if not isinstance(address, str) or protocol != PROTOCOL:
        raise EngineError(
            f"the engine did not say where it listens in protocol {PROTOCOL}: {first[:200]!r}",
            code=UNSTARTED,
        )
    pid = told.get("pid")
    return address, pid if type(pid) is int else None


def _ended(
    program: str, code: int, said: list[dict[str, Any]]
) -> tuple[Literal["held", "contended"], str | None]:
    """Tell what an engine that ended before it listened said.

    Raises:
        EngineError: It could not start, for another reason than another engine.
    """
    for line in reversed(said):
        held = line.get("held")
        if code == 3 and isinstance(held, dict):
            address = cast("dict[str, Any]", held).get("address")
            return "held", address if isinstance(address, str) else None
        if code == 3 and line.get("contended") is True:
            return "contended", None
        if code == 2 and isinstance(line.get("error"), str):
            raise EngineError(str(line["error"]), code=UNSTARTED, data={"exit_code": code})
    why = f"the engine program {program} ended with exit code {code} before it listened"
    if sys.platform == "win32" and code == _LIBRARIES_MISSING:  # pragma: windows only
        why += (
            ": the libraries of FFmpeg that it reads media with (avcodec, avformat, avutil, "
            "swscale) are neither beside it nor on the search path"
        )
    raise EngineError(why, code=UNSTARTED, data={"exit_code": code})
