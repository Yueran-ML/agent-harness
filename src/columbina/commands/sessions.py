"""The command that lists the sessions of the working directory.

This module is a plugin. Loading it registers the command ``/sessions``.
"""

import asyncio
import time

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.session import SESSIONS, Listed, Sessions

__all__ = ["apply", "inject", "name"]

name = "command-sessions"
inject = (COMMANDS, SESSIONS)

_MOST = 20


def _when(modified_ms: int) -> str:
    """Write a moment the way the clock of this machine shows it."""
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(modified_ms / 1000))


def _listed(stored: list[Listed], current: str) -> str:
    """List sessions, the one used last first, and mark the one that is open now."""
    if not stored:
        return "No session of this directory is stored."
    width = max(len(session.id) for session in stored[:_MOST])
    lines: list[str] = []
    for session in stored[:_MOST]:
        mark = "*" if session.id == current else " "
        said = session.first_input or "(nothing said yet)"
        lines.append(f"{mark} {session.id.ljust(width)}  {_when(session.modified_ms)}  {said}")
    if len(stored) > _MOST:
        lines.append(f"... and {len(stored) - _MOST} older ones")
    lines.append("The one with the star is open. Another one is taken up with --session-id.")
    return "\n".join(lines)


def apply(ctx: Context, config: object) -> None:
    """Register the command that lists the sessions."""
    sessions: Sessions = ctx.use(SESSIONS)

    async def run(agent: Agent, rest: str) -> str:
        stored = await asyncio.to_thread(sessions.stored, agent.cwd)
        return _listed(stored, agent.id)

    told = "List the sessions of this directory."
    ctx.use(COMMANDS).registry.add(ctx, "sessions", Command(run, told))
