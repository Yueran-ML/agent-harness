"""A subagent in this process: an agent of its own, under the agent that hands it a task."""

import asyncio
from pathlib import Path
from typing import TypeVar

import pytest
from pydantic import BaseModel

import columbina.approval
import columbina.fs
import columbina.sandbox
import columbina.subagents
import columbina.subagents.spawn
from columbina.agent import Agent, AgentError, Agents
from columbina.approval import APPROVAL, Approval, ApprovalPolicyEvent
from columbina.guard import budget
from columbina.llm import AssistantMessage, LlmFailure, TextBlock, Usage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.sandbox import SANDBOX, SandboxModeEvent
from columbina.session import (
    SESSION_EVENT,
    USAGE,
    AsideEvent,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    Committed,
    EventData,
    Session,
    StoredSession,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.subagents import (
    SUBAGENTS,
    TOOL,
    SubagentEndedEvent,
    SubagentFailed,
    Subagents,
    SubagentStartedEvent,
    Task,
)
from columbina.subagents.spawn import outcome_of
from columbina.tools import Tool, ToolRun
from support.loop import SYSTEM, Harness, idle, user

EventT = TypeVar("EventT", bound=EventData)


class Args(BaseModel):
    pass


async def did(args: Args, run: ToolRun) -> str:
    return "did it"


def tools(harness: Harness, *names: str) -> None:
    """Give the program tools of these names."""
    for name in names:
        harness.tools.add(
            harness.ctx, Tool(name=name, description="Do it.", parameters=Args, execute=did)
        )


async def loaded(harness: Harness, **config: object) -> Subagents:
    await harness.ctx.plugin(columbina.subagents, config)
    await harness.ctx.plugin(columbina.subagents.spawn)
    return harness.ctx.use(SUBAGENTS)


def task(prompt: str = "Count the files.", kind: str | None = None) -> Task:
    return Task(call_id="call-1", description="count files", prompt=prompt, kind=kind)


def of(session: Session | StoredSession, kind: type[EventT]) -> list[EventT]:
    return [event.data for event in session.events if isinstance(event.data, kind)]


def child_of(harness: Harness, parent: Agent) -> StoredSession:
    """Return the session of the one subagent of ``parent``, as it is stored."""
    (started,) = of(parent.session, SubagentStartedEvent)
    return harness.sessions.read(harness.cwd, started.session_id)


def a_kind(root: Path, name: str, *lines: str, prompt: str = "") -> None:
    """Write the file of a kind of subagent into a directory of kinds."""
    root.mkdir(parents=True, exist_ok=True)
    head = "\n".join(["---", f"name: {name}", f"description: The kind {name}.", *lines, "---"])
    (root / f"{name}.md").write_text(f"{head}\n{prompt}", encoding="utf-8")


# An agent of its own


async def test_a_subagent_has_a_session_of_its_own_that_names_its_parent(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="Three files."))

    assert await subagents.run(parent, task()) == "Three files."

    child = child_of(harness, parent)
    assert child.header.id != parent.id
    assert (child.header.parent_session, child.header.delegation_depth) == (parent.id, 1)
    assert of(child, TurnEndEvent) == [TurnEndEvent(turn=1, reason="completed")]
    # It is closed, and nothing of it goes on.
    assert list(harness.agents.live) == [parent.id]


async def test_a_subagent_knows_nothing_of_the_conversation_of_its_parent(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent(Reply(text="I will remember that the word is plover."))
    parent.followup("Remember the word plover.")
    await idle(parent)
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task("Count the files."))

    request = harness.model.requests[-1]
    assert request.messages == (SYSTEM, user("Count the files."))


async def test_the_task_of_a_subagent_is_what_it_is_given_as_its_user(harness: Harness) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task("Count the files."))

    (given,) = of(child_of(harness, parent), UserMessageEvent)
    assert given == UserMessageEvent(message=user("Count the files."), source="user")


async def test_a_subagent_does_not_have_the_tool_that_hands_tasks_on(harness: Harness) -> None:
    subagents = await loaded(harness)
    tools(harness, "read", TOOL, "grep")
    parent = await harness.agent()
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task())

    assert [tool.name for tool in harness.model.requests[-1].tools] == ["read", "grep"]


async def test_a_kind_gives_a_subagent_its_tools_its_prompt_and_its_model(
    harness: Harness, tmp_path: Path
) -> None:
    home = tmp_path / "home" / "agents"
    subagents = await loaded(harness, home=home)
    tools(harness, "read", "grep", "write")
    a_kind(home, "counter", "tools: [grep]", "model: small-model", prompt="You count things.")
    parent = await harness.agent()
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task(kind="counter"))

    request = harness.model.requests[-1]
    assert [tool.name for tool in request.tools] == ["grep"]
    assert request.config.model == "small-model"
    assert request.messages[0].content[0].text == "You count things."  # type: ignore[union-attr]
    (started,) = of(parent.session, SubagentStartedEvent)
    assert started.kind == "counter"


async def test_a_subagent_that_takes_more_steps_than_it_has_is_a_failure(
    harness: Harness,
) -> None:
    subagents = await loaded(harness, max_steps=2)
    await harness.ctx.plugin(budget)
    tools(harness, "work")
    parent = await harness.agent()
    works = Reply(text="Still looking.", calls=(Call(name="work"),))
    harness.model.add(works, works, works)

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.reason == "blocked"
    assert "the turn has taken its 2 steps" in str(raised.value)
    assert raised.value.partial == "Still looking."
    (ended,) = of(parent.session, SubagentEndedEvent)
    assert (ended.reason, ended.detail, ended.steps) == (
        "blocked",
        "the turn has taken its 2 steps",
        2,
    )


# What it may do


async def test_what_a_subagent_may_do_is_written_into_its_log_before_anything_else(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.fs)
    await harness.ctx.plugin(columbina.sandbox)
    await harness.ctx.plugin(columbina.approval, {"policy": "ask"})
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.ctx.use(SANDBOX).set_mode(parent.session, "read-only")
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task())

    child = child_of(harness, parent)
    assert [event.data for event in child.events[:2]] == [
        SandboxModeEvent(mode="read-only"),
        # Nobody watches a subagent, so nobody is asked on its behalf.
        ApprovalPolicyEvent(policy="never"),
    ]
    # The parent goes by what it went by.
    assert harness.ctx.use(APPROVAL).policy(parent.session) == "ask"


async def test_a_subagent_has_the_mode_that_its_parent_has_at_that_moment(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.fs)
    await harness.ctx.plugin(columbina.sandbox, {"mode": "workspace-write"})
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="One."), Reply(text="Two."))

    await subagents.run(parent, task())
    harness.ctx.use(SANDBOX).set_mode(parent.session, "danger-full-access")
    await subagents.run(parent, task())

    first, second = of(parent.session, SubagentStartedEvent)
    modes = [
        of(harness.sessions.read(harness.cwd, started.session_id), SandboxModeEvent)
        for started in (first, second)
    ]
    assert modes == [
        [SandboxModeEvent(mode="workspace-write")],
        [SandboxModeEvent(mode="danger-full-access")],
    ]


async def test_a_program_without_a_sandbox_and_without_approval_runs_subagents_too(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="Three files."))

    assert await subagents.run(parent, task()) == "Three files."
    child = child_of(harness, parent)
    assert of(child, SandboxModeEvent) == []
    assert of(child, ApprovalPolicyEvent) == []


async def test_a_subagent_that_cannot_be_given_its_limits_is_not_left_running(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    await harness.ctx.plugin(columbina.approval)
    subagents = await loaded(harness)
    parent = await harness.agent()

    def broken(self: Approval, session: Session, policy: str) -> None:
        raise OSError("the log cannot be written")

    monkeypatch.setattr(Approval, "set_policy", broken)

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.code == "SUBAGENT_NOT_STARTED"
    assert "the log cannot be written" in str(raised.value)
    # No agent is left that may do more than it should.
    assert list(harness.agents.live) == [parent.id]
    assert harness.model.requests == []


async def test_an_agent_that_cannot_be_made_is_a_subagent_that_could_not_be_started(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()

    async def refused(*args: object, **kwargs: object) -> Agent:
        raise AgentError("there is no agent loop named 'default'")

    monkeypatch.setattr(Agents, "create", refused)

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.code == "SUBAGENT_NOT_STARTED"
    assert "there is no agent loop named 'default'" in str(raised.value)


# What it gives back


async def test_the_last_thing_a_subagent_said_that_is_not_empty_is_handed_back(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    tools(harness, "work")
    parent = await harness.agent()
    harness.model.add(
        Reply(text="Let me look."),
    )
    assert await subagents.run(parent, task()) == "Let me look."

    harness.model.add(
        Reply(text="There are three files.", calls=(Call(name="work"),)),
        # The last reply holds nothing, so the one before it is the answer.
        Reply(text="  "),
    )
    assert await subagents.run(parent, task()) == "There are three files."


async def test_a_subagent_whose_request_failed_is_a_failure_with_what_had_arrived(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(
        Reply(text="I had found two", failure=LlmFailure(message="it broke", code="SERVER"))
    )

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.reason == "error"
    assert "it broke" in str(raised.value)
    assert raised.value.partial == "I had found two"


async def test_a_subagent_whose_reply_was_cut_off_is_a_failure_with_what_had_arrived(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="There are thr", finish="max-tokens"))

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.reason == "max-tokens"
    assert raised.value.partial == "There are thr"


async def test_a_subagent_that_somebody_else_stopped_is_a_failure(harness: Harness) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()

    async def stops(args: Args, run: ToolRun) -> str:
        harness.agents.live[run.session.id].cancel("hook")
        await asyncio.sleep(60)
        return "never"

    harness.tools.add(
        harness.ctx, Tool(name="halt", description="Stop.", parameters=Args, execute=stops)
    )
    harness.model.add(Reply(text="I will stop.", calls=(Call(name="halt"),)))

    with pytest.raises(SubagentFailed) as raised:
        await asyncio.wait_for(subagents.run(parent, task()), timeout=5)

    assert raised.value.reason == "aborted"
    assert "its turn was stopped (hook)" in str(raised.value)
    assert raised.value.partial == "I will stop."


async def test_a_subagent_that_said_nothing_at_all_is_a_failure(harness: Harness) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    # A reply of nothing but blanks is a reply, and no answer.
    harness.model.add(Reply(text="  "))

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.reason == "completed"
    assert "without an answer" in str(raised.value)


def test_a_log_without_the_end_of_a_turn_has_no_reason(harness: Harness) -> None:
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id="s-open")

    outcome = outcome_of(session)

    assert (outcome.reason, outcome.detail, outcome.text, outcome.steps) == (None, None, "", 0)
    assert outcome.usage == Usage()


# What its parent keeps of it


async def test_what_a_subagent_cost_is_counted_for_its_parent(harness: Harness) -> None:
    subagents = await loaded(harness)
    tools(harness, "work")
    parent = await harness.agent()
    harness.model.add(
        Reply(calls=(Call(name="work"),), usage=Usage(input_tokens=100, output_tokens=10)),
        Reply(text="Three.", usage=Usage(input_tokens=5, output_tokens=3, cache_read_tokens=100)),
    )

    await subagents.run(parent, task())

    spent = Usage(input_tokens=105, output_tokens=13, cache_read_tokens=100)
    assert of(parent.session, AsideEvent) == [AsideEvent(purpose="subagent", usage=spent)]
    assert USAGE.of(parent.session).usage == spent
    (ended,) = of(parent.session, SubagentEndedEvent)
    assert (ended.reason, ended.steps) == ("completed", 2)


async def test_what_listens_to_the_parent_does_not_hear_the_subagent(harness: Harness) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    heard: list[Committed] = []
    parent.ctx.on(SESSION_EVENT, heard.append)
    harness.model.add(Reply(text="Three files."))

    await subagents.run(parent, task())

    assert {committed.session.id for committed in heard} == {parent.id}
    assert [committed.event.type for committed in heard] == ["subagent/started", "subagent/ended"]


# When the one who asked no longer waits


async def test_a_subagent_is_stopped_for_its_parent_and_closed_when_the_call_is_cancelled(
    harness: Harness,
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="I have found", hold=True))
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await asyncio.wait_for(harness.model.held(), timeout=5)

        run.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(run, timeout=5)

        child = child_of(harness, parent)
        assert of(child, TurnEndEvent) == [TurnEndEvent(turn=1, reason="aborted", detail="parent")]
        assert list(harness.agents.live) == [parent.id]
        (ended,) = of(parent.session, SubagentEndedEvent)
        assert (ended.reason, ended.detail) == ("aborted", "parent")
    finally:
        harness.model.release()
        run.cancel()


async def test_a_subagent_ends_with_its_parent(harness: Harness) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()
    harness.model.add(Reply(text="I have found", hold=True))
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await asyncio.wait_for(harness.model.held(), timeout=5)
        (started,) = of(parent.session, SubagentStartedEvent)

        await asyncio.wait_for(parent.close(), timeout=5)

        with pytest.raises(SubagentFailed) as raised:
            await asyncio.wait_for(run, timeout=5)
        assert raised.value.reason == "aborted"
        assert list(harness.agents.live) == []
        child = harness.sessions.read(harness.cwd, started.session_id)
        assert [end.reason for end in of(child, TurnEndEvent)] == ["aborted"]
    finally:
        harness.model.release()
        run.cancel()


# How the end of a turn is read


def said(text: str) -> AssistantMessage:
    return AssistantMessage(content=(TextBlock(text=text),))


def test_the_answer_of_a_turn_that_came_to_its_end_is_a_reply_and_no_attempt(
    harness: Harness,
) -> None:
    failure = LlmFailure(message="it broke", code="SERVER")
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id="s-made")
    session.append(TurnStartEvent(turn=1))
    session.append(
        AssistantMessageEvent(message=said("The answer."), usage=Usage(), finish_reason="stop")
    )
    session.append(AssistantAttemptEvent(message=said("Half a"), usage=Usage(), failure=failure))
    session.append(TurnEndEvent(turn=1, reason="completed"))

    assert outcome_of(session).text == "The answer."


def test_what_a_turn_that_failed_had_written_is_the_last_that_arrived(harness: Harness) -> None:
    failure = LlmFailure(message="it broke", code="SERVER")
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id="s-made")
    session.append(TurnStartEvent(turn=1))
    session.append(
        AssistantMessageEvent(message=said("A first reply."), usage=Usage(), finish_reason="stop")
    )
    session.append(AssistantAttemptEvent(message=said("Half a"), usage=Usage(), failure=failure))
    session.append(AssistantAttemptEvent(message=said(""), usage=Usage(), failure=failure))
    session.append(TurnEndEvent(turn=1, reason="error", detail="it broke"))

    outcome = outcome_of(session)

    assert (outcome.reason, outcome.detail, outcome.text) == ("error", "it broke", "Half a")


async def test_a_subagent_that_stopped_for_good_is_a_failure_and_no_error(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    subagents = await loaded(harness)
    parent = await harness.agent()

    async def stopped(self: Agent) -> None:
        raise RuntimeError("a turn could not be written down")

    monkeypatch.setattr(type(parent), "when_idle", stopped)

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    # Its log says how far it came, and that was nowhere.
    assert raised.value.reason is None
    assert "it ended without its turn having come to an end" in str(raised.value)
