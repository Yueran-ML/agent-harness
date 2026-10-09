"""The facts about where an agent runs: gathered from the plugins that know them."""

from collections.abc import Sequence
from pathlib import Path

import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina.agent import RUNTIME_FACTS, Agent, Facts
from columbina.approval import APPROVAL
from columbina.builtin_tools import shell
from columbina.context import runtime
from columbina.llm.adapters.scripted import Reply
from columbina.sandbox import SANDBOX, Mode, Strength
from columbina.session import UserMessageEvent
from support.loop import SYSTEM, Harness, context, idle, user

HELD = "commands are held: not at all. A command can do whatever the user of this machine can do."
ASK = "approval: ask. A call that needs a yes is put to the user, if somebody is there to ask."
WRITE = (
    "sandbox mode: workspace-write. Files in the working directory can be written, except in "
    ".git and .columbina. Outside it no file can be read or written."
)


def facts(agent: Agent) -> list[list[str]]:
    """The facts that the model was told in a session, as the lines of each telling."""
    return [
        event.data.message.content[0].text.splitlines()[1:-1]
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent) and event.data.source == "context/runtime"
    ]


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


async def test_a_model_is_told_where_it_works(harness: Harness) -> None:
    await harness.ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        context("runtime", f"working directory: {harness.cwd}"),
        user("Say hello."),
    )


async def test_without_the_plugin_no_facts_are_told(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert facts(agent) == []


async def test_every_plugin_that_knows_a_fact_adds_it(harness: Harness) -> None:
    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    await ctx.plugin(columbina.permissions)
    await ctx.plugin(shell)
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    kinds = {
        "powershell": "the tool powershell, which is Windows PowerShell 5.1 and not bash",
        "bash": "the tool bash",
    }
    by = f"commands are run by: {kinds[shell.NAME]}"
    assert facts(agent) == [[ASK, HELD, by, WRITE, f"working directory: {harness.cwd}"]]


async def test_facts_are_told_again_when_one_of_them_has_changed_and_only_then(
    harness: Harness,
) -> None:
    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), Reply(text="Three."))
    where = f"working directory: {harness.cwd}"

    await turn(agent, "First.")
    await turn(agent, "Second.")
    ctx.use(SANDBOX).set_mode(agent.session, "read-only")
    await turn(agent, "Third.")

    assert facts(agent) == [
        [HELD, WRITE, where],
        [HELD, "sandbox mode: read-only. No file can be written.", where],
    ]
    # The new facts follow the conversation. What came before them stays as it was.
    assert harness.model.requests[2].messages[:3] == harness.model.requests[0].messages[:3]
    assert harness.model.requests[2].messages[-2] == context(
        "runtime", "\n".join([HELD, "sandbox mode: read-only. No file can be written.", where])
    )


async def test_a_plugin_that_leaves_takes_its_facts_with_it(harness: Harness) -> None:
    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    sandbox = await ctx.plugin(columbina.sandbox)
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    where = f"working directory: {harness.cwd}"
    await turn(agent, "First.")

    await sandbox.dispose()
    await turn(agent, "Second.")

    assert facts(agent) == [[HELD, WRITE, where], [where]]


@pytest.mark.parametrize(
    ("mode", "told"),
    [
        ("read-only", "sandbox mode: read-only. No file can be written."),
        ("workspace-write", WRITE),
        (
            "danger-full-access",
            "sandbox mode: danger-full-access. Files anywhere can be read and written.",
        ),
    ],
)
async def test_the_mode_of_the_sandbox_is_told_with_what_it_means(
    harness: Harness, mode: Mode, told: str
) -> None:
    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, {"mode": mode})
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert told in facts(agent)[0]


@pytest.mark.parametrize(
    ("strength", "told"),
    [
        ("partial", "in part. A command may still reach more than the mode of the sandbox allows."),
        ("full", "fully. A command can do no more than the mode of the sandbox allows."),
    ],
)
async def test_how_well_commands_are_held_is_told_by_what_holds_them(
    harness: Harness, strength: Strength, told: str
) -> None:
    class Holding:
        def __init__(self) -> None:
            self.strength: Strength = strength

        def confine(self, argv: Sequence[str], *, mode: Mode, cwd: Path) -> Sequence[str]:
            return argv

    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, {"provider": "holding"})
    ctx.use(SANDBOX).providers.add(ctx, "holding", Holding())
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert f"commands are held: {told}" in facts(agent)[0]


async def test_a_session_in_which_nobody_is_asked_is_told_so(harness: Harness) -> None:
    ctx = harness.ctx
    await ctx.plugin(columbina.approval)
    await ctx.plugin(runtime)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    await turn(agent, "First.")

    ctx.use(APPROVAL).set_policy(agent.session, "never")
    await turn(agent, "Second.")

    assert [told[0] for told in facts(agent)] == [
        ASK,
        "approval: never. A call that needs a yes is refused without anybody being asked.",
    ]


async def test_a_listener_adds_a_fact_under_a_name_and_facts_come_by_their_names(
    harness: Harness,
) -> None:
    def weather(gathered: Facts) -> None:
        gathered.lines["weather"] = "rain"
        gathered.lines["a first"] = f"for {gathered.agent.id}"

    await harness.ctx.plugin(runtime)
    harness.ctx.on(RUNTIME_FACTS, weather)
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent, "Say hello.")

    assert facts(agent) == [
        ["a first: for s-1", "weather: rain", f"working directory: {harness.cwd}"]
    ]
