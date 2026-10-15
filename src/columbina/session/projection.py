"""Projections: state that is derived from the events of a session."""

from collections.abc import Callable, Iterable
from typing import Generic, TypeVar
from weakref import WeakKeyDictionary

from pydantic import Field

from columbina.data import Data
from columbina.llm.types import Usage
from columbina.session.events import (
    AsideEvent,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.session.log import Session

StateT = TypeVar("StateT")


class Projection(Generic[StateT]):
    """A piece of state that is derived from the events of a session, one event at a time.

    ``apply`` takes the state so far and the next event, and returns the state
    after it. It must not change the state it is given. An event that does not
    matter is answered with the very same object. The state of a session is
    then a function of its log alone, so it is the same after a resume.

    The state is worked out when it is asked for, and each event is folded
    once for each session.

    Args:
        name: A name for logs and error messages.
        initial: The state of a session without events.
        apply: Folds one event into the state.

    Attributes:
        name: The name of the projection.
    """

    __slots__ = ("_apply", "_folded", "_initial", "name")

    def __init__(
        self,
        name: str,
        initial: StateT,
        apply: Callable[[StateT, SessionEvent[EventData]], StateT],
    ) -> None:
        self.name = name
        self._initial = initial
        self._apply = apply
        self._folded: WeakKeyDictionary[Session, tuple[int, StateT]] = WeakKeyDictionary()

    def of(self, session: Session) -> StateT:
        """Return the state of ``session`` as of its last event.

        If ``apply`` raises, the error reaches the caller, and the state stays
        at the last event that was folded.
        """
        count, state = self._folded.get(session, (0, self._initial))
        events = session.events
        try:
            while count < len(events):
                state = self._apply(state, events[count])
                count += 1
        finally:
            self._folded[session] = (count, state)
        return state


class TurnState(Data):
    """Where a session stands in its turns.

    Attributes:
        turn: The number of the last turn that began, or 0 before the first.
        step: The number of the last step that began in that turn, or 0.
        in_turn: Whether that turn is open.
        in_step: Whether that step is open.
    """

    turn: int = 0
    step: int = 0
    in_turn: bool = False
    in_step: bool = False


def _fold_turn(state: TurnState, event: SessionEvent[EventData]) -> TurnState:
    data = event.data
    if isinstance(data, TurnStartEvent):
        return TurnState(turn=data.turn, in_turn=True)
    if isinstance(data, StepStartEvent):
        return TurnState(turn=state.turn, step=data.step, in_turn=True, in_step=True)
    if isinstance(data, StepEndEvent):
        return TurnState(turn=state.turn, step=state.step, in_turn=state.in_turn)
    if isinstance(data, TurnEndEvent):
        return TurnState(turn=state.turn, step=state.step)
    return state


TURN = Projection("turn", TurnState(), _fold_turn)
"""Where a session stands in its turns and steps."""


class UsageTotal(Data):
    """What the calls to a model have cost so far.

    Attributes:
        calls: How many calls the log records, with the ones that failed.
        usage: The sum of their token counts.
    """

    calls: int = 0
    usage: Usage = Usage()


def _fold_usage(state: UsageTotal, event: SessionEvent[EventData]) -> UsageTotal:
    data = event.data
    if isinstance(data, (AssistantMessageEvent, AssistantAttemptEvent, AsideEvent)):
        return UsageTotal(calls=state.calls + 1, usage=state.usage + data.usage)
    return state


USAGE = Projection("usage", UsageTotal(), _fold_usage)
"""The token counts of a session, added up."""


class ToolUse(Data):
    """How the calls of one tool went.

    Attributes:
        calls: How many of its calls have a result.
        failed: How many of those did not succeed.
        duration_ms: How long the tool ran in all, in milliseconds. A call
            that did not run adds nothing.
    """

    calls: int = 0
    failed: int = 0
    duration_ms: int = 0


class ToolsUsed(Data):
    """How the calls of the tools of a session went.

    Attributes:
        tools: What became of the calls of each tool, by its name, in the
            order of the first result of each.
        waiting: The tool of each call that has no result yet, by the id of
            the call.
    """

    tools: dict[str, ToolUse] = Field(default_factory=dict[str, ToolUse])
    waiting: dict[str, str] = Field(default_factory=dict[str, str])


def _fold_tools(state: ToolsUsed, event: SessionEvent[EventData]) -> ToolsUsed:
    data = event.data
    if isinstance(data, ToolCallEvent):
        return ToolsUsed(tools=state.tools, waiting={**state.waiting, data.call_id: data.name})
    if not isinstance(data, ToolResultEvent):
        return state
    call_id = data.message.call_id
    name = state.waiting.get(call_id)
    if name is None:
        # A result that takes the place of another one, made shorter, is no call of its own.
        return state
    before = state.tools.get(name, ToolUse())
    took = (data.meta or {}).get("duration_ms")
    after = ToolUse(
        calls=before.calls + 1,
        failed=before.failed + (data.error is not None),
        duration_ms=before.duration_ms + (took if isinstance(took, int) else 0),
    )
    still = {key: tool for key, tool in state.waiting.items() if key != call_id}
    return ToolsUsed(tools={**state.tools, name: after}, waiting=still)


TOOLS_USED = Projection("tools used", ToolsUsed(), _fold_tools)
"""How the calls of each tool of a session went: how many, how many failed, how long."""


def tools_used(events: Iterable[SessionEvent[EventData]]) -> ToolsUsed:
    """Return how the calls of each tool went in some events of a log, such as a stored one."""
    state = ToolsUsed()
    for event in events:
        state = _fold_tools(state, event)
    return state
