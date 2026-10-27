"""An engine of video for the tests: in the process of a test, or as a program of its own.

It speaks the protocol of ``engine/PROTOCOL.md`` over a named pipe of Windows or a socket of
Unix, one message a line: ``hello``, ``project.get``, ``transaction.apply`` (with names of the
form ``$name``), ``history.undo``, ``history.redo`` and ``media.import``. Every change is told to
every connection that said hello before it is answered, as the engine does. A test makes it
misbehave through the attributes of :class:`Fake`.

In a test, :func:`served` serves a project in the loop of the test. As a program, run it the way
the engine is run, ``python fake_engine.py --project DIR --headless [--idle-seconds N]``, and with
``--mode`` it ends as an engine that cannot start ends instead:

- ``listen``: it takes the project, writes ``engine.lock``, says where it listens, logs a line
  on stderr for each connection, and ends once no client has been connected for the idle time.
- ``held ADDRESS``: another engine has the project open at ``ADDRESS`` (exit code 3).
- ``held-moved ADDRESS``: the same, naming an address where nothing listens, and having
  written ``ADDRESS`` into ``engine.lock``, as an engine that took over meanwhile would.
- ``contended``: another engine is starting (exit code 3). ``contended-once`` says so the first
  time only, and listens the next.
- ``error``: the project cannot be read (exit code 2).
- ``exit``: it ends with exit code 5, logging lines that say nothing of why.
- ``libraries``: it ends as a program of Windows ends whose libraries are not found.
- ``babble``: its first line is not where it listens.
- ``silent``: it says nothing, and waits.

It imports nothing of Columbina.
"""

import argparse
import asyncio
import contextlib
import copy
import json
import os
import sys
import tempfile
import time
import uuid
from collections.abc import AsyncGenerator, Callable
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast

__all__ = ["LIMIT", "Fake", "served"]

LIMIT = 16 * 1024 * 1024
"""The longest line that a client reads."""


def argv_of(
    *mode: str, idle: float | None = None
) -> Callable[[str, Path, float | None], list[str]]:
    """What the engine is started with, for this file to be started in its place.

    It is a script, run by the Python of the test, in a mode (``listen`` if none is given), and
    with ``idle`` for its idle time, if given, in place of the one that it is started with.
    """

    def argv(program: str, directory: Path, idle_seconds: float | None) -> list[str]:
        seconds = idle if idle is not None else idle_seconds
        idling = ["--idle-seconds", str(seconds)] if seconds is not None else []
        return [sys.executable, __file__, "--project", str(directory), "--headless", *idling,
                "--mode", *(mode or ("listen",))]  # fmt: skip

    return argv


def address_of() -> str:
    """A new address that nothing listens at yet."""
    if sys.platform == "win32":
        return rf"\\.\pipe\columbina-fake-{uuid.uuid4().hex}"
    # A socket's path may be only about a hundred bytes long.
    return str(Path(tempfile.gettempdir()) / f"cf-{uuid.uuid4().hex[:12]}.sock")


class RefusedError(Exception):
    """An answer of JSON-RPC that is an error."""

    def __init__(self, code: int, message: str, data: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.error: dict[str, Any] = {"code": code, "message": message}
        if data is not None:
            self.error["data"] = data


_KINDS = {"add_media": "media", "add_timeline": "timeline", "add_track": "track"}
_LETTERS = {"media": "m", "timeline": "t", "clip": "c"}


class Fake:
    """The engine: one project, its connections, and what a test makes it do.

    Attributes:
        refusals: The error that a method is answered with, instead of its result, as it is
            written into the answer.
        results: The result that a method is answered with, instead of its own.
        drop: A method at which the connection is closed instead of answered, once.
        huge: A method that is answered once with a line longer than 16 MiB.
        gap: Whether the next change is told as if a revision had been missed.
        hold: How many of the next answers are held back, and sent the other way round.
        noise: Lines sent to a connection after its answer to hello.
        project_dir: The directory that hello names, if not the one of the project.
        asked: Every method that was asked, in order.
    """

    def __init__(self, directory: Path, address: str) -> None:
        self.directory = directory
        self.address = address
        self.revision = 0
        self.project: dict[str, Any] = {"schema": 1, "next_id": 1, "media": [], "timelines": []}
        self.steps: list[tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]] = []
        self.undone: list[tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]] = []
        self.connections: list[Connection] = []
        self.refusals: dict[str, dict[str, Any]] = {}
        self.results: dict[str, Any] = {}
        self.drop: str | None = None
        self.huge: str | None = None
        self.gap = False
        self.hold = 0
        self.noise: list[bytes] = []
        self.project_dir: str | None = None
        self.asked: list[str] = []
        self.last_left = time.monotonic()
        self.log: Callable[[dict[str, Any]], None] = lambda line: None

    @property
    def greeted(self) -> list["Connection"]:
        """The connections that said hello."""
        return [one for one in self.connections if one.client is not None]

    def tell(self, method: str, params: dict[str, Any]) -> None:
        """Send a notification to every connection that said hello."""
        for one in self.greeted:
            one.send({"jsonrpc": "2.0", "method": method, "params": params})

    def say(self, *lines: bytes) -> None:
        """Send lines as they are to every connection that said hello."""
        for one in self.greeted:
            for line in lines:
                cast("asyncio.Transport", one.transport).write(line + b"\n")

    def changed(self, by: str, done: list[dict[str, Any]], before: dict[str, Any]) -> None:
        """Make a revision of a change, and tell it."""
        self.revision += 2 if self.gap else 1
        self.gap = False
        self.steps.append((before, copy.deepcopy(self.project), done))
        self.undone.clear()
        self._told(by, done)

    def _told(self, by: str, done: list[dict[str, Any]]) -> None:
        self.tell(
            "project.changed",
            {
                "revision": self.revision,
                "by": by,
                "made_by": by,
                "done": done,
                "next_id": self.project["next_id"],
            },
        )

    # The methods

    def hello(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        if asking.client is not None:
            raise RefusedError(-32600, "hello was said before")
        if params.get("protocol") != 1:
            raise RefusedError(-32602, "protocol 1 is spoken", {"protocol": 1})
        asking.client = str(params.get("client"))
        return {
            "engine": "0.1.0-fake",
            "protocol": 1,
            "revision": self.revision,
            "project": self.project_dir or str(self.directory),
            "pid": os.getpid(),
        }

    def project_get(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return {
            "revision": self.revision,
            "next_id": self.project["next_id"],
            "project": copy.deepcopy(self.project),
        }

    def transaction_apply(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        ops = cast("list[dict[str, Any]]", params.get("ops") or [])
        if not ops:
            message = "the transaction has no operations"
            raise RefusedError(-32001, message, {"reason": "empty", "message": message})
        before = copy.deepcopy(self.project)
        try:
            done, names = self._applied(copy.deepcopy(ops))
        except RefusedError:
            self.project = before
            raise
        self.changed(str(asking.client), done, before)
        return {
            "revision": self.revision,
            "done": done,
            "next_id": self.project["next_id"],
            "names": names,
        }

    def history_undo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._stepped(asking, self.steps, self.undone, back=True)

    def history_redo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._stepped(asking, self.undone, self.steps, back=False)

    def media_import(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        paths = cast("list[str]", params.get("paths") or [])
        for index, path in enumerate(paths):
            if not Path(path).is_absolute():
                raise RefusedError(
                    -32602,
                    f"media.import takes absolute paths, and {path} is not one",
                    {"index": index, "path": path, "reason": "relative"},
                )
        for path in paths:
            if not Path(path).is_file():
                message = "it cannot be read: no such file"
                raise RefusedError(
                    -32002,
                    f"a file cannot be read: {path}: {message}",
                    {"path": path, "reason": "not_found", "message": message},
                )
        ops = [{"op": "add_media", "media": _media(Path(path))} for path in paths]
        before = copy.deepcopy(self.project)
        done, _ = self._applied(ops)
        self.changed(str(asking.client), done, before)
        return {
            "revision": self.revision,
            "added": [one["media"]["id"] for one in done],
            "next_id": self.project["next_id"],
        }

    def _stepped(
        self,
        asking: "Connection",
        taken: list[tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]],
        given: list[tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]],
        *,
        back: bool,
    ) -> dict[str, Any]:
        if not taken:
            return {"revision": self.revision, "done": None}
        step = taken.pop()
        before, after, done = step
        given.append(step)
        next_id = self.project["next_id"]
        self.project = copy.deepcopy(before if back else after)
        self.project["next_id"] = next_id
        undone = [{"op": "undo", "of": one} for one in reversed(done)] if back else done
        self.revision += 1
        self._told(str(asking.client), undone)
        return {"revision": self.revision, "done": undone, "next_id": next_id}

    def _applied(self, ops: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, str]]:
        names: dict[str, str] = {}
        for index, op in enumerate(ops):
            kind = _KINDS.get(op["op"], "clip" if op["op"] == "insert_clip" else None)
            for key in ("timeline", "track", "id"):
                if key in op and key != kind:
                    op[key] = self._named(op[key], names, index)
            clip = op.get("clip")
            if isinstance(clip, dict):
                clip = cast("dict[str, Any]", clip)
                clip["media"] = self._named(clip.get("media"), names, index)
            thing = cast("dict[str, Any] | None", op.get(kind) if kind else None)
            if kind is None or thing is None:
                self._removed(op)
                continue
            given = str(thing.get("id", ""))
            if not given or given.startswith("$"):
                if kind == "track":
                    timeline = self._timeline(str(op["timeline"]), index)
                    thing["id"] = f"V{len(timeline['tracks']) + 1}"
                else:
                    thing["id"] = f"{_LETTERS[kind]}{self.project['next_id']}"
                    self.project["next_id"] += 1
                if given:
                    names[given] = thing["id"]
            self._added(op, kind, thing, index)
        return ops, names

    def _named(self, value: object, names: dict[str, str], index: int) -> object:
        if isinstance(value, str) and value.startswith("$"):
            if value not in names:
                raise RefusedError(
                    -32602,
                    f"{value} is named before an operation gives it",
                    {"index": index, "name": value, "reason": "unknown_name"},
                )
            return names[value]
        return value

    def _timeline(self, id: str, index: int) -> dict[str, Any]:
        for timeline in self.project["timelines"]:
            if timeline["id"] == id:
                return timeline
        message = f"there is no timeline {id}"
        raise RefusedError(
            -32001,
            message,
            {"index": index, "reason": "missing", "what": "timeline", "id": id, "message": message},
        )

    def _added(self, op: dict[str, Any], kind: str, thing: dict[str, Any], index: int) -> None:
        if kind == "media":
            places = self.project["media"]
        elif kind == "timeline":
            places = self.project["timelines"]
        elif kind == "track":
            places = self._timeline(str(op["timeline"]), index)["tracks"]
        else:
            tracks = self._timeline(str(op["timeline"]), index)["tracks"]
            places = next(one for one in tracks if one["id"] == op["track"])["clips"]
        op.setdefault("index", len(places))
        places.append(thing)

    def _removed(self, op: dict[str, Any]) -> None:
        if op["op"] == "remove_media":
            self.project["media"] = [m for m in self.project["media"] if m["id"] != op["id"]]
        elif op["op"] == "remove_timeline":
            self.project["timelines"] = [
                t for t in self.project["timelines"] if t["id"] != op["id"]
            ]


def _media(path: Path) -> dict[str, Any]:
    return {
        "id": "",
        "path": str(path),
        "identity": {"size": path.stat().st_size, "modified_us": 0, "digest": "0" * 64},
        "facts": {
            "duration": {"ticks": 25, "scale": 25},
            "video": {
                "width": 16,
                "height": 16,
                "rate": {"num": 25, "den": 1},
                "codec": "h264",
                "bits": 8,
                "rotation": 0,
            },
        },
    }


class Connection(asyncio.Protocol):
    """One client of the fake engine."""

    def __init__(self, fake: Fake) -> None:
        self.fake = fake
        self.buffer = b""
        self.client: str | None = None
        self.transport: asyncio.Transport | None = None
        self.kept: list[dict[str, Any]] = []

    def connection_made(self, transport: asyncio.BaseTransport) -> None:
        self.transport = cast("asyncio.Transport", transport)
        self.fake.connections.append(self)
        self.fake.log({"event": "connected"})

    def connection_lost(self, exc: Exception | None) -> None:
        self.fake.connections.remove(self)
        self.fake.last_left = time.monotonic()

    def data_received(self, data: bytes) -> None:
        self.buffer += data
        while b"\n" in self.buffer:
            line, _, self.buffer = self.buffer.partition(b"\n")
            if line.strip() and self.transport is not None and not self.transport.is_closing():
                self.handled(json.loads(line))

    def send(self, message: dict[str, Any]) -> None:
        """Send a message as a line."""
        assert self.transport is not None
        if not self.transport.is_closing():
            self.transport.write(json.dumps(message).encode() + b"\n")

    def handled(self, request: dict[str, Any]) -> None:
        """Answer a request."""
        fake = self.fake
        method, number = str(request.get("method")), request.get("id")
        params = cast("dict[str, Any]", request.get("params") or {})
        fake.asked.append(method)
        assert self.transport is not None
        if method == fake.drop:
            fake.drop = None
            self.transport.close()
            return
        if method == fake.huge:
            fake.huge = None
            self.transport.write(b'{"pad": "' + b"x" * (LIMIT + 1) + b'"}\n')
            return
        answer: dict[str, Any] = {"jsonrpc": "2.0", "id": number}
        try:
            if self.client is None and method != "hello":
                raise RefusedError(-32600, "hello is said first")
            if method in fake.refusals:
                answer["error"] = fake.refusals[method]
            else:
                found = getattr(fake, method.replace(".", "_"), None)
                if found is None:
                    raise RefusedError(-32601, f"there is no method {method}")
                result = found(self, params)
                answer["result"] = fake.results.get(method, result)
        except RefusedError as refused:
            answer["error"] = refused.error
        self.answered(answer)
        if method == "hello" and "result" in answer:
            for line in fake.noise:
                self.transport.write(line + b"\n")

    def answered(self, answer: dict[str, Any]) -> None:
        if self.fake.hold:
            self.kept.append(answer)
            self.fake.hold -= 1
            if self.fake.hold:
                return
            kept, self.kept = self.kept, []
            for one in reversed(kept):
                self.send(one)
            return
        self.send(answer)


@contextlib.asynccontextmanager
async def served(directory: Path, *, lock: bool = True) -> AsyncGenerator[Fake]:
    """Serve a project in the loop of the test, and say in ``engine.lock`` where, if ``lock``."""
    directory.mkdir(parents=True, exist_ok=True)
    fake = Fake(directory, address_of())
    close = await _listening(fake)
    if lock:
        _locked(fake)
    try:
        yield fake
    finally:
        close()
        for one in list(fake.connections):
            cast("asyncio.Transport", one.transport).close()
        await asyncio.sleep(0)


async def _listening(fake: Fake) -> Callable[[], None]:
    loop = asyncio.get_running_loop()
    if sys.platform == "win32":
        proactor = cast("asyncio.ProactorEventLoop", loop)
        servers = await proactor.start_serving_pipe(
            lambda: Connection(fake),  # pyright: ignore[reportArgumentType]
            fake.address,
        )

        def closed() -> None:
            for server in servers:
                server.close()

        return closed
    server = await loop.create_unix_server(lambda: Connection(fake), fake.address)

    def unlinked() -> None:
        server.close()
        with contextlib.suppress(OSError):
            Path(fake.address).unlink()

    return unlinked


def _locked(fake: Fake) -> None:
    started = datetime.now(UTC).isoformat()
    (fake.directory / "engine.lock").write_text(
        json.dumps({"pid": os.getpid(), "address": fake.address, "started": started}),
        encoding="utf-8",
    )


def _say(line: dict[str, Any], *, to: Any = None) -> None:
    out = to or sys.stderr
    out.write(json.dumps(line) + "\n")
    out.flush()


async def _engine(directory: Path, idle_seconds: float) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    fake = Fake(directory, address_of())
    fake.log = _say
    close = await _listening(fake)
    _locked(fake)
    _say({"listening": fake.address, "pid": os.getpid(), "protocol": 1}, to=sys.stdout)
    _say({"event": "listening"})
    while fake.connections or time.monotonic() - fake.last_left < idle_seconds:  # noqa: ASYNC110
        await asyncio.sleep(0.02)
    close()
    (directory / "engine.lock").unlink()


def main(argv: list[str]) -> int:
    """Run as the engine is run, or end as one that cannot start ends."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True, type=Path)
    parser.add_argument("--headless", action="store_true")
    parser.add_argument("--idle-seconds", type=float, default=30.0)
    parser.add_argument("--mode", nargs="+", default=["listen"])
    args = parser.parse_args(argv)
    mode, *rest = args.mode
    directory: Path = args.project
    if mode == "held":
        _say({"held": {"pid": 1, "address": rest[0] if rest else 7}})
        return 3
    if mode == "held-moved":
        (directory / "engine.lock").write_text(json.dumps({"address": rest[0]}), encoding="utf-8")
        _say({"held": {"pid": 1, "address": address_of()}})
        return 3
    marked = directory / "contended"
    if mode == "contended" or (mode == "contended-once" and not marked.exists()):
        directory.mkdir(parents=True, exist_ok=True)
        marked.touch()
        _say({"contended": True})
        return 3
    if mode == "error":
        _say({"error": "project.json cannot be read"})
        return 2
    if mode == "exit":
        sys.stderr.write("not json\n[1]\n")
        _say({"event": "failed"})
        return 5
    if mode == "libraries":
        sys.stderr.flush()
        os._exit(-1073741515)
    if mode == "babble":
        print("hello there", flush=True)
        return 0
    if mode == "silent":
        time.sleep(60)
        return 0
    asyncio.run(_engine(directory, args.idle_seconds))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
