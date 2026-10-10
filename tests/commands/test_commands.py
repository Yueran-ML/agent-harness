"""Commands: what a person tells the program itself to do, by a line that begins with a slash."""

import asyncio
import logging

import pytest

import columbina.commands
from columbina.agent import Agent
from columbina.commands import COMMANDS, Command, Commands, is_command
from columbina.kernel import Context, Plugin, RegistryConflictError
from support.loop import Harness


async def commands(harness: Harness) -> Commands:
    await harness.ctx.plugin(columbina.commands)
    return harness.ctx.use(COMMANDS)


def saying(heard: list[tuple[str, str]]) -> Command:
    """A command that writes down who it was given to and what stood after its name."""

    async def run(agent: Agent, rest: str) -> str:
        heard.append((agent.id, rest))
        return f"heard {rest!r}"

    return Command(run, "Write down what was said.")


@pytest.mark.parametrize(
    ("line", "command"),
    [
        ("/help", True),
        ("/model deepseek-flash", True),
        ("/", True),
        ("", False),
        ("help", False),
        (" /help", False),
        ("What does /etc/hosts hold?", False),
    ],
)
def test_a_line_that_begins_with_a_slash_is_a_command(line: str, command: bool) -> None:
    assert is_command(line) is command


@pytest.mark.parametrize(
    ("line", "rest"),
    [
        ("/say", ""),
        ("/say  ", ""),
        ("/say one two", "one two"),
        ("/say   kept  as it is ", "kept  as it is"),
        ("/SAY loud", "loud"),
    ],
)
async def test_what_stands_after_the_name_is_handed_to_the_command(
    harness: Harness, line: str, rest: str
) -> None:
    heard: list[tuple[str, str]] = []
    known = await commands(harness)
    known.registry.add(harness.ctx, "say", saying(heard))
    agent = await harness.agent()

    shown = await known.run(agent, line)

    assert heard == [("s-1", rest)]
    assert shown == f"heard {rest!r}"


@pytest.mark.parametrize("line", ["/nowhere", "/nowhere at all", "/", "/ help", "/say-more"])
async def test_a_command_that_is_not_there_is_answered_with_a_hint(
    harness: Harness, line: str
) -> None:
    heard: list[tuple[str, str]] = []
    known = await commands(harness)
    known.registry.add(harness.ctx, "say", saying(heard))
    agent = await harness.agent()

    shown = await known.run(agent, line)

    name = line.removeprefix("/").split(" ")[0]
    assert shown == f"There is no command /{name}. /help lists the commands."
    assert heard == []


async def test_a_line_that_is_no_command_is_not_run(harness: Harness) -> None:
    known = await commands(harness)
    agent = await harness.agent()

    with pytest.raises(ValueError, match="is not a command"):
        await known.run(agent, "help")


async def test_a_command_that_fails_says_so_and_the_program_goes_on(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    async def broken(agent: Agent, rest: str) -> str:
        raise RuntimeError("the disk is full")

    known = await commands(harness)
    known.registry.add(harness.ctx, "break", Command(broken, "Fail."))
    agent = await harness.agent()

    with caplog.at_level(logging.ERROR, logger="columbina.commands"):
        shown = await known.run(agent, "/break now")

    assert shown == "The command /break failed: RuntimeError: the disk is full"
    assert [record.getMessage() for record in caplog.records] == ["the command /break failed"]
    assert caplog.records[0].exc_info is not None


async def test_a_command_whose_work_was_stopped_says_so_and_the_program_goes_on(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    # What a command waits for may be stopped: the agent, when a person presses Escape.
    # Nobody stopped whoever carries the command out.
    async def waits_for_what_is_stopped(agent: Agent, rest: str) -> str:
        stopped: asyncio.Future[str] = asyncio.get_running_loop().create_future()
        stopped.cancel()
        return await stopped

    known = await commands(harness)
    known.registry.add(harness.ctx, "wait", Command(waits_for_what_is_stopped, "Wait."))
    agent = await harness.agent()

    with caplog.at_level(logging.ERROR, logger="columbina.commands"):
        shown = await known.run(agent, "/WAIT now")

    assert shown == "The command /wait was stopped."
    assert caplog.records == []


async def test_a_command_that_is_cancelled_is_cancelled(harness: Harness) -> None:
    started = asyncio.Event()

    async def waiting(agent: Agent, rest: str) -> str:
        started.set()
        await asyncio.Event().wait()
        return "never"

    known = await commands(harness)
    known.registry.add(harness.ctx, "wait", Command(waiting, "Wait."))
    agent = await harness.agent()

    running = asyncio.ensure_future(known.run(agent, "/wait"))
    await asyncio.wait_for(started.wait(), timeout=5)
    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await running


async def test_help_lists_the_commands_by_their_names(harness: Harness) -> None:
    known = await commands(harness)
    known.registry.add(harness.ctx, "tools", saying([]))
    known.registry.add(harness.ctx, "exit", Command(saying([]).run, "Leave."))
    agent = await harness.agent()

    shown = await known.run(agent, "/help")

    assert shown == (
        "/exit   Leave.\n"
        "/help   List the commands.\n"
        "/tools  Write down what was said.\n"
        "A line that does not begin with a slash goes to the agent."
    )


async def test_a_command_leaves_with_the_plugin_that_brought_it(harness: Harness) -> None:
    def brings(ctx: Context, config: object) -> None:
        ctx.use(COMMANDS).registry.add(ctx, "say", saying([]))

    known = await commands(harness)
    plugin = await harness.ctx.plugin(Plugin("brings", brings, inject=(COMMANDS,)))
    agent = await harness.agent()
    assert (await known.run(agent, "/say x")) == "heard 'x'"

    await plugin.dispose()

    assert (await known.run(agent, "/say x")) == (
        "There is no command /say. /help lists the commands."
    )


async def test_two_commands_cannot_have_one_name(harness: Harness) -> None:
    known = await commands(harness)

    with pytest.raises(RegistryConflictError, match="the command 'help' is already registered"):
        known.registry.add(harness.ctx, "help", saying([]))


async def test_the_commands_leave_with_their_plugin(harness: Harness) -> None:
    loaded = await harness.ctx.plugin(columbina.commands)

    await loaded.dispose()

    assert harness.ctx.get(COMMANDS) is None
