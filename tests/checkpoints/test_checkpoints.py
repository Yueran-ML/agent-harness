"""Checkpoints: the files of the working directory as they stood before each turn."""

import logging
import os
import shutil
from pathlib import Path

import pytest
from pydantic import BaseModel, JsonValue

import columbina.checkpoints
import columbina.commands
import columbina.session
from columbina.agent import Agent
from columbina.checkpoints import (
    CHECKPOINTS,
    CheckpointError,
    CheckpointEvent,
    Checkpoints,
    Config,
    checkpoints_of,
)
from columbina.commands import COMMANDS, SWITCH, Commands, Opener
from columbina.commands import rewind as rewind_command
from columbina.kernel import Context
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import TurnStartEvent
from columbina.tools import Tool, ToolRun
from support.loop import Harness, idle

pytestmark = pytest.mark.skipif(shutil.which("git") is None, reason="git is not here")


class Change(BaseModel):
    path: str
    text: str | None = None


async def changed(args: Change, run: ToolRun) -> str:
    """Write a file, or take it away where no text is given."""
    place = run.cwd / args.path
    if args.text is None:
        place.unlink()
    else:
        place.parent.mkdir(parents=True, exist_ok=True)
        place.write_text(args.text, encoding="utf-8")
    return "done"


def change(path: str, text: str | None = None) -> Reply:
    arguments: dict[str, JsonValue] = {"path": path}
    if text is not None:
        arguments["text"] = text
    return Reply(calls=(Call(name="change", arguments=arguments),))


async def loaded(harness: Harness, tmp_path: Path) -> Commands:
    harness.tools.add(
        harness.ctx,
        Tool(name="change", description="Change.", parameters=Change, execute=changed),
    )
    await harness.ctx.plugin(columbina.checkpoints, {"home": str(tmp_path / "checkpoints")})
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(rewind_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


def files(work: Path) -> dict[str, str]:
    return {
        path.relative_to(work).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(work.rglob("*"))
        if path.is_file()
    }


async def three_turns(harness: Harness, tmp_path: Path) -> tuple[Commands, Agent]:
    """A conversation in which a file is made, changed, and another made and one taken away."""
    commands = await loaded(harness, tmp_path)
    work = harness.cwd
    (work / "keep.txt").write_text("kept\n", encoding="utf-8")
    (work / ".gitignore").write_text("ignored.txt\n", encoding="utf-8")
    (work / "ignored.txt").write_text("not kept\n", encoding="utf-8")
    agent = await harness.agent(
        change("a.txt", "one\n"),
        Reply(text="Made a."),
        change("a.txt", "two\n"),
        Reply(text="Changed a."),
        change("keep.txt"),
        change("sub/b.txt", "bee\n"),
        Reply(text="Took keep away, made b."),
    )
    await turn(agent, "Make a.")
    await turn(agent, "Change a.")
    await turn(agent, "Take keep away and make b.")
    return commands, agent


async def test_the_files_are_kept_before_each_turn(harness: Harness, tmp_path: Path) -> None:
    _, agent = await three_turns(harness, tmp_path)

    kept = checkpoints_of(agent.session)

    assert [checkpoint.turn for checkpoint in kept] == [1, 2, 3]
    assert all(len(checkpoint.commit) == 40 for checkpoint in kept)
    assert not any(checkpoint.before_going_back for checkpoint in kept)
    # The repository of the project is not touched: there is none.
    assert not (harness.cwd / ".git").exists()


async def test_the_files_go_back_to_before_a_turn_and_back_again(
    harness: Harness, tmp_path: Path
) -> None:
    commands, agent = await three_turns(harness, tmp_path)
    work = harness.cwd
    (work / "ignored.txt").write_text("changed, and not kept\n", encoding="utf-8")
    now = files(work)

    said = await commands.run(agent, "/rewind 2 files")

    assert said == "3 files were put back as they were before turn 2: a.txt, keep.txt, sub/b.txt."
    assert files(work) == {
        ".gitignore": "ignored.txt\n",
        "a.txt": "one\n",
        "ignored.txt": "changed, and not kept\n",
        "keep.txt": "kept\n",
    }
    assert checkpoints_of(agent.session)[-1] == CheckpointEvent(
        turn=3, commit=checkpoints_of(agent.session)[-1].commit, before_going_back=True
    )
    assert await commands.run(agent, "/rewind back") == (
        "The files are as they were before they were last put back."
    )
    assert files(work) == now


async def test_a_turn_without_changes_since_puts_nothing_back(
    harness: Harness, tmp_path: Path
) -> None:
    commands, agent = await three_turns(harness, tmp_path)
    await commands.run(agent, "/rewind 3 files")

    assert await commands.run(agent, "/rewind 3 files") == (
        "The files are as they were before turn 3: nothing was put back."
    )


async def test_the_turns_are_listed_with_what_was_said(harness: Harness, tmp_path: Path) -> None:
    commands, agent = await three_turns(harness, tmp_path)

    listed = (await commands.run(agent, "/rewind")).split("\n")

    assert listed[0] == "The turns that can be gone back to, the last ones last:"
    assert [line.split()[0] for line in listed[1:4]] == ["1", "2", "3"]
    assert listed[1].endswith("Make a.")
    assert listed[3].endswith("Take keep away and make b.")
    assert listed[-1] == "/rewind N goes back to before turn N."


async def test_the_conversation_goes_back_in_a_copy(harness: Harness, tmp_path: Path) -> None:
    commands, agent = await three_turns(harness, tmp_path)
    copies: list[Agent] = []

    async def switch(make: Opener) -> Agent:
        copy = await make(harness.ctx, agent.spec)
        copies.append(copy)
        return copy

    harness.ctx.provide(SWITCH, switch)

    said = await commands.run(agent, "/rewind 2 conversation")

    (copy,) = copies
    assert said == (
        f"The conversation goes on from before turn 2, in a copy of the session {agent.id}: "
        f"{copy.id}. The session {agent.id} stays as it was.\n"
        "What was said in turn 2: Change a."
    )
    said_in_the_copy = " ".join(
        getattr(block, "text", "") for message in copy.session.messages for block in message.content
    )
    assert "Change a." not in said_in_the_copy
    assert "Make a." in said_in_the_copy
    assert await commands.run(agent, "/rewind 1 conversation") == (
        "Nothing stands before turn 1. /new begins another conversation."
    )


async def test_without_a_way_to_another_session_the_conversation_stays(
    harness: Harness, tmp_path: Path
) -> None:
    commands, agent = await three_turns(harness, tmp_path)

    said = await commands.run(agent, "/rewind 3")

    assert said.split("\n") == [
        "2 files were put back as they were before turn 3: keep.txt, sub/b.txt.",
        "This program cannot go over to another session, so the conversation stays.",
    ]


@pytest.mark.parametrize("words", ["x", "2 nothing", "2 files more", "-1"])
async def test_what_cannot_be_done_is_said(harness: Harness, tmp_path: Path, words: str) -> None:
    commands, agent = await three_turns(harness, tmp_path)

    assert (await commands.run(agent, f"/rewind {words}")).startswith("Say /rewind to list")


async def test_a_turn_without_a_checkpoint_and_going_back_that_never_was(
    harness: Harness, tmp_path: Path
) -> None:
    commands, agent = await three_turns(harness, tmp_path)

    assert await commands.run(agent, "/rewind 9") == "No checkpoint was kept before turn 9."
    assert await commands.run(agent, "/rewind back") == (
        "The files were never put back in this conversation."
    )


async def test_a_conversation_without_checkpoints_has_nothing_to_go_back_to(
    harness: Harness, tmp_path: Path
) -> None:
    commands = await loaded(harness, tmp_path)
    agent = await harness.agent()

    assert await commands.run(agent, "/rewind") == (
        "No checkpoint was kept in this conversation, so there is nothing to go back to."
    )


async def test_a_subagent_keeps_nothing(harness: Harness, tmp_path: Path) -> None:
    await loaded(harness, tmp_path)
    harness.model.add(Reply(text="Done."))
    agent = await harness.agents.create(
        harness.ctx, harness.spec(), session_id="s-child", parent="s-1", delegation_depth=1
    )

    await turn(agent, "Go.")

    assert checkpoints_of(agent.session) == []


async def test_without_git_nothing_is_kept_and_the_log_says_so_once(
    harness: Harness, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    await loaded(harness, tmp_path)
    harness.ctx.use(CHECKPOINTS)._find = lambda outside: None  # type: ignore[attr-defined]
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))

    with caplog.at_level(logging.WARNING, "columbina.checkpoints"):
        await turn(agent, "Go.")
        await turn(agent, "Again.")

    assert checkpoints_of(agent.session) == []
    assert caplog.text.count("git is not on the search path") == 1


async def test_what_git_cannot_do_is_said(tmp_path: Path) -> None:
    git = shutil.which("git")
    assert git is not None
    work = tmp_path / "work"
    work.mkdir()
    checkpoints = Checkpoints(Config(home=tmp_path / "home"), lambda outside: git)

    with pytest.raises(CheckpointError, match="git failed"):
        await checkpoints.go_back(work, "0" * 40)
    with pytest.raises(CheckpointError, match="git could not be started"):
        await Checkpoints(
            Config(home=tmp_path / "home"), lambda outside: str(tmp_path / "none")
        ).keep(work)


async def test_git_that_takes_too_long_is_given_up(tmp_path: Path) -> None:
    git = shutil.which("git")
    assert git is not None
    work = tmp_path / "work"
    work.mkdir()
    checkpoints = Checkpoints(Config(home=tmp_path / "home", seconds=1e-6), lambda outside: git)

    with pytest.raises(CheckpointError, match="git took more than"):
        await checkpoints.keep(work)


async def test_git_is_not_told_where_to_look_by_the_environment(
    harness: Harness, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    project = tmp_path / "project.git"
    monkeypatch.setenv("GIT_DIR", str(project))
    monkeypatch.setenv("GIT_INDEX_FILE", str(tmp_path / "index"))
    _, agent = await three_turns(harness, tmp_path)

    assert len(checkpoints_of(agent.session)) == 3
    assert not project.exists()
    assert not (tmp_path / "index").exists()
    assert os.environ["GIT_DIR"] == str(project)


async def test_the_plugin_provides_the_checkpoints(ctx: Context, tmp_path: Path) -> None:
    await ctx.plugin(columbina.session)
    await ctx.plugin(columbina.checkpoints, {"home": str(tmp_path)})

    assert isinstance(ctx.use(CHECKPOINTS), Checkpoints)


async def test_a_turn_that_is_running_is_not_gone_back_from(
    harness: Harness, tmp_path: Path
) -> None:
    commands, agent = await three_turns(harness, tmp_path)
    agent.session.append(TurnStartEvent(turn=4))

    assert await commands.run(agent, "/rewind 2") == (
        "A turn is running. Going back waits until the agent is at rest."
    )
    agent.session.append(TurnStartEvent(turn=5))
    assert rewind_command.said_in(agent.session, 2) == "Change a."
    assert rewind_command.said_in(agent.session, 4) == ""
    assert rewind_command.said_in(agent.session, 9) == ""


async def test_files_that_cannot_be_put_back_are_said(
    harness: Harness, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    commands, agent = await three_turns(harness, tmp_path)
    await commands.run(agent, "/rewind 2 files")

    async def refused(checkpoints: Checkpoints, cwd: Path, commit: str) -> None:
        raise CheckpointError("git failed: no")

    monkeypatch.setattr(Checkpoints, "go_back", refused)

    assert await commands.run(agent, "/rewind 1 files") == (
        "The files were not put back: git failed: no."
    )
    assert await commands.run(agent, "/rewind back") == (
        "The files were not put back: git failed: no."
    )
