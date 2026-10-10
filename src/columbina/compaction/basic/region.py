"""Which part of a conversation a summary is to take the place of.

It is the oldest part. The newest part stays as it is, because it is what
the model is working with: so much of it as was asked for, counted from the
end.

Four things move the place where the one ends and the other begins.

- A call of a tool and its results are not parted. A model that is shown a
  result without the call it answers, or a call without a result, is refused
  by some services and confused by the rest. So if the place falls among
  results, it moves back to before their call.
- What the user said last stays in their own words. It is the task at hand,
  and a retelling of it would be all that is left otherwise. Only if what
  follows it has grown to more than half of what the model takes does it go.
- If so much has to go, more goes: a model that has refused a conversation
  as too long by so many tokens has to be shown that many fewer.
- The last message, with its call and results if it is one of those, always
  stays. It is what the model is about to be asked about.

The instructions at the top are no part of the conversation, and neither is
the context that was given with them before anything was said. They stay
where they are, so what a service has kept of the beginning of earlier
requests still fits.

A summary that is there from an earlier time is summed up with what followed
it. By itself it is left alone: there is nothing to gain.
"""

from collections.abc import Callable, Sequence
from dataclasses import dataclass

from columbina.agent import CONTEXT_PREFIX
from columbina.compaction import SUMMARY_SOURCE
from columbina.llm import Message
from columbina.session import (
    EventData,
    Session,
    SessionEvent,
    SystemMessageEvent,
    ToolResultEvent,
    UserMessageEvent,
)
from columbina.token_meter import estimate_message

__all__ = ["HUMAN", "Region", "choose"]

HUMAN = ("user", "steer")
"""The sources of what the user wrote themselves."""


@dataclass(frozen=True, slots=True)
class Region:
    """A part of a conversation that a summary is to take the place of.

    Attributes:
        seqs: The events whose messages it is made of, in the order in which
            the model sees them. They are neighbours.
        tokens: What the messages come to, as they are estimated.
    """

    seqs: tuple[int, ...]
    tokens: int


def _source(event: SessionEvent[EventData]) -> str:
    """Return where a message in the role of the user came from, or nothing for another event."""
    data = event.data
    return data.source if isinstance(data, UserMessageEvent) else ""


def _before_the_call(events: Sequence[SessionEvent[EventData]], at: int, start: int) -> int:
    """Return the place at or before ``at`` that does not stand among the results of a call."""
    while at > start and isinstance(events[at].data, ToolResultEvent):
        at -= 1
    return at


def choose(
    session: Session,
    *,
    retain: int,
    half: int,
    shed: int = 0,
    size: Callable[[Message], int] = estimate_message,
) -> Region | None:
    """Choose the part of a conversation that a summary is to take the place of.

    Args:
        session: The session whose messages are looked at.
        retain: How many tokens of the newest messages are to stay, at least.
        half: Half of what the model takes. What the user said last goes
            only if what follows it comes to more.
        shed: How many tokens have to go, if the model has said so. More
            goes than otherwise would, as far as that is needed.
        size: Tells how many tokens a message comes to.

    Returns:
        The part, or ``None`` if nothing can go: the conversation is no
        longer than what is to stay, or all that could go is an earlier
        summary.
    """
    events = [session.events[seq] for seq in session.surface]
    sizes = [size(message) for message in session.messages]
    start = 0
    while start < len(events) and (
        isinstance(events[start].data, SystemMessageEvent)
        or _source(events[start]).startswith(CONTEXT_PREFIX)
    ):
        start += 1
    if start == len(events):
        return None
    # From the end back, until what stays comes to enough.
    end = len(events)
    kept = 0
    while end > start and kept < retain:
        end -= 1
        kept += sizes[end]
    # The last message stays with its call, and no call is parted from its results.
    last = _before_the_call(events, len(events) - 1, start)
    end = _before_the_call(events, min(end, last), start)
    human = [at for at, event in enumerate(events) if at >= start and _source(event) in HUMAN]
    if human and human[-1] < end and sum(sizes[human[-1] :]) <= half:
        # What the user said last is in the part that would go, and it need not.
        end = human[-1]
    # More goes if so much has to go, one message or one call with its results at a time.
    while end < last and sum(sizes[start:end]) < shed:
        end += 1
        while isinstance(events[end].data, ToolResultEvent):
            end += 1
    chosen = events[start:end]
    if all(_source(event) == SUMMARY_SOURCE for event in chosen):
        return None
    return Region(tuple(event.seq for event in chosen), sum(sizes[start:end]))
