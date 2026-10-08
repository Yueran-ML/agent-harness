"""Closing a log that stops in the middle of a turn."""

from collections.abc import Sequence
from typing import Literal

from tack.llm.types import TextBlock, ToolMessage
from tack.session.events import (
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)

CloseReason = Literal["interrupted", "forked"]
"""Why a log is closed from outside: its run died, or a fork was cut at this point."""

_WHAT_HAPPENED = {
    "interrupted": "the run was interrupted",
    "forked": "the conversation was forked",
}


def _result(call_id: str, was_dispatched: bool, reason: CloseReason) -> ToolResultEvent:
    happened = _WHAT_HAPPENED[reason]
    sentence = happened[0].upper() + happened[1:]
    if was_dispatched:
        error = ToolError(
            code="TOOL_OUTCOME_UNKNOWN",
            reason=f"{happened} while the tool call was in progress",
        )
        text = (
            f"{sentence} while this tool call was in progress. Whether it took effect is "
            "unknown. Check the current state before you rely on it, and repeat the call only "
            "if it only reads or is safe to run twice."
        )
    else:
        error = ToolError(
            code="TOOL_NOT_STARTED", reason=f"{happened} before the tool call started"
        )
        text = f"{sentence} before this tool call started. It did not run."
    message = ToolMessage(call_id=call_id, content=(TextBlock(text=text),), is_error=True)
    return ToolResultEvent(message=message, error=error)


def closing_events(
    events: Sequence[SessionEvent[EventData]], reason: CloseReason
) -> list[EventData]:
    """Return the events that close a log which stops in the middle of a turn.

    The open turn is closed in three moves. Every tool call of the turn that
    has no result gets one, so that calls and results stay paired. The open
    step is ended. The turn is ended with ``reason``.

    A result that is made up here tells two cases apart. A call that had been
    dispatched may or may not have taken effect: ``TOOL_OUTCOME_UNKNOWN``. A
    call that had not been dispatched did not run: ``TOOL_NOT_STARTED``.

    Returns:
        The events to append, in order. The list is empty if no turn is open.
    """
    turn: int | None = None
    step: int | None = None
    asked: dict[str, None] = {}
    dispatched: set[str] = set()
    answered: set[str] = set()
    for event in events:
        data = event.data
        if isinstance(data, TurnStartEvent):
            turn, step = data.turn, None
            asked.clear()
            dispatched.clear()
            answered.clear()
        elif isinstance(data, TurnEndEvent):
            turn = step = None
        elif isinstance(data, StepStartEvent):
            step = data.step
        elif isinstance(data, StepEndEvent):
            step = None
        elif isinstance(data, AssistantMessageEvent):
            asked.update(dict.fromkeys(call.call_id for call in data.message.tool_calls))
        elif isinstance(data, ToolCallEvent):
            dispatched.add(data.call_id)
        elif isinstance(data, ToolResultEvent):
            answered.add(data.message.call_id)
    if turn is None:
        return []
    closers: list[EventData] = [
        _result(call_id, call_id in dispatched, reason)
        for call_id in asked
        if call_id not in answered
    ]
    if step is not None:
        closers.append(StepEndEvent(turn=turn, step=step))
    closers.append(TurnEndEvent(turn=turn, reason=reason))
    return closers
