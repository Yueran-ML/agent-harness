"""What a model said, reply by reply."""

from collections.abc import Iterable

from columbina.agent.agent import CONTEXT_PREFIX
from columbina.session import AssistantMessageEvent, EventData, UserMessageEvent

__all__ = ["CUT_OFF", "replies"]

CUT_OFF = CONTEXT_PREFIX + "cut-off"
"""The source of the message that tells a model to go on from a reply that was cut off."""


def replies(events: Iterable[EventData]) -> list[str]:
    """Return what a model said in some events, reply by reply.

    A reply that was cut off by the limit on its length, and that the model
    was told to go on from, is one reply with the next: the model wrote on
    where it had stopped, so the two are one text. Whoever takes the last
    thing a model said for its answer gets the whole of it this way.

    Only the reply that comes next goes on from one that was cut off, and
    not if somebody said something in between. A reply that says nothing is
    told as an empty text.
    """
    said: list[str] = []
    cut_off = False
    goes_on = False
    for data in events:
        if isinstance(data, AssistantMessageEvent):
            if goes_on:
                said[-1] += data.message.text
            else:
                said.append(data.message.text)
            cut_off = data.finish_reason == "max-tokens"
            goes_on = False
        elif isinstance(data, UserMessageEvent):
            if data.source == CUT_OFF:
                goes_on = cut_off
            elif not data.source.startswith(CONTEXT_PREFIX):
                # What somebody said stands between: the next reply answers that.
                goes_on = cut_off = False
    return said
