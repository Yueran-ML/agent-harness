"""The command that lists the tools.

This module is a plugin. Loading it registers the command ``/tools``.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.tools import TOOLS, Tools

__all__ = ["apply", "inject", "name"]

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


def apply(ctx: Context, config: object) -> None:
    """Register the command that lists the tools."""
    tools = ctx.use(TOOLS)

    async def run(agent: Agent, rest: str) -> str:
        return _listed(tools)

    ctx.use(COMMANDS).registry.add(ctx, "tools", Command(run, "List the tools of the agent."))
