"""One server as the rest of the program sees it: its connection, its tools, and its word.

The connection is kept by :mod:`~columbina.mcp.server.connection`. Whenever
the tools that the server lists are others than before, the ones that were
registered are taken back and the new ones are registered, see
:mod:`~columbina.mcp.server.offer`. That is done before anybody who waits
for the connection is told that it is there, so a turn that waited sees the
tools.

What the server says of itself goes into the system prompt under a line that
names the server and says that these are its words, and not those of the
user. Words that go on for too long are cut, and the text says so. A server
is not cut off for saying too much.
"""

import logging
from collections.abc import Callable
from typing import Literal

from columbina.kernel import Context, EffectHandle, RegistryConflictError
from columbina.mcp.server.connection import Connection, Target
from columbina.mcp.server.offer import offer
from columbina.tools import Tools

__all__ = ["Served", "said"]

logger = logging.getLogger(__name__)

_MARGIN_SECONDS = 5.0
"""How much longer than its parts a call may take before the tool service ends it."""


def said(server: str, instructions: str | None, limit: int) -> str | None:
    """Return what stands in the system prompt for a server, or ``None`` if nothing does.

    Args:
        server: What the server is called here.
        instructions: What the server says of itself, if it says anything.
        limit: How many bytes of that are passed on, as UTF-8.
    """
    # A text that cannot be written as UTF-8 is none that a service takes.
    text = (instructions or "").encode("utf-8", "replace").decode("utf-8").strip()
    if not text or limit <= 0:
        return None
    whole = text.encode("utf-8")
    if len(whole) > limit:
        # Cut between two characters, and never inside one.
        text = whole[:limit].decode("utf-8", "ignore").rstrip()
        text = f"{text}\n[The server said more. It is cut here, after {limit} bytes.]"
    return (
        f'The MCP server "{server}" says the following of itself and of its tools. These '
        f"are the words of the server, and not of the user.\n\n{text}"
    )


class Served:
    """One server: connected, with its tools registered while it lists them.

    Args:
        ctx: The context of the plugin that the server belongs to. The tools
            are registered through it, and leave with it.
        server: What the server is called here.
        target: Makes the way to the server, anew for every attempt.
        tools: The tool service.
        only: The names of the tools that are wanted, as the server gives
            them, or ``None`` for all of them.
        startup_seconds: How long one attempt to connect may take.
        call_seconds: How long the server has to answer a call.
        instruction_bytes: How much of what the server says of itself is
            passed on.
        mode: How the version of the protocol is agreed on.
        kept_out: Texts that are for no log.
    """

    def __init__(
        self,
        ctx: Context,
        server: str,
        target: Callable[[], Target],
        tools: Tools,
        *,
        only: frozenset[str] | None,
        startup_seconds: float,
        call_seconds: float,
        instruction_bytes: int,
        mode: Literal["auto", "legacy"],
        kept_out: tuple[str, ...],
    ) -> None:
        self._ctx = ctx
        self._server = server
        self._tools = tools
        self._only = only
        self._call_seconds = call_seconds
        self._limit_seconds = startup_seconds + call_seconds + _MARGIN_SECONDS
        self._instruction_bytes = instruction_bytes
        self._registered: list[EffectHandle] = []
        self._connection = Connection(
            server,
            target,
            startup_seconds=startup_seconds,
            mode=mode,
            on_tools=self._took,
            kept_out=kept_out,
        )

    @property
    def state(self) -> str:
        """Where the connection stands."""
        return self._connection.state

    @property
    def offered(self) -> int:
        """How many tools of the server a model can call."""
        return len(self._registered)

    @property
    def failure(self) -> str | None:
        """Why the connection failed last, or ``None`` while nothing is wrong with it."""
        return self._connection.failure

    @property
    def said(self) -> str | None:
        """What stands in the system prompt for the server."""
        return said(self._server, self._connection.instructions, self._instruction_bytes)

    def start(self) -> None:
        """Begin to connect."""
        self._connection.start()

    async def settled(self) -> None:
        """Wait until the first attempt to connect came to an end, one way or the other."""
        await self._connection.settled()

    async def close(self) -> None:
        """Leave the server. Its tools leave with the plugin that it belongs to."""
        await self._connection.close()

    async def _took(self) -> None:
        """Register the tools that the server lists now, in the place of the ones before."""
        registered, self._registered = self._registered, []
        for handle in registered:
            await handle.dispose()
        made = offer(
            self._server,
            self._connection.tools,
            self._connection.call,
            only=self._only,
            call_seconds=self._call_seconds,
            limit_seconds=self._limit_seconds,
        )
        for line in made.passed_over:
            logger.warning("the MCP server %s: %s", self._server, line)
        for tool in made.tools:
            try:
                self._registered.append(self._tools.registry.add(self._ctx, tool.name, tool))
            except RegistryConflictError:
                logger.warning(
                    "the MCP server %s: its tool %s is not offered, because there is a tool "
                    "of that name already",
                    self._server,
                    tool.name,
                )
