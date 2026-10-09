"""Closing a log that stops in the middle of a turn."""

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Literal

from columbina.llm.types import TextBlock, ToolMessage
from columbina.session.events import (
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


@dataclass(slots=True)
class _Waiting:
    """A tool call of the open turn that has no result yet."""

    call_id: str
    dispatched: bool = False


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

    Calls are matched with their results one by one and in order, not by id
    alone: a model may use an id again in a later reply. An event that takes
    the place of earlier ones is passed over. It says again what the log
    holds, so it asks for no new call and answers none.

    Returns:
        The events to append, in order. The list is empty if no turn is open.
    """
    turn: int | None = None
    step: int | None = None
    waiting: list[_Waiting] = []
    for event in events:
        data = event.data
        if event.replaces:
            continue
        if isinstance(data, TurnStartEvent):
            turn, step = data.turn, None
            waiting.clear()
        elif isinstance(data, TurnEndEvent):
            turn = step = None
        elif isinstance(data, StepStartEvent):
            step = data.step
        elif isinstance(data, StepEndEvent):
            step = None
        elif isinstance(data, AssistantMessageEvent):
            waiting.extend(_Waiting(call.call_id) for call in data.message.tool_calls)
        elif isinstance(data, ToolCallEvent):
            for call in waiting:
                if call.call_id == data.call_id and not call.dispatched:
                    call.dispatched = True
                    break
        elif isinstance(data, ToolResultEvent):
            for call in waiting:
                if call.call_id == data.message.call_id:
                    waiting.remove(call)
                    break
    if turn is None:
        return []
    closers: list[EventData] = [_result(call.call_id, call.dispatched, reason) for call in waiting]
    if step is not None:
        closers.append(StepEndEvent(turn=turn, step=step))
    closers.append(TurnEndEvent(turn=turn, reason=reason))
    return closers
