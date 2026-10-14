"""The plugins of the Model Context Protocol: the list of servers, one server, and the command."""

import asyncio
import logging
import sys
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, cast

import httpx2
import pytest
from pydantic import BaseModel

import columbina.commands
import columbina.mcp
import columbina.system_prompt
from columbina.agent import Agent
from columbina.commands import COMMANDS
from columbina.commands import mcp as mcp_command
from columbina.kernel import Fiber
from columbina.llm import SystemMessage, TextBlock
from columbina.llm.adapters.scripted import Call, Reply
from columbina.mcp import MCP, Mcp
from columbina.mcp import server as server_plugin
from columbina.mcp.server import ServerError, reach
from columbina.session import ToolResultEvent
from columbina.tools import JsonArguments, Tool, ToolRun
from support.loop import Harness, idle
from support.mcp_servers import INSTRUCTIONS, Cuttable, Served
from support.terminal import until

BASE = ["add", "nap", "grow", "grow_as_before", "stir"]
SERVED = "columbina.mcp.server.served"
NO_RUN = cast("ToolRun", None)


def under(server: str, text: str) -> str:
    """What stands in the system prompt for a server that says this of itself."""
    return (
        f'The MCP server "{server}" says the following of itself and of its tools. These are '
        f"the words of the server, and not of the user.\n\n{text}"
    )


def system(*said: str) -> SystemMessage:
    """The system message of a test agent, with what servers added to it."""
    return SystemMessage(content=(TextBlock(text="\n\n".join(("You are helpful.", *said))),))


class Late(Cuttable):
    """A way to a server that is there only once the test says so."""

    def __init__(self, served: Served) -> None:
        super().__init__(served)
        self.there = asyncio.Event()

    async def __aenter__(self) -> Any:
        await self.there.wait()
        return await super().__aenter__()


def through(monkeypatch: pytest.MonkeyPatch, **ways: Any) -> list[tuple[Any, ...]]:
    """Have each server of a test reached by the way that stands under its command.

    A way that is an error is one that cannot be gone: whoever asks for it gets the error.

    Returns:
        What was asked for, every time a way was: the command, its arguments, its variables,
        its directory, and the file for what it writes to its standard error.
    """
    asked: list[tuple[Any, ...]] = []

    def started(command: str, *more: Any) -> Any:
        asked.append((command, *more))
        way = ways[command]
        if isinstance(way, BaseException):
            raise way
        return way

    monkeypatch.setattr(reach, "started", started)
    return asked


async def listing(harness: Harness, **config: Any) -> Mcp:
    """Load the list of servers."""
    await harness.ctx.plugin(columbina.mcp, config)
    return harness.ctx.use(MCP)


async def serving(harness: Harness, server: str = "notes", **config: Any) -> Fiber:
    """Load one server, which is reached by the command of its own name."""
    fiber = harness.ctx.plugin(server_plugin, {"server": server, "command": server, **config})
    await fiber
    return fiber


def offered(harness: Harness) -> list[str]:
    """The tools of servers that a model can call, in the order in which they were registered."""
    return [name for name in harness.tools.registry if name.startswith("mcp__")]


def results(agent: Agent) -> list[tuple[str, str | None]]:
    """The tool results of a session: the text, and the code of the error if it is one."""
    return [
        (
            event.data.message.content[0].text,
            event.data.error.code if event.data.error else None,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


async def turn(agent: Agent, text: str = "Go on.") -> None:
    agent.followup(text)
    await idle(agent)


@pytest.fixture
def served() -> Served:
    return Served()


# The tools


async def test_the_tools_of_a_server_are_tools_of_the_program_once_it_is_connected(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    served.with_a_reader()
    asked = through(monkeypatch, notes=served.server)
    mcp = await listing(harness)

    await serving(harness)
    await asyncio.wait_for(mcp.ready(), 5)

    assert offered(harness) == [f"mcp__notes__{name}" for name in [*BASE, "peek"]]
    add = harness.tools.registry["mcp__notes__add"]
    peek = harness.tools.registry["mcp__notes__peek"]
    assert (add.description, add.effect, add.concurrency_safe) == ("Add two numbers.", "run", False)
    assert (peek.effect, peek.concurrency_safe) == ("read", True)
    assert add.schema.parameters["required"] == ["a", "b"]
    # As long as a connection and an answer may take, and a little more.
    assert add.timeout_seconds == 30 + 60 + 5
    server = mcp.servers["notes"]
    assert (server.state, server.offered, server.failure) == ("connected", 6, None)
    # The program is started as the config says, and what it writes is thrown away.
    assert asked == [("notes", (), {}, None, None)]


async def test_a_server_is_started_with_what_its_config_says(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    asked = through(monkeypatch, uvx=served.server)
    await listing(harness, log_dir=tmp_path / "logs")

    await serving(
        harness,
        "notes",
        command="uvx",
        args=["notes-mcp", "--quiet"],
        env={"NOTES_HOME": "/srv/notes"},
        cwd=tmp_path,
        startup_timeout_seconds=7,
    )
    await asyncio.wait_for(harness.ctx.use(MCP).ready(), 5)

    # A call may take as long as a connection and an answer may take, and a little more.
    assert harness.tools.registry["mcp__notes__add"].timeout_seconds == 7 + 60 + 5
    assert asked == [
        (
            "uvx",
            ("notes-mcp", "--quiet"),
            {"NOTES_HOME": "/srv/notes"},
            tmp_path,
            tmp_path / "logs" / "mcp-notes.log",
        )
    ]


async def test_a_server_at_an_address_is_reached_there_with_its_headers(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    asked: list[tuple[str, dict[str, str]]] = []

    def reached(url: str, headers: dict[str, str]) -> Any:
        asked.append((url, headers))
        return served.server

    monkeypatch.setattr(reach, "reached", reached)
    mcp = await listing(harness)

    await harness.ctx.plugin(
        server_plugin,
        {"server": "notes", "url": "https://host.example/mcp", "headers": {"X-Team": "alpha"}},
    )
    await asyncio.wait_for(mcp.ready(), 5)

    assert asked == [("https://host.example/mcp", {"X-Team": "alpha"})]
    assert len(offered(harness)) == len(BASE)


async def test_what_an_address_and_its_headers_hold_is_kept_out_of_why_a_server_failed(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def reached(url: str, headers: dict[str, str]) -> Any:
        raise ConnectionRefusedError(f"refused {url} with {headers['Authorization']}")

    monkeypatch.setattr(reach, "reached", reached)
    mcp = await listing(harness)

    await harness.ctx.plugin(
        server_plugin,
        {
            "server": "notes",
            "url": "https://host.example/mcp?key=s3cret-value",
            "headers": {"Authorization": "Bearer abcdefgh"},
        },
    )
    await asyncio.wait_for(mcp.ready(), 5)

    assert mcp.servers["notes"].failure == "refused https://host.example/mcp?[...] with [...]"


async def test_what_an_address_and_its_headers_hold_is_kept_out_of_what_a_library_logs(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    def reached(url: str, headers: dict[str, str]) -> Any:
        return served.server

    monkeypatch.setattr(reach, "reached", reached)
    mcp = await listing(harness)
    library = logging.getLogger("httpx2")
    written: list[str] = []

    class Kept(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            written.append(self.format(record))

    kept = Kept()
    library.addHandler(kept)
    try:
        fiber = harness.ctx.plugin(
            server_plugin,
            {
                "server": "notes",
                "url": "https://host.example/mcp?key=s3cret-value",
                "headers": {"Authorization": "Bearer abcdefgh"},
            },
        )
        await fiber
        await asyncio.wait_for(mcp.ready(), 5)

        # The SDK and the library that speaks HTTP log by themselves, and repeat where a
        # request went and what it carried. Their words are not written by this program,
        # and what lets somebody in is out of them all the same.
        library.warning("POST https://host.example/mcp?key=s3cret-value failed")
        library.error("refused the header %r", "Bearer abcdefgh")
        await fiber.dispose()
        # The server is gone, and so is what there was to keep out for it.
        library.warning("key=s3cret-value")
    finally:
        library.removeHandler(kept)

    assert written == [
        "POST https://host.example/mcp?[...] failed",
        "refused the header '[...]'",
        "key=s3cret-value",
    ]


async def test_a_server_from_before_2026_is_spoken_to_as_one_where_the_config_says_so(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=Cuttable(served))
    mcp = await listing(harness)
    await serving(harness, protocol="legacy")
    await asyncio.wait_for(mcp.ready(), 5)
    grow = harness.tools.registry["mcp__notes__grow_as_before"]

    # Such a server says that its tools changed in a way that only reaches a client which
    # speaks to it as to an older one.
    assert await grow.execute(JsonArguments({}), NO_RUN) == "grown"

    await until(lambda: "mcp__notes__more1" in offered(harness))


async def test_a_model_calls_a_tool_of_a_server_and_reads_what_it_gave(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=served.server)
    await listing(harness)
    await serving(harness)
    agent = await harness.agent(
        Reply(calls=(Call(name="mcp__notes__add", arguments={"a": 2, "b": 3}),)),
        Reply(text="Five."),
    )

    await turn(agent, "Add two and three.")

    assert results(agent) == [("5", None)]
    assert [tool.name for tool in harness.model.requests[0].tools] == offered(harness)


async def test_what_goes_wrong_with_a_call_is_a_failure_of_the_tool_that_the_model_reads(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=served.server)
    await listing(harness)
    await serving(harness, call_timeout_seconds=0.2)
    agent = await harness.agent(
        Reply(calls=(Call(name="mcp__notes__add", arguments={"a": "two"}),)),
        Reply(calls=(Call(name="mcp__notes__nap", arguments={"seconds": 3}),)),
        Reply(text="Neither worked."),
    )

    await turn(agent)

    (refused, refused_code), late = results(agent)
    # The server says in its own words that the arguments are of no use.
    assert refused_code == "TOOL_ERROR"
    assert "add" in refused
    assert late == (
        "the MCP server notes did not answer within 0.2 seconds. The call may have taken effect "
        "all the same",
        "TOOL_ERROR",
    )


async def test_only_the_tools_that_the_config_asks_for_are_offered(
    harness: Harness,
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    through(monkeypatch, notes=served.server)
    mcp = await listing(harness)

    with caplog.at_level(logging.WARNING, logger=SERVED):
        await serving(harness, tools=["nap", "add", "nowhere"])
        await asyncio.wait_for(mcp.ready(), 5)

    assert offered(harness) == ["mcp__notes__add", "mcp__notes__nap"]
    assert caplog.messages == [
        "the MCP server notes: the server lists no tool 'nowhere', which the config asks for"
    ]


async def test_tools_that_a_server_lists_anew_take_the_place_of_the_ones_before(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=served.server)
    mcp = await listing(harness)
    await serving(harness)
    await asyncio.wait_for(mcp.ready(), 5)
    grow = harness.tools.registry["mcp__notes__grow"]

    assert await grow.execute(JsonArguments({}), NO_RUN) == "grown"

    await until(lambda: offered(harness) == [f"mcp__notes__{name}" for name in [*BASE, "more1"]])
    assert mcp.servers["notes"].offered == len(BASE) + 1
    more = harness.tools.registry["mcp__notes__more1"]
    assert await more.execute(JsonArguments({}), NO_RUN) == "more1"


class Nothing(BaseModel):
    """The arguments of a tool that takes none."""


async def test_a_tool_whose_name_is_taken_is_passed_over_and_the_others_are_offered(
    harness: Harness,
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    async def nothing(arguments: Nothing, run: ToolRun) -> str:
        return "the one that was there"

    there = Tool(
        name="mcp__notes__add", description="There first.", parameters=Nothing, execute=nothing
    )
    harness.tools.add(harness.ctx, there)
    through(monkeypatch, notes=served.server)
    mcp = await listing(harness)

    with caplog.at_level(logging.WARNING, logger=SERVED):
        await serving(harness)
        await asyncio.wait_for(mcp.ready(), 5)

    assert caplog.messages == [
        "the MCP server notes: its tool mcp__notes__add is not offered, because there is a tool "
        "of that name already"
    ]
    assert harness.tools.registry["mcp__notes__add"] is there
    assert offered(harness) == [f"mcp__notes__{name}" for name in BASE]
    assert mcp.servers["notes"].offered == len(BASE) - 1


# What a server says of itself


async def test_what_the_servers_say_of_themselves_stands_in_the_system_prompt_by_their_names(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(
        monkeypatch,
        notes=served.server,
        archive=Served(instructions="Old notes live here.").server,
        mute=Served(instructions=None).server,
    )
    await harness.ctx.plugin(columbina.system_prompt)
    await listing(harness)
    for server in ("notes", "mute", "archive"):
        await serving(harness, server)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert harness.model.requests[0].messages[0] == system(
        under("archive", "Old notes live here."), under("notes", INSTRUCTIONS)
    )


async def test_a_server_that_says_too_much_of_itself_is_cut_short_and_stays_connected(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=served.server)
    await harness.ctx.plugin(columbina.system_prompt)
    mcp = await listing(harness)
    await serving(harness, max_instruction_bytes=10)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert harness.model.requests[0].messages[0] == system(
        under("notes", "Use the to\n[The server said more. It is cut here, after 10 bytes.]")
    )
    assert mcp.servers["notes"].state == "connected"


async def test_without_servers_a_system_prompt_is_what_it_was(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.system_prompt)
    await listing(harness)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert harness.model.requests[0].messages[0] == system()


# A turn waits for the servers


@pytest.mark.parametrize("with_a_system_prompt", [True, False], ids=["the prompt", "the step"])
async def test_a_turn_waits_for_a_server_that_is_still_being_connected(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch, with_a_system_prompt: bool
) -> None:
    transport = Late(served)
    through(monkeypatch, notes=transport)
    if with_a_system_prompt:
        await harness.ctx.plugin(columbina.system_prompt)
    mcp = await listing(harness)
    await serving(harness)
    agent = await harness.agent(Reply(text="Hello!"))

    agent.followup("Say hello.")
    await asyncio.sleep(0.1)

    # Nothing was asked of the model yet: the server may still offer its tools.
    assert harness.model.requests == []
    assert mcp.servers["notes"].state == "connecting"
    transport.there.set()
    await idle(agent)
    request = harness.model.requests[0]
    assert [tool.name for tool in request.tools] == [f"mcp__notes__{name}" for name in BASE]
    said = (under("notes", INSTRUCTIONS),) if with_a_system_prompt else ()
    assert request.messages[0] == system(*said)


async def test_a_server_that_cannot_be_reached_is_told_of_and_the_turn_goes_on_without_it(
    harness: Harness,
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    through(monkeypatch, notes=served.server, gone=ConnectionRefusedError("nobody answers"))
    await harness.ctx.plugin(columbina.system_prompt)
    mcp = await listing(harness)
    with caplog.at_level(logging.WARNING, logger="columbina.mcp.server.connection"):
        await serving(harness, "gone")
        await serving(harness, "notes")
        agent = await harness.agent(Reply(text="Hello!"))

        await turn(agent, "Say hello.")

    request = harness.model.requests[0]
    assert [tool.name for tool in request.tools] == [f"mcp__notes__{name}" for name in BASE]
    assert request.messages[0] == system(under("notes", INSTRUCTIONS))
    assert "the MCP server gone is not connected: nobody answers" in caplog.messages
    gone = mcp.servers["gone"]
    assert (gone.state, gone.offered, gone.failure) == ("connecting", 0, "nobody answers")


# A server that is required


async def test_a_server_that_is_required_and_cannot_be_reached_keeps_its_plugin_from_loading(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    through(monkeypatch, notes=ConnectionRefusedError("nobody answers"))
    mcp = await listing(harness)

    with pytest.raises(ServerError) as refused:
        await serving(harness, required=True)

    assert str(refused.value) == (
        "the MCP server notes is required, and it could not be reached: nobody answers"
    )
    assert "notes" not in mcp.servers


async def test_a_server_that_is_required_is_there_when_its_plugin_is_loaded(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    transport = Late(served)
    through(monkeypatch, notes=transport)
    await listing(harness)

    loading = asyncio.ensure_future(serving(harness, required=True))
    await asyncio.sleep(0.1)
    assert not loading.done()
    transport.there.set()
    await asyncio.wait_for(loading, 5)

    assert offered(harness) == [f"mcp__notes__{name}" for name in BASE]


# Without the SDK


def without(monkeypatch: pytest.MonkeyPatch, missing: str) -> None:
    """Make the plugin of a server import its parts anew, with one module that is not there."""
    for part in ("reach", "served", "connection", "offer", "content"):
        monkeypatch.delitem(sys.modules, f"columbina.mcp.server.{part}", raising=False)
        monkeypatch.delattr(server_plugin, part, raising=False)
    for loaded in [name for name in sys.modules if name == "mcp" or name.startswith("mcp.")]:
        monkeypatch.delitem(sys.modules, loaded)
    monkeypatch.setitem(sys.modules, missing, None)


async def test_where_the_sdk_is_not_installed_a_server_says_what_to_install(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    mcp = await listing(harness)
    without(monkeypatch, "mcp")

    with pytest.raises(ServerError) as refused:
        await serving(harness)

    assert str(refused.value) == (
        "the MCP server notes needs the SDK of the Model Context Protocol, which is not "
        "installed. Install it with: pip install columbina[mcp]"
    )
    assert refused.value.__cause__ is None
    assert "notes" not in mcp.servers


async def test_another_module_that_is_missing_is_not_taken_for_the_sdk(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    await listing(harness)
    without(monkeypatch, "columbina.mcp.server.offer")

    with pytest.raises(ModuleNotFoundError, match=r"columbina\.mcp\.server\.offer"):
        await serving(harness)


# A server that leaves


async def test_a_server_whose_plugin_is_unloaded_takes_its_tools_and_its_word_with_it(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    transport = Cuttable(served)
    through(monkeypatch, notes=transport)
    await harness.ctx.plugin(columbina.system_prompt)
    mcp = await listing(harness)
    fiber = await serving(harness)
    await asyncio.wait_for(mcp.ready(), 5)
    assert len(offered(harness)) == len(BASE)

    await fiber.dispose()

    assert offered(harness) == []
    assert "notes" not in mcp.servers
    assert (transport.entered, transport.left) == (1, 1)
    agent = await harness.agent(Reply(text="Hello!"))
    await turn(agent, "Say hello.")
    assert harness.model.requests[0].messages[0] == system()
    assert harness.model.requests[0].tools == ()


# The command


async def test_the_command_lists_the_servers_with_their_tools_and_their_failures(
    harness: Harness, served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    one = Served()
    one.server.remove_tool("nap")
    for name in ("grow", "grow_as_before", "stir"):
        one.server.remove_tool(name)
    through(
        monkeypatch,
        notes=served.server,
        calculator=one.server,
        gone=ConnectionRefusedError("nobody answers"),
    )
    mcp = await listing(harness)
    for server in ("notes", "gone", "calculator"):
        await serving(harness, server)
    await asyncio.wait_for(mcp.ready(), 5)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(mcp_command)
    commands = harness.ctx.use(COMMANDS)
    agent = await harness.agent()

    said = await commands.run(agent, "/mcp")

    assert said.split("\n") == [
        "calculator  connected  1 tool",
        "gone        connecting  0 tools  (nobody answers)",
        "notes       connected  5 tools",
    ]
    assert commands.registry["mcp"].summary == (
        "List the MCP servers: how each is connected, and how many tools it has."
    )


async def test_the_command_says_so_where_no_server_is_set_up(harness: Harness) -> None:
    await listing(harness)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(mcp_command)
    agent = await harness.agent()

    assert await harness.ctx.use(COMMANDS).run(agent, "/mcp") == "No MCP server is set up."


# What a server that is started writes to its standard error


@pytest.fixture
def writing(monkeypatch: pytest.MonkeyPatch) -> list[Any]:
    """A program in the place of one that is started: it writes a line, and that is all of it."""
    begun: list[Any] = []

    @asynccontextmanager
    async def stdio_client(parameters: Any, errlog: Any) -> AsyncGenerator[tuple[str, str]]:
        # What it was started with, and the file that it writes into.
        begun.append((parameters, errlog))
        errlog.write("the server is up\n")
        errlog.flush()
        yield ("what it says", "what it is told")

    monkeypatch.setattr(reach, "stdio_client", stdio_client)
    return begun


async def test_what_a_server_writes_to_its_standard_error_goes_into_its_file(
    writing: list[Any], tmp_path: Path
) -> None:
    log = tmp_path / "logs" / "mcp-notes.log"

    for _ in range(2):
        way: Any = reach.started("uvx", ("notes-mcp",), {"A": "1"}, tmp_path, log)
        async with way as streams:
            assert streams == ("what it says", "what it is told")

    # What an earlier run wrote stays.
    assert log.read_text(encoding="utf-8") == "the server is up\nthe server is up\n"
    ((first, file), (_, again)) = writing
    assert (first.command, first.args, first.env, first.cwd) == (
        "uvx",
        ["notes-mcp"],
        {"A": "1"},
        tmp_path,
    )
    # The file is closed once the server is left.
    assert (file.closed, again.closed) == (True, True)


async def test_a_file_that_grew_beyond_what_anybody_reads_is_begun_anew(
    writing: list[Any], tmp_path: Path
) -> None:
    grown = tmp_path / "mcp-grown.log"
    grown.write_bytes(b"x" * (1024 * 1024 + 1))
    full = tmp_path / "mcp-full.log"
    full.write_bytes(b"x" * (1024 * 1024))

    for log in (grown, full):
        way: Any = reach.started("uvx", (), {}, None, log)
        async with way:
            pass

    assert grown.read_text(encoding="utf-8") == "the server is up\n"
    assert full.read_text(encoding="utf-8") == "x" * (1024 * 1024) + "the server is up\n"


async def test_without_a_place_for_it_what_a_server_writes_is_thrown_away(
    writing: list[Any], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)

    way: Any = reach.started("uvx", (), {}, None, None)
    async with way:
        pass

    ((started, _),) = writing
    # A server that is given no variables gets the few that every server gets, and no others.
    assert started.env is None
    assert list(tmp_path.iterdir()) == []


# A server at an address


async def test_a_server_at_an_address_is_talked_to_with_its_headers_and_within_limits(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    made: list[dict[str, Any]] = []
    asked: list[tuple[str, Any]] = []
    left: list[str] = []

    class Http:
        def __init__(self, **options: Any) -> None:
            made.append(options)

        async def __aenter__(self) -> "Http":
            return self

        async def __aexit__(self, *why: object) -> None:
            left.append("the client")

    @asynccontextmanager
    async def over_http(url: str, *, http_client: Any) -> AsyncGenerator[tuple[str, str]]:
        asked.append((url, http_client))
        try:
            yield ("what it says", "what it is told")
        finally:
            left.append("the stream")

    monkeypatch.setattr(httpx2, "AsyncClient", Http)
    monkeypatch.setattr(reach, "streamable_http_client", over_http)

    way: Any = reach.reached("https://host.example/mcp", {"Authorization": "Bearer abcdefgh"})
    async with way as streams:
        assert streams == ("what it says", "what it is told")

    (options,) = made
    assert options["headers"] == {"Authorization": "Bearer abcdefgh"}
    limits = options["timeout"]
    # As long as the SDK itself gives a server: it may keep a stream open for minutes.
    assert (limits.connect, limits.read, limits.write, limits.pool) == (30.0, 300.0, 30.0, 30.0)
    ((url, client),) = asked
    assert (url, type(client)) == ("https://host.example/mcp", Http)
    assert left == ["the stream", "the client"]


# A call that a server at an address turns away, because it knows nothing of the session

CALL = b'{"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "add"}}'


def standing_in(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """Have a server at an address reached without HTTP, and return how its clients are made."""
    made: list[dict[str, Any]] = []

    class Http:
        def __init__(self, **options: Any) -> None:
            made.append(options)

        async def __aenter__(self) -> "Http":
            return self

        async def __aexit__(self, *why: object) -> None:
            pass

    @asynccontextmanager
    async def over_http(url: str, *, http_client: Any) -> AsyncGenerator[tuple[str, str]]:
        yield ("what it says", "what it is told")

    monkeypatch.setattr(httpx2, "AsyncClient", Http)
    monkeypatch.setattr(reach, "streamable_http_client", over_http)
    return made


async def looking(monkeypatch: pytest.MonkeyPatch) -> Any:
    """Return what a server at an address has look at every answer that it gets."""
    made = standing_in(monkeypatch)
    way: Any = reach.reached("https://host.example/mcp", {})
    async with way:
        pass
    (options,) = made
    told: list[str] = []
    monkeypatch.setattr(reach, "turned_away", lambda: told.append("turned away"))
    (looks,) = options["event_hooks"]["response"]
    return looks, told


def answer(status: int, *, session: str | None = "abc", asks: Any = CALL) -> httpx2.Response:
    """What a server answers to a request, with the request that it answers."""
    headers = {} if session is None else {"mcp-session-id": session}
    request = httpx2.Request("POST", "https://host.example/mcp", headers=headers, content=asks)
    return httpx2.Response(status, request=request)


async def in_pieces() -> AsyncGenerator[bytes]:
    """What a request holds that is sent piece by piece."""
    yield CALL


async def test_a_call_that_a_server_turns_away_for_want_of_its_session_is_told_so(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    looks, told = await looking(monkeypatch)

    await looks(answer(404))

    assert told == ["turned away"]


async def test_a_call_that_was_sent_on_before_it_was_turned_away_is_told_so_as_well(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    looks, told = await looking(monkeypatch)
    # A server sends a request on to another path, and the request that goes there is made
    # from the first one: with what that one held, which is not kept with it a second time.
    sent_on = httpx2.Request(
        "POST",
        "https://host.example/mcp/",
        headers={"mcp-session-id": "abc"},
        stream=httpx2.ByteStream(CALL),
    )
    with pytest.raises(httpx2.RequestNotRead):
        _ = sent_on.content

    await looks(httpx2.Response(404, request=sent_on))

    assert told == ["turned away"]


async def test_a_request_that_went_to_the_server_piece_by_piece_turns_no_call_away(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    looks, told = await looking(monkeypatch)
    answered = answer(404, asks=in_pieces())
    # The pieces went to the server, and are not there to be read a second time.
    async for _ in cast("AsyncGenerator[bytes]", answered.request.stream):
        pass

    # Nothing is known of what it asked for, and whoever looks at its answer goes on.
    await looks(answered)

    assert told == []


@pytest.mark.parametrize(
    "answered",
    [
        pytest.param(lambda: answer(200), id="an answer that turns nothing away"),
        pytest.param(lambda: answer(400), id="another refusal"),
        pytest.param(lambda: answer(500), id="a failure of the server"),
        pytest.param(lambda: answer(404, session=None), id="a request that names no session"),
        pytest.param(
            lambda: answer(404, asks=b'{"jsonrpc": "2.0", "id": 8, "method": "tools/list"}'),
            id="a request that is no call",
        ),
        pytest.param(lambda: answer(404, asks=b"no request"), id="what is no request at all"),
        pytest.param(
            lambda: answer(404, asks=b'[{"method": "tools/call"}]'), id="requests in a row"
        ),
    ],
)
async def test_what_else_a_server_answers_turns_no_call_away(
    monkeypatch: pytest.MonkeyPatch, answered: Any
) -> None:
    looks, told = await looking(monkeypatch)

    await looks(answered())

    assert told == []


async def test_that_a_session_which_was_gone_could_not_be_ended_is_not_logged(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    standing_in(monkeypatch)
    sdk = logging.getLogger("mcp.client.streamable_http")
    gone = "Session termination failed: 404"
    other = "Session termination failed: 500"
    way: Any = reach.reached("https://host.example/mcp", {})
    again: Any = reach.reached("https://host.example/mcp", {})

    with caplog.at_level(logging.WARNING):
        async with way:
            async with again:
                # What the SDK says where it leaves a session that the server had ended.
                sdk.warning(gone)
                sdk.warning(other)
            # A server at an address that is still entered keeps it out as well.
            sdk.warning(gone)
        # Nothing is entered any more, and the SDK is heard as it speaks.
        sdk.warning(gone)

    assert caplog.messages == [other, gone]
