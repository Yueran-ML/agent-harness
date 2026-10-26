"""One language server, talked to over its standard input and output.

The Language Server Protocol frames each message of JSON-RPC with a header
that says how many bytes it has. :class:`Server` starts a server, says hello
to it, tells it which files are open and what they hold, asks it questions,
and keeps what it says of each file unasked: its diagnostics.

A server that ends, or that writes something that is no message, is done
with: every question that waits for it fails, and it is asked nothing more.
A request that a server asks of this side is answered with nothing, which is
what a client that does not do such a thing answers.
"""

import asyncio
import contextlib
import json
import logging
import os
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast
from urllib.parse import unquote, urlsplit

__all__ = ["Diagnostic", "LspError", "Server", "path_of", "uri_of"]

logger = logging.getLogger("columbina.lsp")

_HEADER_MOST = 4096
"""How long the header of a message may be."""

_BODY_MOST = 64 * 1024 * 1024
"""How long a message may be."""

_DRIVE = re.compile(r"^/[A-Za-z]:")
"""The beginning of the path of an address of a file on Windows, which has a slash before it."""


class LspError(Exception):
    """A server could not be asked, or did not answer, and why."""


def uri_of(path: Path) -> str:
    """Return the address of a file, as a server is told it."""
    return path.resolve().as_uri()


def path_of(uri: str) -> Path:
    """Return the file that an address names.

    A server may write the address of a file in another way than it was told
    it: with the drive of Windows in small letters, or with its colon written
    as ``%3A``.
    """
    written = unquote(urlsplit(uri).path)
    if _DRIVE.match(written):
        written = written[1:]
    return Path(written)


def _same(uri: str) -> str:
    """Return what two addresses of the same file have in common, however they are written."""
    return os.path.normcase(os.path.normpath(path_of(uri)))


@dataclass(frozen=True, slots=True)
class Diagnostic:
    """What a server says is wrong in a file, or may be.

    Attributes:
        line: The line, from 1.
        column: The column, from 1.
        severity: ``error``, ``warning``, ``information`` or ``hint``.
        message: What it says.
        source: Who says it, such as ``Pyright``, if the server names one.
    """

    line: int
    column: int
    severity: str
    message: str
    source: str | None = None


_SEVERITIES = {1: "error", 2: "warning", 3: "information", 4: "hint"}


def _diagnostic(raw: Mapping[str, Any]) -> Diagnostic:
    start = raw.get("range", {}).get("start", {})
    source = raw.get("source")
    return Diagnostic(
        line=int(start.get("line", 0)) + 1,
        column=int(start.get("character", 0)) + 1,
        severity=_SEVERITIES.get(raw.get("severity", 1), "error"),
        message=str(raw.get("message", "")).strip(),
        source=source if isinstance(source, str) else None,
    )


@dataclass(slots=True)
class _Said:
    """What a server last said of a file, and how many times it has said something of it."""

    diagnostics: tuple[Diagnostic, ...] = ()
    times: int = 0
    heard: asyncio.Event = field(default_factory=asyncio.Event)


async def _heard(said: _Said, seconds: float) -> bool:
    """Wait until the server says something more of a file, for so long, and say whether it did.

    The waiting is a task of its own that is given up when the time is out,
    so nothing is thrown into whoever waits.
    """
    if seconds <= 0:
        return False
    waiting = asyncio.ensure_future(said.heard.wait())
    done, _ = await asyncio.wait({waiting}, timeout=seconds)
    if not done:
        waiting.cancel()
        return False
    said.heard.clear()
    return True


class Server:
    """A language server that runs.

    Args:
        argv: How the server is started.
        root: The directory that it works in, the working directory.
        language: What the files are written in, as the protocol names it,
            such as ``python``.
    """

    def __init__(self, argv: Sequence[str], root: Path, language: str) -> None:
        self.argv = tuple(argv)
        self.root = root
        self.language = language
        self._process: asyncio.subprocess.Process | None = None
        self._reading: asyncio.Task[None] | None = None
        self._next = 0
        self._waiting: dict[int, asyncio.Future[Any]] = {}
        self._said: dict[str, _Said] = {}
        self._versions: dict[str, int] = {}
        self._ended: str | None = None

    @property
    def ended(self) -> str | None:
        """Why the server is done with, or ``None`` while it can be asked."""
        return self._ended

    async def start(self, seconds: float) -> None:
        """Start the server and say hello to it.

        Raises:
            LspError: It could not be started, or did not answer in time.
        """
        try:
            self._process = await asyncio.create_subprocess_exec(
                *self.argv,
                cwd=self.root,
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.DEVNULL,
            )
        except OSError as error:
            self._ended = f"it could not be started: {error}"
            raise LspError(self._ended) from None
        self._reading = asyncio.ensure_future(self._read())
        root = uri_of(self.root)
        await self.request(
            "initialize",
            {
                "processId": None,
                "rootUri": root,
                "workspaceFolders": [{"uri": root, "name": self.root.name}],
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": {"relatedInformation": False},
                        "synchronization": {"didSave": False},
                        "hover": {"contentFormat": ["plaintext", "markdown"]},
                        "documentSymbol": {"hierarchicalDocumentSymbolSupport": True},
                    },
                    "workspace": {"workspaceFolders": True, "configuration": True},
                },
            },
            seconds=seconds,
        )
        await self.notify("initialized", {})

    async def _send(self, message: Mapping[str, object]) -> None:
        if self._ended is not None or self._process is None or self._process.stdin is None:
            raise LspError(f"the server is done with: {self._ended or 'it is not running'}")
        body = json.dumps(message, ensure_ascii=False).encode("utf-8")
        try:
            self._process.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
            await self._process.stdin.drain()
        except (OSError, RuntimeError) as error:
            self._end(f"it could not be written to: {error}")
            raise LspError(f"the server is done with: {self._ended}") from None

    async def notify(self, method: str, params: Mapping[str, object]) -> None:
        """Tell the server something that it does not answer."""
        await self._send({"jsonrpc": "2.0", "method": method, "params": params})

    async def request(self, method: str, params: Mapping[str, object], *, seconds: float) -> Any:
        """Ask the server something, and return what it answers.

        Raises:
            LspError: It answered with an error, did not answer in time, or
                is done with.
        """
        self._next += 1
        number = self._next
        waiting: asyncio.Future[Any] = asyncio.get_running_loop().create_future()
        self._waiting[number] = waiting
        try:
            await self._send({"jsonrpc": "2.0", "id": number, "method": method, "params": params})
            async with asyncio.timeout(seconds):
                return await waiting
        except TimeoutError:
            raise LspError(f"it did not answer {method} within {seconds:g} seconds") from None
        finally:
            self._waiting.pop(number, None)

    async def show(self, path: Path, text: str) -> None:
        """Tell the server what a file holds now, opening it the first time."""
        uri = uri_of(path)
        version = self._versions.get(uri, 0) + 1
        self._versions[uri] = version
        if version == 1:
            await self.notify(
                "textDocument/didOpen",
                {
                    "textDocument": {
                        "uri": uri,
                        "languageId": self.language,
                        "version": version,
                        "text": text,
                    }
                },
            )
        else:
            await self.notify(
                "textDocument/didChange",
                {
                    "textDocument": {"uri": uri, "version": version},
                    "contentChanges": [{"text": text}],
                },
            )

    async def diagnostics(
        self, path: Path, text: str, *, seconds: float, settle: float
    ) -> tuple[Diagnostic, ...] | None:
        """Show the server a file, and return what it says of it then.

        A server says something of a file a while after it is shown it, and
        may say it more than once as it looks further. So what it says is
        waited for, and then a moment longer for what it may say after.

        Returns:
            What it said, or ``None`` if it said nothing in time.
        """
        said = self._said.setdefault(_same(uri_of(path)), _Said())
        before = said.times
        said.heard.clear()
        await self.show(path, text)
        loop = asyncio.get_running_loop()
        until = loop.time() + seconds
        while said.times == before:
            if self._ended is not None or not await _heard(said, until - loop.time()):
                return None
        until = loop.time() + settle
        while await _heard(said, until - loop.time()):
            pass
        return said.diagnostics

    async def _read(self) -> None:
        """Read what the server writes, until it ends."""
        assert self._process is not None
        out = self._process.stdout
        assert out is not None
        try:
            while True:
                length = None
                while True:
                    line = await out.readline()
                    if not line:
                        self._end("it ended")
                        return
                    if len(line) > _HEADER_MOST:
                        self._end("it wrote a header that is too long")
                        return
                    stripped = line.strip()
                    if not stripped:
                        break
                    name, _, value = stripped.decode("ascii", "replace").partition(":")
                    if name.strip().lower() == "content-length":
                        length = int(value.strip())
                if length is None or not 0 <= length <= _BODY_MOST:
                    self._end("it wrote a message without a length that can be read")
                    return
                body = await out.readexactly(length)
                await self._heard(json.loads(body))
        except (ValueError, asyncio.IncompleteReadError, OSError) as error:
            self._end(f"it wrote what is no message: {error}")

    async def _heard(self, message: object) -> None:
        if not isinstance(message, dict):
            return
        said = cast("dict[str, Any]", message)
        method = said.get("method")
        if "id" in said and method is None:
            waiting = self._waiting.get(said["id"])
            if waiting is not None and not waiting.done():
                error = said.get("error")
                if error is not None:
                    waiting.set_exception(LspError(str(error.get("message", error))))
                else:
                    waiting.set_result(said.get("result"))
            return
        if method == "textDocument/publishDiagnostics":
            params = said.get("params", {})
            uri = params.get("uri")
            if isinstance(uri, str):
                kept = self._said.setdefault(_same(uri), _Said())
                kept.diagnostics = tuple(_diagnostic(raw) for raw in params.get("diagnostics", []))
                kept.times += 1
                kept.heard.set()
            return
        if "id" in said and isinstance(method, str):
            # A request of the server, such as one for its configuration. Nothing is set here.
            answer: object = (
                [None] * len(said.get("params", {}).get("items", []))
                if (method == "workspace/configuration")
                else None
            )
            with contextlib.suppress(LspError):
                await self._send({"jsonrpc": "2.0", "id": said["id"], "result": answer})

    def _end(self, why: str) -> None:
        if self._ended is None:
            self._ended = why
            logger.info("the language server %s is done with: %s", self.argv[0], why)
        for waiting in self._waiting.values():
            if not waiting.done():
                waiting.set_exception(LspError(f"the server is done with: {why}"))
        for said in self._said.values():
            said.heard.set()

    async def stop(self) -> None:
        """Ask the server to end, and end it if it does not."""
        process = self._process
        if process is None:
            return
        if self._ended is None:
            with contextlib.suppress(LspError):
                await self.request("shutdown", {}, seconds=2)
                await self.notify("exit", {})
        self._end("it was stopped")
        # A process that has ended already cannot be found to be ended again.
        with contextlib.suppress(ProcessLookupError, OSError):
            process.kill()
        with contextlib.suppress(Exception):
            await asyncio.wait_for(process.wait(), timeout=5)
        # Whatever has a process reads from it.
        reading = self._reading
        assert reading is not None
        reading.cancel()
        with contextlib.suppress(asyncio.CancelledError, Exception):
            await reading
