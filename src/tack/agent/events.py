"""What a running agent tells the plugins around it, and what it asks them.

These are kernel events: they are for code that runs in the same process and
wants to watch or steer a turn while it happens. What a turn leaves behind is
in the session log.
"""

from dataclasses import dataclass

from tack.agent.agent import Agent
from tack.kernel import EmitEvent
from tack.llm.types import Chunk


@dataclass(frozen=True, slots=True)
class StreamFrame:
    """One chunk of a reply, as it arrives.

    Attributes:
        agent: The agent that is being answered.
        turn: The number of the turn.
        step: The number of the step in that turn.
        chunk: The chunk.
    """

    agent: Agent
    turn: int
    step: int
    chunk: Chunk


STREAM: EmitEvent[StreamFrame] = EmitEvent("agent/assistant-stream")
"""Emitted for every chunk of every reply, the finish included."""
