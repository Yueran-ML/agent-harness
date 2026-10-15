"""The terminal interface: a conversation with an agent, held at a terminal that is a test."""

import asyncio
import logging
import os
import re
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

import columbina
import columbina.approval
import columbina.apps.cli
import columbina.commands
import columbina.fs
import columbina.permissions
import columbina.sandbox
import columbina.token_meter
from columbina import cancellation
from columbina.agent import INBOX, STREAM, Agent, AgentSpec, StreamFrame
from columbina.approval import APPROVAL, ApprovalRequest
from columbina.apps import FRONTEND, Frontend
from columbina.apps.cli.terminal import TERMINAL
from columbina.commands import COMMANDS, SWITCH, Command
from columbina.commands import compact as compact_command
from columbina.commands import fork as fork_command
from columbina.compaction import COMPACTION, Outcome, Why
from columbina.kernel import Context
from columbina.llm import BlockEnd, BlockStart, Chunk, LlmFailure, TextDelta, Usage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import TurnEndEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE
from columbina.token_meter import TOKEN_METER
from columbina.tools import Tool, ToolRun
from support.loop import MODEL, Harness, assistant, from_a_source, lingering, user
from support.terminal import CTRL_D, ESCAPE, Desk, desk, stands, until

STEER = chr(0xBB)


async def frontend(harness: Harness, at: Desk | None, **config: Any) -> Frontend:
    """Load the terminal interface on top of the loop of the harness."""
    if at is not None:
        harness.ctx.provide(TERMINAL, at.terminal)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(
        columbina.apps.cli, {"model": MODEL.model_dump(), "cwd": harness.cwd, **config}
    )
    return harness.ctx.use(FRONTEND)


def agent_of(harness: Harness) -> Agent:
    """The one agent that the interface talks to."""
    (agent,) = harness.agents.live.values()
    return agent


async def left(running: "asyncio.Future[int]") -> int:
    return await asyncio.wait_for(running, 5)


class Slow:
    """A command that has the agent do something, which takes until it is let go or stopped.

    Attributes:
        begun: Set when the agent has begun with it.
        let_go: Set by a test to have it come to its end.
        ended: How the command ended, once it has.
    """

    def __init__(self) -> None:
        self.begun = asyncio.Event()
        self.let_go = asyncio.Event()
        self.ended: list[str] = []

    async def __call__(self, agent: Agent, rest: str) -> str:
        async def work() -> None:
            self.begun.set()
            await self.let_go.wait()

        try:
            await cancellation.guard(agent.run_maintenance(work), what="what takes long")
        except cancellation.CancelledFromInside:
            self.ended.append("the agent was stopped")
            return "It was stopped."
        except asyncio.CancelledError:
            self.ended.append("the command was stopped")
            raise
        self.ended.append("it is done")
        return "It is done."


def slow(harness: Harness) -> Slow:
    """Give the program a command that takes long, by the name ``/slow``."""
    command = Slow()
    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "slow", Command(command, "Take long."))
    return command


class Summing:
    """Whatever makes conversations shorter, and never comes to an end by itself here."""

    def __init__(self) -> None:
        self.begun = asyncio.Event()

    async def compact(self, agent: Agent, why: Why) -> Outcome:
        self.begun.set()
        await asyncio.Event().wait()
        raise AssertionError("it is stopped before it ends")


def said_in(harness: Harness, session_id: str) -> list[object]:
    """What a person said in a stored session, in order."""
    stored = harness.sessions.read(harness.cwd, session_id)
    return [
        event.data.message
        for event in stored.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]


# A conversation


async def test_a_line_is_a_task_and_the_reply_is_shown_above_the_prompt(harness: Harness) -> None:
    harness.model.add(Reply(text="Hello!\nWhat shall we do?"))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Say hello.")
        # The empty line sets the turn apart from what comes after it.
        await at.sees("Hello!\nWhat shall we do?\n\n")
        at.enter("/exit")

        assert await left(running) == 0
    assert harness.model.requests[0].messages[-1] == user("Say hello.")


async def test_the_conversation_begins_with_a_word_about_itself(harness: Harness) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        await at.sees(
            f"Columbina {columbina.__version__}   {MODEL.model}   {harness.cwd}\n"
            "/help lists the commands. Ctrl+D leaves.\n"
            "\n"
        )
        at.type(CTRL_D)

        assert await left(running) == 0


async def test_tasks_follow_one_another_in_one_session(harness: Harness) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("Second.")
        await at.sees("Two.\n")
        at.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    ends = [event.data for event in stored.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]


async def test_a_line_of_spaces_is_nothing(harness: Harness) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("   ")
        at.enter("")
        at.enter("  Padded.  ")
        await at.sees("One.\n")
        at.type(CTRL_D)
        await left(running)

    assert len(harness.model.requests) == 1
    assert harness.model.requests[0].messages[-1] == user("Padded.")


async def test_a_line_that_is_typed_while_a_turn_runs_is_read_at_the_next_step(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="Reading a.txt.", hold=True), Reply(text="Reading b.txt."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        at.enter("Read a.txt.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.enter("No, read b.txt.")
        # The line counts as said on the way once the interface has taken it. A line that
        # is submitted in the moment in which the turn ends is a task for the next turn.
        agent = agent_of(harness)
        await until(lambda: len(INBOX.of(agent.session).next_step) == 1)
        harness.model.release()
        await at.sees(f"{STEER} No, read b.txt.\n")
        await at.sees("Reading b.txt.\n")
        at.type(CTRL_D)
        await left(running)

    assert harness.model.requests[1].messages[-1] == user("No, read b.txt.")


async def test_escape_stops_the_turn_and_the_next_task_is_taken(harness: Harness) -> None:
    harness.model.add(Reply(text="thinking", hold=True), Reply(text="Here I am."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.type(ESCAPE)
        await at.sees("(stopped)\n")
        at.enter("Are you there?")
        await at.sees("Here I am.\n")
        at.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    ends = [event.data for event in stored.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="aborted", detail="user"),
        TurnEndEvent(turn=2, reason="completed"),
    ]


async def test_a_turn_that_fails_says_why_and_the_conversation_goes_on(harness: Harness) -> None:
    failed = LlmFailure(message="the key is not valid", code="AUTH", status=401)
    harness.model.add(Reply(failure=failed), Reply(text="Now it works."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Hello?")
        await at.sees("(the turn ended with an error: AUTH: the key is not valid)\n")
        at.enter("Again?")
        await at.sees("Now it works.\n")
        at.type(CTRL_D)

        assert await left(running) == 0


# Commands


async def test_a_line_that_begins_with_a_slash_is_carried_out_and_not_sent(
    harness: Harness,
) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("/help")
        await at.sees(
            "/exit   Leave.\n"
            "/help   List the commands.\n"
            "/model  Show the model, or name another.\n"
            "/new    End this session and begin another one.\n"
            "A line that does not begin with a slash goes to the agent.\n"
            "\n"
        )
        at.enter("/nowhere")
        await at.sees("There is no command /nowhere. /help lists the commands.\n\n")
        at.enter("/exit")

        assert await left(running) == 0
        # A command that has nothing to say shows nothing, not an empty line either.
        assert at.seen().endswith("There is no command /nowhere. /help lists the commands.\n\n")
    assert harness.model.requests == []


async def test_a_command_may_have_spaces_before_it(harness: Harness) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("   /nowhere  ")
        await at.sees("There is no command /nowhere. /help lists the commands.\n")
        at.enter("  /exit")

        assert await left(running) == 0
    assert harness.model.requests == []


async def test_a_command_is_not_carried_out_while_a_turn_runs(harness: Harness) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.enter("/exit")
        await at.sees("(a command waits until the agent is at rest. Escape stops the agent.)\n")
        assert not running.done()
        assert INBOX.of(agent_of(harness).session).next_step == ()
        harness.model.release()
        await at.sees("thinking\n")
        at.enter("/exit")

        assert await left(running) == 0


async def test_another_model_can_be_named_and_the_next_request_goes_to_it(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        cli = await frontend(harness, at)
        assert isinstance(cli, columbina.apps.cli.Cli)
        running = asyncio.ensure_future(cli.run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("/model")
        await at.sees(f"The model is {MODEL.model}. Another one is named with /model ID.\n")
        at.enter("/model other-model")
        await at.sees("The model is other-model from the next request on.\n")
        # The line below the prompt names the model that the next request goes to.
        assert cli.status_line().split(" | ")[0] == "other-model"
        at.enter("Second.")
        await at.sees("Two.\n")
        at.type(CTRL_D)
        await left(running)

    asked = [request.config.model for request in harness.model.requests]
    assert asked == [MODEL.model, "other-model"]


async def test_a_model_that_is_named_has_the_window_of_the_first_model_only_where_it_is_that_one(
    harness: Harness,
) -> None:
    harness.model.add(
        *(Reply(text=said, usage=Usage(input_tokens=500)) for said in ("One.", "Two.", "Three."))
    )
    with desk() as at:
        cli = await frontend(harness, at, model={**MODEL.model_dump(), "context_window": 1000})
        assert isinstance(cli, columbina.apps.cli.Cli)
        running = asyncio.ensure_future(cli.run())
        shown: list[list[str]] = []

        at.enter("First.")
        await at.sees("One.\n")
        shown.append(cli.status_line().split(" | ")[:2])
        at.enter("/model other-model")
        await at.sees("The model is other-model from the next request on.\n")
        shown.append(cli.status_line().split(" | ")[:2])
        at.enter("Second.")
        await at.sees("Two.\n")
        at.enter(f"/model {MODEL.model}")
        await at.sees(f"The model is {MODEL.model} from the next request on.\n")
        shown.append(cli.status_line().split(" | ")[:2])
        at.enter("Third.")
        await at.sees("Three.\n")
        at.type(CTRL_D)
        await left(running)

    # Of the window, what is known is known of the model that the run started with. The
    # share of it that is used is said of that model only.
    assert shown == [
        [MODEL.model, "500 tokens (50%)"],
        ["other-model", "500 tokens"],
        [MODEL.model, "500 tokens (50%)"],
    ]
    asked = [
        (request.config.model, request.config.context_window) for request in harness.model.requests
    ]
    assert asked == [(MODEL.model, 1000), ("other-model", None), (MODEL.model, 1000)]


async def test_a_new_session_can_be_begun_without_leaving(harness: Harness) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("/new")
        await at.sees("A new session begins: ")
        at.enter("Second.")
        await at.sees("Two.\n")
        (agent,) = harness.agents.live.values()
        second = agent.id
        at.type(CTRL_D)
        await left(running)

    assert second != "first"
    assert f"A new session begins: {second}\n" in at.seen()
    # The model of the new session was not sent what the old one held.
    assert user("First.") not in harness.model.requests[1].messages
    assert harness.model.requests[1].messages[-1] == user("Second.")
    first = harness.sessions.read(harness.cwd, "first")
    said = [
        event.data.message
        for event in first.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]
    assert said == [user("First.")]


async def test_the_conversation_can_go_on_in_a_copy_of_its_session(harness: Harness) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        interface = await frontend(harness, at, session_id="first")
        await harness.ctx.plugin(fork_command)
        running = asyncio.ensure_future(interface.run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("/fork")
        await at.sees("The session first stays as it was.\n")
        # The session that the conversation left is closed, and its agent is gone.
        (agent,) = harness.agents.live.values()
        second = agent.id
        at.enter("Second.")
        await at.sees("Two.\n")
        at.type(CTRL_D)
        await left(running)

    assert second != "first"
    # What the copy holds of the conversation is shown, and then what the command says.
    assert (
        "(this session is taken up again. Its last turn:)\n"
        "> First.\n"
        "One.\n"
        "\n"
        f"The conversation goes on in a copy of the session first: {second}\n"
        "The session first stays as it was.\n"
    ) in at.seen()
    # The model of the copy was shown what was said before the two parted.
    assert harness.model.requests[1].messages[-3:] == (
        user("First."),
        assistant("One."),
        user("Second."),
    )
    stored = harness.sessions.read(harness.cwd, "first")
    said = [
        event.data.message
        for event in stored.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]
    assert said == [user("First.")]
    assert isinstance(stored.events[-1].data, TurnEndEvent)
    assert harness.sessions.read(harness.cwd, second).header.parent_session == "first"


async def test_the_conversation_stays_where_it_is_if_it_cannot_go_over(harness: Harness) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        await at.sees("Ctrl+D leaves.\n")

        async def broken(scope: Context, spec: AgentSpec) -> Agent:
            raise RuntimeError("no such agent")

        with pytest.raises(RuntimeError, match="no such agent"):
            await harness.ctx.use(SWITCH)(broken)

        # The agent that it had is the one that answers.
        at.enter("First.")
        await at.sees("One.\n")
        assert agent_of(harness).id == "first"
        at.type(CTRL_D)
        await left(running)


# A command that takes long


async def test_escape_stops_what_a_command_has_the_agent_do(harness: Harness) -> None:
    summing = Summing()
    harness.ctx.provide(COMPACTION, summing)
    with desk() as at:
        cli = await frontend(harness, at)
        assert isinstance(cli, columbina.apps.cli.Cli)
        await harness.ctx.plugin(compact_command)
        running = asyncio.ensure_future(cli.run())

        at.enter("/compact")
        await asyncio.wait_for(summing.begun.wait(), timeout=5)
        # The line below the prompt says how it is stopped.
        assert cli.status_line().split(" | ")[-1] == "esc stops"
        at.type(ESCAPE)

        await at.sees("It was stopped. The conversation is as it was.\n\n")
        await until(lambda: "esc stops" not in cli.status_line())
        # The prompt takes lines as it did before.
        at.enter("/exit")
        assert await left(running) == 0


async def test_what_is_submitted_while_a_command_is_carried_out_waits_for_it(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        interface = await frontend(harness, at)
        command = slow(harness)
        running = asyncio.ensure_future(interface.run())
        at.enter("/slow")
        await asyncio.wait_for(command.begun.wait(), timeout=5)

        at.enter("/model")
        at.enter("First.")
        # The keys are read, and nothing comes of the two lines yet.
        await asyncio.sleep(0.2)
        assert "The model is" not in at.seen()
        assert INBOX.of(agent_of(harness).session).next_turn == ()
        command.let_go.set()

        await at.sees("One.\n")
        at.type(CTRL_D)
        assert await left(running) == 0
    shown = at.seen()
    # They are taken in the order in which they were submitted, once the command is over.
    assert shown.index("It is done.\n") < shown.index("The model is ") < shown.index("One.\n")
    assert command.ended == ["it is done"]
    assert harness.model.requests[0].messages[-1] == user("First.")


async def test_leaving_stops_a_command_that_is_being_carried_out(harness: Harness) -> None:
    with desk() as at:
        interface = await frontend(harness, at)
        command = slow(harness)
        running = asyncio.ensure_future(interface.run())
        at.enter("/slow")
        await asyncio.wait_for(command.begun.wait(), timeout=5)

        at.type(CTRL_D)

        assert await left(running) == 0
        assert command.ended == ["the command was stopped"]
    assert "It was stopped." not in at.seen()
    assert list(harness.agents.live) == []


async def test_what_a_command_lets_through_ends_the_conversation(harness: Harness) -> None:
    class Odd(BaseException):
        """Nothing that a command is expected to raise."""

    async def lets_through(agent: Agent, rest: str) -> str:
        raise Odd

    with desk() as at:
        interface = await frontend(harness, at)
        harness.ctx.use(COMMANDS).registry.add(harness.ctx, "odd", Command(lets_through, "Break."))
        running = asyncio.ensure_future(interface.run())
        at.enter("/odd")

        with pytest.raises(Odd):
            await left(running)
    assert list(harness.agents.live) == []


# Going over to another agent


async def test_a_line_goes_to_the_agent_that_the_conversation_went_over_to(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        await at.sees("Ctrl+D leaves.\n")
        first = agent_of(harness)

        # Somebody else has the conversation go over while the prompt waits for a line.
        other = await harness.ctx.use(SWITCH)(harness.agents.create)

        at.enter("First.")
        await at.sees("One.\n")
        at.type(CTRL_D)
        assert await left(running) == 0
    assert other is not first
    assert said_in(harness, other.id) == [user("First.")]
    # Nothing was said to the agent that the conversation had left.
    assert first.session.closed
    assert list(first.session.events) == []


async def test_going_over_that_is_cut_short_leaves_no_agent_to_itself(harness: Harness) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        await at.sees("Ctrl+D leaves.\n")
        first = agent_of(harness)
        # The agent that the conversation leaves takes its time to end.
        with lingering(first) as held:
            going = asyncio.ensure_future(harness.ctx.use(SWITCH)(harness.agents.create))
            await asyncio.wait_for(held.reached.wait(), timeout=5)

            # Whoever asked for it is stopped meanwhile.
            going.cancel()
            with pytest.raises(asyncio.CancelledError):
                await going
        await until(lambda: first.session.closed)

        # The conversation is with the other agent all the same.
        other = agent_of(harness)
        at.enter("First.")
        await at.sees("One.\n")
        at.type(CTRL_D)
        assert await left(running) == 0
    assert said_in(harness, other.id) == [user("First.")]
    # And the program ended it as it left, like every agent of a conversation.
    assert other.session.closed
    assert list(harness.agents.live) == []


async def test_an_agent_is_heard_from_the_moment_the_conversation_goes_over_to_it(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        await at.sees("Ctrl+D leaves.\n")

        async def with_a_task(scope: Context, spec: AgentSpec) -> Agent:
            agent = await harness.agents.create(scope, spec)
            agent.followup("Think.")
            return agent

        # The agent that the conversation leaves takes its time to end.
        with lingering(agent_of(harness)):
            going = asyncio.ensure_future(harness.ctx.use(SWITCH)(with_a_task))
            await asyncio.wait_for(harness.model.held(), timeout=5)
            assert not going.done()

            # Escape stops the turn of the other agent, although the one before it is
            # still ending.
            at.type(ESCAPE)
            await at.sees("(stopped)\n")
        other = await asyncio.wait_for(going, timeout=5)
        at.type(CTRL_D)
        assert await left(running) == 0
    assert other.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")


async def test_an_agent_that_is_in_a_turn_when_the_conversation_goes_over_to_it_can_be_stopped(
    harness: Harness,
) -> None:
    harness.model.add(
        Reply(text="first thoughts", hold=True), Reply(text="second thoughts", hold=True)
    )
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        first = agent_of(harness)

        async def in_a_turn(scope: Context, spec: AgentSpec) -> Agent:
            agent = await harness.agents.create(scope, spec)
            agent.followup("Think too.")
            await until(lambda: len(harness.model.requests) == 2)
            return agent

        # The agent that the conversation leaves takes its time to end, and writes meanwhile.
        with lingering(first):
            going = asyncio.ensure_future(harness.ctx.use(SWITCH)(in_a_turn))
            # The turn of the first agent is stopped as the conversation leaves it.
            await until(lambda: isinstance(first.session.events[-1].data, TurnEndEvent))
            (other,) = (agent for agent in harness.agents.live.values() if agent is not first)

            at.type(ESCAPE)

            await until(lambda: isinstance(other.session.events[-1].data, TurnEndEvent))
        assert await asyncio.wait_for(going, timeout=5) is other
        at.type(CTRL_D)
        assert await left(running) == 0
    stopped = TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert first.session.events[-1].data == stopped
    assert other.session.events[-1].data == stopped
    # What the first agent wrote down as it ended is in its log. It was not shown as if
    # it were of the conversation.
    assert at.seen().count("(stopped)\n") == 1


async def test_an_agent_at_rest_takes_a_command_although_the_one_before_it_was_in_a_turn(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        first = agent_of(harness)

        other = await asyncio.wait_for(harness.ctx.use(SWITCH)(harness.agents.create), timeout=5)

        # Nobody waits any longer for the first agent to come to rest. The one that the
        # conversation is with is at rest, so a command is carried out.
        at.enter("/exit")
        assert await left(running) == 0
    assert first.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert other.session.closed
    assert "a command waits" not in at.seen()


async def test_what_an_agent_writes_after_the_conversation_left_it_is_not_shown(
    harness: Harness,
) -> None:
    with desk(rows=10, columns=400) as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())
        await at.sees("/help lists the commands.")
        first = agent_of(harness)

        def arrives(chunk: Chunk) -> None:
            first.ctx.emit(STREAM, StreamFrame(first, 1, 1, chunk))

        # The first agent is in the middle of a line of its reply.
        arrives(BlockStart(index=0, kind="text"))
        arrives(TextDelta(index=0, text="Half a li"))
        await until(lambda: "Half a li" in at.window())

        # It takes its time to end, and is listened to until it has ended.
        with lingering(first):
            going = asyncio.ensure_future(harness.ctx.use(SWITCH)(harness.agents.create))
            # The half of a line is no part of the conversation any more.
            await until(lambda: "Half a li" not in at.window())
            arrives(TextDelta(index=0, text="ne that comes late"))
            await asyncio.sleep(0.2)
            assert not any("comes late" in row for row in at.window())
        await asyncio.wait_for(going, timeout=5)
        at.type(CTRL_D)
        assert await left(running) == 0
    assert "comes late" not in at.shown()


# Leaving


async def test_leaving_closes_the_agent_and_ends_a_turn_that_was_running(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.type(CTRL_D)

        assert await left(running) == 0
    stored = harness.sessions.read(harness.cwd, "s-1")
    assert stored.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert list(harness.agents.live) == []


async def test_a_terminal_that_goes_away_ends_the_conversation(harness: Harness) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        at.hang_up()

        assert await left(running) == 0
    assert list(harness.agents.live) == []


async def test_without_a_terminal_there_is_no_conversation(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    runner = await frontend(harness, None)

    code = await runner.run()

    assert code == 2
    assert capsys.readouterr().err == (
        "columbina: this is not a terminal, and the conversation needs one. "
        "`columbina run TASK` gives an agent one task without it.\n"
    )
    assert list(harness.agents.live) == []


async def test_an_agent_that_stops_for_good_ends_the_conversation(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        monkeypatch.setattr("columbina.session.store.encode_event", full)

        harness.model.release()
        await at.sees("(the agent has stopped: ")
        code = await left(running)
        monkeypatch.undo()

    assert code == 1
    assert "No space left on device" in at.seen()
    # It is said once, though closing the agent finds the same thing again.
    assert at.seen().count("(the agent has stopped: ") == 1


async def test_an_agent_that_stops_for_good_in_a_turn_from_elsewhere_ends_the_conversation(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        await at.sees("/help lists the commands.")
        # Another part of the program gives the agent a task. Nothing was typed.
        agent_of(harness).followup("From elsewhere.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        monkeypatch.setattr("columbina.session.store.encode_event", full)

        harness.model.release()
        await at.sees("(the agent has stopped: ")
        code = await left(running)
        monkeypatch.undo()

    assert code == 1
    assert "No space left on device" in at.seen()


async def test_leaving_when_the_end_of_the_turn_cannot_be_written_is_no_success(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        monkeypatch.setattr("columbina.session.store.encode_event", full)

        # Leaving stops the turn, and what is left of it has to be written down.
        at.type(CTRL_D)
        code = await left(running)
        monkeypatch.undo()

    assert code == 1
    assert at.seen().count("(the agent has stopped: ") == 1
    assert "No space left on device" in at.seen()


# Sessions


async def test_an_id_that_cannot_name_a_session_ends_the_conversation(harness: Harness) -> None:
    with desk() as at:
        runner = await frontend(harness, at, session_id="../escape")

        code = await asyncio.wait_for(runner.run(), 5)

        assert code == 1
        await at.sees("columbina: '../escape' is not a usable session id")
    assert harness.model.requests == []


async def test_a_session_that_is_named_is_taken_up_and_its_last_turn_is_shown(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="The parser is fixed."), Reply(text="The lexer too."))
    await harness.ctx.plugin(columbina.commands)
    config = {"model": MODEL.model_dump(), "cwd": harness.cwd, "session_id": "s-1"}
    with desk() as at:
        provided = harness.ctx.provide(TERMINAL, at.terminal)
        loaded = await harness.ctx.plugin(columbina.apps.cli, config)
        running = asyncio.ensure_future(harness.ctx.use(FRONTEND).run())
        at.enter("Fix the parser.")
        await at.sees("The parser is fixed.\n")
        at.type(CTRL_D)
        await left(running)
        await loaded.dispose()
        await provided.dispose()

    with desk() as again:
        harness.ctx.provide(TERMINAL, again.terminal)
        await harness.ctx.plugin(columbina.apps.cli, config)
        running = asyncio.ensure_future(harness.ctx.use(FRONTEND).run())

        await again.sees(
            "(this session is taken up again. Its last turn:)\n"
            "> Fix the parser.\n"
            "The parser is fixed.\n"
        )
        again.enter("And the lexer.")
        await again.sees("The lexer too.\n")
        again.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    said = [
        event.data.message
        for event in stored.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]
    assert said == [user("Fix the parser."), user("And the lexer.")]


async def an_older_and_a_newer_session(harness: Harness) -> None:
    """Store two sessions of the working directory, one used after the other."""
    for name, at_second in (("older", 1000), ("newer", 2000)):
        session = harness.sessions.create(harness.ctx, harness.cwd, session_id=name)
        session.append(UserMessageEvent(message=user(f"A task of {name}.")))
        await session.flush()
        session.close()
        stamp = at_second * 1_000_000_000
        file = harness.sessions.directory(harness.cwd, name) / SESSION_FILE
        os.utime(file, ns=(stamp, stamp))


def went_on(harness: Harness, name: str) -> bool:
    """Tell whether a turn was held in a stored session."""
    stored = harness.sessions.read(harness.cwd, name)
    return any(isinstance(event.data, TurnEndEvent) for event in stored.events)


async def test_a_session_that_is_named_is_taken_up_although_another_one_was_used_last(
    harness: Harness,
) -> None:
    await an_older_and_a_newer_session(harness)
    harness.model.add(Reply(text="Going on."))
    with desk() as at:
        named = await frontend(harness, at, session_id="older", resume=True)
        running = asyncio.ensure_future(named.run())
        await at.sees("/help lists the commands.")

        at.enter("Go on.")
        await at.sees("Going on.\n")
        at.type(CTRL_D)
        await left(running)

    assert (went_on(harness, "older"), went_on(harness, "newer")) == (True, False)


async def test_the_session_that_was_used_last_can_be_taken_up_without_its_name(
    harness: Harness,
) -> None:
    await an_older_and_a_newer_session(harness)
    harness.model.add(Reply(text="Going on."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, resume=True)).run())
        await at.sees("/help lists the commands.")

        at.enter("Go on.")
        await at.sees("Going on.\n")
        at.type(CTRL_D)
        await left(running)

    assert (went_on(harness, "older"), went_on(harness, "newer")) == (False, True)


async def test_the_session_of_a_subagent_is_not_what_is_taken_up_again(harness: Harness) -> None:
    await an_older_and_a_newer_session(harness)
    # An agent handed a task on after that, and the session of the subagent was written last.
    child = harness.sessions.create(
        harness.ctx, harness.cwd, session_id="child", parent="newer", delegation_depth=1
    )
    child.append(UserMessageEvent(message=user("A task that was handed on.")))
    await child.flush()
    child.close()
    stamp = 3000 * 1_000_000_000
    os.utime(harness.sessions.directory(harness.cwd, "child") / SESSION_FILE, ns=(stamp, stamp))
    harness.model.add(Reply(text="Going on."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, resume=True)).run())
        await at.sees("/help lists the commands.")

        at.enter("Go on.")
        await at.sees("Going on.\n")
        at.type(CTRL_D)
        await left(running)

    assert (went_on(harness, "newer"), went_on(harness, "child")) == (True, False)


async def test_taking_up_a_session_where_none_is_stored_begins_a_new_one(
    harness: Harness, tmp_path: Path
) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, resume=True)).run())

        await at.sees("(no session of this directory is stored yet. A new one begins.)\n")
        at.type(CTRL_D)

        assert await left(running) == 0


# What else is shown


async def test_what_the_program_logs_is_shown_above_the_prompt_while_it_is_open(
    harness: Harness,
) -> None:
    log = logging.getLogger("columbina.somewhere")
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        log.warning("careful with %s", "that")
        log.error("that went wrong")
        await at.sees("columbina: careful with that\ncolumbina: that went wrong\n")
        at.type(CTRL_D)
        await left(running)
        handlers = list(logging.getLogger("columbina").handlers)

    assert not any(type(handler).__name__ == "_Shown" for handler in handlers)


async def test_what_a_library_warns_of_while_the_prompt_is_open_is_shown_above_it(
    harness: Harness,
) -> None:
    # A library logs by itself, and nothing of this program sets up its log. Python would
    # write what it warns of to the terminal by itself, through the prompt.
    library = logging.getLogger("somelibrary.part")
    library.setLevel(logging.INFO)
    try:
        with desk() as at:
            running = asyncio.ensure_future((await frontend(harness, at)).run())
            await at.sees("/help lists the commands.")

            library.info("what it does now")
            library.warning("the line went away")
            logging.getLogger("columbina.somewhere").warning("said once")
            # A library whose name begins like the name of this program is another.
            logging.getLogger("columbinas").warning("not ours")
            await at.sees(
                "somelibrary: the line went away\ncolumbina: said once\ncolumbinas: not ours\n"
            )
            at.type(CTRL_D)
            await left(running)
            handlers = list(logging.getLogger().handlers)
    finally:
        library.setLevel(logging.NOTSET)

    # What it does is no warning, and a line of this program is shown once.
    assert at.seen().count("what it does now") == 0
    assert at.seen().count("said once") == 1
    assert not any(type(handler).__name__ == "_Shown" for handler in handlers)


async def test_the_log_is_taken_over_for_the_conversation_and_handed_back_after_it(
    harness: Harness,
) -> None:
    # Whoever started the program may have the log written somewhere. While the prompt is
    # there, that would tear the prompt apart. Before and after, it is where the log goes.
    log = logging.getLogger("columbina")
    elsewhere = logging.NullHandler()
    log.addHandler(elsewhere)
    try:
        with desk() as at:
            running = asyncio.ensure_future((await frontend(harness, at)).run())
            await at.sees("/help lists the commands.")
            during = list(log.handlers)
            at.type(CTRL_D)
            await left(running)
        after = list(log.handlers)
    finally:
        log.removeHandler(elsewhere)

    assert elsewhere not in during
    assert elsewhere in after
    assert len(during) == 1


async def test_an_error_in_the_log_is_shown_in_red_and_a_warning_is_not(
    harness: Harness,
) -> None:
    red = "\x1b[0;31m"
    log = logging.getLogger("columbina.somewhere")
    with desk(colours=True) as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        log.warning("careful")
        await at.sees("columbina: careful\n")
        until_the_warning = at.sent()
        log.error("that went wrong")
        await at.sees("columbina: that went wrong\n")
        at.type(CTRL_D)
        await left(running)

    assert red not in until_the_warning
    assert red in at.sent()[len(until_the_warning) :]


async def test_the_line_below_the_prompt_tells_of_the_session(harness: Harness) -> None:
    harness.model.add(Reply(text="One.", usage=Usage(input_tokens=1200, output_tokens=5)))
    with desk() as at:
        runner = await frontend(harness, at)
        assert isinstance(runner, columbina.apps.cli.Cli)
        before = runner.status_line()
        running = asyncio.ensure_future(runner.run())

        at.enter("First.")
        await at.sees("One.\n")
        await asyncio.wait_for(agent_of(harness).when_idle(), 5)
        after = runner.status_line()
        at.type(CTRL_D)
        await left(running)

    assert before == ""
    assert after == f"{MODEL.model} | 1.2k tokens | cache 0%"


async def test_the_line_below_the_prompt_shows_the_estimate_where_the_program_has_one(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.token_meter)
    harness.model.add(Reply(text="One.", usage=Usage(input_tokens=1200, output_tokens=5)))
    with desk() as at:
        runner = await frontend(harness, at)
        assert isinstance(runner, columbina.apps.cli.Cli)
        running = asyncio.ensure_future(runner.run())

        at.enter("First.")
        await at.sees("One.\n")
        await asyncio.wait_for(agent_of(harness).when_idle(), 5)
        after = runner.status_line()
        held = harness.ctx.use(TOKEN_METER).measure(agent_of(harness).session).tokens
        at.type(CTRL_D)
        await left(running)

    # What the service counted for the last request is no longer what is shown.
    assert 0 < held < 1000
    assert after == f"{MODEL.model} | ~{held} tokens | cache 0%"


async def test_the_line_below_the_prompt_goes_by_the_preset_before_the_first_turn(
    harness: Harness,
) -> None:
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await harness.ctx.plugin(plugin)
    await harness.ctx.plugin(columbina.permissions, {"preset": "full-access"})
    with desk() as at:
        runner = await frontend(harness, at)
        assert isinstance(runner, columbina.apps.cli.Cli)
        running = asyncio.ensure_future(runner.run())
        await at.sees("/help lists the commands.")

        before = runner.status_line()
        at.type(CTRL_D)
        await left(running)

    assert before == f"{MODEL.model} | danger-full-access, commands not held"


async def test_a_question_of_approval_is_put_to_the_person_at_the_terminal(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.approval)
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")
        agent = agent_of(harness)
        asked = ApprovalRequest(
            session=agent.session,
            call_id="c-1",
            tool="bash",
            summary="bash: git status",
            reason="No rule allows this command.",
            target="git status",
        )

        answering = asyncio.ensure_future(harness.ctx.use(APPROVAL).ask(agent.ctx, asked))
        await at.sees("bash asks for a yes:\n    git status\nNo rule allows this command.\n")
        await at.draws("[y/n]")
        at.type("y")

        assert await asyncio.wait_for(answering, 5) == "allowed-once"
        at.type(CTRL_D)
        await left(running)


# The screen


async def test_a_conversation_stands_on_the_screen_with_what_the_user_typed(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="Hello!\nWhat shall we do?"))
    # The window is wide, so that the name of the working directory fits into one row.
    with desk(rows=12, columns=400) as at:
        cli = await frontend(harness, at)
        assert isinstance(cli, columbina.apps.cli.Cli)
        running = asyncio.ensure_future(cli.run())

        at.enter("Say hello.")
        await at.sees("What shall we do?\n\n")
        await stands(lambda: at.window()[7:8], [">"])

        assert at.window() == [
            f"Columbina {columbina.__version__}   {MODEL.model}   {harness.cwd}",
            "/help lists the commands. Ctrl+D leaves.",
            "",
            "> Say hello.",
            "Hello!",
            "What shall we do?",
            "",
            ">",
            "",
            "",
            "",
            cli.status_line(),
        ]
        assert at.cursor() == (7, 2)
        at.enter("/exit")
        assert await left(running) == 0
        assert at.screen()[7:] == ["> /exit"]


async def test_what_is_said_on_the_way_stands_where_it_was_typed_and_where_it_was_read(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="Reading a.txt.", hold=True), Reply(text="Reading b.txt."))
    with desk(rows=14, columns=400) as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        at.enter("Read a.txt.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.enter("No, read b.txt.")
        # The model has written its line and is held before it ends the reply.
        typed = ["> Read a.txt.", "Reading a.txt.", "> No, read b.txt."]
        await stands(lambda: at.screen()[3:6], typed)
        # The line stands on the screen a moment before the interface has taken it.
        agent = agent_of(harness)
        await until(lambda: len(INBOX.of(agent.session).next_step) == 1)
        harness.model.release()
        await at.sees("Reading b.txt.\n\n")
        await stands(lambda: at.window()[9:10], [">"])

        assert at.window()[3:10] == [
            "> Read a.txt.",
            "Reading a.txt.",
            "> No, read b.txt.",
            f"{STEER} No, read b.txt.",
            "Reading b.txt.",
            "",
            ">",
        ]
        at.type(CTRL_D)
        await left(running)


class Nothing(BaseModel):
    pass


async def test_the_marks_before_the_lines_are_those_that_the_terminal_can_show(
    harness: Harness,
) -> None:
    async def look(args: Nothing, run: ToolRun) -> str:
        return "seen"

    harness.tools.add(
        harness.ctx, Tool(name="look", description="Look.", parameters=Nothing, execute=look)
    )
    harness.model.add(Reply(calls=(Call(name="look"),)), Reply(text="Done."))
    with desk(encoding="ascii") as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Look.")
        await at.sees("Done.\n")
        at.type(CTRL_D)
        await left(running)

    assert re.search(r"^\* look\n  ok \d+ ms, 1 line\nDone\.$", at.seen(), re.MULTILINE)


async def test_the_line_that_is_being_written_stands_above_the_prompt_until_it_is_complete(
    harness: Harness,
) -> None:
    with desk(rows=10, columns=400) as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")
        agent = agent_of(harness)

        def arrives(chunk: Chunk) -> None:
            agent.ctx.emit(STREAM, StreamFrame(agent, 1, 1, chunk))

        arrives(BlockStart(index=0, kind="text"))
        arrives(TextDelta(index=0, text="First line.\nHalf a li"))
        await stands(lambda: at.window()[3:6], ["First line.", "Half a li", ">"])
        printed_so_far = at.seen()
        arrives(TextDelta(index=0, text="ne"))
        await stands(lambda: at.window()[3:6], ["First line.", "Half a line", ">"])
        arrives(BlockEnd(index=0))
        await at.sees("Half a line\n")
        await stands(lambda: at.window()[3:7], ["First line.", "Half a line", ">", ""])

        # The line was drawn with the prompt while it grew, and printed when it was complete.
        assert printed_so_far.endswith("First line.\n")
        at.type(CTRL_D)
        await left(running)


async def test_what_arrived_of_a_reply_that_broke_off_stands_on_the_screen_once(
    harness: Harness,
) -> None:
    # A stream that ends in the middle of a line, without the end of a reply.
    harness.model.add([BlockStart(index=0, kind="text"), TextDelta(index=0, text="Half a li")])
    with desk(rows=12, columns=400) as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Go.")
        await at.sees("(the turn ended with an error: ")
        await stands(lambda: at.window()[7:8], [">"])

        assert at.window()[3:5] == ["> Go.", "Half a li"]
        assert at.window()[5].startswith("(the turn ended with an error: STREAM_CLOSED")
        assert at.window()[6:9] == ["", ">", ""]
        at.type(CTRL_D)
        await left(running)


async def test_a_turn_that_was_not_begun_from_the_keys_can_be_stopped_from_them(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        # Another part of the program gives the agent a task. Nothing was typed.
        agent_of(harness).followup("From elsewhere.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        at.type(ESCAPE)

        await at.sees("(stopped)\n")
        at.type(CTRL_D)
        await left(running)
