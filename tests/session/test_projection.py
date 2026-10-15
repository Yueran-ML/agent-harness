"""Projections: state that is derived from the events of a session."""

import gc
import weakref
from pathlib import Path

import pytest
from pydantic import JsonValue

import columbina.session
from columbina.kernel import Context
from columbina.llm import AssistantMessage, LlmFailure, TextBlock, ToolMessage, Usage
from columbina.session import (
    CORE_EVENTS,
    SESSIONS,
    TOOLS_USED,
    TURN,
    USAGE,
    AsideEvent,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EndSeedEvent,
    EventData,
    Projection,
    Session,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    ToolsUsed,
    ToolUse,
    TurnEndEvent,
    TurnStartEvent,
    TurnState,
    UsageTotal,
    tools_used,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}


def count_turns(state: int, event: SessionEvent[EventData]) -> int:
    return state + 1 if isinstance(event.data, TurnStartEvent) else state


def replied(usage: Usage) -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=AssistantMessage(content=(TextBlock(text="ok"),)),
        usage=usage,
        finish_reason="stop",
    )


async def test_a_session_without_events_is_in_the_initial_state(ctx: Context) -> None:
    turns = Projection("turns", 0, count_turns)

    assert turns.of(Session(ctx, "s-1", KINDS)) == 0
    assert turns.name == "turns"


async def test_the_state_is_folded_from_the_events_in_order(ctx: Context) -> None:
    def trace(state: tuple[int, ...], event: SessionEvent[EventData]) -> tuple[int, ...]:
        return (*state, event.seq)

    session = Session(ctx, "s-1", KINDS)
    for turn in (1, 2, 3):
        session.append(TurnStartEvent(turn=turn))

    assert Projection("trace", (), trace).of(session) == (0, 1, 2)


async def test_each_event_is_folded_once(ctx: Context) -> None:
    folded: list[int] = []

    def watch(state: int, event: SessionEvent[EventData]) -> int:
        folded.append(event.seq)
        return count_turns(state, event)

    turns = Projection("turns", 0, watch)
    session = Session(ctx, "s-1", KINDS)
    session.append(TurnStartEvent(turn=1))
    session.append(TurnEndEvent(turn=1, reason="completed"))

    assert turns.of(session) == 1
    assert turns.of(session) == 1
    session.append(TurnStartEvent(turn=2))
    assert turns.of(session) == 2

    assert folded == [0, 1, 2]


async def test_each_session_has_its_own_state(ctx: Context) -> None:
    turns = Projection("turns", 0, count_turns)
    one = Session(ctx, "s-1", KINDS)
    other = Session(ctx, "s-2", KINDS)
    one.append(TurnStartEvent(turn=1))

    assert turns.of(one) == 1
    assert turns.of(other) == 0


async def test_an_event_that_does_not_matter_leaves_the_state_as_it_is(ctx: Context) -> None:
    def collect(
        state: tuple[TurnStartEvent, ...], event: SessionEvent[EventData]
    ) -> tuple[TurnStartEvent, ...]:
        return (*state, event.data) if isinstance(event.data, TurnStartEvent) else state

    starts = Projection("starts", (), collect)
    session = Session(ctx, "s-1", KINDS)
    session.append(TurnStartEvent(turn=1))
    before = starts.of(session)

    session.append(TurnEndEvent(turn=1, reason="completed"))

    assert starts.of(session) is before


async def test_a_fold_that_fails_keeps_the_state_before_the_event(ctx: Context) -> None:
    broken = True

    def fragile(state: int, event: SessionEvent[EventData]) -> int:
        if broken and event.seq == 1:
            raise RuntimeError("cannot fold this")
        return state + 1

    count = Projection("count", 0, fragile)
    session = Session(ctx, "s-1", KINDS)
    for turn in (1, 2, 3):
        session.append(TurnStartEvent(turn=turn))

    with pytest.raises(RuntimeError, match="cannot fold this"):
        count.of(session)
    broken = False

    assert count.of(session) == 3


async def test_a_projection_does_not_keep_a_session_alive(ctx: Context) -> None:
    turns = Projection("turns", 0, count_turns)
    session = Session(ctx, "s-1", KINDS)
    session.append(TurnStartEvent(turn=1))
    assert turns.of(session) == 1
    gone = weakref.ref(session)

    del session
    gc.collect()

    assert gone() is None


async def test_a_resumed_session_is_folded_from_its_first_event(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path})
    sessions = ctx.use(SESSIONS)
    first = sessions.create(ctx, tmp_path, session_id="s-1")
    first.append(TurnStartEvent(turn=1))
    first.append(TurnEndEvent(turn=1, reason="completed"))
    first.close()

    second = sessions.resume(ctx, tmp_path, "s-1")
    second.append(TurnStartEvent(turn=2))

    assert Projection("turns", 0, count_turns).of(second) == 2


# The projections that come with the session log


async def test_the_turn_state_follows_turns_and_steps(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)
    seen: list[TurnState] = [TURN.of(session)]
    for data in (
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        StepEndEvent(turn=1, step=1),
        StepStartEvent(turn=1, step=2),
        StepEndEvent(turn=1, step=2),
        TurnEndEvent(turn=1, reason="completed"),
        TurnStartEvent(turn=2),
    ):
        session.append(data)
        seen.append(TURN.of(session))

    assert seen == [
        TurnState(),
        TurnState(turn=1, in_turn=True),
        TurnState(turn=1, step=1, in_turn=True, in_step=True),
        TurnState(turn=1, step=1, in_turn=True),
        TurnState(turn=1, step=2, in_turn=True, in_step=True),
        TurnState(turn=1, step=2, in_turn=True),
        TurnState(turn=1, step=2),
        TurnState(turn=2, in_turn=True),
    ]

    session.append(EndSeedEvent(inherited=False))

    assert TURN.of(session) is seen[-1]


async def test_the_usage_adds_up_over_replies_and_failed_attempts(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)
    assert USAGE.of(session) == UsageTotal()

    session.append(replied(Usage(input_tokens=100, output_tokens=20, cache_write_tokens=50)))
    session.append(
        AssistantAttemptEvent(
            message=AssistantMessage(content=()),
            usage=Usage(input_tokens=10, output_tokens=1),
            failure=LlmFailure(message="slow down", code="RATE_LIMIT"),
        )
    )
    session.append(replied(Usage(input_tokens=30, output_tokens=5, cache_read_tokens=150)))
    session.append(TurnStartEvent(turn=1))

    assert USAGE.of(session) == UsageTotal(
        calls=3,
        usage=Usage(
            input_tokens=140, output_tokens=26, cache_read_tokens=150, cache_write_tokens=50
        ),
    )


async def test_requests_beside_the_conversation_cost_what_any_request_costs(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)
    session.append(replied(Usage(input_tokens=100, output_tokens=20)))

    # A summary is asked for: once in vain, and once more.
    session.append(
        AsideEvent(
            purpose="summary",
            usage=Usage(input_tokens=7),
            failure=LlmFailure(message="overloaded", code="SERVER"),
        )
    )
    told = session.append(
        AsideEvent(purpose="summary", usage=Usage(input_tokens=900, output_tokens=60))
    )

    assert USAGE.of(session) == UsageTotal(
        calls=3, usage=Usage(input_tokens=1007, output_tokens=80)
    )
    # The model is never shown it, and a reader that does not know it can pass it over.
    assert not told.on_surface
    assert AsideEvent.ignorable
    assert AsideEvent.event_type == "request/aside"
    assert AsideEvent in CORE_EVENTS


def called(call_id: str, name: str) -> ToolCallEvent:
    return ToolCallEvent(call_id=call_id, name=name, arguments="{}")


def answered(
    call_id: str, meta: dict[str, JsonValue] | None = None, *, failed: bool = False
) -> ToolResultEvent:
    return ToolResultEvent(
        message=ToolMessage(call_id=call_id, content=(TextBlock(text="done"),), is_error=failed),
        error=ToolError(code="TOOL_ERROR", reason="done") if failed else None,
        meta=meta,
    )


async def test_the_calls_of_each_tool_add_up_with_their_failures_and_their_time(
    ctx: Context,
) -> None:
    session = Session(ctx, "s-1", KINDS)
    assert TOOLS_USED.of(session) == ToolsUsed()

    session.append(called("c-1", "bash"))
    session.append(called("c-2", "read"))
    session.append(answered("c-2", {"duration_ms": 12}))
    session.append(answered("c-1", {"duration_ms": 1500}, failed=True))
    session.append(called("c-3", "bash"))
    session.append(answered("c-3", {"duration_ms": 250}))
    session.append(called("c-4", "bash"))

    used = TOOLS_USED.of(session)
    assert used.tools == {
        "read": ToolUse(calls=1, duration_ms=12),
        "bash": ToolUse(calls=2, failed=1, duration_ms=1750),
    }
    # In the order of the first result of each tool.
    assert list(used.tools) == ["read", "bash"]
    assert used.waiting == {"c-4": "bash"}


@pytest.mark.parametrize(
    "meta",
    [None, {}, {"duration_ms": "fast"}, {"duration_ms": 2.5}],
    ids=["none", "no time", "no number", "no whole number"],
)
async def test_a_call_whose_tool_did_not_run_counts_without_time(
    ctx: Context, meta: dict[str, JsonValue] | None
) -> None:
    session = Session(ctx, "s-1", KINDS)
    session.append(called("c-1", "bash"))
    session.append(answered("c-1", meta, failed=True))

    assert TOOLS_USED.of(session).tools == {"bash": ToolUse(calls=1, failed=1)}


async def test_a_result_that_takes_the_place_of_another_is_no_call_of_its_own(
    ctx: Context,
) -> None:
    session = Session(ctx, "s-1", KINDS)
    session.append(called("c-1", "bash"))
    first = session.append(answered("c-1", {"duration_ms": 300}))
    session.append(answered("c-1", {"duration_ms": 300}), replaces=[first.seq])

    assert TOOLS_USED.of(session) == ToolsUsed(tools={"bash": ToolUse(calls=1, duration_ms=300)})


def test_the_calls_in_the_events_of_a_stored_log_add_up_the_same_way() -> None:
    told = [
        called("c-1", "grep"),
        answered("c-1", {"duration_ms": 40}),
        called("c-2", "grep"),
        TurnEndEvent(turn=1, reason="completed"),
    ]
    events = [SessionEvent(seq=seq, time=0, data=data) for seq, data in enumerate(told)]

    assert tools_used(events) == ToolsUsed(
        tools={"grep": ToolUse(calls=1, duration_ms=40)}, waiting={"c-2": "grep"}
    )
    assert tools_used([]) == ToolsUsed()


def test_usages_can_be_added() -> None:
    total = Usage(input_tokens=1, output_tokens=2) + Usage(
        input_tokens=10, output_tokens=20, cache_read_tokens=30, cache_write_tokens=40
    )

    assert total == Usage(
        input_tokens=11, output_tokens=22, cache_read_tokens=30, cache_write_tokens=40
    )
