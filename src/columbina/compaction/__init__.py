"""Making a conversation shorter, so that it goes on fitting its model.

A conversation grows with every step, and a model takes only so much. Before
it is too much, the oldest part is put away: results of tools that the model
has long read are cut down to their ends, and what was said and done is
written up in a summary that takes the place of it. Nothing leaves the log.
The model sees less, and the log tells how that came about.

This module is the seam: the events that tell of it, what a session says
about it, and what the service that does it is asked to do.
:mod:`columbina.compaction.basic` is such a service.

Three events tell how the messages came to be fewer. None of them changes
what the model sees. That is done by an event that every reader of a log
knows: a message of the user with the source :data:`SUMMARY_SOURCE` that
replaces the ones it sums up, or a result of a tool that replaces a longer
one. So a reader that does not know the three can pass them over and still
sees the conversation as the model saw it.

- ``compaction/start`` is written when the work begins.
- ``compaction/summary`` is written when a summary is there, before the
  message that carries it. It keeps what the summary cost.
- ``compaction/end`` is written when the work is over, however it ended.

A start without an end is an orphan: the program ended in between. The
messages are then as they were, because the one event that changes them had
not been written.
"""

from typing import ClassVar, Literal, Protocol

from columbina.agent import Agent
from columbina.data import Data
from columbina.kernel import ServiceKey
from columbina.llm import Usage
from columbina.session import EndSeedEvent, EventData, Projection, SessionEvent

__all__ = [
    "COMPACTION",
    "COMPACTIONS",
    "EVENTS",
    "SUMMARY_SOURCE",
    "Compaction",
    "CompactionEndEvent",
    "CompactionStartEvent",
    "CompactionSummaryEvent",
    "Compactions",
    "Outcome",
    "Why",
]

Why = Literal["pressure", "overflow", "manual"]
"""Why a conversation is made shorter.

It is near what its model takes, the model has refused it as too long, or
the user asked for it.
"""

SUMMARY_SOURCE = "compaction"
"""The source of the message that carries a summary. It is a message of the user."""


class Outcome(Data):
    """How making a conversation shorter ended.

    Attributes:
        trimmed: How many results of tools were cut down to their ends.
        summarised: How many messages a summary took the place of, or 0 if
            none was written.
        before: What the messages came to before, in tokens as they are
            estimated.
        after: What they came to afterwards.
        error: Why it did not do all that it set out to do, or ``None``.
            What was trimmed by then stays trimmed.
    """

    trimmed: int = 0
    summarised: int = 0
    before: int
    after: int
    error: str | None = None

    @property
    def changed(self) -> bool:
        """Whether the model sees something else than it did before."""
        return bool(self.trimmed or self.summarised)


class CompactionStartEvent(EventData):
    """A conversation is about to be made shorter.

    Attributes:
        why: What it is done for.
        tokens: What the messages come to, in tokens as they are estimated.
    """

    event_type: ClassVar[str] = "compaction/start"
    ignorable: ClassVar[bool] = True

    why: Why
    tokens: int


class CompactionSummaryEvent(EventData):
    """A summary was written. The message that carries it comes next.

    Attributes:
        summary: The summary, as the model that wrote it gave it.
        replaces: The events whose messages it is to take the place of.
        region_tokens: What those messages came to, as estimated.
        summary_tokens: What the message that carries the summary comes to.
        usage: What the request for the summary cost.
    """

    event_type: ClassVar[str] = "compaction/summary"
    ignorable: ClassVar[bool] = True

    summary: str
    replaces: tuple[int, ...]
    region_tokens: int
    summary_tokens: int
    usage: Usage


class CompactionEndEvent(EventData):
    """Making a conversation shorter is over.

    Attributes:
        why: What it was done for.
        outcome: How it ended.
    """

    event_type: ClassVar[str] = "compaction/end"
    ignorable: ClassVar[bool] = True

    why: Why
    outcome: Outcome


EVENTS: tuple[type[EventData], ...] = (
    CompactionStartEvent,
    CompactionSummaryEvent,
    CompactionEndEvent,
)
"""The events of this module. Whatever writes them declares them to the session service."""


class Compactions(Data):
    """What a session says about having been made shorter.

    Attributes:
        count: How many times the model came to see less.
        last: How the last attempt ended, or ``None`` if there was none.
        open: Whether one has begun and not ended. That is so while it is
            under way, and no longer once the session was taken up again.
        orphans: How many began and never ended.
    """

    count: int = 0
    last: Outcome | None = None
    open: bool = False
    orphans: int = 0


def _fold(state: Compactions, event: SessionEvent[EventData]) -> Compactions:
    data = event.data
    if isinstance(data, CompactionStartEvent):
        # One that is still open will not end any more: this one begins after it.
        return state.model_copy(update={"open": True, "orphans": state.orphans + state.open})
    if isinstance(data, CompactionEndEvent):
        count = state.count + data.outcome.changed
        return state.model_copy(update={"count": count, "last": data.outcome, "open": False})
    if isinstance(data, EndSeedEvent) and state.open:
        # The program that had begun it is gone.
        return state.model_copy(update={"open": False, "orphans": state.orphans + 1})
    return state


COMPACTIONS = Projection("compactions", Compactions(), _fold)
"""What a session says about having been made shorter."""


class Compaction(Protocol):
    """Whatever makes conversations shorter."""

    async def compact(self, agent: Agent, why: Why) -> Outcome:
        """Make the conversation of an agent shorter, and say how that ended.

        This reads and replaces messages, so it is for the two moments at
        which no step is under way: before a step begins, and while the agent
        waits for input.

        A failure is no error of the caller. It is in the outcome, and the
        messages are as they were, but for what was trimmed before it.
        """
        ...


COMPACTION = ServiceKey[Compaction]("compaction")
"""Whatever makes conversations shorter."""
