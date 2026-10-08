"""Projections: state that is derived from the events of a session."""

from collections.abc import Callable
from typing import Generic, TypeVar
from weakref import WeakKeyDictionary

from tack.data import Data
from tack.llm.types import Usage
from tack.session.events import (
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    TurnEndEvent,
    TurnStartEvent,
)
from tack.session.log import Session

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
    if isinstance(data, (AssistantMessageEvent, AssistantAttemptEvent)):
        return UsageTotal(calls=state.calls + 1, usage=state.usage + data.usage)
    return state


USAGE = Projection("usage", UsageTotal(), _fold_usage)
"""The token counts of a session, added up."""
