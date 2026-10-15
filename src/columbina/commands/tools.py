"""The command that lists the tools, with how their calls went in the session.

This module is a plugin. Loading it registers the command ``/tools``.

The table of how the calls went is also what ``columbina sessions show`` ends
with, see :func:`used`.
"""

from collections.abc import Mapping

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.session import TOOLS_USED, ToolUse
from columbina.tools import TOOLS, Tools

__all__ = ["apply", "inject", "name", "used"]

name = "command-tools"
inject = (COMMANDS, TOOLS)

_DOES = {
    "none": "changes nothing",
    "read": "reads files",
    "write": "writes files",
    "run": "runs commands",
}


def _listed(tools: Tools) -> str:
    """List the tools that a model is offered, each with what a call of it does."""
    if not tools.registry:
        return "There are no tools."
    width = max(len(tool) for tool in tools.registry)
    lines: list[str] = []
    for tool in tools.registry.values():
        told = tool.description.strip().partition("\n")[0]
        lines.append(f"{tool.name.ljust(width)}  ({_DOES[tool.effect]})  {told}")
    return "\n".join(lines)


def _seconds(milliseconds: int) -> str:
    """Write how long something took the way a column has room for."""
    seconds = milliseconds / 1000
    return f"{seconds:.1f} s" if seconds < 100 else f"{seconds:.0f} s"


def used(tools: Mapping[str, ToolUse]) -> list[str]:
    """Return a table of how the calls of each tool went: how many, how many failed, how long.

    The tools stand in the order in which a call of each first ended. How
    many failed is said with the share of the calls that it is. The time is
    how long the tool ran: what a call waited for, such as a question to the
    user, is not in it.
    """
    rows = [("tool", "calls", "failed", "time")]
    for tool, use in tools.items():
        failed = f"{use.failed} ({round(100 * use.failed / use.calls)}%)" if use.failed else "0"
        rows.append((tool, str(use.calls), failed, _seconds(use.duration_ms)))
    widths = [max(len(row[column]) for row in rows) for column in range(4)]
    return [
        f"{first:<{widths[0]}}  {calls:>{widths[1]}}  {failed:>{widths[2]}}  {time:>{widths[3]}}"
        for first, calls, failed, time in rows
    ]


def apply(ctx: Context, config: object) -> None:
    """Register the command that lists the tools."""
    tools = ctx.use(TOOLS)

    async def run(agent: Agent, rest: str) -> str:
        listed = _listed(tools)
        calls = TOOLS_USED.of(agent.session).tools
        if not calls:
            return listed
        return "\n".join([listed, "", "Calls in this session:", *used(calls)])

    ctx.use(COMMANDS).registry.add(
        ctx, "tools", Command(run, "List the tools of the agent, and how their calls went.")
    )
