"""Tools from servers that speak the Model Context Protocol.

A server of that protocol offers tools of its own, and a model can call them
like the tools that come with Columbina. What a server says, the names of
its tools, what they are for and what they give back, is text that somebody
else wrote. So each of these is given a shape here before it reaches a
model: see :mod:`columbina.mcp.server`.

This module is a plugin. Loading it provides :data:`MCP`, the list of the
servers that are set up. Each server is a plugin of its own, loaded once for
every server, see :mod:`columbina.mcp.server`. This module knows nothing of
the SDK of the protocol, so it loads where that is not installed.

A turn waits for the servers
    The servers connect while the program starts, and the start does not
    wait for them. A turn does. Before its system prompt is put together,
    and before each of its steps, it waits until every server was connected
    or failed to be. That takes as long as one attempt to connect may take,
    and it takes no time once that is over. So a run of a single turn sees
    the tools as well.

What a server says of itself
    A server may say how its tools are meant to be used. That is added to
    the system prompt, each server's words under a line that says whose they
    are. They are the words of the server, and the line says so.
"""

import asyncio
from pathlib import Path
from typing import Protocol

from pydantic import BaseModel

from columbina.agent import PRE_STEP, StepDecision, StepInfo
from columbina.kernel import Context, Next, Registry, ServiceKey
from columbina.system_prompt import SYSTEM_PROMPT_ASSEMBLE, Draft

__all__ = ["MCP", "Config", "Mcp", "Server", "apply", "name"]

name = "mcp"


class Config(BaseModel):
    """The config of the list of servers.

    Attributes:
        log_dir: The directory for what the servers write to their standard
            error, one file for each server. Without it that is thrown away.
            It never goes to the terminal, where it would run through what
            is typed there.
    """

    log_dir: Path | None = None


class Server(Protocol):
    """What is known here of one server."""

    @property
    def state(self) -> str:
        """Where its connection stands, in a word or two."""
        ...

    @property
    def offered(self) -> int:
        """How many of its tools a model can call."""
        ...

    @property
    def failure(self) -> str | None:
        """Why its connection failed last, or ``None`` while nothing is wrong with it."""
        ...

    @property
    def said(self) -> str | None:
        """What stands in the system prompt for it, or ``None`` if it says nothing of itself."""
        ...

    async def settled(self) -> None:
        """Wait until its first attempt to connect came to an end, one way or the other."""
        ...


class Mcp:
    """The servers that are set up.

    Attributes:
        servers: The servers, by the names that they have here.
        log_dir: The directory for what the servers write to their standard
            error, or ``None`` if that is thrown away.
    """

    __slots__ = ("log_dir", "servers")

    def __init__(self, log_dir: Path | None) -> None:
        self.servers: Registry[Server] = Registry("MCP server")
        self.log_dir = log_dir

    async def ready(self) -> None:
        """Wait until every server was connected or failed to be."""
        await asyncio.gather(*(server.settled() for server in self.servers.values()))

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Let a step begin once the servers had their chance to offer their tools."""
        await self.ready()
        return await proceed(info)

    async def assembled(self, draft: Draft, proceed: Next[Draft, str]) -> str:
        """Add what the servers say of themselves to a system prompt, ordered by their names."""
        await self.ready()
        text = await proceed(draft)
        said = (server.said for _, server in sorted(self.servers.items()))
        return "\n\n".join(part for part in (text, *said) if part)


MCP = ServiceKey[Mcp]("mcp")
"""The servers of the Model Context Protocol that are set up."""


def apply(ctx: Context, config: Config) -> None:
    """Provide the list of servers, and have every turn wait for them."""
    mcp = Mcp(config.log_dir)
    ctx.provide(MCP, mcp)
    ctx.on(PRE_STEP, mcp.before_a_step)
    ctx.on(SYSTEM_PROMPT_ASSEMBLE, mcp.assembled)
