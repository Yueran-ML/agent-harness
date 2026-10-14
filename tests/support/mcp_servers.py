"""Servers of the Model Context Protocol inside a test, and ways to them that a test can cut.

A server here is one of the SDK itself, so a test talks to what a real server is made of,
and needs no process and no network for it. One way to it is over HTTP all the same, see
:class:`Online`: at an address of this machine, where nothing but the test comes by.
"""

import asyncio
import json
import socket
import threading
import time
import warnings
from collections.abc import AsyncGenerator, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from types import TracebackType
from typing import Any, Self, cast

import anyio
import httpx2
import uvicorn
from mcp.server.mcpserver import Context, MCPServer
from mcp.shared.memory import create_client_server_memory_streams
from mcp.types import ToolAnnotations
from starlette.types import Message, Receive, Scope, Send

__all__ = ["Cuttable", "Hanging", "Online", "Served"]

INSTRUCTIONS = "Use the tools of this server."

SESSION = "mcp-session-id"
"""The header in which a request over HTTP names its session."""

Asked = dict[str, Any]
"""A request of the protocol, as the JSON that it is sent as."""

Choice = Callable[[Asked], bool]
"""Says which requests something is done with."""


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

    def with_its_session_told(self) -> None:
        """Add a tool that tells in which session it was called, for a server at an address."""

        def session(ctx: Context) -> str:
            return (ctx.headers or {}).get(SESSION, "")

        self.server.add_tool(session, name="session", description="Tell the session.")

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


class Online:
    """A server at an address of this machine, as a server is that is talked to over HTTP.

    The application of the SDK answers, and uvicorn serves it from a thread of its own. So
    a test talks to what a real server is made of, over a real socket. Before the
    application stands a doorkeeper, which a test can tell to turn requests away, or to
    have them wait.

    It is entered as soon as it is made, before anything connects to it, and left when
    everything has let go of it.

    Two tools of :class:`Served`, ``grow_as_before`` and ``stir``, silence a warning while
    they wait. The filters of warnings are one set for the whole process, and the tools of
    this server run in another thread, so those two are not for a test to call here.

    Attributes:
        url: The address.
        turned_away: How many requests the doorkeeper turned away.
        waiting: How many requests the doorkeeper has wait now.
    """

    def __init__(self, served: Served) -> None:
        # No session ends for having been idle: a test ends one when it means to.
        self._inner = served.server.streamable_http_app(session_idle_timeout=None)
        self._refused: tuple[str, int, str, int, Choice] | None = None
        self._slowed: tuple[float, Choice] | None = None
        self.turned_away = 0
        self.waiting = 0
        self._listening = socket.create_server(("127.0.0.1", 0))
        self.url = f"http://127.0.0.1:{self._listening.getsockname()[1]}/mcp"
        self._web = uvicorn.Server(
            uvicorn.Config(
                self._doorkeeper,
                # Named, because a method is not seen for the kind of application that it is.
                interface="asgi3",
                # A loop of the same kind on every system. A client of the SDK breaks off
                # an answer once it has read what it waited for, and the loop that Windows
                # gets otherwise fails while it closes a connection that was broken off:
                # the connection stays open, and the server waits for it when it stops.
                loop="asyncio:SelectorEventLoop",
                http="h11",
                ws="none",
                lifespan="on",
                # The logging of the test stays as the test set it up.
                log_config=None,
                timeout_graceful_shutdown=1,
            )
        )
        self._thread = threading.Thread(
            target=self._web.run, kwargs={"sockets": [self._listening]}, daemon=True
        )

    def __enter__(self) -> Self:
        self._thread.start()
        latest = time.monotonic() + 10
        while not self._web.started:
            if not self._thread.is_alive() or time.monotonic() > latest:
                self._web.should_exit = True
                self._thread.join(5)
                self._listening.close()
                raise RuntimeError("the server at an address did not start")
            time.sleep(0.005)
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        trace: TracebackType | None,
    ) -> None:
        self._web.should_exit = True
        self._thread.join(10)
        if self._thread.is_alive():
            # Whoever is still connected is waited for no longer. The thread ends with the
            # tests at the latest, so nothing here waits without an end.
            self._web.force_exit = True
            self._thread.join(5)
        self._listening.close()

    def turn_away(
        self,
        method: str | None,
        status: int = 404,
        said: str = "Session not found",
        *,
        code: int = -32600,
        only: Choice = lambda asked: True,
    ) -> None:
        """Have the requests of a method of the protocol answered with an error from now on.

        Args:
            method: The method, ``tools/call`` for one, or ``None`` to let everything
                through again.
            status: The status of the answer. 404 is what a server answers that knows
                nothing of the session that a request names.
            said: What the error says.
            code: The code of the error.
            only: Which of those requests are turned away. It is asked once for every
                request of the method, in the order in which they come, and the others
                are let through.
        """
        self._refused = None if method is None else (method, status, said, code, only)

    def slow(self, seconds: float, only: Choice) -> None:
        """Have some requests wait before anything else is done with them.

        Such a request waits for so many seconds, or until whoever sent it goes away, and
        is then turned away or let through like any other.
        """
        self._slowed = (seconds, only)

    def end(self, session: str) -> None:
        """End a session the way a client ends its own. The server knows nothing of it afterwards.

        This talks to the server and waits for its answer, so a test with a loop that runs
        calls it from a thread.
        """
        answer = httpx2.delete(self.url, headers={SESSION: session}, trust_env=False)
        assert answer.status_code == 200, answer.status_code

    async def _doorkeeper(self, scope: Scope, receive: Receive, send: Send) -> None:
        refused, slowed = self._refused, self._slowed
        if (
            (refused is None and slowed is None)
            or scope["type"] != "http"
            or scope["method"] != "POST"
        ):
            await self._inner(scope, receive, send)
            return
        # What the request holds is read to see what it asks for, and handed on as it came.
        read: list[Message] = []
        body = b""
        while True:
            message = await receive()
            read.append(message)
            if message["type"] != "http.request":
                break
            body += message.get("body", b"")
            if not message.get("more_body", False):
                break
        try:
            parsed: object = json.loads(body)
        except ValueError:
            parsed = None
        asked = cast("Asked", parsed) if isinstance(parsed, dict) else {}
        if slowed is not None and slowed[1](asked):
            self.waiting += 1
            try:
                if await _left_within(receive, slowed[0]):
                    return
            finally:
                self.waiting -= 1
        if refused is not None and asked.get("method") == refused[0] and refused[4](asked):
            self.turned_away += 1
            _, status, said, code, _ = refused
            error = {"jsonrpc": "2.0", "id": None, "error": {"code": code, "message": said}}
            await send(
                {
                    "type": "http.response.start",
                    "status": status,
                    "headers": [(b"content-type", b"application/json")],
                }
            )
            await send({"type": "http.response.body", "body": json.dumps(error).encode()})
            return

        async def again() -> Message:
            return read.pop(0) if read else await receive()

        await self._inner(scope, again, send)


async def _left_within(receive: Receive, seconds: float) -> bool:
    """Wait so many seconds, and tell whether whoever sent the request went away meanwhile."""
    try:
        async with asyncio.timeout(seconds):
            while (await receive())["type"] != "http.disconnect":
                pass
    except TimeoutError:
        return False
    return True
