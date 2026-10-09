"""The agent service: where agents are created, and what every agent offers."""

from pathlib import Path

import pytest

import columbina.agent
import columbina.session
from columbina.agent import (
    AGENTS,
    INBOX,
    Agent,
    AgentError,
    Agents,
    AgentSpec,
    CancelCause,
    Inbox,
    InboxItem,
)
from columbina.kernel import Context, Plugin
from columbina.llm import CallConfig, TextBlock, UserMessage
from columbina.session import (
    SESSION_EVENT,
    Committed,
    Session,
    SessionClosedError,
    TurnStartEvent,
    UnreadableSessionError,
)

MODEL = CallConfig(provider="scripted", model="script")


class Still(Agent):
    """An agent that never runs a turn, and remembers what it was asked."""

    def __init__(self, ctx: Context, spec: AgentSpec, session: Session) -> None:
        super().__init__(ctx, spec, session)
        self.woken = 0
        self.cancelled: list[tuple[CancelCause, bool]] = []
        ctx.effect(lambda: self.stopped)

    def stopped(self) -> None:
        self.cancelled.append(("disposed", False))

    def wake(self) -> None:
        self.woken += 1

    def cancel(self, cause: CancelCause = "user", *, keep_inbox: bool = False) -> None:
        self.cancelled.append((cause, keep_inbox))

    async def when_idle(self) -> None:
        return None


def item(text: str, source: str) -> InboxItem:
    return InboxItem(message=UserMessage(content=(TextBlock(text=text),)), source=source)


@pytest.fixture
async def agents(ctx: Context, tmp_path: Path) -> Agents:
    """The agent service, with sessions stored under a temporary directory and a still loop."""
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.agent)
    service = ctx.use(AGENTS)
    service.loops.add(ctx, "default", Still)
    return service


def spec(tmp_path: Path, **changes: object) -> AgentSpec:
    return AgentSpec.model_validate({"cwd": tmp_path, "model": MODEL, **changes})


async def test_an_agent_is_created_by_the_loop_that_its_spec_names(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path))

    assert isinstance(agent, Still)
    assert agent.spec == spec(tmp_path)
    assert agent.cwd == tmp_path
    assert agent.id == agent.session.id
    assert agents.live[agent.id] is agent


async def test_an_agent_can_be_given_the_id_of_its_session(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path), session_id="s-1")

    assert agent.id == "s-1"
    assert agent.session.id == "s-1"


async def test_a_spec_that_names_no_loop_gets_the_default_one(tmp_path: Path) -> None:
    assert spec(tmp_path).loop == "default"
    assert spec(tmp_path).system_prompt == ""


async def test_a_loop_that_is_not_there_cannot_create_an_agent(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    with pytest.raises(AgentError, match="there is no agent loop named 'careful'"):
        await agents.create(ctx, spec(tmp_path, loop="careful"))

    assert list(agents.live) == []


async def test_each_agent_hears_only_its_own_session(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    one = await agents.create(ctx, spec(tmp_path), session_id="s-1")
    other = await agents.create(ctx, spec(tmp_path), session_id="s-2")
    heard: list[str] = []

    def listen(committed: Committed) -> None:
        heard.append(committed.session.id)

    one.ctx.on(SESSION_EVENT, listen)

    other.session.append(TurnStartEvent(turn=1))
    one.session.append(TurnStartEvent(turn=1))

    assert heard == ["s-1"]


async def test_a_task_waits_for_the_next_turn_and_wakes_the_agent(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path))
    assert isinstance(agent, Still)

    agent.followup("Fix the bug.")

    assert INBOX.of(agent.session) == Inbox(next_turn=(item("Fix the bug.", "user"),))
    assert agent.woken == 1


async def test_a_steer_waits_for_the_next_step_and_wakes_the_agent(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path))
    assert isinstance(agent, Still)

    agent.steer("Use the other file.")

    assert INBOX.of(agent.session) == Inbox(next_step=(item("Use the other file.", "steer"),))
    assert agent.woken == 1


async def test_injected_context_waits_for_the_next_step_without_waking_the_agent(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path))
    assert isinstance(agent, Still)

    agent.inject("The tests were run a minute ago.")

    assert INBOX.of(agent.session) == Inbox(
        next_step=(item("The tests were run a minute ago.", "inject"),)
    )
    assert agent.woken == 0


async def test_input_can_say_where_it_comes_from(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path))

    agent.followup("Continue with the goal.", source="goal")
    agent.steer(UserMessage(content=(TextBlock(text="A"), TextBlock(text="B"))), source="hook")

    assert INBOX.of(agent.session) == Inbox(
        next_turn=(item("Continue with the goal.", "goal"),),
        next_step=(
            InboxItem(
                message=UserMessage(content=(TextBlock(text="A"), TextBlock(text="B"))),
                source="hook",
            ),
        ),
    )


async def test_closing_an_agent_stops_it_and_closes_its_session(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    agent = await agents.create(ctx, spec(tmp_path), session_id="s-1")
    assert isinstance(agent, Still)
    agent.session.append(TurnStartEvent(turn=1))

    await agent.close()

    assert agent.cancelled == [("disposed", False)]
    assert agent.session.closed
    assert list(agents.live) == []
    with pytest.raises(SessionClosedError):
        agent.followup("too late")


async def test_an_agent_goes_with_the_plugin_that_created_it(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    made: list[Agent] = []

    async def apply(inner: Context, config: object) -> None:
        made.append(await inner.use(AGENTS).create(inner, spec(tmp_path)))

    owner = await ctx.plugin(Plugin("owner", apply, inject=(AGENTS,)))
    assert list(agents.live) == [made[0].id]

    await owner.dispose()

    assert list(agents.live) == []
    assert made[0].session.closed


async def test_an_agent_can_take_up_a_stored_session(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    first = await agents.create(ctx, spec(tmp_path), session_id="s-1")
    first.session.append(TurnStartEvent(turn=1))
    first.followup("Fix the bug.")
    await first.close()

    second = await agents.resume(ctx, spec(tmp_path), "s-1")

    assert second.id == "s-1"
    assert second is not first
    assert second.session.events[0].data == TurnStartEvent(turn=1)
    assert INBOX.of(second.session).next_turn == (item("Fix the bug.", "user"),)
    assert agents.live["s-1"] is second


async def test_two_agents_cannot_share_a_session(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    await agents.create(ctx, spec(tmp_path), session_id="s-1")

    with pytest.raises(AgentError, match="an agent with the id 's-1' is running"):
        await agents.create(ctx, spec(tmp_path), session_id="s-1")

    assert list(agents.live) == ["s-1"]


async def test_an_agent_that_cannot_be_made_leaves_nothing_behind(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    def broken(scope: Context, spec: AgentSpec, session: Session) -> Agent:
        raise RuntimeError("the loop cannot start")

    agents.loops.add(ctx, "broken", broken)

    with pytest.raises(RuntimeError, match="the loop cannot start"):
        await agents.create(ctx, spec(tmp_path, loop="broken"), session_id="s-1")

    assert list(agents.live) == []
    agent = await agents.create(ctx, spec(tmp_path), session_id="s-1")
    agent.session.append(TurnStartEvent(turn=1))
    assert len(agent.session) == 1


async def test_a_session_that_cannot_be_resumed_leaves_nothing_behind(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    with pytest.raises(UnreadableSessionError):
        await agents.resume(ctx, spec(tmp_path), "s-1")

    assert list(agents.live) == []
    assert (await agents.create(ctx, spec(tmp_path), session_id="s-1")).id == "s-1"


async def test_an_id_that_cannot_name_a_session_is_refused(
    ctx: Context, agents: Agents, tmp_path: Path
) -> None:
    with pytest.raises(ValueError, match="is not a usable session id"):
        await agents.create(ctx, spec(tmp_path), session_id="../escape")
    with pytest.raises(ValueError, match="is not a usable session id"):
        await agents.resume(ctx, spec(tmp_path), "../escape")
