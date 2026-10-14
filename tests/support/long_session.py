"""A session of many steps, made from constants, for the measurements of how long things take.

Each step is what a model that reads one file after another leaves in the log: the step
begins, the model asks for a file, the call is written down, its result comes back, and the
step ends. The text of every result is the same, so that two sessions of the same length are
the same.
"""

import json
from pathlib import Path

from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    AssistantMessageEvent,
    Sessions,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)

__all__ = ["LINES", "STEPS", "long_session"]

STEPS = 1000
"""How many steps a long session has, as the design asks for."""

LINES = 40
"""How many lines the result of each step has."""

_LINE = "    return sum(value for value in values if value is not None) / len(values)\n"


def long_session(sessions: Sessions, ctx: Context, cwd: Path, session_id: str = "long") -> None:
    """Store a session of one turn of :data:`STEPS` steps under ``cwd``."""
    session = sessions.create(ctx, cwd, session_id=session_id)
    session.append(TurnStartEvent(turn=1))
    session.append(
        UserMessageEvent(message=UserMessage(content=(TextBlock(text="Read every part."),)))
    )
    for step in range(1, STEPS + 1):
        call = f"call-{step}"
        arguments = json.dumps({"path": f"part{step}.py"})
        asking = AssistantMessage(
            content=(
                TextBlock(text=f"Part {step} comes next."),
                ToolCallBlock(call_id=call, name="read", arguments=arguments),
            )
        )
        usage = Usage(input_tokens=200, output_tokens=30, cache_read_tokens=step * 600)
        session.append(StepStartEvent(turn=1, step=step))
        session.append(
            AssistantMessageEvent(message=asking, usage=usage, finish_reason="tool-calls")
        )
        session.append(ToolCallEvent(call_id=call, name="read", arguments=arguments))
        result = ToolMessage(call_id=call, content=(TextBlock(text=_LINE * LINES),))
        session.append(ToolResultEvent(message=result))
        session.append(StepEndEvent(turn=1, step=step))
    session.append(TurnEndEvent(turn=1, reason="completed"))
    session.close()
