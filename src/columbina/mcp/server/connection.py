"""A connection to one server that is kept, and made again where it is lost.

One task keeps it
    A client of the SDK has to be left by the task that entered it. So each
    connection has a keeper, which gives every attempt a task of its own.
    That task enters a client, lists the tools, waits, and leaves the client
    at the end. A call comes from the task of whoever makes it, and goes to
    the client directly.

A connection that is lost
    Nobody tells the keeper that a server died. Whoever calls a tool is told:
    the call fails with the word that the connection is closed. A caller that
    hears this tells the keeper, see :meth:`Connection.call`. That call is
    answered with an error and is not made again, since nobody knows how far
    it got. The keeper leaves the old client, whatever leaving it raises,
    waits, and connects again. The waits begin at half a second and double up
    to thirty seconds. After ten failures in a row the keeper gives up, and
    the tools are withdrawn. The count begins anew once a connection has
    lasted thirty seconds, so a server that dies as often as it is started is
    not started without end.

    While the keeper connects again, the tools are still there. A call that
    comes then waits for the connection, as long as a connection may take to
    be made.

Tools that change
    A server may say that its tools changed. A server that speaks the
    protocol of 2026 says so to a client that listens, and an older one sends
    a notification. Either way the tools are listed anew. A server says
    nothing again of what changed before it was listened to. So it is asked
    to say when its tools change first, and for its tools after that: no
    change falls between the two. A list that cannot be had, or that names
    one tool twice, leaves the old one in place, and the next list that can
    be used ends what was wrong with it. A server whose word breaks off is
    asked for its list as well, and that shows whether the connection broke
    off with it.

Nothing here knows what a tool is to the rest of the program. The tools are
handed out as the server listed them, and a result as the SDK made it.
"""

import asyncio
import logging
import time
from collections.abc import AsyncIterator, Awaitable, Callable, Sequence
from contextlib import AsyncExitStack
from typing import TYPE_CHECKING, Any, Literal, TypeAlias, cast

from mcp import Client, MCPError
from mcp.client import IncomingMessage
from mcp.client.subscriptions import ListenNotSupportedError
from mcp.types import (
    CONNECTION_CLOSED,
    CallToolResult,
    Implementation,
    Tool,
    ToolListChangedNotification,
)

import columbina
from columbina import cancellation
from columbina.cancellation import CancelledFromInside

if TYPE_CHECKING:
    from mcp import StdioServerParameters
    from mcp.client import Transport
    from mcp.server.mcpserver import MCPServer

__all__ = [
    "ATTEMPTS",
    "FIRST_WAIT",
    "LEAVE_SECONDS",
    "LONGEST_WAIT",
    "STABLE_SECONDS",
    "CallError",
    "Connection",
    "State",
    "Target",
]

logger = logging.getLogger(__name__)

State = Literal["connecting", "connected", "reconnecting", "gave up", "closed"]
"""Where a connection stands.

``connecting`` until it was made for the first time, ``reconnecting`` while it
is made again, ``gave up`` after too many failures in a row, and ``closed``
once it was left for good.
"""

Target: TypeAlias = "StdioServerParameters | Transport | MCPServer | str"
"""What a client of the SDK connects to: a program to start, a transport, or an address.

A server object of the SDK itself is connected to inside this process. That is
for tests.
"""

FIRST_WAIT = 0.5
"""How many seconds the keeper waits after the first failure."""

LONGEST_WAIT = 30.0
"""How many seconds the keeper waits at most. Each wait is twice the one before it."""

ATTEMPTS = 10
"""How many failures in a row the keeper answers with another attempt."""

STABLE_SECONDS = 30.0
"""How long a connection has to last for the count of failures to begin anew."""

LEAVE_SECONDS = 10.0
"""How long leaving a client may take. A server that is dead may not let go by itself."""

_SAID = 300
"""How many characters of the reason for a failure are passed on."""


class CallError(Exception):
    """A call did not come to a result. The text says why, to whoever made the call.

    A result in which the server says that the tool failed is a result, and
    is no error of this kind.
    """


async def _listed(client: Client) -> list[Tool]:
    """Return every tool that a server lists, page after page."""
    tools: list[Tool] = []
    cursor: str | None = None
    while True:
        page = await client.list_tools(cursor=cursor)
        tools.extend(page.tools)
        cursor = page.next_cursor
        if cursor is None:
            return tools


def _named_twice(tools: Sequence[Tool]) -> str | None:
    """Return a name that two of the tools have, or ``None`` if there is none."""
    seen: set[str] = set()
    for tool in tools:
        if tool.name in seen:
            return tool.name
        seen.add(tool.name)
    return None


class Connection:
    """One server, kept connected.

    Args:
        server: What the server is called here. The name stands in what is
            logged and in what a caller is told.
        target: Makes what a client of the SDK connects to. It is asked anew
            for every attempt, since a transport is entered once.
        startup_seconds: How long one attempt to connect may take, the
            listing of the tools included. A call waits as long for a
            connection that is being made again.
        mode: How the version of the protocol is agreed on. ``auto`` asks
            the server what it speaks, and ``legacy`` speaks to it as to a
            server from before 2026.
        on_tools: Called and waited for when the tools are others than
            before: after a connection was made, after the server listed them
            anew, and when they are withdrawn. Whoever waits for the first
            attempt is told that it is over only after this has come back.
        kept_out: Texts that are in no reason for a failure: what the
            address or the headers of a server hold that is not for a log.
        sleep: Waits for a number of seconds. A test gives one that does not.
        clock: Tells the time, in seconds that only go up.
    """

    def __init__(
        self,
        server: str,
        target: Callable[[], Target],
        *,
        startup_seconds: float = 30.0,
        mode: Literal["auto", "legacy"] = "auto",
        on_tools: Callable[[], Awaitable[None]] | None = None,
        kept_out: Sequence[str] = (),
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self.server = server
        self._target = target
        self._startup = startup_seconds
        self._mode: Literal["auto", "legacy"] = mode
        self._on_tools = on_tools
        # A text of no length is in every text, and nothing is kept out by taking it out.
        self._kept_out = tuple(text for text in kept_out if text)
        self._sleep = sleep
        self._clock = clock
        self._tools: tuple[Tool, ...] = ()
        self._instructions: str | None = None
        self._failure: str | None = None
        self._client: Client | None = None
        self._keeper: asyncio.Future[None] | None = None
        self._ever = False
        self._gave_up = False
        self._closing = False
        # Set once the first attempt came to an end, one way or the other.
        self._settled = asyncio.Event()
        # Set, and a new one put in its place, whenever a caller that waits has something
        # to look at: a client is there, or none will come.
        self._moved = asyncio.Event()
        # Set when the client that is there found its connection closed.
        self._lost = asyncio.Event()
        # Set when a server of the older protocol said that its tools changed.
        self._changed = asyncio.Event()

    @property
    def state(self) -> State:
        """Where the connection stands."""
        if self._closing:
            return "closed"
        if self._client is not None:
            return "connected"
        if self._gave_up:
            return "gave up"
        return "reconnecting" if self._ever else "connecting"

    @property
    def tools(self) -> tuple[Tool, ...]:
        """The tools as the server listed them last: none before it did, none after giving up."""
        return self._tools

    @property
    def instructions(self) -> str | None:
        """What the server says of itself, as it said it when it was connected last."""
        return self._instructions

    @property
    def failure(self) -> str | None:
        """Why the connection failed last, or ``None`` while nothing is wrong with it."""
        return self._failure

    def start(self) -> None:
        """Begin to connect, and to keep the connection.

        A second call does nothing, and neither does one after the connection was closed.
        """
        if self._keeper is None and not self._closing:
            self._keeper = asyncio.ensure_future(self._keep())

    async def settled(self) -> None:
        """Wait until the first attempt to connect came to an end, one way or the other."""
        await self._settled.wait()

    async def close(self) -> None:
        """Leave the connection for good. Whoever waits for a call is answered."""
        self._closing = True
        self._move()
        keeper, self._keeper = self._keeper, None
        if keeper is None:
            return
        keeper.cancel()
        await asyncio.wait([keeper], timeout=LEAVE_SECONDS)
        if not keeper.done():
            logger.warning(
                "the MCP server %s did not let go within %g seconds", self.server, LEAVE_SECONDS
            )

    async def call(self, name: str, arguments: dict[str, Any], seconds: float) -> CallToolResult:
        """Call a tool of the server.

        A call that comes while the connection is made again waits for it.
        A call that finds the connection closed tells the keeper, and is not
        made again.

        Args:
            name: The name of the tool, as the server gave it.
            arguments: The arguments of the call.
            seconds: How long the call may take: to be written to the server
                and to be answered by it.

        Returns:
            What the server answered. It may say that the tool failed.

        Raises:
            CallError: The call did not come to a result.
            asyncio.CancelledError: Whoever made the call was cancelled. The
                server was told to stop.
        """
        try:
            return await cancellation.guard(
                self._asked(name, arguments, seconds),
                what=f"the call of the MCP server {self.server}",
            )
        except CancelledFromInside:
            raise CallError(
                f"the call was broken off inside the connection to the MCP server {self.server}, "
                "and not by whoever made it. It may have taken effect all the same"
            ) from None

    async def _asked(self, name: str, arguments: dict[str, Any], seconds: float) -> CallToolResult:
        """Make a call over the connection, or say in the words of this program why not."""
        client = await self._connected()
        # The whole call has so long, and not only its answer. The SDK would count from when
        # the call is written, and a server that reads no more keeps it from being written.
        limit = asyncio.timeout(seconds)
        try:
            async with limit:
                return await client.call_tool(name, arguments)
        except MCPError as error:
            if error.code == CONNECTION_CLOSED:
                self._gone(client)
                raise CallError(
                    f"the connection to the MCP server {self.server} was lost while the call "
                    "was under way. It is not known how far the call got, and it was not made "
                    "again"
                ) from None
            raise CallError(
                f"the MCP server {self.server} answered the call with an error: {self._why(error)}"
            ) from None
        except Exception as error:
            if limit.expired():
                # The SDK has told the server to stop, as far as that could be written.
                raise CallError(
                    f"the MCP server {self.server} did not answer within {seconds:g} seconds. "
                    "The call may have taken effect all the same"
                ) from None
            raise CallError(
                f"what the MCP server {self.server} answered cannot be used: {self._why(error)}"
            ) from None

    async def _connected(self) -> Client:
        """Return the client to call, and wait for it while the connection is being made."""
        try:
            async with asyncio.timeout(self._startup):
                while self._client is None and not (self._closing or self._gave_up):
                    await self._moved.wait()
        except TimeoutError:
            raise CallError(
                f"the MCP server {self.server} is being connected again, and was not within "
                f"{self._startup:g} seconds"
            ) from None
        if self._client is not None:
            return self._client
        if self._closing:
            raise CallError(f"the connection to the MCP server {self.server} is closed")
        raise CallError(
            f"the MCP server {self.server} is not connected, and it is no longer tried: "
            f"{self._failure}"
        )

    def _move(self) -> None:
        """Wake whoever waits for the connection to be made, given up or closed."""
        moved, self._moved = self._moved, asyncio.Event()
        moved.set()

    def _gone(self, client: Client) -> None:
        """Tell the keeper that a client found its connection closed."""
        if client is self._client:
            self._lost.set()

    async def _keep(self) -> None:
        """Connect, and connect again after every loss, until there were too many in a row."""
        failures = 0
        while True:
            # Every attempt has a task of its own. A client that had to be broken off while
            # it was left may leave that task in no state to enter another one.
            lasted = await asyncio.ensure_future(self._attempt())
            if self._closing:
                return
            if lasted is not None and lasted >= STABLE_SECONDS:
                failures = 0
            failures += 1
            if failures > ATTEMPTS:
                self._gave_up = True
                self._move()
                logger.error(
                    "the MCP server %s is no longer tried after %d failures in a row: %s",
                    self.server,
                    failures,
                    self._failure,
                )
                await self._offer(())
                return
            await self._sleep(min(FIRST_WAIT * 2 ** (failures - 1), LONGEST_WAIT))

    async def _attempt(self) -> float | None:
        """Connect, offer the tools for as long as the connection lasts, and leave it.

        Returns:
            How many seconds the connection lasted, or ``None`` if none was
            made.
        """
        lasted: float | None = None
        stack = AsyncExitStack()
        limit = asyncio.timeout(self._startup)
        try:
            try:
                async with limit:
                    client = await stack.enter_async_context(self._made())
                    # First the server listens, and then it lists. It says nothing again
                    # of what changed before it listened, so nothing may change in between.
                    said = await self._listening(stack, client)
                    listed = await _listed(client)
                since = self._clock()
                try:
                    await self._held(client, listed, said)
                finally:
                    # It lasted until it was lost. Leaving it may take seconds more, and in
                    # those the server was of no use to anyone.
                    lasted = self._clock() - since
                self._failure = "the connection was lost"
            finally:
                # Whoever calls from now on waits for the next connection.
                self._client = None
                await self._leave(stack)
        except (asyncio.CancelledError, Exception, BaseExceptionGroup) as error:
            if not self._closing:
                self._failure = (
                    f"no connection within {self._startup:g} seconds"
                    if limit.expired()
                    else self._why(error)
                )
                logger.warning("the MCP server %s is not connected: %s", self.server, self._failure)
        self._settled.set()
        return lasted

    def _made(self) -> Client:
        """Make a client for one attempt."""
        return Client(
            self._target(),
            mode=self._mode,
            message_handler=self._told,
            client_info=Implementation(name="columbina", version=columbina.__version__),
            # The tools are listed when the server says that they changed, and then the
            # answer has to be the one of the server.
            cache=None,
        )

    async def _listening(
        self, stack: AsyncExitStack, client: Client
    ) -> AsyncIterator[object] | None:
        """Have a server say when its tools change, for as long as its client is entered.

        Returns:
            What gives something whenever the server says that its tools
            changed, or ``None`` for a server that does not say so.
        """
        try:
            return await stack.enter_async_context(client.listen(tools_list_changed=True))
        except ListenNotSupportedError:
            # An older server sends a notification without being asked, see :meth:`_told`.
            return self._notified()
        except Exception as error:
            # It refuses, or its word broke off at once. Whether the connection broke off
            # with it shows when the tools are asked for.
            logger.debug(
                "the MCP server %s does not say when its tools change: %s",
                self.server,
                self._why(error),
            )
            return None

    async def _notified(self) -> AsyncIterator[None]:
        """Give something whenever a server of the older protocol said that its tools changed."""
        while True:
            await self._changed.wait()
            self._changed.clear()
            yield

    async def _held(
        self, client: Client, listed: Sequence[Tool], said: AsyncIterator[object] | None
    ) -> None:
        """Offer what a connection gives until the connection is lost."""
        self._lost.clear()
        self._client = client
        self._ever = True
        self._instructions = client.instructions
        await self._offer(listed)
        self._move()
        self._settled.set()
        watching = asyncio.ensure_future(self._watch(client, said))
        try:
            await self._lost.wait()
        finally:
            watching.cancel()
            await asyncio.wait([watching])

    async def _leave(self, stack: AsyncExitStack) -> None:
        """Leave a client, whatever leaving it raises."""
        try:
            async with asyncio.timeout(LEAVE_SECONDS):
                await stack.aclose()
        except (asyncio.CancelledError, Exception, BaseExceptionGroup) as error:
            # A connection that broke says so once more when it is left. Nothing follows from it.
            logger.debug("leaving the MCP server %s: %s", self.server, self._why(error))

    async def _offer(self, listed: Sequence[Tool]) -> None:
        """Offer the tools that were listed, unless the list names one of them twice."""
        twice = _named_twice(listed)
        if twice is not None:
            self._failure = f"the server lists the tool {twice!r} twice, so its list is not used"
            logger.warning("the MCP server %s: %s", self.server, self._failure)
            return
        if self._client is not None:
            # A list that can be used. What was wrong before it, with a list or with the
            # connection, is over. Where the server is given up, why it is stays to be read.
            self._failure = None
        if tuple(listed) == self._tools:
            return
        self._tools = tuple(listed)
        if self._on_tools is None:
            return
        try:
            await self._on_tools()
        except Exception:
            logger.exception(
                "the MCP server %s: what was to be told of its tools failed", self.server
            )

    async def _told(self, message: IncomingMessage) -> None:
        """Take what a server of the older protocol says without being asked."""
        if isinstance(message, ToolListChangedNotification):
            self._changed.set()

    async def _watch(self, client: Client, said: AsyncIterator[object] | None) -> None:
        """List the tools anew whenever the server says that they changed."""
        if said is None:
            return
        try:
            async for _ in said:
                await self._relist(client)
        except Exception as error:
            logger.debug(
                "the MCP server %s says no more of its tools: %s", self.server, self._why(error)
            )
        # The server says no more, because it ended that in good order or its word broke
        # off. Asking for the list once more shows whether the connection broke off with it.
        await self._relist(client)

    async def _relist(self, client: Client) -> None:
        """Ask the server for its tools, and offer them. No list, and nothing changes."""
        try:
            async with asyncio.timeout(self._startup):
                listed = await _listed(client)
        except Exception as error:
            if isinstance(error, MCPError) and error.code == CONNECTION_CLOSED:
                self._gone(client)
            logger.warning(
                "the MCP server %s did not list its tools anew, and the list that was there "
                "stays: %s",
                self.server,
                self._why(error),
            )
            return
        await self._offer(listed)

    def _why(self, error: BaseException) -> str:
        """Say in one line why something failed, without what is to be kept out of it."""
        if isinstance(error, BaseExceptionGroup):
            several = cast("BaseExceptionGroup[BaseException]", error)
            said = "; ".join(self._why(inner) for inner in several.exceptions)
        elif isinstance(error, asyncio.CancelledError):
            said = "it was broken off from inside"
        else:
            said = " ".join(str(error).split()) or type(error).__name__
        # The longest first. A text that is part of a longer one would leave the rest of it.
        for secret in sorted(self._kept_out, key=len, reverse=True):
            said = said.replace(secret, "[...]")
        return said if len(said) <= _SAID else f"{said[:_SAID].rstrip()} [...]"
