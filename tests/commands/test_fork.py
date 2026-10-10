"""The command that has the conversation go on in a copy of its session."""

import asyncio

import pytest

import columbina.commands
from columbina.agent import INBOX, Agent
from columbina.commands import COMMANDS, SWITCH, Commands, Opener
from columbina.commands import fork as fork_command
from columbina.llm.adapters.scripted import Reply
from columbina.session import EndSeedEvent, SessionStoreError
from columbina.session.store import SESSION_FILE
from support.loop import Harness, assistant, idle, types, user


class Holding:
    """What holds a conversation: it goes over to the agent that it is told to create."""

    def __init__(self, harness: Harness, agent: Agent) -> None:
        self.harness = harness
        self.agent = agent
        self.before: list[Agent] = []

    async def switch(self, opener: Opener) -> Agent:
        opened = await opener(self.harness.ctx, self.harness.spec())
        self.before.append(self.agent)
        self.agent = opened
        return opened


async def loaded(harness: Harness, *script: Reply) -> tuple[Commands, Holding]:
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(fork_command)
    holding = Holding(harness, await harness.agent(*script))
    harness.ctx.provide(SWITCH, holding.switch)
    return harness.ctx.use(COMMANDS), holding


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


def file_of(harness: Harness, agent: Agent) -> bytes:
    return (harness.sessions.directory(harness.cwd, agent.id) / SESSION_FILE).read_bytes()


async def test_the_conversation_goes_on_in_a_copy_and_the_session_stays(harness: Harness) -> None:
    commands, holding = await loaded(harness, Reply(text="One."), Reply(text="Two."))
    first = holding.agent
    await turn(first, "First.")
    held, stored = len(first.session), file_of(harness, first)

    said = await commands.run(first, "/fork")

    copy = holding.agent
    assert copy is not first
    assert said == (
        f"The conversation goes on in a copy of the session s-1: {copy.id}\n"
        "The session s-1 stays as it was."
    )
    # The copy holds what the session held, and says that it came from elsewhere.
    assert types(copy.session) == [*types(first.session), "session/end-seed"]
    assert copy.session.events[-1].data == EndSeedEvent(inherited=True)
    assert harness.sessions.read(harness.cwd, copy.id).header.parent_session == "s-1"
    # The session is as it was, in memory and in its file.
    assert len(first.session) == held
    assert file_of(harness, first) == stored
    # The model of the copy is shown what was said before the two parted.
    await turn(copy, "Second.")
    assert harness.model.requests[1].messages[-3:] == (
        user("First."),
        assistant("One."),
        user("Second."),
    )
    assert copy.session.messages[-1] == assistant("Two.")
    assert len(first.session) == held


async def test_what_was_done_to_the_session_since_its_last_turn_is_copied_too(
    harness: Harness,
) -> None:
    commands, holding = await loaded(harness, Reply(text="One."))
    first = holding.agent
    await turn(first, "First.")
    first.inject("The disk is almost full.")

    await commands.run(first, "/fork")

    assert INBOX.of(holding.agent.session).next_step == INBOX.of(first.session).next_step
    assert len(INBOX.of(holding.agent.session).next_step) == 1


async def test_a_session_in_which_no_turn_has_ended_is_not_copied(harness: Harness) -> None:
    commands, holding = await loaded(harness)

    said = await commands.run(holding.agent, "/fork")

    assert said == "No turn of this session has come to its end, so there is nothing to copy yet."
    assert holding.before == []
    assert list(harness.agents.live) == ["s-1"]


async def test_a_session_is_not_copied_while_a_turn_runs(harness: Harness) -> None:
    commands, holding = await loaded(harness, Reply(text="One."), Reply(text="thinking", hold=True))
    first = holding.agent
    await turn(first, "First.")
    first.followup("Second.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    said = await commands.run(first, "/fork")

    assert said == "A turn is running. The session can be copied when the agent is at rest."
    assert holding.before == []
    harness.model.release()
    await idle(first)


async def test_where_nothing_can_go_over_to_another_session_nothing_is_copied(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(fork_command)
    agent = await harness.agent(Reply(text="One."))
    await turn(agent, "First.")

    said = await harness.ctx.use(COMMANDS).run(agent, "/fork")

    assert said == (
        "This program cannot go over to another session, so the session was not copied. "
        "`columbina sessions fork ID` copies a stored one."
    )
    assert list(harness.agents.live) == ["s-1"]
    assert [listed.id for listed in harness.sessions.stored(harness.cwd)] == ["s-1"]


async def test_a_copy_that_cannot_be_made_is_told_and_the_conversation_stays(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    commands, holding = await loaded(harness, Reply(text="One."), Reply(text="Two."))
    first = holding.agent
    await turn(first, "First.")

    def full(*args: object, **more: object) -> None:
        raise SessionStoreError("the disk is full")

    monkeypatch.setattr(type(harness.sessions), "fork", full)

    said = await commands.run(first, "/fork")

    assert said == "The command /fork failed: SessionStoreError: the disk is full"
    assert holding.agent is first
    assert list(harness.agents.live) == ["s-1"]
    await turn(first, "Second.")
    assert first.session.messages[-1] == assistant("Two.")


async def test_the_command_is_listed_and_leaves_with_its_plugin(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.commands)
    plugin = await harness.ctx.plugin(fork_command)
    commands = harness.ctx.use(COMMANDS)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")
    assert "/fork  Go on in a copy of this session, and leave the session as it is." in listed

    await plugin.dispose()

    assert await commands.run(agent, "/fork") == (
        "There is no command /fork. /help lists the commands."
    )
