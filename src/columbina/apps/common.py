"""What the frontends share: the words an agent begins with, and how its session is found."""

from columbina.agent import Agent, Agents, AgentSpec
from columbina.kernel import Context
from columbina.session import Sessions
from columbina.session.store import SESSION_FILE

__all__ = ["DEFAULT_PROMPT", "open_agent"]

DEFAULT_PROMPT = (
    "You are Columbina, an agent that works in a directory on the user's machine. "
    "Use the tools you are given to look at what is there before you answer or change "
    "anything. After a change, check that it does what it should, if you have a way to check. "
    "If a call is refused because it is not allowed, do not try to get the same thing done "
    "another way: say in your answer what was not allowed. "
    "When the task is done, say briefly what you found or did."
)
"""The instructions that open a conversation when a profile gives no others."""


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
