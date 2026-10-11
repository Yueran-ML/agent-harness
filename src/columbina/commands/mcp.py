"""The command that lists the servers of the Model Context Protocol.

This module is a plugin. Loading it registers the command ``/mcp``.

``/mcp`` shows every server that is set up, in the order of the names: where
its connection stands, how many of its tools a model can call, and why the
connection failed last if it did. A server that is being connected again
keeps its tools meanwhile, and one that was given up has none.

Why a connection failed is text that the server or the system wrote. Whoever
shows what the command answers makes it fit for a terminal first.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.mcp import MCP, Mcp

__all__ = ["apply", "inject", "name"]

name = "command-mcp"
inject = (COMMANDS, MCP)


def _listed(mcp: Mcp) -> str:
    """List the servers for a person."""
    if not mcp.servers:
        return "No MCP server is set up."
    width = max(len(named) for named in mcp.servers)
    lines: list[str] = []
    for named, server in sorted(mcp.servers.items()):
        tools = "1 tool" if server.offered == 1 else f"{server.offered} tools"
        line = f"{named.ljust(width)}  {server.state}  {tools}"
        lines.append(line if server.failure is None else f"{line}  ({server.failure})")
    return "\n".join(lines)


def apply(ctx: Context, config: object) -> None:
    """Register the command that lists the servers."""
    mcp: Mcp = ctx.use(MCP)

    async def show(agent: Agent, rest: str) -> str:
        return _listed(mcp)

    ctx.use(COMMANDS).registry.add(
        ctx,
        "mcp",
        Command(show, "List the MCP servers: how each is connected, and how many tools it has."),
    )
