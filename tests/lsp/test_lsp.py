"""Language servers: what they say after a change, and the tool that asks them."""

import asyncio
import json
import sys
from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from pydantic import BaseModel

import columbina.lsp
import columbina.tools
from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.lsp import Config, Language, LspArgs, Servers, lsp_tool, told
from columbina.lsp.client import Diagnostic, LspError, Server, path_of, uri_of
from columbina.session import Session
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun, Tools

FAKE = str(Path(__file__).parents[1] / "support" / "fake_lsp.py")


def language(mode: str = "normal") -> Language:
    return Language(id="python", endings=(".py",), servers=(("fake-server", FAKE, mode),))


@pytest.fixture(autouse=True)
def fake_is_found(monkeypatch: pytest.MonkeyPatch) -> None:
    """The fake server is found on the search path, and no other server is."""

    def found(name: str, search: str, outside: object = (), **more: object) -> str | None:
        return sys.executable if name == "fake-server" else None

    monkeypatch.setattr("columbina.programs.found", found)


@pytest.fixture
async def servers() -> AsyncIterator[Servers]:
    made = Servers(Config(languages=(language(),), seconds=5, settle_seconds=0.2))
    yield made
    await made.stop()


def run(work: Path) -> ToolRun:
    return ToolRun("call-1", work, Session(Context(), "s-1", {}))


# What a server says of a file


async def test_what_is_wrong_is_said_errors_first(servers: Servers, tmp_path: Path) -> None:
    (tmp_path / "a.py").write_text("x = 1  # WARN\ny = ERROR\n", encoding="utf-8")

    found = await servers.diagnostics(tmp_path, tmp_path / "a.py")

    assert found == (
        Diagnostic(1, 10, "warning", "warn here\nmore", "fake"),
        Diagnostic(2, 5, "error", "error here\nmore", "fake"),
    )
    assert told("a.py", found or ()) == (
        "  a.py:2:5 error: error here\n  a.py:1:10 warning: warn here"
    )


async def test_a_file_that_is_shown_again_is_said_anew(servers: Servers, tmp_path: Path) -> None:
    file = tmp_path / "a.py"
    file.write_text("ERROR\n", encoding="utf-8")
    assert len(await servers.diagnostics(tmp_path, file) or ()) == 1
    file.write_text("fine\n", encoding="utf-8")

    assert await servers.diagnostics(tmp_path, file) == ()


async def test_many_things_are_told_up_to_a_number() -> None:
    many = [Diagnostic(line, 1, "error", "") for line in range(1, 25)]

    said = told("a.py", many, shown=3).split("\n")

    assert said == [
        "  a.py:1:1 error: ",
        "  a.py:2:1 error: ",
        "  a.py:3:1 error: ",
        "  and 21 more",
    ]


async def test_a_file_of_no_known_language_has_no_server(servers: Servers, tmp_path: Path) -> None:
    (tmp_path / "notes.txt").write_text("ERROR", encoding="utf-8")

    assert await servers.diagnostics(tmp_path, tmp_path / "notes.txt") is None
    assert servers.language_of(tmp_path / "x.rs") is not None


async def test_a_language_without_a_server_on_the_path_is_given_up(tmp_path: Path) -> None:
    servers = Servers(Config())
    (tmp_path / "a.py").write_text("ERROR", encoding="utf-8")

    assert await servers.diagnostics(tmp_path, tmp_path / "a.py") is None
    assert await servers.of(tmp_path, tmp_path / "a.py") is None


@pytest.mark.parametrize("mode", ["crash", "garbage", "silent", "long", "headless"])
async def test_a_server_that_says_nothing_says_nothing(tmp_path: Path, mode: str) -> None:
    servers = Servers(Config(languages=(language(mode),), seconds=0.5, settle_seconds=0))
    (tmp_path / "a.py").write_text("ERROR\n", encoding="utf-8")
    try:
        assert await servers.diagnostics(tmp_path, tmp_path / "a.py") is None
        if mode != "silent":
            # A server that is done with is not started again.
            assert await servers.of(tmp_path, tmp_path / "a.py") is None
    finally:
        await servers.stop()


async def test_a_server_that_does_not_answer_its_greeting_is_given_up(tmp_path: Path) -> None:
    servers = Servers(Config(languages=(language("mute"),), start_seconds=0.3))

    assert await servers.of(tmp_path, tmp_path / "a.py") is None
    assert await servers.of(tmp_path, tmp_path / "b.py") is None
    await servers.stop()


async def test_a_file_that_cannot_be_read_is_said_nothing_of(
    servers: Servers, tmp_path: Path
) -> None:
    assert await servers.diagnostics(tmp_path, tmp_path / "gone.py") is None


async def test_one_server_is_started_for_files_asked_about_at_once(
    servers: Servers, tmp_path: Path
) -> None:
    first, second = await asyncio.gather(
        servers.of(tmp_path, tmp_path / "a.py"), servers.of(tmp_path, tmp_path / "b.py")
    )

    assert first is second
    assert first is not None


# After a change


class Write(BaseModel):
    path: str
    text: str


async def wrote(args: Write, run: ToolRun) -> str:
    (run.cwd / args.path).write_text(args.text, encoding="utf-8")
    return "Written."


async def loaded(ctx: Context, **config: object) -> Tools:
    await ctx.plugin(columbina.tools)
    await ctx.plugin(
        columbina.lsp,
        {"languages": [language().model_dump()], "seconds": 5, "settle_seconds": 0.2, **config},
    )
    tools = ctx.use(TOOLS)
    tools.add(
        ctx,
        Tool(
            name="write",
            description="Write.",
            parameters=Write,
            execute=wrote,
            effect="write",
            target=lambda args, run: args.path,
        ),
    )
    return tools


async def written(ctx: Context, tools: Tools, work: Path, path: str, text: str) -> str:

    call = ToolCallBlock(
        call_id="c-1", name="write", arguments=json.dumps({"path": path, "text": text})
    )
    outcome = await tools.run(ctx, call, ToolRun("c-1", work, Session(ctx, "s-1", {})))
    return outcome.text


async def test_what_is_wrong_after_a_change_is_put_after_its_result(
    ctx: Context, tmp_path: Path
) -> None:
    tools = await loaded(ctx)

    said = await written(ctx, tools, tmp_path, "a.py", "y = ERROR\n")

    assert said == (
        "Written.\n\nWhat the language server says of a.py now:\n  a.py:1:5 error: error here"
    )


async def test_a_change_that_is_right_or_of_another_language_is_told_as_ever(
    ctx: Context, tmp_path: Path
) -> None:
    tools = await loaded(ctx)

    assert await written(ctx, tools, tmp_path, "a.py", "fine\n") == "Written."
    assert await written(ctx, tools, tmp_path, "notes.txt", "ERROR\n") == "Written."


async def test_the_config_may_keep_changes_untold(ctx: Context, tmp_path: Path) -> None:
    tools = await loaded(ctx, after_a_change=False)

    assert await written(ctx, tools, tmp_path, "a.py", "ERROR\n") == "Written."


# The tool


async def asked(servers: Servers, work: Path, **args: object) -> str:
    tool = lsp_tool(servers)
    return await tool.execute(LspArgs.model_validate(args), run(work))


async def test_the_tool_asks_where_a_name_is_and_what_it_is(
    servers: Servers, tmp_path: Path
) -> None:
    (tmp_path / "a.py").write_text("def f():\n    x = 1\n", encoding="utf-8")

    assert await asked(servers, tmp_path, operation="definition", path="a.py") == "a.py:1:5"
    assert await asked(servers, tmp_path, operation="references", path="a.py", line=2) == (
        "a.py:2:1\n" + str(path_of("file:///elsewhere/other.py")) + ":1:1"
    )
    assert await asked(servers, tmp_path, operation="hover", path="a.py") == "a thing\nmore"
    assert await asked(servers, tmp_path, operation="symbols", path="a.py") == (
        "function f (line 1)\n  variable x (line 2)\nname odd (line 1)"
    )
    assert await asked(servers, tmp_path, operation="diagnostics", path="a.py") == (
        "The server says nothing is wrong in a.py."
    )
    (tmp_path / "b.py").write_text("ERROR\n", encoding="utf-8")
    assert await asked(servers, tmp_path, operation="diagnostics", path="b.py") == (
        "  b.py:1:1 error: error here"
    )


async def test_a_server_that_answers_nothing_is_said_to(tmp_path: Path) -> None:
    servers = Servers(Config(languages=(language("nothing"),), seconds=0.5, settle_seconds=0))
    (tmp_path / "a.py").write_text("x\n", encoding="utf-8")
    try:
        for operation, said in (
            ("definition", "The server knows of no such place."),
            ("hover", "The server says nothing of the name there."),
            ("symbols", "The server names nothing in the file."),
        ):
            assert await asked(servers, tmp_path, operation=operation, path="a.py") == said
    finally:
        await servers.stop()


@pytest.mark.parametrize(
    ("mode", "path", "said"),
    [
        ("normal", "notes.txt", "No language server is known for notes.txt."),
        ("normal", "gone.py", "There is no file gone.py."),
        ("refuse", "a.py", "The language server could not answer: no"),
        ("mute", "a.py", "No language server for a.py is on the search path."),
        ("silent", "a.py", "No language server said anything of a.py."),
        ("die", "a.py", "The language server could not answer: the server is done with: it ended"),
    ],
)
async def test_what_the_tool_cannot_do_is_said(
    tmp_path: Path, mode: str, path: str, said: str
) -> None:
    servers = Servers(
        Config(languages=(language(mode),), start_seconds=0.5, seconds=0.5, settle_seconds=0)
    )
    (tmp_path / "a.py").write_text("x\n", encoding="utf-8")
    (tmp_path / "notes.txt").write_text("x\n", encoding="utf-8")
    operation = "diagnostics" if mode == "silent" else "hover"
    try:
        with pytest.raises(ToolFailure) as caught:
            await asked(servers, tmp_path, operation=operation, path=path)
    finally:
        await servers.stop()

    assert str(caught.value) == said


def test_the_tool_only_reads_and_names_the_file() -> None:
    tool = lsp_tool(Servers(Config()))
    args = LspArgs(operation="hover", path="a.py")

    assert tool.effect == "read"
    assert tool.target is not None
    assert tool.target(args, run(Path())) == "a.py"


# The client


async def test_a_server_that_cannot_be_started_says_why(tmp_path: Path) -> None:
    server = Server((str(tmp_path / "none"),), tmp_path, "python")

    with pytest.raises(LspError, match="it could not be started"):
        await server.start(1)
    assert server.ended is not None
    with pytest.raises(LspError, match="the server is done with"):
        await server.notify("x", {})
    await server.stop()


async def test_a_server_that_was_never_started_is_stopped_at_once(tmp_path: Path) -> None:
    await Server(("x",), tmp_path, "python").stop()


def test_an_address_names_its_file_however_it_is_written(tmp_path: Path) -> None:
    file = tmp_path / "a b.py"

    assert path_of(uri_of(file)) == file.resolve()
    assert str(path_of("file:///c%3A/x/y.py")) in ("c:\\x\\y.py", "c:/x/y.py")


async def test_a_run_that_ends_while_a_server_starts_leaves_none_running(tmp_path: Path) -> None:
    servers = Servers(Config(languages=(language("mute"),), start_seconds=30))
    asking = asyncio.ensure_future(servers.of(tmp_path, tmp_path / "a.py"))
    await asyncio.sleep(0.5)

    await servers.stop()

    with pytest.raises(asyncio.CancelledError):
        await asking


async def test_a_server_that_ends_while_it_is_asked_fails_the_question(tmp_path: Path) -> None:
    server = Server((sys.executable, FAKE, "die"), tmp_path, "python")
    await server.start(10)
    try:
        with pytest.raises(LspError, match="the server is done with: it ended"):
            await server.request("textDocument/hover", {}, seconds=10)
    finally:
        await server.stop()


async def test_no_time_to_wait_is_no_answer(tmp_path: Path) -> None:
    server = Server((sys.executable, FAKE, "normal"), tmp_path, "python")
    await server.start(10)
    try:
        assert await server.diagnostics(tmp_path / "a.py", "ERROR", seconds=0, settle=0) is None
    finally:
        await server.stop()


async def test_a_server_that_cannot_be_written_to_is_done_with(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    server = Server((sys.executable, FAKE, "normal"), tmp_path, "python")
    await server.start(10)
    process = server._process  # pyright: ignore[reportPrivateUsage]
    assert process is not None
    assert process.stdin is not None

    def broken(data: bytes) -> None:
        raise BrokenPipeError("gone")

    monkeypatch.setattr(process.stdin, "write", broken)
    try:
        with pytest.raises(LspError, match="it could not be written to: gone"):
            await server.notify("x", {})
        assert server.ended == "it could not be written to: gone"
    finally:
        await server.stop()
