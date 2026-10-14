"""Servers of the Model Context Protocol inside a test, and ways to them that a test can cut.

A server here is one of the SDK itself, so a test talks to what a real server is made of,
and needs no process and no network for it.
"""

import warnings
from collections.abc import AsyncGenerator
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from types import TracebackType
from typing import Any

import anyio
from mcp.server.mcpserver import Context, MCPServer
from mcp.shared.memory import create_client_server_memory_streams
from mcp.types import ToolAnnotations

__all__ = ["Cuttable", "Hanging", "Served"]

INSTRUCTIONS = "Use the tools of this server."


class Served:
    """A server for a test.

    Attributes:
        server: The server, as the SDK makes it.
        napping: How many naps were begun.
        cancelled: How many of its naps were broken off.
        grown: How many tools were added to it while it ran.
        peeked: How often the tool was called of which it says that it only reads.
    """

    def __init__(self, instructions: str | None = INSTRUCTIONS) -> None:
        self.server = MCPServer("served", instructions=instructions)
        self.napping = 0
        self.cancelled = 0
        self.grown = 0
        self.peeked = 0
        served = self

        @self.server.tool()
        def add(a: int, b: int) -> int:
            """Add two numbers."""
            return a + b

        @self.server.tool()
        async def nap(seconds: float) -> str:
            """Sleep for a while."""
            served.napping += 1
            try:
                await anyio.sleep(seconds)
            except BaseException:
                served.cancelled += 1
                raise
            return "slept"

        @self.server.tool()
        async def grow(ctx: Context) -> str:
            """Add a tool, and say so as a server of 2026 does."""
            served.one_more()
            await ctx.notify_tools_changed()
            return "grown"

        @self.server.tool()
        async def grow_as_before(ctx: Context) -> str:
            """Add a tool, and say so as servers did before 2026."""
            served.one_more()
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                await ctx.session.send_tool_list_changed()
            return "grown"

        @self.server.tool()
        async def stir(ctx: Context) -> str:
            """Say that something changed which is no tool, as servers did before 2026."""
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                await ctx.session.send_resource_list_changed()
            return "stirred"

    def with_a_reader(self) -> None:
        """Add a tool of which the server says that it only reads."""

        def peek() -> str:
            self.peeked += 1
            return "peeked"

        self.server.add_tool(
            peek,
            name="peek",
            description="Look without touching.",
            annotations=ToolAnnotations(read_only_hint=True),
        )

    def with_a_reader_that_writes_now(self) -> None:
        """Have the tool that only read be one of which the server says no such thing."""
        self.server.remove_tool("peek")

        def peek() -> str:
            self.peeked += 1
            return "wrote"

        self.server.add_tool(peek, name="peek", description="Look without touching.")

    def one_more(self) -> str:
        """Add a tool to the server without a word, and return its name."""
        self.grown += 1
        name = f"more{self.grown}"

        def more() -> str:
            return name

        self.server.add_tool(more, name=name, description="One more tool.")
        return name


class Cuttable:
    """A way to a server in this process that a test can cut, as a server that dies cuts it.

    It is entered once for every connection, and every time the server begins anew to
    listen, as a program does that is started again.

    Attributes:
        entered: How often it was entered.
        left: How often it was left.
        leaving: What leaving it raises after it was left, if anything.
    """

    def __init__(self, served: Served) -> None:
        self._served = served
        self._open: AbstractAsyncContextManager[Any] | None = None
        self._serving: anyio.CancelScope | None = None
        self._to_client: Any = None
        self.entered = 0
        self.left = 0
        self.leaving: BaseException | None = None

    @asynccontextmanager
    async def _connection(self) -> AsyncGenerator[Any]:
        low = self._served.server._lowlevel_server  # pyright: ignore[reportPrivateUsage]
        async with create_client_server_memory_streams() as (ours, theirs):
            read, write = ours
            their_read, their_write = theirs

            async def serve() -> None:
                with anyio.CancelScope() as scope:
                    self._serving = scope
                    await low.run(their_read, their_write, low.create_initialization_options())

            self._to_client = their_write
            async with anyio.create_task_group() as group:
                group.start_soon(serve)
                try:
                    yield read, write
                finally:
                    await write.aclose()
                    group.cancel_scope.cancel()

    async def __aenter__(self) -> Any:
        self.entered += 1
        self._open = self._connection()
        return await self._open.__aenter__()

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        trace: TracebackType | None,
    ) -> None:
        opened, self._open = self._open, None
        assert opened is not None
        await opened.__aexit__(kind, error, trace)
        self.left += 1
        if self.leaving is not None:
            raise self.leaving

    async def cut(self) -> None:
        """End the server and close its end, without a word to whoever is connected."""
        assert self._serving is not None
        self._serving.cancel()
        await self._to_client.aclose()


class Hanging:
    """A way to a server that never answers whoever enters it."""

    def __init__(self) -> None:
        self.entered = 0

    async def __aenter__(self) -> Any:
        self.entered += 1
        await anyio.sleep_forever()

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        trace: TracebackType | None,
    ) -> None:
        raise AssertionError("what was never entered is not left")
