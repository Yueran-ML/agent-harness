"""What the agent service itself tells a model when a turn begins.

A loop asks the sources of context that are registered with the agent
service. One of them is here, because it speaks about nothing but the log of
a session: the note that the turn before did not come to its end.
"""

from columbina.agent.agent import Agent, ContextSource
from columbina.session import EventData, Projection, SessionEvent, TurnEndEvent

__all__ = ["INTERRUPTION"]


def _last_end(state: TurnEndEvent | None, event: SessionEvent[EventData]) -> TurnEndEvent | None:
    return event.data if isinstance(event.data, TurnEndEvent) else state


_ENDED: Projection[TurnEndEvent | None] = Projection("last-turn-end", None, _last_end)
"""How the last turn that ended came to its end."""


async def _interruption(agent: Agent) -> str | None:
    """Tell the model that the turn before this one did not come to its end.

    Without this, a reply that was cut short is followed by the next thing
    the user says, and nothing tells the model what happened in between.
    """
    ended = _ENDED.of(agent.session)
    if ended is None or ended.reason not in ("aborted", "interrupted"):
        return None
    how = (
        "The program ended while the turn before this one was under way."
        if ended.reason == "interrupted"
        else "The turn before this one was stopped before it had come to its end."
    )
    return (
        f"{how} A tool that was running then was stopped and may have done a part of its "
        "work, and calls that had not begun did not run. See how things stand before you "
        "rely on what that turn was doing."
    )


INTERRUPTION = ContextSource(_interruption, order=200, kind="note")
"""The note after a turn that was cut short. It is given once for each such turn."""
