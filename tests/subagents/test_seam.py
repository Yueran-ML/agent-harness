"""Handing a task to a subagent: what is decided before one starts, and what is made of its end."""

import asyncio
import logging
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, TypeVar, cast

import pytest
from pydantic import BaseModel, ValidationError

import columbina.session
import columbina.subagents
from columbina.agent import Agent, AgentSpec
from columbina.fs import bound
from columbina.kernel import Context
from columbina.llm import CallConfig, Usage
from columbina.session import (
    SESSIONS,
    AsideEvent,
    EventData,
    Session,
    SessionStoreError,
    TurnEndReason,
)
from columbina.subagents import (
    SUBAGENTS,
    TOOL,
    Order,
    Outcome,
    SubagentEndedEvent,
    SubagentFailed,
    Subagents,
    SubagentStartedEvent,
    Task,
)
from columbina.tools import Tool, ToolRun
from support.loop import Harness

DONE = Outcome(reason="completed", detail=None, text="Three files.", steps=2, usage=Usage())

EventT = TypeVar("EventT", bound=EventData)


class Args(BaseModel):
    pass


class Child:
    """A subagent that a test holds the strings of."""

    def __init__(self, session_id: str, outcome: Outcome, gate: asyncio.Event | None) -> None:
        self.session_id = session_id
        self.stopped: list[str] = []
        self.closed = False
        self.breaks: Exception | None = None
        # What a subagent that was told to stop waits for before it has ended.
        self.winding_down: asyncio.Event | None = None
        self._outcome = outcome
        self._gate = gate

    async def ended(self) -> Outcome:
        if self.breaks is not None:
            raise self.breaks
        if self.stopped:
            if self.winding_down is not None:
                await self.winding_down.wait()
            return replace(self._outcome, reason="aborted", detail=self.stopped[-1])
        if self._gate is not None:
            await self._gate.wait()
        return self._outcome

    def stop(self, cause: str) -> None:
        self.stopped.append(cause)

    async def close(self) -> None:
        self.closed = True


@dataclass
class Provider:
    """Starts the subagents of a test, and writes down what it was asked for."""

    outcome: Outcome = DONE
    gate: asyncio.Event | None = None
    fails_with: Exception | None = None
    orders: list[Order] = field(default_factory=list[Order])
    children: list[Child] = field(default_factory=list[Child])

    async def start(self, parent: Agent, order: Order) -> Child:
        self.orders.append(order)
        if self.fails_with is not None:
            raise self.fails_with
        child = Child(f"child-{len(self.orders)}", self.outcome, self.gate)
        self.children.append(child)
        return child


async def nothing(args: Args, run: ToolRun) -> str:
    return "done"


def tools(harness: Harness, *names: str) -> None:
    """Give the program tools of these names."""
    for name in names:
        harness.tools.add(
            harness.ctx, Tool(name=name, description="x", parameters=Args, execute=nothing)
        )


async def loaded(harness: Harness, provider: Provider | None, **config: object) -> Subagents:
    await harness.ctx.plugin(columbina.subagents, config)
    subagents = harness.ctx.use(SUBAGENTS)
    if provider is not None:
        subagents.providers.add(harness.ctx, "spawn", provider)
    return subagents


def task(kind: str | None = None, call_id: str = "call-1") -> Task:
    return Task(call_id=call_id, description="count files", prompt="Count the files.", kind=kind)


def of(session: Session, kind: type[EventT]) -> list[EventT]:
    return [event.data for event in session.events if isinstance(event.data, kind)]


def a_kind(root: Path, name: str, *lines: str, prompt: str = "") -> None:
    """Write the file of a kind of subagent into a directory of kinds."""
    root.mkdir(parents=True, exist_ok=True)
    head = "\n".join(["---", f"name: {name}", f"description: The kind {name}.", *lines, "---"])
    (root / f"{name}.md").write_text(f"{head}\n{prompt}", encoding="utf-8")


def of_the_project(harness: Harness) -> Path:
    return harness.cwd / ".columbina" / "agents"


async def settled() -> None:
    """Let everything that can go on without anybody's help go on."""
    for _ in range(20):
        await asyncio.sleep(0)


# What is handed back


async def test_what_a_subagent_answered_is_handed_back(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    parent = await harness.agent()

    assert await subagents.run(parent, task()) == "Three files."
    assert [child.closed for child in provider.children] == [True]


async def test_the_parent_is_told_of_the_subagent_in_its_log(harness: Harness) -> None:
    spent = Usage(input_tokens=120, output_tokens=30, cache_read_tokens=400)
    provider = Provider(outcome=replace(DONE, usage=spent))
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    before = len(parent.session.events)

    await subagents.run(parent, task())

    assert [event.data for event in parent.session.events[before:]] == [
        SubagentStartedEvent(
            call_id="call-1", session_id="child-1", kind=None, description="count files"
        ),
        SubagentEndedEvent(
            call_id="call-1", session_id="child-1", reason="completed", detail=None, steps=2
        ),
        AsideEvent(purpose="subagent", usage=spent),
    ]


async def test_a_subagent_that_asked_no_model_costs_the_parent_nothing(harness: Harness) -> None:
    subagents = await loaded(harness, Provider())
    parent = await harness.agent()

    await subagents.run(parent, task())

    assert of(parent.session, AsideEvent) == []


@pytest.mark.parametrize(
    ("reason", "detail", "said"),
    [
        ("aborted", "user", "its turn was stopped (user)"),
        ("blocked", "the turn has taken its 50 steps", "the turn has taken its 50 steps"),
        ("error", "RATE_LIMIT: too many requests", "RATE_LIMIT: too many requests"),
        ("max-tokens", None, "a reply of its model was cut off at the limit of its length"),
        (None, None, "it ended without its turn having come to an end"),
        ("aborted", None, "its turn was stopped (nobody said by whom)"),
        ("blocked", None, "its turn was ended before it was over"),
        ("error", None, "a request to its model failed"),
        ("interrupted", None, "its turn ended as interrupted"),
    ],
)
async def test_a_subagent_that_did_not_finish_is_a_failure_that_says_how_it_ended(
    harness: Harness, reason: str | None, detail: str | None, said: str
) -> None:
    how = cast("TurnEndReason | None", reason)
    outcome = Outcome(reason=how, detail=detail, text="I had found two.", steps=3, usage=Usage())
    provider = Provider(outcome=outcome)
    subagents = await loaded(harness, provider)
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    failed = raised.value
    assert failed.code == "SUBAGENT_FAILED"
    assert failed.reason == reason
    assert failed.partial == "I had found two."
    assert said in str(failed)
    # What it had written is not passed off as an answer.
    assert str(failed).endswith("It is not a finished answer.\n\nI had found two.")
    assert [child.closed for child in provider.children] == [True]
    ended = of(parent.session, SubagentEndedEvent)
    assert ended == [
        SubagentEndedEvent(
            call_id="call-1",
            session_id="child-1",
            reason=reason or "unknown",
            detail=detail,
            steps=3,
        )
    ]


async def test_a_failure_of_a_subagent_that_wrote_nothing_has_nothing_after_it(
    harness: Harness,
) -> None:
    outcome = Outcome(reason="max-tokens", detail=None, text="", steps=1, usage=Usage())
    subagents = await loaded(harness, Provider(outcome=outcome))
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.partial == ""
    assert "finished answer" not in str(raised.value)
    assert str(raised.value).endswith("length.")


async def test_a_subagent_that_completed_its_turn_without_a_word_is_a_failure(
    harness: Harness,
) -> None:
    outcome = Outcome(reason="completed", detail=None, text="", steps=4, usage=Usage())
    subagents = await loaded(harness, Provider(outcome=outcome))
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.code == "SUBAGENT_FAILED"
    assert raised.value.reason == "completed"
    assert "without an answer" in str(raised.value)


async def test_what_a_failed_subagent_cost_is_counted_all_the_same(harness: Harness) -> None:
    spent = Usage(input_tokens=7)
    outcome = Outcome(reason="error", detail="it broke", text="", steps=1, usage=spent)
    subagents = await loaded(harness, Provider(outcome=outcome))
    parent = await harness.agent()

    with pytest.raises(SubagentFailed):
        await subagents.run(parent, task())

    assert of(parent.session, AsideEvent) == [AsideEvent(purpose="subagent", usage=spent)]


# What a subagent is made from


async def test_a_subagent_is_made_like_its_parent_but_for_the_tool_that_hands_tasks_on(
    harness: Harness,
) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    tools(harness, "read", TOOL, "grep")
    parent = await harness.agent(system_prompt="You are the parent.")

    await subagents.run(parent, task())

    (order,) = provider.orders
    assert order.task == task()
    assert order.depth == 1
    assert order.spec == AgentSpec(
        cwd=harness.cwd,
        model=parent.spec.model,
        system_prompt="You are the parent.",
        tools=frozenset({"read", "grep"}),
        max_steps=50,
    )


async def test_the_limit_of_steps_is_the_one_of_the_config(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider, max_steps=7)
    parent = await harness.agent()

    await subagents.run(parent, task())

    assert provider.orders[0].spec.max_steps == 7


async def test_a_kind_gives_a_subagent_its_tools_its_prompt_and_fewer_steps(
    harness: Harness,
) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    tools(harness, "read", "grep", "write")
    a_kind(
        of_the_project(harness),
        "reviewer",
        "tools: [read, grep]",
        "max_steps: 12",
        prompt="You review a change.\n",
    )
    parent = await harness.agent()

    await subagents.run(parent, task(kind="reviewer"))

    spec = provider.orders[0].spec
    assert spec.tools == frozenset({"read", "grep"})
    assert spec.system_prompt == "You review a change."
    assert spec.max_steps == 12
    assert spec.model == parent.spec.model
    assert of(parent.session, SubagentStartedEvent)[0].kind == "reviewer"


async def test_a_kind_cannot_name_the_tool_that_hands_tasks_on(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    tools(harness, "read", TOOL)
    a_kind(of_the_project(harness), "helper", f"tools: [read, {TOOL}]")
    parent = await harness.agent()

    await subagents.run(parent, task(kind="helper"))

    assert provider.orders[0].spec.tools == frozenset({"read"})


async def test_a_kind_without_a_prompt_of_its_own_has_the_one_of_the_parent(
    harness: Harness,
) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    a_kind(of_the_project(harness), "plain")
    parent = await harness.agent(system_prompt="You are the parent.")

    await subagents.run(parent, task(kind="plain"))

    spec = provider.orders[0].spec
    assert spec.system_prompt == "You are the parent."
    assert spec.tools == frozenset()
    assert spec.max_steps == 50


async def test_a_kind_cannot_give_a_subagent_more_steps_than_the_config_allows(
    harness: Harness,
) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider, max_steps=20)
    a_kind(of_the_project(harness), "long", "max_steps: 400")
    parent = await harness.agent()

    await subagents.run(parent, task(kind="long"))

    assert provider.orders[0].spec.max_steps == 20


async def test_a_kind_of_the_user_may_name_the_model_and_one_of_a_project_may_not(
    harness: Harness, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    home = tmp_path / "home" / "agents"
    provider = Provider()
    subagents = await loaded(harness, provider, home=home)
    a_kind(home, "thinker", "model: big-model")
    a_kind(of_the_project(harness), "spender", "model: dear-model")
    parent = await harness.agent()

    await subagents.run(parent, task(kind="thinker"))
    await subagents.run(parent, task(kind="spender", call_id="call-2"))
    await subagents.run(parent, task(kind="spender", call_id="call-3"))

    own, foreign, again = (order.spec.model for order in provider.orders)
    assert own == parent.spec.model.model_copy(update={"model": "big-model"})
    # Whose money a model costs is not for a project to decide.
    assert foreign == parent.spec.model == again
    # The user is told so, once.
    assert caplog.text.count("dear-model") == 1
    assert "big-model" not in caplog.text


async def test_a_kind_of_a_project_comes_before_one_of_the_same_name_of_the_user(
    harness: Harness, tmp_path: Path
) -> None:
    home = tmp_path / "home" / "agents"
    provider = Provider()
    subagents = await loaded(harness, provider, home=home)
    a_kind(home, "reviewer", prompt="From the home.")
    a_kind(of_the_project(harness), "reviewer", prompt="From the project.")
    parent = await harness.agent()

    await subagents.run(parent, task(kind="reviewer"))

    assert provider.orders[0].spec.system_prompt == "From the project."


async def test_a_kind_that_is_not_there_is_refused_with_the_kinds_that_are(
    harness: Harness,
) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    a_kind(of_the_project(harness), "reviewer")
    a_kind(of_the_project(harness), "builder")
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task(kind="tester"))

    assert raised.value.code == "UNKNOWN_SUBAGENT_TYPE"
    assert str(raised.value) == (
        "There is no kind of subagent named 'tester'. The kinds are: builder, reviewer. "
        "Leave `type` out for a subagent that is like you."
    )
    assert provider.orders == []
    assert of(parent.session, SubagentStartedEvent) == []


async def test_a_kind_is_refused_where_there_are_none(harness: Harness) -> None:
    subagents = await loaded(harness, Provider())
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task(kind="tester"))

    assert str(raised.value) == (
        "There is no kind of subagent named 'tester'. There are no kinds. "
        "Leave `type` out for a subagent that is like you."
    )


async def test_a_kind_that_names_a_tool_which_is_not_there_is_no_kind(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    a_kind(of_the_project(harness), "reviewer", "tools: [telescope]")
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task(kind="reviewer"))

    assert raised.value.code == "UNKNOWN_SUBAGENT_TYPE"
    found = await subagents.found(harness.cwd)
    assert dict(found.types) == {}
    reasons = [one.reason for one in found.invalid]
    assert reasons == ["it names a tool that is not there: 'telescope'"]


# Who may hand a task on, and to whom


async def test_a_subagent_cannot_hand_a_task_on(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    child = await harness.agents.create(
        parent.ctx, harness.spec(), session_id="s-child", parent=parent.id, delegation_depth=1
    )

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(child, task())

    assert raised.value.code == "SUBAGENT_TOO_DEEP"
    assert str(raised.value) == (
        "A subagent cannot hand its task on any further. Do the task yourself."
    )
    assert provider.orders == []


async def test_how_deep_tasks_may_be_handed_on_is_a_matter_of_the_config(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider, max_depth=2)
    parent = await harness.agent()
    child = await harness.agents.create(
        parent.ctx, harness.spec(), session_id="s-child", parent=parent.id, delegation_depth=1
    )

    await subagents.run(child, task())

    assert provider.orders[0].depth == 2


async def test_without_a_provider_no_task_is_handed_over(harness: Harness) -> None:
    subagents = await loaded(harness, None)
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.code == "NO_SUBAGENTS"
    assert "'spawn'" in str(raised.value)


async def test_a_subagent_that_cannot_be_started_is_a_failure_and_nothing_is_written(
    harness: Harness,
) -> None:
    provider = Provider(fails_with=OSError("the disk is full"))
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    before = len(parent.session.events)

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value.code == "SUBAGENT_NOT_STARTED"
    assert "the disk is full" in str(raised.value)
    assert len(parent.session.events) == before


# How many at once


async def test_no_more_subagents_run_at_once_than_the_config_allows(harness: Harness) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider, max_running=2)
    parent = await harness.agent()

    runs = [
        asyncio.ensure_future(subagents.run(parent, task(call_id=f"call-{number}")))
        for number in (1, 2, 3)
    ]
    try:
        await settled()
        # The third one waits for a place before anything of it is made.
        assert [order.task.call_id for order in provider.orders] == ["call-1", "call-2"]

        gate.set()
        assert await asyncio.wait_for(asyncio.gather(*runs), timeout=5) == ["Three files."] * 3
        assert [order.task.call_id for order in provider.orders] == ["call-1", "call-2", "call-3"]
    finally:
        gate.set()
        for run in runs:
            run.cancel()


async def test_the_place_of_a_subagent_that_could_not_be_started_is_free_again(
    harness: Harness,
) -> None:
    provider = Provider(fails_with=OSError("no"))
    subagents = await loaded(harness, provider, max_running=1)
    parent = await harness.agent()
    with pytest.raises(SubagentFailed):
        await subagents.run(parent, task())

    provider.fails_with = None

    assert await asyncio.wait_for(subagents.run(parent, task()), timeout=5) == "Three files."


async def test_the_place_of_a_subagent_that_failed_is_free_again(harness: Harness) -> None:
    provider = Provider(outcome=replace(DONE, reason="error", detail="it broke"))
    subagents = await loaded(harness, provider, max_running=1)
    parent = await harness.agent()
    with pytest.raises(SubagentFailed):
        await subagents.run(parent, task())

    provider.outcome = DONE

    assert await asyncio.wait_for(subagents.run(parent, task()), timeout=5) == "Three files."


# When the one who asked no longer waits


async def test_a_subagent_is_stopped_for_its_parent_when_the_call_is_cancelled(
    harness: Harness,
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate, outcome=replace(DONE, usage=Usage(output_tokens=5)))
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await settled()
        (child,) = provider.children

        run.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(run, timeout=5)

        assert child.stopped == ["parent"]
        assert child.closed
        assert of(parent.session, SubagentEndedEvent) == [
            SubagentEndedEvent(
                call_id="call-1", session_id="child-1", reason="aborted", detail="parent", steps=2
            )
        ]
        # What it had cost until then is counted.
        assert of(parent.session, AsideEvent) == [
            AsideEvent(purpose="subagent", usage=Usage(output_tokens=5))
        ]
    finally:
        gate.set()
        run.cancel()


async def test_a_call_that_is_cancelled_while_it_waits_for_a_place_starts_nothing(
    harness: Harness,
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider, max_running=1)
    parent = await harness.agent()
    first = asyncio.ensure_future(subagents.run(parent, task(call_id="call-1")))
    second = asyncio.ensure_future(subagents.run(parent, task(call_id="call-2")))
    try:
        await settled()

        second.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(second, timeout=5)
        gate.set()

        assert await asyncio.wait_for(first, timeout=5) == "Three files."
        assert [order.task.call_id for order in provider.orders] == ["call-1"]
        started = of(parent.session, SubagentStartedEvent)
        assert [event.call_id for event in started] == ["call-1"]
    finally:
        gate.set()
        first.cancel()
        second.cancel()


async def test_a_parent_whose_session_is_closed_is_told_nothing_and_nothing_breaks(
    harness: Harness,
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await settled()
        parent.session.close()
        gate.set()

        assert await asyncio.wait_for(run, timeout=5) == "Three files."
        assert [child.closed for child in provider.children] == [True]
    finally:
        gate.set()
        run.cancel()


# The config


@pytest.mark.parametrize(
    "config",
    [
        {"max_running": 0},
        {"max_depth": 0},
        {"max_steps": 0},
        {"home": "agents"},
        {"provider": ""},
    ],
)
def test_a_config_that_makes_no_sense_is_refused(config: dict[str, object]) -> None:
    with pytest.raises(ValidationError):
        columbina.subagents.Config.model_validate(config)


# What else there is to hold on to


async def test_a_subagent_never_has_a_tool_that_its_parent_has_not(harness: Harness) -> None:
    provider = Provider()
    subagents = await loaded(harness, provider)
    tools(harness, "read", "grep", "write")
    a_kind(of_the_project(harness), "searcher", "tools: [read, grep]")
    parent = await harness.agent(tools=frozenset({"read", "write"}))

    await subagents.run(parent, task())
    await subagents.run(parent, task(kind="searcher", call_id="call-2"))

    plain, of_a_kind = (order.spec.tools for order in provider.orders)
    assert plain == frozenset({"read", "write"})
    assert of_a_kind == frozenset({"read"})


async def test_half_a_character_in_the_words_about_a_task_does_not_stop_it(
    harness: Harness,
) -> None:
    subagents = await loaded(harness, Provider())
    parent = await harness.agent()
    cut = Task(call_id="call-1", description="count " + chr(0xD83D), prompt="Count the files.")

    assert await subagents.run(parent, cut) == "Three files."

    (started,) = of(parent.session, SubagentStartedEvent)
    assert started.description == "count " + chr(0xFFFD)


async def test_what_a_provider_says_of_a_subagent_that_it_cannot_start_is_passed_on(
    harness: Harness,
) -> None:
    refusal = SubagentFailed("This provider starts nothing on a Sunday.", code="CLOSED")
    subagents = await loaded(harness, Provider(fails_with=refusal))
    parent = await harness.agent()

    with pytest.raises(SubagentFailed) as raised:
        await subagents.run(parent, task())

    assert raised.value is refusal


async def test_what_is_wrong_with_the_kinds_is_written_to_the_log_once(
    harness: Harness, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    home = tmp_path / "home" / "agents"
    subagents = await loaded(harness, Provider(), home=home)
    a_kind(of_the_project(harness), "reviewer")
    a_kind(of_the_project(harness), "broken", "tools: yes")
    a_kind(home, "reviewer")
    a_kind(home, "builder")

    with caplog.at_level(logging.INFO, logger="columbina.subagents"):
        first = await subagents.found(harness.cwd)
        second = await subagents.found(harness.cwd)

    assert list(first.types) == ["builder", "reviewer"] == list(second.types)
    said = [record.getMessage() for record in caplog.records]
    assert [line for line in said if "is no kind of subagent that can be used" in line] == [
        f"{of_the_project(harness) / 'broken.md'} is no kind of subagent that can be used: "
        "the key 'tools' gives True, which is no list of the names of tools. Write it like "
        "[read, grep], or leave the key out for all tools"
    ]
    assert [line for line in said if "passed over" in line] == [
        f"the kind in {home / 'reviewer.md'} is passed over for one of the same name"
    ]


async def test_a_place_that_cannot_be_looked_into_holds_nothing_this_time(
    harness: Harness,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    home = tmp_path / "home" / "agents"
    subagents = await loaded(harness, Provider(), home=home)
    a_kind(of_the_project(harness), "reviewer")
    a_kind(home, "builder")
    listing = bound.listing

    def refuses(path: Path, at_most: int) -> Any:
        if "home" in path.parts:
            raise PermissionError(13, "Permission denied")
        return listing(path, at_most)

    monkeypatch.setattr(bound, "listing", refuses)

    first = await subagents.found(harness.cwd)
    second = await subagents.found(harness.cwd)

    # The kinds of the other place are there all the same.
    assert list(first.types) == ["reviewer"] == list(second.types)
    said = [record.getMessage() for record in caplog.records]
    assert said == [f"the kinds of subagent in {home} cannot be looked for now: Permission denied"]


async def test_four_subagents_run_at_once_unless_the_config_says_otherwise(
    harness: Harness,
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    runs = [
        asyncio.ensure_future(subagents.run(parent, task(call_id=f"call-{number}")))
        for number in range(1, 6)
    ]
    try:
        await settled()
        assert len(provider.orders) == 4

        gate.set()
        await asyncio.wait_for(asyncio.gather(*runs), timeout=5)
        assert len(provider.orders) == 5
    finally:
        gate.set()
        for run in runs:
            run.cancel()


async def test_a_program_without_subagents_can_read_the_log_of_a_parent(
    harness: Harness, tmp_path: Path
) -> None:
    subagents = await loaded(harness, Provider())
    parent = await harness.agent()
    await subagents.run(parent, task())
    await parent.session.flush()

    other = Context()
    try:
        await other.plugin(columbina.session, {"root": tmp_path / "sessions"})
        stored = other.use(SESSIONS).read(harness.cwd, parent.id)
    finally:
        await asyncio.wait_for(other.fiber.dispose(), timeout=5)

    # It does not know the two events, and they are none that it has to know.
    assert [event.type for event in stored.events][-2:] == ["subagent/started", "subagent/ended"]


async def test_a_subagent_of_which_nobody_can_say_how_it_ended_is_closed_and_written_down(
    harness: Harness,
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await settled()
        (child,) = provider.children
        child.breaks = RuntimeError("its log cannot be read")
        run.cancel()

        # What went wrong while the subagent was stopped is what whoever asked is told.
        with pytest.raises(RuntimeError, match="its log cannot be read"):
            await asyncio.wait_for(run, timeout=5)

        assert child.stopped == ["parent"]
        assert child.closed
        assert of(parent.session, SubagentEndedEvent) == [
            SubagentEndedEvent(
                call_id="call-1", session_id="child-1", reason="unknown", detail=None, steps=0
            )
        ]
        assert of(parent.session, AsideEvent) == []
    finally:
        gate.set()
        run.cancel()


# What a review found


async def test_a_subagent_whose_beginning_cannot_be_written_down_is_closed(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate = asyncio.Event()
    provider = Provider(gate=gate)
    subagents = await loaded(harness, provider, max_running=1)
    parent = await harness.agent()
    append = Session.append

    def full(self: Session, data: EventData, **more: Any) -> Any:
        if isinstance(data, SubagentStartedEvent):
            raise SessionStoreError("the disk is full")
        return append(self, data, **more)

    monkeypatch.setattr(Session, "append", full)

    with pytest.raises(SessionStoreError, match="the disk is full"):
        await asyncio.wait_for(subagents.run(parent, task()), timeout=5)

    # The subagent had begun to work by then. It does not go on by itself.
    (child,) = provider.children
    assert child.closed
    # No end is told of a subagent of which no beginning was told.
    assert of(parent.session, SubagentEndedEvent) == []
    # And its place is free for the next one.
    monkeypatch.undo()
    gate.set()
    again = subagents.run(parent, task(call_id="call-2"))
    assert await asyncio.wait_for(again, timeout=5) == "Three files."


async def test_a_call_that_is_cancelled_twice_still_writes_down_what_its_subagent_cost(
    harness: Harness,
) -> None:
    gate, winding_down = asyncio.Event(), asyncio.Event()
    provider = Provider(gate=gate, outcome=replace(DONE, usage=Usage(output_tokens=5)))
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await settled()
        (child,) = provider.children
        child.winding_down = winding_down

        run.cancel()
        await settled()
        # The subagent was told to stop and has not ended yet. Whoever waits is cancelled again.
        run.cancel()
        await settled()
        still_waiting = not run.done()
        winding_down.set()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(run, timeout=5)

        assert still_waiting
        assert child.stopped == ["parent"]
        assert child.closed
        assert of(parent.session, SubagentEndedEvent) == [
            SubagentEndedEvent(
                call_id="call-1", session_id="child-1", reason="aborted", detail="parent", steps=2
            )
        ]
        assert of(parent.session, AsideEvent) == [
            AsideEvent(purpose="subagent", usage=Usage(output_tokens=5))
        ]
    finally:
        gate.set()
        winding_down.set()
        run.cancel()


async def test_a_subagent_that_does_not_end_when_it_is_told_to_stop_is_given_up(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(columbina.subagents, "STOP_SECONDS", 0.05)
    gate, never = asyncio.Event(), asyncio.Event()
    provider = Provider(gate=gate, outcome=replace(DONE, usage=Usage(output_tokens=5)))
    subagents = await loaded(harness, provider)
    parent = await harness.agent()
    run = asyncio.ensure_future(subagents.run(parent, task()))
    try:
        await settled()
        (child,) = provider.children
        child.winding_down = never

        run.cancel()
        await settled()
        run.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(run, timeout=5)

        # Nobody knows how it ended or what it cost. It is closed all the same.
        assert child.closed
        assert of(parent.session, SubagentEndedEvent) == [
            SubagentEndedEvent(
                call_id="call-1", session_id="child-1", reason="unknown", detail=None, steps=0
            )
        ]
        assert of(parent.session, AsideEvent) == []
    finally:
        gate.set()
        never.set()
        run.cancel()


def test_a_subagent_that_was_told_to_stop_is_waited_for_ten_seconds() -> None:
    assert columbina.subagents.STOP_SECONDS == 10.0


async def test_what_is_known_of_the_window_of_one_model_is_not_said_of_another(
    harness: Harness, tmp_path: Path
) -> None:
    home = tmp_path / "home" / "agents"
    provider = Provider()
    subagents = await loaded(harness, provider, home=home)
    a_kind(home, "thinker", "model: big-model")
    a_kind(home, "same", "model: script")
    wide = CallConfig(provider="scripted", model="script", context_window=1_000_000)
    parent = await harness.agent(model=wide)

    await subagents.run(parent, task(kind="thinker"))
    await subagents.run(parent, task(kind="same", call_id="call-2"))
    await subagents.run(parent, task(call_id="call-3"))

    another, the_same, like_the_parent = (order.spec.model for order in provider.orders)
    # A window of a million tokens is the window of the parent's model. Nothing says what
    # the other model takes, so nothing is said of it.
    assert another == CallConfig(provider="scripted", model="big-model")
    assert the_same == wide == like_the_parent
