"""The tool that hands a task to a subagent, and what a model is told about subagents."""

import asyncio
from pathlib import Path

import pytest
from pydantic import BaseModel, JsonValue, ValidationError

import columbina.subagents
import columbina.subagents.spawn
from columbina.agent import Agent
from columbina.builtin_tools import subagent
from columbina.builtin_tools.subagent import MAX_DESCRIPTION, catalog
from columbina.llm import LlmRequest, ToolCallBlock, ToolMessage, UserMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.places import Root
from columbina.session import ToolResultEvent, TurnEndEvent
from columbina.subagents import TOOL, SubagentStartedEvent
from columbina.subagents.types import SubagentType
from columbina.tools import Tool, ToolRun
from support.loop import SYSTEM, Harness, context, idle, user

KINDS = (
    "These kinds of subagent are there. Name one as `type` when you call the tool `subagent`, "
    "or name none for one that has all your tools."
)
DELEGATED = (
    "You are a subagent. Another agent handed you this task, and it reads only your last "
    "reply: nothing else that you write or do reaches it. Nobody watches you work, so nobody "
    "can be asked anything, and a call that would need a yes from a person is refused. If "
    "something stays open or could not be done, say so in your last reply. Make that reply "
    "the whole answer to the task."
)


class Args(BaseModel):
    pass


async def loaded(harness: Harness, **config: object) -> None:
    await harness.ctx.plugin(columbina.subagents)
    await harness.ctx.plugin(columbina.subagents.spawn)
    await harness.ctx.plugin(subagent, config)


def hands_over(*tasks: tuple[str, str | None]) -> Reply:
    """A reply that hands each of these tasks to a subagent: the prompt and the kind."""
    calls: list[Call] = []
    for number, (prompt, kind) in enumerate(tasks, 1):
        arguments: dict[str, JsonValue] = {"description": f"task {number}", "prompt": prompt}
        if kind is not None:
            arguments["type"] = kind
        calls.append(Call(name=TOOL, arguments=arguments, call_id=f"call-{number}"))
    return Reply(calls=tuple(calls))


def results(agent: Agent) -> dict[str, tuple[str, str | None]]:
    """The results of the calls of an agent, by call: the text, and the code if it failed."""
    return {
        event.data.message.call_id: (
            "".join(block.text for block in event.data.message.content),
            None if event.data.error is None else event.data.error.code,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    }


def task_of(request: LlmRequest) -> str:
    """What the agent that asks was given as its user, after what it was told beside that."""
    given = [message for message in request.messages if isinstance(message, UserMessage)]
    return given[-1].content[0].text


def a_kind(harness: Harness, name: str, description: str, *lines: str) -> None:
    root = harness.cwd / ".columbina" / "agents"
    root.mkdir(parents=True, exist_ok=True)
    head = "\n".join(["---", f"name: {name}", f"description: {description}", *lines, "---"])
    (root / f"{name}.md").write_text(head + "\n", encoding="utf-8")


def kind(name: str, description: str) -> SubagentType:
    root = Root(Path("/kinds"), "home", own=True)
    return SubagentType(name, description, None, None, None, None, root.path / f"{name}.md", root)


async def turn(agent: Agent, text: str = "Go on.") -> None:
    agent.followup(text)
    await idle(agent)


# The tool


async def test_a_task_is_handed_to_a_subagent_and_its_answer_is_the_result(
    harness: Harness,
) -> None:
    await loaded(harness)
    parent = await harness.agent(
        hands_over(("Count the files.", None)),
        Reply(text="Three files."),
        Reply(text="The subagent counted three."),
    )

    await turn(parent)

    assert results(parent) == {"call-1": ("Three files.", None)}
    (started,) = [
        event.data
        for event in parent.session.events
        if isinstance(event.data, SubagentStartedEvent)
    ]
    assert (started.call_id, started.kind, started.description) == ("call-1", None, "task 1")
    # The parent goes on with the answer, and its own request has it as the result of the call.
    last = harness.model.requests[-1].messages[-1]
    assert isinstance(last, ToolMessage)
    assert last.content[0].text == "Three files."


async def test_the_tasks_of_one_reply_are_worked_on_side_by_side(harness: Harness) -> None:
    await loaded(harness)
    arrived: list[str] = []
    both = asyncio.Event()

    async def meet(args: Args, run: ToolRun) -> str:
        arrived.append(run.session.id)
        if len(arrived) == 2:
            both.set()
        # One subagent alone would wait here for ever.
        await asyncio.wait_for(both.wait(), timeout=5)
        return "met"

    harness.tools.add(
        harness.ctx, Tool(name="meet", description="Meet.", parameters=Args, execute=meet)
    )

    def works(request: LlmRequest) -> Reply:
        if isinstance(request.messages[-1], ToolMessage):
            return Reply(text=f"Done: {task_of(request)}")
        return Reply(calls=(Call(name="meet"),))

    parent = await harness.agent(
        hands_over(("Count the files.", None), ("Find the parser.", None)),
        works,
        works,
        works,
        works,
        Reply(text="Both are done."),
    )

    await turn(parent)

    assert results(parent) == {
        "call-1": ("Done: Count the files.", None),
        "call-2": ("Done: Find the parser.", None),
    }
    assert len(set(arrived)) == 2
    assert parent.id not in arrived


async def test_a_task_that_was_not_done_is_a_call_that_failed(harness: Harness) -> None:
    await loaded(harness)
    parent = await harness.agent(
        hands_over(("Count the files.", None)),
        Reply(text="There are thr", finish="max-tokens"),
        Reply(text="It did not finish."),
    )

    await turn(parent)

    text, code = results(parent)["call-1"]
    assert code == "SUBAGENT_FAILED"
    assert text == (
        "The subagent did not finish its task: a reply of its model was cut off at the limit "
        "of its length.\nWhat it had written until then follows. It is not a finished answer."
        "\n\nThere are thr"
    )


async def test_a_kind_that_is_not_there_is_a_call_that_failed(harness: Harness) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change.")
    parent = await harness.agent(
        hands_over(("Count the files.", "counter")), Reply(text="There is no such kind.")
    )

    await turn(parent)

    assert results(parent) == {
        "call-1": (
            "There is no kind of subagent named 'counter'. The kinds are: reviewer. "
            "Leave `type` out for a subagent that is like you.",
            "UNKNOWN_SUBAGENT_TYPE",
        )
    }


async def test_a_kind_that_is_named_is_the_kind_of_the_subagent(harness: Harness) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change.", "tools: []")
    parent = await harness.agent(
        hands_over(("Read the change.", "reviewer")),
        Reply(text="It is fine."),
        Reply(text="The reviewer says it is fine."),
    )

    await turn(parent)

    assert results(parent) == {"call-1": ("It is fine.", None)}
    # A subagent of that kind has no tool at all.
    assert harness.model.requests[1].tools == ()


async def test_a_task_of_nothing_is_refused_before_anything_is_started(harness: Harness) -> None:
    await loaded(harness)
    parent = await harness.agent(hands_over(("", None)), Reply(text="I will write a task."))

    await turn(parent)

    text, code = results(parent)["call-1"]
    assert code == "INVALID_ARGS"
    assert "prompt" in text
    assert len(harness.model.requests) == 2


async def test_a_call_for_a_session_without_an_agent_fails(harness: Harness) -> None:
    await loaded(harness)
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id="s-bare")
    call = ToolCallBlock(
        call_id="call-1", name=TOOL, arguments='{"description": "count", "prompt": "Count."}'
    )

    outcome = await harness.tools.run(harness.ctx, call, ToolRun("call-1", harness.cwd, session))

    assert outcome.error is not None
    assert outcome.error.code == "NO_AGENT"


async def test_a_subagent_has_longer_than_other_calls(harness: Harness) -> None:
    await loaded(harness)

    assert harness.tools.registry[TOOL].timeout_seconds == 1800


async def test_how_long_a_subagent_has_is_a_matter_of_the_config(harness: Harness) -> None:
    await loaded(harness, timeout_seconds=90)

    tool = harness.tools.registry[TOOL]
    assert tool.timeout_seconds == 90
    assert tool.effect == "none"
    assert tool.is_concurrency_safe(tool.parse('{"description": "x", "prompt": "y"}'))


@pytest.mark.parametrize("config", [{"timeout_seconds": 0}, {"catalog_max_chars": 499}])
def test_a_config_that_makes_no_sense_is_refused(config: dict[str, object]) -> None:
    with pytest.raises(ValidationError):
        subagent.Config.model_validate(config)


async def test_a_subagent_is_stopped_when_the_turn_of_its_parent_is(harness: Harness) -> None:
    await loaded(harness)
    parent = await harness.agent(
        hands_over(("Count the files.", None)), Reply(text="I have found", hold=True)
    )
    parent.followup("Go on.")
    try:
        await asyncio.wait_for(harness.model.held(), timeout=5)
        (started,) = [
            event.data
            for event in parent.session.events
            if isinstance(event.data, SubagentStartedEvent)
        ]

        parent.cancel("user")
        await idle(parent)

        child = harness.sessions.read(harness.cwd, started.session_id)
        ends = [event.data for event in child.events if isinstance(event.data, TurnEndEvent)]
        assert ends == [TurnEndEvent(turn=1, reason="aborted", detail="parent")]
        assert list(harness.agents.live) == [parent.id]
    finally:
        harness.model.release()


# What a model is told


async def test_an_agent_is_told_of_the_kinds_of_subagent_that_there_are(harness: Harness) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change   and says\n  what is wrong with it.")
    a_kind(harness, "builder", "Builds things.")
    parent = await harness.agent(Reply(text="Hello."))

    await turn(parent, "Hello.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        context(
            "agents",
            f"{KINDS}\n- builder: Builds things.\n"
            "- reviewer: Reads a change and says what is wrong with it.",
        ),
        user("Hello."),
    )


async def test_nothing_is_said_of_kinds_where_there_are_none(harness: Harness) -> None:
    await loaded(harness)
    parent = await harness.agent(Reply(text="Hello."))

    await turn(parent, "Hello.")

    assert harness.model.requests[0].messages == (SYSTEM, user("Hello."))


async def test_an_agent_is_told_when_the_kinds_are_gone(harness: Harness) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change.")
    parent = await harness.agent(Reply(text="Hello."), Reply(text="Hello again."))
    await turn(parent, "Hello.")

    (harness.cwd / ".columbina" / "agents" / "reviewer.md").unlink()
    await turn(parent, "Hello again.")

    gone = context(
        "agents",
        "No kind of subagent is there any more. The kinds that you were told of before cannot "
        "be named. The tool `subagent` works without a type.",
    )
    assert gone in harness.model.requests[1].messages
    assert all(KINDS not in str(message) for message in harness.model.requests[1].messages)


async def test_an_agent_that_was_never_told_of_kinds_is_not_told_that_they_are_gone(
    harness: Harness,
) -> None:
    await loaded(harness)
    parent = await harness.agent(Reply(text="Hello."), Reply(text="Hello again."))
    await turn(parent, "Hello.")

    # By now the session holds what the user wrote, and still no word of kinds.
    await turn(parent, "Hello again.")

    second = harness.model.requests[1].messages
    assert user("Hello.") in second
    assert all("No kind of subagent" not in str(message) for message in second)


async def test_a_subagent_is_told_where_it_stands_and_nothing_of_kinds(harness: Harness) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change.")
    parent = await harness.agent(
        hands_over(("Count the files.", None)),
        Reply(text="Three files."),
        Reply(text="Three."),
    )

    await turn(parent)

    of_the_parent, of_the_child, _ = harness.model.requests
    assert of_the_child.messages == (
        SYSTEM,
        context("delegated", DELEGATED),
        user("Count the files."),
    )
    # The parent is told of the kinds, and not that it is a subagent.
    assert [message for message in of_the_parent.messages if DELEGATED in str(message)] == []
    assert context("agents", f"{KINDS}\n- reviewer: Reads a change.") in of_the_parent.messages


async def test_an_agent_that_does_not_have_the_tool_is_told_nothing_of_kinds(
    harness: Harness,
) -> None:
    await loaded(harness)
    a_kind(harness, "reviewer", "Reads a change.")
    parent = await harness.agent(Reply(text="Hello."), tools=frozenset({"read"}))

    await turn(parent, "Hello.")

    assert harness.model.requests[0].messages == (SYSTEM, user("Hello."))


# The list of kinds


def test_what_a_kind_is_for_is_told_on_one_line_and_not_too_long() -> None:
    long = "word " * 100

    said = catalog([kind("wordy", long), kind("plain", "Does a thing.")], 10_000)

    assert said is not None
    first, second = said.splitlines()[1:]
    assert first == "- wordy: " + ("word " * 100)[: MAX_DESCRIPTION - 1] + "…"
    assert len(first) == len("- wordy: ") + MAX_DESCRIPTION
    assert second == "- plain: Does a thing."


def test_a_description_of_just_the_length_that_is_allowed_is_told_whole() -> None:
    exact = "x" * MAX_DESCRIPTION

    said = catalog([kind("exact", exact)], 10_000)

    assert said == f"{KINDS}\n- exact: {exact}"


def test_kinds_that_do_not_fit_are_left_out_and_counted() -> None:
    kinds = [kind(f"kind-{number}", "x" * 40) for number in range(1, 6)]
    line = len("- kind-1: " + "x" * 40)

    # Two lines and the ends of two lines fit, and not a character of the third.
    said = catalog(kinds, 2 * line + 2)
    one = catalog(kinds[:4], 3 * line + 3)

    assert said == "\n".join(
        [
            KINDS,
            f"- kind-1: {'x' * 40}",
            f"- kind-2: {'x' * 40}",
            "[3 more kinds are there and not listed, because the list would be too long.]",
        ]
    )
    assert one is not None
    assert one.splitlines()[-1] == (
        "[1 more kind is there and not listed, because the list would be too long.]"
    )


def test_the_list_may_be_as_long_as_is_allowed_to_the_character() -> None:
    kinds = [kind("first", "x" * 40), kind("second", "y" * 40)]
    lines = [f"- first: {'x' * 40}", f"- second: {'y' * 40}"]
    # The two lines and the end of the first one.
    whole = len("\n".join(lines))

    fits = catalog(kinds, whole)
    short = catalog(kinds, whole - 1)

    assert fits == "\n".join([KINDS, *lines])
    assert short == "\n".join(
        [
            KINDS,
            lines[0],
            "[1 more kind is there and not listed, because the list would be too long.]",
        ]
    )


def test_the_first_kind_is_listed_however_long_its_line_is() -> None:
    said = catalog([kind("only", "x" * 40), kind("next", "y")], 5)

    assert said == "\n".join(
        [
            KINDS,
            f"- only: {'x' * 40}",
            "[1 more kind is there and not listed, because the list would be too long.]",
        ]
    )


def test_no_kinds_make_no_list() -> None:
    assert catalog([], 10_000) is None
