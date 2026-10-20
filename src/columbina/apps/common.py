"""What the frontends share.

The words an agent begins with, how its session is found, what is known of a
model that is named during a run, how a person is told that a turn did not
end well, and how another program that runs a command stops it.
"""

import asyncio
import contextlib
import os
import sys
import threading

from columbina.agent import Agent, Agents, AgentSpec
from columbina.kernel import Context
from columbina.llm import CallConfig
from columbina.session import Sessions, TurnEndEvent
from columbina.session.store import SESSION_FILE

__all__ = [
    "DEFAULT_PROMPT",
    "how_it_ended",
    "model_named",
    "open_agent",
    "when_the_input_closes",
]

DEFAULT_PROMPT = (
    "You are Columbina, an agent that works in a directory on the user's machine. "
    "The user gives you tasks. You do them with the tools you are given, and you tell the "
    "user what came of them."
)
"""The words that open a conversation when a profile gives no others.

They say who the agent is and no more. How an agent goes about its work is
said after them by :mod:`columbina.system_prompt.conduct`, whoever the agent is.
"""


async def open_agent(
    ctx: Context, agents: Agents, sessions: Sessions, spec: AgentSpec, session_id: str | None
) -> Agent:
    """Create an agent, on the stored session of that id if there is one.

    Without an id, and with an id that no stored session has, a new session
    begins. An id that is given becomes the id of the new session.

    Raises:
        SessionError: The stored session cannot be read or is open elsewhere.
        ValueError: The id cannot name a session.
    """
    if session_id is not None and sessions.root is not None:
        stored = sessions.directory(spec.cwd, session_id) / SESSION_FILE
        if stored.exists():
            return await agents.resume(ctx, spec, session_id)
    return await agents.create(ctx, spec, session_id=session_id)


def model_named(started: CallConfig, model: str) -> CallConfig:
    """Return the model of a run once a model is named during it, with what is known of its window.

    What is known of the size of a window is known of one model, as it is where a run starts
    and where a subagent names a model of its own. The window that the run started with is
    that of the model that it started with, and naming that model again gives the window
    back. Of any other model nothing is known.

    Args:
        started: The model that the run started with.
        model: The model that is named.
    """
    window = started.context_window if model == started.model else None
    return started.model_copy(update={"model": model, "context_window": window})


def how_it_ended(end: TurnEndEvent) -> str | None:
    """Say how a turn ended, for a person, or return ``None`` if it ended as turns are meant to."""
    detail = end.detail
    if end.reason == "aborted":
        return "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"
    if end.reason == "error":
        return f"(the turn ended with an error: {detail})"
    if end.reason == "max-tokens":
        return "(the reply was cut off: the model reached its limit of output)"
    if end.reason == "blocked":
        return f"(the turn was not allowed to go on: {detail})"
    if end.reason == "interrupted":
        return "(the turn was cut short when the program ended)"
    if end.reason == "forked":
        return "(the turn was cut short where its session was copied)"
    return None


def when_the_input_closes() -> asyncio.Event:
    """Return an event that is set when the standard input comes to its end.

    Another program stops a command that it runs, such as a sign-in or the assistant in
    Feishu, by closing the standard input of the command. Where there is no standard input
    to watch, the event is never set.
    """
    loop = asyncio.get_running_loop()
    closed = asyncio.Event()
    try:
        descriptor = sys.stdin.fileno()
    except (AttributeError, OSError, ValueError):
        return closed

    def watch() -> None:
        # The descriptor, and not sys.stdin: a thread that is still in a read of a stream of
        # Python when the program ends holds its lock, and Python cannot end cleanly.
        with contextlib.suppress(OSError):
            while os.read(descriptor, 4096):
                pass
        with contextlib.suppress(RuntimeError):
            loop.call_soon_threadsafe(closed.set)

    threading.Thread(target=watch, name="columbina-input", daemon=True).start()
    return closed
