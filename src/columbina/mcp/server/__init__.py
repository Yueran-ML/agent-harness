"""One server of the Model Context Protocol, and what its tools look like to a model.

This module is a plugin, and it is loaded once for every server. It needs
:mod:`columbina.mcp`, where the server is listed, and the tool service, where
its tools are registered while it lists them.

The server connects in the background, and the program does not wait for it:
a turn does, see :mod:`columbina.mcp`. A server that cannot be reached is
tried again, and its failure is logged. The other servers and the rest of the
program go on without it. A server whose config says that it is required is
another matter: the program does not start without it.

The SDK of the protocol is needed from here on, and only from here on. It is
not installed with Columbina unless that is asked for, and where it is
missing this plugin says what to install.

Three things of a server are text that somebody else wrote, and each has a
module that gives it a shape which a model and a service can take.

:mod:`~columbina.mcp.server.names`
    The name under which a tool is offered. It says which server the tool is
    of, and it is made of the characters that a tool name may have.
:mod:`~columbina.mcp.server.schema`
    What a tool is for, and what its arguments look like.
:mod:`~columbina.mcp.server.content`
    What a call gave back, as the text that a model reads.

None of the three talks to a server. The first two know nothing of the SDK
of the protocol, and the third takes a result as the SDK hands it over.

:mod:`~columbina.mcp.server.connection`
    The connection to the server: made, kept, and made again where it is
    lost. It hands out the tools as the server lists them, and calls them by
    the names that the server gave them.
:mod:`~columbina.mcp.server.reach`
    How the server is reached: by starting a program, or at an address.
:mod:`~columbina.mcp.server.offer`
    The tools of the server as tools of the tool service.
:mod:`~columbina.mcp.server.served`
    The server as the rest of the program sees it.
"""

from pathlib import Path
from typing import Literal, Self
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, Field, model_validator

from columbina.kernel import Context
from columbina.mcp import MCP
from columbina.mcp.server.names import SERVER
from columbina.tools import TOOLS

__all__ = ["Config", "ServerError", "apply", "inject", "name"]

name = "mcp-server"
inject = (MCP, TOOLS)

_INSTALL = "pip install columbina[mcp]"


class ServerError(Exception):
    """A server cannot be set up, or one that is required cannot be reached."""


class Config(BaseModel):
    """The config of one server.

    A server is reached in one of two ways, and exactly one of ``command``
    and ``url`` says which.

    Attributes:
        server: What the server is called here: up to 32 letters, digits
            and dashes, with single underscores inside. The names of its
            tools begin with it.
        command: The program to start, for a server that is talked to over
            its standard input and output.
        args: The arguments of that program.
        env: The variables that the program gets, beside the few that every
            server gets. The environment of Columbina is not passed on.
        cwd: The directory that the program is started in.
        url: The address, for a server that is talked to over HTTP.
        headers: What every request to that address carries.
        tools: The tools that are wanted, by the names that the server
            gives them. Without it all of them are offered.
        startup_timeout_seconds: How long one attempt to connect may take.
        call_timeout_seconds: How long the server has to answer one call.
        required: Whether the program refuses to start where the server
            cannot be reached.
        max_instruction_bytes: How much of what the server says of itself
            is passed on to a model.
        protocol: ``auto`` asks the server which version of the protocol it
            speaks. ``legacy`` speaks to it as to a server from before 2026,
            for one that does not take the question.
    """

    model_config = ConfigDict(extra="forbid")

    server: str
    command: str | None = None
    args: tuple[str, ...] = ()
    env: dict[str, str] = Field(default_factory=dict)
    cwd: Path | None = None
    url: str | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    tools: frozenset[str] | None = None
    startup_timeout_seconds: float = Field(default=30.0, gt=0)
    call_timeout_seconds: float = Field(default=60.0, gt=0)
    required: bool = False
    max_instruction_bytes: int = Field(default=4096, ge=0)
    protocol: Literal["auto", "legacy"] = "auto"

    @model_validator(mode="after")
    def _reached_in_one_way(self) -> Self:
        if SERVER.fullmatch(self.server) is None:
            raise ValueError(
                f"{self.server!r} is no name for a server: a name has up to 32 letters, digits "
                "and dashes, and single underscores inside"
            )
        if (self.command is None) == (self.url is None):
            raise ValueError(
                "a server is reached by a command or at a url: give one of the two, and not both"
            )
        if self.url is not None:
            if self.args or self.env or self.cwd is not None:
                raise ValueError("args, env and cwd go with a command, and this server has a url")
            parts = urlsplit(self.url)
            if parts.scheme not in ("http", "https") or not parts.hostname:
                raise ValueError("the url of a server begins with http:// or https://")
        elif self.headers:
            raise ValueError("headers go with a url, and this server has a command")
        return self


async def apply(ctx: Context, config: Config) -> None:
    """Connect to the server, list it, and register its tools while it offers them.

    Raises:
        ServerError: The SDK of the protocol is not installed, or the server
            is required and could not be reached.
    """
    try:
        from columbina.mcp.server import reach
        from columbina.mcp.server.connection import Target
        from columbina.mcp.server.served import Served
    except ModuleNotFoundError as error:
        if (error.name or "").partition(".")[0] not in ("mcp", "httpx2"):
            raise
        raise ServerError(
            f"the MCP server {config.server} needs the SDK of the Model Context Protocol, "
            f"which is not installed. Install it with: {_INSTALL}"
        ) from None
    mcp = ctx.use(MCP)
    log = None if mcp.log_dir is None else mcp.log_dir / f"mcp-{config.server}.log"

    def target() -> Target:
        if config.command is not None:
            return reach.started(config.command, config.args, config.env, config.cwd, log)
        assert config.url is not None
        return reach.reached(config.url, config.headers)

    served = Served(
        ctx,
        config.server,
        target,
        ctx.use(TOOLS),
        only=config.tools,
        startup_seconds=config.startup_timeout_seconds,
        call_seconds=config.call_timeout_seconds,
        instruction_bytes=config.max_instruction_bytes,
        mode=config.protocol,
        kept_out=reach.kept_out(config.url, config.headers),
    )
    mcp.servers.add(ctx, config.server, served)
    ctx.effect(lambda: served.close)
    served.start()
    if not config.required:
        return
    await served.settled()
    if served.state != "connected":
        raise ServerError(
            f"the MCP server {config.server} is required, and it could not be reached: "
            f"{served.failure}"
        )
