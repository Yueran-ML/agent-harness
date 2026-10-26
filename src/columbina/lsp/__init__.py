"""Language servers: what they say of a file after it was changed, and where its names lead.

This module is a plugin. Loading it does two things.

**After a change.** When a tool that writes has changed a file, the language
server of the language of the file is shown it, and what it says is wrong is
put after the result of the call: errors first, then warnings, at most
:data:`SHOWN` of them. The model sees at once that a name is missing or a
type does not fit, and mends it in the same step. Where the server says
nothing is wrong, nothing is added.

**A tool.** ``lsp`` asks the server of a file where a name is defined, where
it is used, what it is, which names a file defines, or what is wrong in it.

**Which server.** A file is known by the ending of its name. Each language has
a few servers that may be on the search path, and the first that is there is
started, in the working directory, the first time a file of that language is
changed or asked about. :data:`SERVERS` lists them: Pyright and basedpyright
or pylsp for Python, typescript-language-server for TypeScript and
JavaScript, rust-analyzer for Rust and gopls for Go. The config may name
other servers, or other commands for these. A server is not taken from the
working directory or what lies in it, since what runs there could put one
there. A server that cannot be started, or that ends, is not asked again in
this run. None of this needs anything that is not there: without a server a
change is told as it always was.

Servers keep running while the program does, one for each language and
working directory, and are ended with it.
"""

import asyncio
import logging
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, cast

from pydantic import BaseModel, Field

from columbina import programs
from columbina.kernel import Context, Next
from columbina.lsp.client import Diagnostic, LspError, Server, path_of, uri_of
from columbina.tools import (
    POST_EXECUTE,
    TOOLS,
    Accept,
    Executed,
    Tool,
    ToolFailure,
    ToolRun,
    Verdict,
)

__all__ = [
    "SERVERS",
    "SHOWN",
    "Config",
    "Language",
    "LspArgs",
    "Servers",
    "apply",
    "inject",
    "lsp_tool",
    "name",
    "told",
]

name = "lsp"
inject = (TOOLS,)

logger = logging.getLogger("columbina.lsp")

SHOWN = 20
"""How many things that are wrong are told after a change, at most."""


class Language(BaseModel):
    """A language and the servers that know it.

    Attributes:
        id: What the protocol calls it, such as ``python``.
        endings: The endings of the names of its files, with their dots.
        servers: The commands that start a server, each a program on the
            search path and its arguments, the one to try first first.
    """

    id: str
    endings: tuple[str, ...]
    servers: tuple[tuple[str, ...], ...]


SERVERS: tuple[Language, ...] = (
    Language(
        id="python",
        endings=(".py", ".pyi"),
        servers=(
            ("basedpyright-langserver", "--stdio"),
            ("pyright-langserver", "--stdio"),
            ("pylsp",),
        ),
    ),
    Language(
        id="typescript",
        endings=(".ts", ".tsx", ".mts", ".cts"),
        servers=(("typescript-language-server", "--stdio"),),
    ),
    Language(
        id="javascript",
        endings=(".js", ".jsx", ".mjs", ".cjs"),
        servers=(("typescript-language-server", "--stdio"),),
    ),
    Language(id="rust", endings=(".rs",), servers=(("rust-analyzer",),)),
    Language(id="go", endings=(".go",), servers=(("gopls",),)),
)
"""The languages that are known, and their servers."""


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        languages: Languages beside those that are known, or in their place
            where they have the same id.
        start_seconds: How long a server may take to start.
        seconds: How long what a server says of a change is waited for.
        settle_seconds: How long it is waited for after that, for what it
            says further.
        after_a_change: Whether what is wrong is told after a change.
    """

    languages: tuple[Language, ...] = ()
    start_seconds: float = Field(default=30.0, gt=0)
    seconds: float = Field(default=5.0, gt=0)
    settle_seconds: float = Field(default=0.3, ge=0)
    after_a_change: bool = True


def _first_line(text: str) -> str:
    lines = text.splitlines()
    return lines[0] if lines else ""


def told(path: str, diagnostics: Sequence[Diagnostic], *, shown: int = SHOWN) -> str:
    """Say what a server says is wrong in a file, errors first."""
    order = {"error": 0, "warning": 1, "information": 2, "hint": 3}
    ranked = sorted(diagnostics, key=lambda one: (order.get(one.severity, 4), one.line, one.column))
    lines = [
        f"  {path}:{one.line}:{one.column} {one.severity}: {_first_line(one.message)}"
        for one in ranked[:shown]
    ]
    if len(ranked) > shown:
        lines.append(f"  and {len(ranked) - shown} more")
    return "\n".join(lines)


@dataclass(frozen=True, slots=True)
class _Key:
    root: Path
    language: str


class Servers:
    """The servers of a run, started when first needed, one for each language and root."""

    def __init__(self, config: Config) -> None:
        self._config = config
        known = {language.id: language for language in SERVERS}
        known.update({language.id: language for language in config.languages})
        self._languages = tuple(known.values())
        self._running: dict[_Key, Server] = {}
        self._starting: dict[_Key, asyncio.Future[Server | None]] = {}
        self._failed: set[_Key] = set()

    @property
    def seconds(self) -> float:
        """How long a question to a server is waited for."""
        return self._config.seconds * 2

    def language_of(self, path: Path) -> Language | None:
        """Return the language of a file, by the ending of its name."""
        ending = path.suffix.lower()
        return next((language for language in self._languages if ending in language.endings), None)

    async def of(self, root: Path, path: Path) -> Server | None:
        """Return the server of a file, started if it is not yet, or ``None`` if there is none."""
        language = self.language_of(path)
        if language is None:
            return None
        key = _Key(root, language.id)
        running = self._running.get(key)
        if running is not None and running.ended is None:
            return running
        if key in self._failed or (running is not None and running.ended is not None):
            return None
        starting = self._starting.get(key)
        if starting is None:
            starting = self._starting[key] = asyncio.ensure_future(self._start(key, language))
        return await asyncio.shield(starting)

    async def _start(self, key: _Key, language: Language) -> Server | None:
        try:
            for command in language.servers:
                program = programs.found(
                    command[0], _search_path(), outside=[key.root], windows=(".exe", ".cmd")
                )
                if program is None:
                    continue
                server = Server((program, *command[1:]), key.root, language.id)
                try:
                    await server.start(self._config.start_seconds)
                except LspError as error:
                    logger.warning("the language server %s did not start: %s", command[0], error)
                    await server.stop()
                    continue
                except asyncio.CancelledError:
                    # The run ends while the server starts. It is not left running.
                    await server.stop()
                    raise
                self._running[key] = server
                return server
            self._failed.add(key)
            return None
        finally:
            self._starting.pop(key, None)

    async def diagnostics(self, root: Path, path: Path) -> tuple[Diagnostic, ...] | None:
        """Return what the server of a file says of it now, or ``None`` if it says nothing."""
        server = await self.of(root, path)
        if server is None:
            return None
        try:
            text = await asyncio.to_thread(path.read_text, encoding="utf-8", errors="replace")
            return await server.diagnostics(
                path, text, seconds=self._config.seconds, settle=self._config.settle_seconds
            )
        except (OSError, LspError) as error:
            logger.info("the language server said nothing of %s: %s", path, error)
            return None

    async def stop(self) -> None:
        """End every server."""
        for starting in list(self._starting.values()):
            starting.cancel()
        await asyncio.gather(*(server.stop() for server in self._running.values()))
        self._running.clear()


def _search_path() -> str:
    return os.environ.get("PATH", "")


class LspArgs(BaseModel):
    """The arguments of the ``lsp`` tool."""

    operation: Literal["definition", "references", "hover", "symbols", "diagnostics"] = Field(
        description=(
            "definition: where the name at the place is defined; references: where it is "
            "used; hover: what it is; symbols: the names that the file defines; "
            "diagnostics: what is wrong in the file."
        )
    )
    path: str = Field(description="The file. A relative path starts at the working directory.")
    line: int = Field(default=1, ge=1, description="The line of the name, from 1.")
    column: int = Field(default=1, ge=1, description="The column of the name, from 1.")


def _relative(uri: str, root: Path) -> str:
    """Return where a file that a server names lies, from the working directory if it is there."""
    path = path_of(uri)
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return str(path)


def _places(found: object, root: Path) -> list[str]:
    """Return the places that a server answered, as ``path:line:column``."""
    if found is None:
        return []
    items = cast("list[Any]", found) if isinstance(found, list) else [found]
    places: list[str] = []
    for item in items:
        if not isinstance(item, dict):
            continue
        one = cast("dict[str, Any]", item)
        uri = one.get("uri") or one.get("targetUri")
        if isinstance(uri, str):
            line, column = _start(one.get("range") or one.get("targetSelectionRange"))
            places.append(f"{_relative(uri, root)}:{line}:{column}")
    return places


def _start(where: object) -> tuple[int, int]:
    """Return the line and the column, from 1, where a range of the protocol begins."""
    start: object = cast("dict[str, Any]", where).get("start") if isinstance(where, dict) else None
    if not isinstance(start, dict):
        return 1, 1
    at = cast("dict[str, Any]", start)
    return int(at.get("line", 0)) + 1, int(at.get("character", 0)) + 1


def _hover(found: object) -> str:
    if not isinstance(found, dict):
        return ""
    contents: object = cast("dict[str, Any]", found).get("contents")
    parts = cast("list[object]", contents) if isinstance(contents, list) else [contents]
    said: list[str] = []
    for part in parts:
        if isinstance(part, str):
            said.append(part)
        elif isinstance(part, dict):
            value = cast("dict[str, Any]", part).get("value")
            if isinstance(value, str):
                said.append(value)
    return "\n".join(said).strip()


_KINDS = {
    1: "file", 2: "module", 3: "namespace", 4: "package", 5: "class", 6: "method",
    7: "property", 8: "field", 9: "constructor", 10: "enum", 11: "interface",
    12: "function", 13: "variable", 14: "constant", 22: "member", 23: "struct",
}  # fmt: skip


def _symbols(found: object, depth: int = 0) -> list[str]:
    if not isinstance(found, list):
        return []
    lines: list[str] = []
    for item in cast("list[Any]", found):
        if not isinstance(item, dict):
            continue
        one = cast("dict[str, Any]", item)
        location: object = one.get("location")
        where = (
            one.get("selectionRange")
            or one.get("range")
            or (
                cast("dict[str, Any]", location).get("range")
                if isinstance(location, dict)
                else None
            )
        )
        line, _ = _start(where)
        kind = _KINDS.get(one.get("kind", 0), "name")
        lines.append(f"{'  ' * depth}{kind} {one.get('name', '?')} (line {line})")
        lines += _symbols(one.get("children"), depth + 1)
    return lines


def lsp_tool(servers: Servers) -> Tool[LspArgs, str]:
    """Make the tool that asks a language server about a file."""

    async def ask(args: LspArgs, run: ToolRun) -> str:
        path = (run.cwd / args.path).resolve()
        if servers.language_of(path) is None:
            raise ToolFailure(f"No language server is known for {args.path}.", code="NO_SERVER")
        if not path.is_file():
            raise ToolFailure(f"There is no file {args.path}.", code="NOT_FOUND")
        if args.operation == "diagnostics":
            found = await servers.diagnostics(run.cwd, path)
            if found is None:
                raise ToolFailure(
                    f"No language server said anything of {args.path}.", code="NO_SERVER"
                )
            return told(args.path, found) or f"The server says nothing is wrong in {args.path}."
        server = await servers.of(run.cwd, path)
        if server is None:
            raise ToolFailure(
                f"No language server for {args.path} is on the search path.", code="NO_SERVER"
            )
        text = await asyncio.to_thread(path.read_text, encoding="utf-8", errors="replace")
        await server.show(path, text)
        document = {"uri": uri_of(path)}
        place = {"line": args.line - 1, "character": args.column - 1}
        methods: Mapping[str, tuple[str, dict[str, object]]] = {
            "definition": (
                "textDocument/definition",
                {"textDocument": document, "position": place},
            ),
            "references": (
                "textDocument/references",
                {
                    "textDocument": document,
                    "position": place,
                    "context": {"includeDeclaration": False},
                },
            ),
            "hover": ("textDocument/hover", {"textDocument": document, "position": place}),
            "symbols": ("textDocument/documentSymbol", {"textDocument": document}),
        }
        method, params = methods[args.operation]
        try:
            found = await server.request(method, params, seconds=servers.seconds)
        except LspError as error:
            raise ToolFailure(f"The language server could not answer: {error}") from None
        if args.operation == "hover":
            return _hover(found) or "The server says nothing of the name there."
        if args.operation == "symbols":
            return "\n".join(_symbols(found)) or "The server names nothing in the file."
        places = _places(found, run.cwd)
        return "\n".join(places) or "The server knows of no such place."

    return Tool(
        name="lsp",
        description=(
            "Ask the language server of a file: where the name at a line and column is "
            "defined, where it is used, what it is, which names the file defines, or what is "
            "wrong in it. For Python, TypeScript, JavaScript, Rust and Go, where a server is "
            "installed."
        ),
        parameters=LspArgs,
        execute=ask,
        effect="read",
        concurrency_safe=True,
        target=lambda args, run: args.path,
    )


def apply(ctx: Context, config: Config) -> None:
    """Start servers as they are needed, tell what they say after a change, and give the tool."""
    servers = Servers(config)

    async def after_a_call(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        verdict = await proceed(executed)
        invocation = executed.invocation
        target = invocation.run.target
        if (
            not config.after_a_change
            or not isinstance(verdict, Accept)
            or invocation.tool.effect != "write"
            or target is None
        ):
            return verdict
        path = invocation.run.cwd / target
        if servers.language_of(path) is None:
            return verdict
        found = await servers.diagnostics(invocation.run.cwd, path)
        if not found:
            return verdict
        wrong = told(target, found)
        return Accept(
            text=f"{verdict.text}\n\nWhat the language server says of {target} now:\n{wrong}"
        )

    ctx.on(POST_EXECUTE, after_a_call)
    ctx.use(TOOLS).add(ctx, lsp_tool(servers))
    ctx.effect(lambda: servers.stop)
