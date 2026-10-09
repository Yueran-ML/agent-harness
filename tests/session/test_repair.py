"""Closing a log that stops in the middle of a turn."""

from typing import Literal

import pytest

from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    CORE_EVENTS,
    AssistantMessageEvent,
    EndSeedEvent,
    EventData,
    Session,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
    closing_events,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}

SAID = UserMessageEvent(message=UserMessage(content=(TextBlock(text="Read both files."),)))


def asking(*call_ids: str, finished: bool = True) -> AssistantMessageEvent:
    """A reply that asks for one ``read`` call per id."""
    calls = tuple(
        ToolCallBlock(call_id=call_id, name="read", arguments="{}") for call_id in call_ids
    )
    return AssistantMessageEvent(
        message=AssistantMessage(content=(TextBlock(text="I will look."), *calls)),
        usage=Usage(),
        finish_reason="tool-calls" if finished else None,
    )


def dispatched(call_id: str) -> ToolCallEvent:
    return ToolCallEvent(call_id=call_id, name="read", arguments="{}")


def answered(call_id: str) -> ToolResultEvent:
    return ToolResultEvent(message=ToolMessage(call_id=call_id, content=(TextBlock(text="ok"),)))


def log_of(ctx: Context, *events: EventData) -> Session:
    session = Session(ctx, "s-1", KINDS)
    for data in events:
        session.append(data)
    return session


def closers(
    ctx: Context, *events: EventData, reason: Literal["interrupted", "forked"] = "interrupted"
) -> list[EventData]:
    return closing_events(log_of(ctx, *events).events, reason)


async def test_an_empty_log_needs_no_closing(ctx: Context) -> None:
    assert closers(ctx) == []


async def test_a_log_that_ends_with_a_closed_turn_needs_no_closing(ctx: Context) -> None:
    assert (
        closers(
            ctx,
            TurnStartEvent(turn=1),
            StepStartEvent(turn=1, step=1),
            SAID,
            asking("call-1"),
            dispatched("call-1"),
            StepEndEvent(turn=1, step=1),
            TurnEndEvent(turn=1, reason="aborted"),
        )
        == []
    )


async def test_events_outside_a_turn_need_no_closing(ctx: Context) -> None:
    assert closers(ctx, EndSeedEvent(inherited=False), asking("call-1")) == []


async def test_a_turn_that_had_not_begun_a_step_is_ended(ctx: Context) -> None:
    assert closers(ctx, TurnStartEvent(turn=3)) == [TurnEndEvent(turn=3, reason="interrupted")]


async def test_an_open_step_is_ended_before_its_turn(ctx: Context) -> None:
    assert closers(ctx, TurnStartEvent(turn=3), StepStartEvent(turn=3, step=2), SAID) == [
        StepEndEvent(turn=3, step=2),
        TurnEndEvent(turn=3, reason="interrupted"),
    ]


async def test_a_turn_between_two_steps_is_ended(ctx: Context) -> None:
    assert closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        SAID,
        StepEndEvent(turn=1, step=1),
    ) == [TurnEndEvent(turn=1, reason="interrupted")]


async def test_every_tool_call_without_a_result_gets_one(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        SAID,
        asking("call-1", "call-2", "call-3"),
        dispatched("call-1"),
        dispatched("call-2"),
        answered("call-1"),
    )

    assert [type(data) for data in found] == [
        ToolResultEvent,
        ToolResultEvent,
        StepEndEvent,
        TurnEndEvent,
    ]
    unknown, not_started = found[0], found[1]
    assert isinstance(unknown, ToolResultEvent)
    assert isinstance(not_started, ToolResultEvent)
    assert unknown.message.call_id == "call-2"
    assert unknown.message.is_error
    assert unknown.error == ToolError(
        code="TOOL_OUTCOME_UNKNOWN",
        reason="the run was interrupted while the tool call was in progress",
    )
    assert unknown.message.content[0].text == (
        "The run was interrupted while this tool call was in progress. Whether it took effect "
        "is unknown. Check the current state before you rely on it, and repeat the call only "
        "if it only reads or is safe to run twice."
    )
    assert not_started.message.call_id == "call-3"
    assert not_started.message.is_error
    assert not_started.error == ToolError(
        code="TOOL_NOT_STARTED", reason="the run was interrupted before the tool call started"
    )
    assert not_started.message.content[0].text == (
        "The run was interrupted before this tool call started. It did not run."
    )


async def test_a_result_that_arrived_out_of_order_still_counts(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("call-1", "call-2"),
        dispatched("call-1"),
        dispatched("call-2"),
        answered("call-2"),
    )

    results = [data for data in found if isinstance(data, ToolResultEvent)]
    assert [result.message.call_id for result in results] == ["call-1"]


async def test_the_calls_of_a_reply_that_was_cut_short_get_results_too(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("call-1", finished=False),
    )

    assert isinstance(found[0], ToolResultEvent)
    assert found[0].error is not None
    assert found[0].error.code == "TOOL_NOT_STARTED"


async def test_only_the_open_turn_counts(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("old-call"),
        TurnEndEvent(turn=1, reason="error"),
        TurnStartEvent(turn=2),
        StepStartEvent(turn=2, step=1),
        asking("call-1"),
        dispatched("call-1"),
        answered("call-1"),
    )

    assert found == [StepEndEvent(turn=2, step=1), TurnEndEvent(turn=2, reason="interrupted")]


async def test_a_fork_closes_the_turn_with_its_own_reason_and_wording(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("call-1", "call-2"),
        dispatched("call-1"),
        reason="forked",
    )

    unknown, not_started = found[0], found[1]
    assert isinstance(unknown, ToolResultEvent)
    assert isinstance(not_started, ToolResultEvent)
    assert unknown.message.content[0].text.startswith(
        "The conversation was forked while this tool call was in progress."
    )
    assert unknown.error == ToolError(
        code="TOOL_OUTCOME_UNKNOWN",
        reason="the conversation was forked while the tool call was in progress",
    )
    assert not_started.message.content[0].text == (
        "The conversation was forked before this tool call started. It did not run."
    )
    assert found[2:] == [StepEndEvent(turn=1, step=1), TurnEndEvent(turn=1, reason="forked")]


@pytest.mark.parametrize("cut", range(1, 9))
async def test_a_log_that_was_closed_needs_no_more_closing(ctx: Context, cut: int) -> None:
    whole: list[EventData] = [
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        SAID,
        asking("call-1", "call-2"),
        dispatched("call-1"),
        answered("call-1"),
        dispatched("call-2"),
        answered("call-2"),
    ]
    session = log_of(ctx, *whole[:cut])

    for data in closing_events(session.events, "interrupted"):
        session.append(data)

    assert closing_events(session.events, "interrupted") == []
    assert isinstance(session.events[-1].data, TurnEndEvent)
    everything = [event.data for event in session.events]
    asked = [
        call.call_id
        for data in everything
        if isinstance(data, AssistantMessageEvent)
        for call in data.message.tool_calls
    ]
    results = [data.message.call_id for data in everything if isinstance(data, ToolResultEvent)]
    assert sorted(results) == sorted(asked)


async def test_a_call_id_that_is_used_again_in_a_later_step_counts_again(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("same"),
        dispatched("same"),
        answered("same"),
        StepEndEvent(turn=1, step=1),
        StepStartEvent(turn=1, step=2),
        asking("same"),
    )

    assert [type(data) for data in found] == [ToolResultEvent, StepEndEvent, TurnEndEvent]
    first = found[0]
    assert isinstance(first, ToolResultEvent)
    assert first.message.call_id == "same"
    assert first.error is not None
    assert first.error.code == "TOOL_NOT_STARTED"


async def test_two_calls_with_one_id_in_one_reply_each_need_a_result(ctx: Context) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("same", "same"),
        dispatched("same"),
        answered("same"),
        dispatched("same"),
    )

    results = [data for data in found if isinstance(data, ToolResultEvent)]
    assert len(results) == 1
    assert results[0].error is not None
    assert results[0].error.code == "TOOL_OUTCOME_UNKNOWN"


async def test_a_result_that_takes_the_place_of_another_answers_no_new_call(ctx: Context) -> None:
    session = log_of(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("same"),
        dispatched("same"),
        answered("same"),
        StepEndEvent(turn=1, step=1),
        StepStartEvent(turn=1, step=2),
        asking("same"),
    )
    session.append(answered("same"), replaces=[4])

    found = closing_events(session.events, "interrupted")

    assert [type(data) for data in found] == [ToolResultEvent, StepEndEvent, TurnEndEvent]


async def test_a_reply_that_takes_the_place_of_another_asks_for_no_new_call(ctx: Context) -> None:
    session = log_of(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("call-1"),
        dispatched("call-1"),
        answered("call-1"),
    )
    # The same reply again, say with its reasoning left out. Its call was answered long ago.
    session.append(asking("call-1"), replaces=[2])

    found = closing_events(session.events, "interrupted")

    assert found == [
        StepEndEvent(turn=1, step=1),
        TurnEndEvent(turn=1, reason="interrupted"),
    ]


async def test_a_call_or_a_result_that_belongs_to_no_waiting_call_changes_nothing(
    ctx: Context,
) -> None:
    found = closers(
        ctx,
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        asking("call-1"),
        dispatched("stray"),
        answered("stray"),
    )

    results = [data for data in found if isinstance(data, ToolResultEvent)]
    assert [result.message.call_id for result in results] == ["call-1"]
    assert results[0].error is not None
    assert results[0].error.code == "TOOL_NOT_STARTED"
