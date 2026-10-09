"""The commands that come with the program: the tools, the permissions, the sessions."""

import os
import time
from pathlib import Path

import pytest
from pydantic import BaseModel

import columbina.approval
import columbina.commands
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina.approval import APPROVAL
from columbina.commands import COMMANDS, Commands
from columbina.commands import permissions as permissions_command
from columbina.commands import sessions as sessions_command
from columbina.commands import tools as tools_command
from columbina.llm.adapters.scripted import Reply
from columbina.permissions import PERMISSIONS
from columbina.sandbox import SANDBOX
from columbina.session import TurnStartEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE
from columbina.tools import Tool, ToolRun
from support.loop import Harness, idle, user


class Nothing(BaseModel):
    pass


async def nothing(args: Nothing, run: ToolRun) -> str:
    return ""


async def commands(harness: Harness, *plugins: object) -> Commands:
    await harness.ctx.plugin(columbina.commands)
    for plugin in plugins:
        await harness.ctx.plugin(plugin)
    return harness.ctx.use(COMMANDS)


# The tools


async def test_the_tools_are_listed_with_what_a_call_of_each_does(harness: Harness) -> None:
    harness.tools.add(
        harness.ctx,
        Tool(
            name="read",
            description="Read a file.\nMore about it.",
            parameters=Nothing,
            execute=nothing,
            effect="read",
        ),
        Tool(name="note", description="Take a note.", parameters=Nothing, execute=nothing),
        Tool(
            name="write", description="Write.", parameters=Nothing, execute=nothing, effect="write"
        ),
        Tool(name="bash", description="Run.", parameters=Nothing, execute=nothing, effect="run"),
    )
    known = await commands(harness, tools_command)
    agent = await harness.agent()

    assert await known.run(agent, "/tools") == (
        "read   (reads files)  Read a file.\n"
        "note   (changes nothing)  Take a note.\n"
        "write  (writes files)  Write.\n"
        "bash   (runs commands)  Run."
    )


async def test_a_program_without_tools_says_so(harness: Harness) -> None:
    known = await commands(harness, tools_command)
    agent = await harness.agent()

    assert await known.run(agent, "/tools") == "There are no tools."


# The permissions


async def with_permissions(harness: Harness, **config: object) -> Commands:
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await harness.ctx.plugin(plugin)
    await harness.ctx.plugin(columbina.permissions, config)
    return await commands(harness, permissions_command)


async def test_what_a_session_may_do_is_shown(harness: Harness) -> None:
    known = await with_permissions(harness, deny=["read(.env)"], allow=["run(git status*)", "grep"])
    agent = await harness.agent()

    assert await known.run(agent, "/permissions") == (
        "sandbox mode: workspace-write\n"
        "approval: ask\n"
        "deny: read(.env)\n"
        "ask: (none)\n"
        "allow: run(git status*), grep\n"
        "To change the mode and the policy: /permissions read-only | workspace-write | full-access"
    )


async def test_the_rules_of_the_project_are_among_those_that_are_shown(harness: Harness) -> None:
    (harness.cwd / ".columbina").mkdir()
    (harness.cwd / ".columbina" / "permissions.yml").write_text(
        "ask:\n  - write(docs)\n", encoding="utf-8"
    )
    known = await with_permissions(harness)
    agent = await harness.agent()

    assert "ask: write(docs)\n" in await known.run(agent, "/permissions")


async def test_rules_that_cannot_be_read_are_said_to_be_unreadable(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    (harness.cwd / ".columbina").mkdir()
    (harness.cwd / ".columbina" / "permissions.yml").write_text("deny: [", encoding="utf-8")
    known = await with_permissions(harness)
    agent = await harness.agent()

    shown = await known.run(agent, "/permissions")

    assert "The rules of this project cannot be read, so nothing is allowed.\n" in shown
    assert "deny:" not in shown


async def test_what_was_allowed_for_the_session_is_shown(harness: Harness) -> None:
    harness.tools.add(
        harness.ctx, Tool(name="note", description="Note.", parameters=Nothing, execute=nothing)
    )
    known = await with_permissions(harness)
    agent = await harness.agent()
    granted = harness.ctx.use(PERMISSIONS)
    granted.grant(agent.session, "note", None)

    shown = await known.run(agent, "/permissions")

    assert "allowed for the rest of this session: note\n" in shown


@pytest.mark.parametrize(
    ("preset", "mode", "policy"),
    [
        ("read-only", "read-only", "ask"),
        ("workspace-write", "workspace-write", "ask"),
        ("full-access", "danger-full-access", "never"),
    ],
)
async def test_a_preset_changes_the_mode_and_the_policy_of_the_session(
    harness: Harness, preset: str, mode: str, policy: str
) -> None:
    known = await with_permissions(harness, preset="read-only")
    agent = await harness.agent()

    shown = await known.run(agent, f"/permissions {preset}")

    assert shown == (
        f"sandbox mode: {mode}\napproval: {policy}\nThe agent is told when its next turn begins."
    )
    assert harness.ctx.use(SANDBOX).mode(agent.session) == mode
    assert harness.ctx.use(APPROVAL).policy(agent.session) == policy


async def test_a_preset_that_is_not_there_changes_nothing(harness: Harness) -> None:
    known = await with_permissions(harness)
    agent = await harness.agent()

    shown = await known.run(agent, "/permissions everything")

    assert shown == (
        "There is no preset 'everything'. The presets are: read-only, workspace-write, full-access."
    )
    assert harness.ctx.use(SANDBOX).mode(agent.session) == "workspace-write"
    with pytest.raises(ValueError, match="there is no preset 'everything'"):
        harness.ctx.use(PERMISSIONS).set_preset(agent.session, "everything")


# The sessions


async def kept(harness: Harness, session_id: str, said: str | None, at: int) -> None:
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id=session_id)
    session.append(TurnStartEvent(turn=1))
    if said is not None:
        session.append(UserMessageEvent(message=user(said)))
    await session.flush()
    session.close()
    file = harness.sessions.directory(harness.cwd, session_id) / SESSION_FILE
    os.utime(file, ns=(at * 1_000_000_000, at * 1_000_000_000))


def clock(second: int) -> str:
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(second))


async def test_the_sessions_of_the_directory_are_listed_and_the_open_one_is_marked(
    harness: Harness,
) -> None:
    await kept(harness, "older", "Fix the parser.", 1_700_000_000)
    await kept(harness, "silent", None, 1_700_003_600)
    known = await commands(harness, sessions_command)
    agent = await harness.agent(Reply(text="Hello."), session_id="now")
    agent.followup("Say hello.")
    await idle(agent)
    await agent.session.flush()
    file = harness.sessions.directory(harness.cwd, "now") / SESSION_FILE
    os.utime(file, ns=(1_700_007_200 * 1_000_000_000, 1_700_007_200 * 1_000_000_000))

    assert await known.run(agent, "/sessions") == (
        f"* now     {clock(1_700_007_200)}  Say hello.\n"
        f"  silent  {clock(1_700_003_600)}  (nothing said yet)\n"
        f"  older   {clock(1_700_000_000)}  Fix the parser.\n"
        "The one with the star is open. Another one is taken up with --session-id."
    )


async def test_a_long_list_of_sessions_is_shown_by_its_beginning(harness: Harness) -> None:
    for number in range(23):
        await kept(harness, f"s-{number:02}", f"Task {number}.", 1_700_000_000 + number * 60)
    known = await commands(harness, sessions_command)
    agent = await harness.agent(session_id="now")

    shown = (await known.run(agent, "/sessions")).splitlines()

    assert len(shown) == 22
    assert shown[0].startswith("  s-22  ")
    assert shown[19].startswith("  s-03  ")
    assert shown[20] == "... and 3 older ones"


async def test_a_directory_without_stored_sessions_says_so(
    harness: Harness, tmp_path: Path
) -> None:
    known = await commands(harness, sessions_command)
    harness.sessions.root = None
    agent = await harness.agent()

    assert await known.run(agent, "/sessions") == "No session of this directory is stored."
