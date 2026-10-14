"""The tools of a server as tools that a model can call here.

A server lists its tools, each with a name, a word on what it is for, a
schema of its arguments and a few hints. Each becomes a tool of the tool
service, under a name that says which server it is of.

What a tool does to the world
    Whether a call needs to be approved goes by what the tool does. A server
    may hint that a tool only reads, and then it is taken for one that
    reads. Every other tool is taken for one that runs a program, which is
    what it is: something of somebody else's that does what it likes. The
    hint may be wrong. For a server on the same machine that adds no risk,
    since the server runs as the user anyway. For a server elsewhere, a call
    sends its arguments there whatever the tool then does with them.

What a call gives back
    The result of the server, as text, see
    :mod:`~columbina.mcp.server.content`. A result in which the server says
    that the tool failed is a failure of the tool here too, with that text,
    and so is a call that did not come to a result.

A list that is not used
    Two tools of one list that would come out under one name are a list
    that cannot be offered, and none of it is. The names are made so that
    this does not happen by chance.
"""

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Any, Protocol

from mcp.types import CallToolResult
from mcp.types import Tool as Listed

from columbina.mcp.server.connection import CallError
from columbina.mcp.server.content import in_part, shown
from columbina.mcp.server.names import tool_name
from columbina.mcp.server.schema import briefly, described, refused, repaired
from columbina.tools import JsonArguments, Tool, ToolFailure, ToolRun

__all__ = ["Call", "Offer", "offer"]


class Call(Protocol):
    """Calls a tool of the server by the name that the server gave it, with a limit in seconds."""

    def __call__(
        self,
        name: str,
        arguments: dict[str, Any],
        seconds: float,
        *,
        listed: Listed | None,
        reads: bool,
    ) -> Awaitable[CallToolResult]:
        """Make the call.

        ``listed`` is the tool as the server listed it when it was offered
        here. Whether a call is let through goes by that, so a call is made
        only for as long as the server lists the tool that way. ``reads``
        says whether the server says that the tool only reads: such a call
        may be made once more where the server turned it away.
        """
        ...


@dataclass(frozen=True, slots=True)
class Offer:
    """What comes of a list of tools.

    Attributes:
        tools: The tools that a model can call.
        passed_over: Why tools are not among them, a line for each reason.
    """

    tools: tuple[Tool[Any, Any], ...]
    passed_over: tuple[str, ...]


def _calling(
    listed: Listed, call: Call, seconds: float, reads: bool
) -> Callable[..., Awaitable[str]]:
    """Return what a tool does when it is called: it calls the tool of the server.

    The call says what the server said of the tool when it was offered here.
    A call that waits for a connection may find another tool under the name
    when the connection is there.
    """

    async def execute(arguments: JsonArguments, run: ToolRun) -> str:
        try:
            result = await call(
                listed.name, dict(arguments.root), seconds, listed=listed, reads=reads
            )
        except CallError as error:
            raise ToolFailure(str(error)) from None
        text = shown(result)
        if result.is_error:
            # A failure is not looked at by what keeps results short.
            raise ToolFailure(in_part(text))
        return text

    return execute


def offer(
    server: str,
    listed: Sequence[Listed],
    call: Call,
    *,
    only: frozenset[str] | None,
    call_seconds: float,
    limit_seconds: float,
) -> Offer:
    """Make tools of what a server lists.

    Args:
        server: What the server is called here.
        listed: The tools as the server lists them.
        call: Calls a tool of the server.
        only: The names, as the server gives them, of the tools that are
            wanted, or ``None`` for all of them.
        call_seconds: How long the server has to answer a call.
        limit_seconds: How long a call may take altogether, the wait for a
            connection that is being made again included.
    """
    tools: list[Tool[Any, Any]] = []
    passed_over: list[str] = []
    for tool in listed:
        raw = tool.name
        if only is not None and raw not in only:
            continue
        schema = repaired(tool.input_schema)
        reason = refused(schema)
        if reason is not None:
            passed_over.append(f"the tool {briefly(raw)} is not offered: {reason}")
            continue
        hints = tool.annotations
        reads = hints is not None and hints.read_only_hint is True
        tools.append(
            Tool(
                name=tool_name(server, raw),
                description=described(tool.description),
                parameters=JsonArguments,
                execute=_calling(tool, call, call_seconds, reads),
                timeout_seconds=limit_seconds,
                concurrency_safe=reads,
                effect="read" if reads else "run",
                input_schema=schema,
            )
        )
    names = [tool.name for tool in tools]
    twice = next((name for name in names if names.count(name) > 1), None)
    if twice is not None:
        return Offer(
            (), (f"two of the tools come out under the name {twice!r}, so none is offered",)
        )
    if only is not None:
        there = {tool.name for tool in listed}
        passed_over.extend(
            f"the server lists no tool {wanted!r}, which the config asks for"
            for wanted in sorted(only - there)
        )
    return Offer(tuple(tools), tuple(passed_over))
