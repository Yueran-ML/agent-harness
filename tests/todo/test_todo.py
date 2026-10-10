"""The list of things to do that a model keeps for itself."""

from typing import Any

import pytest
from pydantic import JsonValue

import columbina.todo
from columbina.agent import AGENTS, PRE_STEP, Agent, StepDecision, StepInfo
from columbina.kernel import Next
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import (
    SESSIONS,
    AssistantMessageEvent,
    ToolResultEvent,
    UserMessageEvent,
)
from columbina.todo import (
    MAX_CHARACTERS,
    MAX_ITEMS,
    TODOS,
    TOOL,
    Item,
    Todos,
    TodoWriteEvent,
    shown,
)
from columbina.tools import TOOLS
from support.loop import Harness, context, idle, user

PLAN = (
    ("Read the check", "completed"),
    ("Mend calc.py", "in_progress"),
    ("Run the check", "pending"),
)
ITEMS = tuple(Item(content=content, status=status) for content, status in PLAN)  # pyright: ignore[reportArgumentType]
RECALLED = context(
    "todo",
    "This is your list of things to do, as you wrote it last. The call that wrote it is in the "
    "part of the conversation that was summed up.\n"
    "[x] Read the check\n"
    "[>] Mend calc.py\n"
    "[ ] Run the check\n"
    "Write the whole list again with todo_write when something on it changes.",
)


def wrote(*things: tuple[str, str] | dict[str, Any]) -> Reply:
    """A reply of the model that writes the list."""
    items: list[JsonValue] = [
        thing if isinstance(thing, dict) else {"content": thing[0], "status": thing[1]}
        for thing in things
    ]
    return Reply(calls=(Call(name=TOOL, arguments={"items": items}),))


async def working(harness: Harness, *script: Reply) -> Agent:
    await harness.ctx.plugin(columbina.todo)
    return await harness.agent(*script)


async def turn(agent: Agent, text: str = "Go on.") -> None:
    agent.followup(text)
    await idle(agent)


def results(agent: Agent) -> list[tuple[str, str | None]]:
    """What each call of a tool gave back, and the code of its error if it failed."""
    return [
        (
            "".join(block.text for block in event.data.message.content),
            None if event.data.error is None else event.data.error.code,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def written(agent: Agent) -> list[TodoWriteEvent]:
    return [event.data for event in agent.session.events if isinstance(event.data, TodoWriteEvent)]


def where(agent: Agent) -> list[int]:
    """Where in the log the list was written, each time that it was."""
    return [event.seq for event in agent.session.events if isinstance(event.data, TodoWriteEvent)]


def recalled(agent: Agent) -> list[UserMessageEvent]:
    """The messages on the surface in which the model is told its list as context."""
    session = agent.session
    return [
        data
        for seq in session.surface
        if isinstance(data := session.events[seq].data, UserMessageEvent)
        and data.source == "context/todo"
    ]


def sum_up(agent: Agent, *, since: int = 0) -> None:
    """Put a summary in the place of every reply and result that the model sees, from one on.

    That is what happens to the beginning of a conversation that has grown too long.
    """
    session = agent.session
    gone = [
        seq
        for seq in session.surface
        if seq >= since
        and isinstance(session.events[seq].data, AssistantMessageEvent | ToolResultEvent)
    ]
    summary = UserMessageEvent(message=user("A summary of what was done."), source="compaction")
    session.append(summary, replaces=gone)


# Writing the list


async def test_the_list_is_written_whole_and_kept_in_the_log(harness: Harness) -> None:
    agent = await working(harness, wrote(*PLAN), Reply(text="On it."))

    await turn(agent)

    assert results(agent) == [
        ("The list holds 3 things: 1 completed, 1 in progress, 1 pending.", None)
    ]
    assert written(agent) == [TodoWriteEvent(call_id="call-1", items=ITEMS)]
    assert TODOS.of(agent.session) == Todos(items=ITEMS, written_at=where(agent)[0])
    # A program that reads the log and does not know the list may pass it over.
    assert TodoWriteEvent.ignorable


async def test_a_new_list_takes_the_place_of_the_old_one(harness: Harness) -> None:
    agent = await working(
        harness,
        wrote(*PLAN),
        wrote(("Run the check", "in_progress")),
        wrote(),
        Reply(text="Done."),
    )

    await turn(agent)

    assert [text for text, _ in results(agent)] == [
        "The list holds 3 things: 1 completed, 1 in progress, 1 pending.",
        "The list holds 1 thing: 0 completed, 1 in progress, 0 pending.",
        "The list is empty now.",
    ]
    assert [len(event.items) for event in written(agent)] == [3, 1, 0]
    assert TODOS.of(agent.session) == Todos(items=(), written_at=where(agent)[2])


async def test_a_thing_is_pending_unless_the_model_says_otherwise(harness: Harness) -> None:
    agent = await working(harness, wrote({"content": "  Read\n the   check \n"}), Reply(text="Ok."))

    await turn(agent)

    # One thing is one line. The space around the words is not part of them, and a break
    # or a run of spaces between them is one space.
    assert TODOS.of(agent.session).items == (Item(content="Read the check", status="pending"),)


async def test_before_anything_is_written_there_is_no_list(harness: Harness) -> None:
    agent = await working(harness, Reply(text="Hello."))

    await turn(agent)

    assert TODOS.of(agent.session) == Todos()
    assert recalled(agent) == []


def test_a_list_is_shown_with_a_mark_before_each_thing() -> None:
    assert shown(ITEMS) == ["[x] Read the check", "[>] Mend calc.py", "[ ] Run the check"]
    assert shown(()) == []


# What is no list


@pytest.mark.parametrize(
    ("things", "said"),
    [
        (
            [("Read the check", "pending"), ("   ", "pending")],
            "Thing 2 of the list says nothing.",
        ),
        (
            [("x" * (MAX_CHARACTERS + 1), "pending")],
            f"Thing 1 of the list has {MAX_CHARACTERS + 1} characters, and one thing has "
            f"{MAX_CHARACTERS} at most. Say it in a few words.",
        ),
        (
            [("a", "in_progress"), ("b", "pending"), ("c", "in_progress")],
            "Things 1, 3 are all in progress, and one thing is at a time. "
            "Leave one in progress and make the others pending.",
        ),
        (
            [("a", "pending")] * (MAX_ITEMS + 1),
            f"The list has {MAX_ITEMS + 1} things, and it holds {MAX_ITEMS} at most. "
            "Put small steps together.",
        ),
    ],
)
async def test_a_list_that_cannot_be_kept_is_refused_and_the_old_one_stays(
    harness: Harness, things: list[tuple[str, str]], said: str
) -> None:
    agent = await working(harness, wrote(*PLAN), wrote(*things), Reply(text="Sorry."))

    await turn(agent)

    assert results(agent)[1] == (said, "INVALID_ARGS")
    assert len(written(agent)) == 1
    assert TODOS.of(agent.session) == Todos(items=ITEMS, written_at=where(agent)[0])


async def test_a_list_of_just_the_size_that_is_kept_is_kept(harness: Harness) -> None:
    longest = "x" * MAX_CHARACTERS
    things = [(longest, "in_progress"), *[("a", "completed")] * (MAX_ITEMS - 1)]
    agent = await working(harness, wrote(*things), Reply(text="Ok."))

    await turn(agent)

    assert results(agent) == [
        (
            f"The list holds {MAX_ITEMS} things: {MAX_ITEMS - 1} completed, 1 in progress, "
            "0 pending.",
            None,
        )
    ]
    assert TODOS.of(agent.session).items[0] == Item(content=longest, status="in_progress")


async def test_a_state_that_is_none_of_the_three_is_refused(harness: Harness) -> None:
    agent = await working(harness, wrote(("Read the check", "done")), Reply(text="Sorry."))

    await turn(agent)

    ((said, code),) = results(agent)
    assert code == "INVALID_ARGS"
    assert "status" in said
    assert written(agent) == []


# Keeping the list


async def test_the_list_stays_from_turn_to_turn_and_when_a_session_is_taken_up_again(
    harness: Harness,
) -> None:
    agent = await working(harness, wrote(*PLAN), Reply(text="On it."), Reply(text="Still on it."))
    await turn(agent)
    await turn(agent)
    assert TODOS.of(agent.session) == Todos(items=ITEMS, written_at=where(agent)[0])

    await agent.close()
    again = await harness.agents.resume(harness.ctx, harness.spec(), "s-1")

    assert TODOS.of(again.session) == Todos(items=ITEMS, written_at=where(again)[0])


# Telling the model its list again


async def test_a_model_that_sees_the_call_is_not_told_its_list_again(harness: Harness) -> None:
    agent = await working(harness, wrote(*PLAN), Reply(text="On it."), Reply(text="Still on it."))
    await turn(agent)

    await turn(agent)

    assert recalled(agent) == []
    # A result that was cut down is the result of the same call: the model sees the call.
    session = agent.session
    (result,) = [
        seq for seq in session.surface if isinstance(session.events[seq].data, ToolResultEvent)
    ]
    session.append(session.events[result].data, replaces=[result])
    harness.model.add(Reply(text="Yes."))
    await turn(agent)
    assert recalled(agent) == []


async def test_the_list_is_told_again_when_the_call_that_wrote_it_was_summed_up(
    harness: Harness,
) -> None:
    agent = await working(harness, wrote(*PLAN), Reply(text="On it."), Reply(text="Mended."))
    await turn(agent)
    sum_up(agent)

    await turn(agent)

    assert [data.message for data in recalled(agent)] == [RECALLED]
    # The model had it before it when it was asked.
    assert RECALLED in harness.model.requests[-1].messages
    assert harness.model.requests[-1].messages[-1] == user("Go on.")


async def test_the_result_of_another_call_is_not_the_call_that_wrote_the_list(
    harness: Harness,
) -> None:
    agent = await working(
        harness, wrote(("", "pending")), wrote(*PLAN), Reply(text="On it."), Reply(text="Mended.")
    )
    await turn(agent)
    session = agent.session
    replies = [
        seq
        for seq in session.surface
        if isinstance(session.events[seq].data, AssistantMessageEvent | ToolResultEvent)
    ]
    # Only the call that wrote the list is summed up, with its result. The call before it,
    # which was refused, stays where it is, and so does its result.
    summary = UserMessageEvent(message=user("A summary of what was done."), source="compaction")
    session.append(summary, replaces=replies[2:4])

    await turn(agent)

    assert [data.message for data in recalled(agent)] == [RECALLED]


async def test_a_later_call_with_the_same_id_is_not_the_call_that_wrote_the_list(
    harness: Harness,
) -> None:
    # A model may use an id again in a later reply.
    things: list[JsonValue] = [{"content": content, "status": status} for content, status in PLAN]
    agent = await working(
        harness,
        Reply(calls=(Call(name=TOOL, arguments={"items": things}, call_id="same"),)),
        Reply(calls=(Call(name=TOOL, arguments={"items": "none"}, call_id="same"),)),
        Reply(text="On it."),
        Reply(text="Mended."),
    )
    await turn(agent)
    session = agent.session
    assert results(agent)[1][1] == "INVALID_ARGS"
    replies = [
        seq
        for seq in session.surface
        if isinstance(session.events[seq].data, AssistantMessageEvent | ToolResultEvent)
    ]
    # The call that wrote the list is summed up, with its result. The later call, which
    # wrote nothing, stays, and so does its result. The model sees no list any more.
    summary = UserMessageEvent(message=user("A summary of what was done."), source="compaction")
    session.append(summary, replaces=replies[:2])

    await turn(agent)

    assert [data.message for data in recalled(agent)] == [RECALLED]


async def test_a_list_that_was_told_again_is_not_told_twice(harness: Harness) -> None:
    agent = await working(
        harness, wrote(*PLAN), Reply(text="On it."), Reply(text="Mended."), Reply(text="Run.")
    )
    await turn(agent)
    sum_up(agent)
    await turn(agent)
    count = sum(event.type == "user/message" for event in agent.session.events)

    await turn(agent)

    assert len(recalled(agent)) == 1
    # Nothing was written for it: the one that stands is the one that the model sees.
    assert sum(event.type == "user/message" for event in agent.session.events) == count + 1


async def test_the_list_is_told_again_in_the_middle_of_a_turn(harness: Harness) -> None:
    agent = await working(harness, wrote(*PLAN), Reply(text="Mended."))

    async def before_the_second_step(
        info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        if info.step == 2:
            sum_up(info.agent)
        return await proceed(info)

    harness.ctx.on(PRE_STEP, before_the_second_step)

    await turn(agent)

    # The request that follows the summary has the list in it.
    assert harness.model.requests[1].messages[-1] == RECALLED
    assert len(recalled(agent)) == 1


async def test_what_is_told_again_is_the_list_that_was_written_last(harness: Harness) -> None:
    agent = await working(
        harness,
        wrote(*PLAN),
        Reply(text="On it."),
        Reply(text="Mended."),
        wrote(("Run the check", "in_progress")),
        Reply(text="Running."),
        Reply(text="Ran."),
    )
    await turn(agent)
    sum_up(agent)
    await turn(agent)
    old = agent.session.surface.index(
        next(event.seq for event in agent.session.events if event.data == recalled(agent)[0])
    )
    # The model writes a shorter list. It sees that call, so it is told nothing.
    before = len(agent.session)
    await turn(agent)
    assert [data.message for data in recalled(agent)] == [RECALLED]
    # That call is summed up as well.
    sum_up(agent, since=before)

    await turn(agent)

    (now,) = recalled(agent)
    assert now.message == context(
        "todo",
        "This is your list of things to do, as you wrote it last. The call that wrote it is in "
        "the part of the conversation that was summed up.\n"
        "[>] Run the check\n"
        "Write the whole list again with todo_write when something on it changes.",
    )
    # It stands where the old one stood, which the model still saw.
    session = agent.session
    assert session.events[session.surface[old]].data == now


async def test_a_list_that_was_emptied_is_not_told_again(harness: Harness) -> None:
    agent = await working(harness, wrote(*PLAN), wrote(), Reply(text="Done."), Reply(text="Yes."))
    await turn(agent)
    sum_up(agent)

    await turn(agent)

    assert recalled(agent) == []


# The plugin


async def test_the_plugin_brings_one_tool_one_event_and_one_source_of_context(
    harness: Harness,
) -> None:
    loaded = await harness.ctx.plugin(columbina.todo)

    assert TOOL in harness.ctx.use(TOOLS).registry
    assert harness.ctx.use(TOOLS).registry[TOOL].effect == "none"
    assert harness.ctx.use(SESSIONS).kinds["todo/write"] is TodoWriteEvent
    source = harness.ctx.use(AGENTS).context["todo"]
    assert (source.order, source.kind) == (150, "replace")

    await loaded.dispose()

    assert TOOL not in harness.ctx.use(TOOLS).registry
    assert "todo/write" not in harness.ctx.use(SESSIONS).kinds
    assert "todo" not in harness.ctx.use(AGENTS).context
