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
    waits, and connects again. It logs the loss in one line, however many
    found the connection closed. The waits begin at half a second and double
    up to thirty seconds. After ten failures in a row the keeper gives up,
    and the tools are withdrawn. The count begins anew once a connection has
    lasted thirty seconds, so a server that dies as often as it is started is
    not started without end.

    While the keeper connects again, the tools are still there. A call that
    comes then waits for the connection, as long as a connection may take to
    be made.

A session that the server ended
    A server that is talked to over HTTP in the way of before 2026 gives a
    connection a session, and ends it when it likes: after a while without a
    call, or because it was started again. From then on it answers whatever
    names that session with a status that says so, and the SDK hands that on
    as an error like any other. The connection would stay as it is, and
    every call would fail. So the way to the server looks at the status, and
    says so where a call was answered like this, see :func:`turned_away`.

    The keeper is told, and lets the other calls that are under way hear
    their own answers for a moment before it leaves the client. By the
    protocol the server took nothing of a call that it turned away like
    this. A server may be worse than the protocol, so only a call of a tool
    that only reads is made again by itself, over the next connection and
    once. Any other call is answered with an error that says so, and
    whoever made it decides. A session that the server ended is no failure
    of the server, and it does not count towards giving the server up.

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
from contextlib import AsyncExitStack, suppress
from contextvars import ContextVar
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
    "HEAR_SECONDS",
    "LEAVE_SECONDS",
    "LONGEST_WAIT",
    "STABLE_SECONDS",
    "CallError",
    "Connection",
    "State",
    "Target",
    "turned_away",
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

HEAR_SECONDS = 2.0
"""How long the calls that are under way have to hear their answers, where a session ended.

The keeper waits so long at most before it leaves the client. A call that was sent after the
server ended the session is turned away within a moment, and one that the server took before
is cut off when the time is up.
"""

_GRACE = 0.5
"""How many seconds longer closing waits than leaving a client may take.

The limit on leaving begins a moment after closing begins to wait. With the
same number for both, closing would give up just before the client is left.
"""

_SAID = 300
"""How many characters of the reason for a failure are passed on."""

_LOST = "the connection was lost"
"""Why a server is not connected where its connection ended without an error."""

_ENDED = "the server no longer knew the session"
"""Why a server is not connected where it turned a call away for want of its session."""


class CallError(Exception):
    """A call did not come to a result. The text says why, to whoever made the call.

    A result in which the server says that the tool failed is a result, and
    is no error of this kind.
    """


class _TurnedAway(Exception):  # noqa: N818 - named after what happened
    """A server turned a call away as one that names a session it knows nothing of.

    Attributes:
        said: What the server said, in one line.
    """

    def __init__(self, said: str) -> None:
        super().__init__(said)
        self.said = said


class _Call:
    """What is known of a call that is being made, beside what it comes back with.

    Attributes:
        turned_away: Whether the server answered that it knows nothing of
            the session, and so took nothing of the call.
    """

    __slots__ = ("turned_away",)

    def __init__(self) -> None:
        self.turned_away = False


_calling: ContextVar[_Call | None] = ContextVar("columbina.mcp.calling", default=None)
"""The call that is being made in this context, if one is."""


def turned_away() -> None:
    """Say that the server turned away the call that is being made here.

    For a way to a server, which calls this where it sees that the server
    answered a call as one that names a session it knows nothing of. The SDK
    writes a request to the server in a copy of the context of whoever made
    it. The copy holds the same object as the context of the caller, and
    what is noted on that object is seen there: so what is said here
    reaches the call that was turned away and no other. Setting the
    variable anew would reach nobody. Where no call is being made, nothing
    comes of it.
    """
    call = _calling.get()
    if call is not None:
        call.turned_away = True


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
        # Why the client that was there last is none any more, as whoever found it so said.
        self._lost_for = _LOST
        # How many calls are under way, and an event that is set while none is.
        self._under_way = 0
        self._quiet = asyncio.Event()
        self._quiet.set()
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
        """Leave the connection for good. Whoever waits for a call is answered.

        Leaving a client has a limit of its own. This waits a moment longer
        than that, so that it sees the limit kept and comes back when the
        client is left. A connection that does not let go even then is not
        waited for for good: this says so and comes back. Whoever closes it
        again waits for it again, and does not break off a second time what
        may be letting go by then.
        """
        told = self._closing
        self._closing = True
        self._move()
        keeper = self._keeper
        if keeper is None:
            return
        if not told:
            keeper.cancel()
        await asyncio.wait([keeper], timeout=LEAVE_SECONDS + _GRACE)
        if not keeper.done():
            logger.warning(
                "the MCP server %s did not let go within %g seconds", self.server, LEAVE_SECONDS
            )

    async def call(
        self,
        name: str,
        arguments: dict[str, Any],
        seconds: float,
        *,
        listed: Tool | None = None,
        reads: bool = False,
    ) -> CallToolResult:
        """Call a tool of the server.

        A call that comes while the connection is made again waits for it.
        A call that finds the connection closed tells the keeper, and is not
        made again. A call that the server turns away because it knows
        nothing of the session tells the keeper too, and is made once more
        over the next connection where its tool only reads.

        Args:
            name: The name of the tool, as the server gave it.
            arguments: The arguments of the call.
            seconds: How long the call may take: to be written to the server
                and to be answered by it. A call that is made once more has
                as long again.
            listed: The tool as the server listed it when the call was
                decided on, if whoever calls goes by that. A call may wait
                for a connection, and the server that comes back may list a
                tool of that name of which it says something else: that it
                no longer only reads, for one. A call that was let through
                for what the tool was is then not made.
            reads: Whether the server says that the tool only reads. Such a
                call does no harm if it is made twice.

        Returns:
            What the server answered. It may say that the tool failed.

        Raises:
            CallError: The call did not come to a result.
            asyncio.CancelledError: Whoever made the call was cancelled. The
                server was told to stop.
        """
        try:
            return await cancellation.guard(
                self._asked(name, arguments, seconds, listed, reads),
                what=f"the call of the MCP server {self.server}",
            )
        except CancelledFromInside:
            raise CallError(
                f"the call was broken off inside the connection to the MCP server {self.server}, "
                "and not by whoever made it. It may have taken effect all the same"
            ) from None

    async def _asked(
        self,
        name: str,
        arguments: dict[str, Any],
        seconds: float,
        listed: Tool | None,
        reads: bool,
    ) -> CallToolResult:
        """Make a call over the connection, or say in the words of this program why not."""
        try:
            return await self._once(name, arguments, seconds, listed)
        except _TurnedAway as away:
            if not reads:
                # By the protocol the server took nothing of the call. A server may be worse
                # than the protocol, and a call that changes something would then change it
                # twice: so whoever made the call decides whether to make it again.
                raise CallError(
                    f"the MCP server {self.server} turned the call away as one that names a "
                    f"session it knows nothing of, and said: {away.said}. The connection is "
                    "being made again. Call the tool again if that is still wanted"
                ) from None
        # The keeper makes the connection again, and over that the call is made, this once.
        try:
            return await self._once(name, arguments, seconds, listed)
        except _TurnedAway as away:
            raise CallError(
                f"the MCP server {self.server} turned the call away as one that names a "
                "session it knows nothing of, also over a connection that was made anew for "
                f"it, and said: {away.said}. The call was not made"
            ) from None

    async def _once(
        self, name: str, arguments: dict[str, Any], seconds: float, listed: Tool | None
    ) -> CallToolResult:
        """Make a call over the connection that is there, or wait for the one that comes.

        Raises:
            _TurnedAway: The server turned the call away as one that names a
                session it knows nothing of. The keeper was told.
        """
        client = await self._connected()
        if listed is not None and listed not in self._tools:
            # Looked at once the client is there: the tools of a connection are in place
            # before anybody who waited for it is woken.
            raise CallError(
                f"the MCP server {self.server} no longer lists the tool {name} as it did when "
                "this call was made, so the call was not made. Look at the tool as it is now, "
                "and call it again if that is still wanted"
            )
        # The whole call has so long, and not only its answer. The SDK would count from when
        # the call is written, and a server that reads no more keeps it from being written.
        limit = asyncio.timeout(seconds)
        # What the way to the server finds out of this call is said here, see `turned_away`.
        call = _Call()
        calling = _calling.set(call)
        self._under_way += 1
        self._quiet.clear()
        try:
            async with limit:
                return await client.call_tool(name, arguments)
        except MCPError as error:
            if call.turned_away:
                # Looked at before anything else. Once one call was turned away, the keeper
                # leaves the client, and a call whose answer came by then may be handed the
                # word that the connection is closed in its place. A server may also say so
                # with the code of a connection that is closed.
                self._gone(client, _ENDED)
                raise _TurnedAway(self._why(error)) from None
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
        finally:
            _calling.reset(calling)
            self._under_way -= 1
            if not self._under_way:
                self._quiet.set()

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

    def _gone(self, client: Client, why: str = _LOST) -> None:
        """Tell the keeper that a client is of no use any more, and why.

        It found its connection closed, or its server knew nothing of the
        session. The client is handed to nobody from here on. The keeper
        comes to leave it a moment later, and whoever calls in that moment
        would get it and fail with it, of a loss that is known already. Such
        a call waits for the next connection instead.
        """
        if client is self._client:
            self._client = None
            self._lost_for = why
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
            if lasted is not None and (lasted >= STABLE_SECONDS or self._failure == _ENDED):
                # A server that ended a session is alive, and the next connection is as good
                # as the first. Reconnecting for it happens only when a call is turned away.
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
            if lasted is not None:
                # An attempt that failed was logged where it failed. A connection that was
                # there and is lost is logged here, once, however many found it closed.
                why = "" if self._failure in (None, _LOST) else f": {self._failure}"
                logger.warning(
                    "the connection to the MCP server %s was lost%s. It is being made again",
                    self.server,
                    why,
                )
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
                    # What listens to the server is left before its client, and may be
                    # left and entered anew while the client stays.
                    heard = await stack.enter_async_context(AsyncExitStack())
                    # First the server listens, and then it lists. It says nothing again
                    # of what changed before it listened, so nothing may change in between.
                    said = await self._listening(heard, client)
                    listed = await _listed(client)
                since = self._clock()
                try:
                    await self._held(client, heard, listed, said)
                finally:
                    # It lasted until it was lost. Leaving it may take seconds more, and in
                    # those the server was of no use to anyone.
                    lasted = self._clock() - since
                self._failure = self._lost_for
                if self._lost_for == _ENDED:
                    await self._heard_out()
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
                if lasted is None:
                    # A connection that was there and broke is told by the keeper, like
                    # every loss, and with this reason.
                    logger.warning(
                        "the MCP server %s is not connected: %s", self.server, self._failure
                    )
        self._settled.set()
        return lasted

    async def _heard_out(self) -> None:
        """Let the calls that are under way hear their answers, for a moment at most.

        The server ended the session, and a call that was sent since is turned
        away as well. Its answer is on its way, and it says that the call can
        be made again. Leaving the client now would break the call off, and it
        would fail as one of which nobody knows how far it got.
        """
        with suppress(TimeoutError):
            async with asyncio.timeout(HEAR_SECONDS):
                await self._quiet.wait()

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
        self,
        client: Client,
        heard: AsyncExitStack,
        listed: Sequence[Tool],
        said: AsyncIterator[object] | None,
    ) -> None:
        """Offer what a connection gives until the connection is lost."""
        self._lost.clear()
        self._client = client
        self._ever = True
        self._instructions = client.instructions
        await self._offer(listed)
        self._move()
        self._settled.set()
        lost = asyncio.ensure_future(self._lost.wait())
        try:
            await self._listened(client, heard, said, lost)
            await lost
        finally:
            lost.cancel()
            await asyncio.wait([lost])

    async def _listened(
        self,
        client: Client,
        heard: AsyncExitStack,
        said: AsyncIterator[object] | None,
        lost: asyncio.Future[Any],
    ) -> None:
        """Have the tools listed anew whenever the server says so, and listen again when it stops.

        A server ends its word of its tools in good order, or the word breaks
        off while the connection stays. Either way nothing more is heard of a
        change, and the SDK does not listen again by itself. So the server
        is listened to anew, after a wait like the one before a connection
        is made again, and asked for its tools after that. A server whose
        word ends soon after it was given, time after time, is left alone
        for the rest of the connection.

        This comes back when the connection is lost, and when the server is
        no longer listened to.
        """
        soon = 0
        while said is not None:
            since = self._clock()
            if await self._until(self._watch(client, said), lost):
                return
            soon = 1 if self._clock() - since >= STABLE_SECONDS else soon + 1
            if soon > ATTEMPTS:
                logger.warning(
                    "the MCP server %s ended its word of its tools %d times in a row, each soon "
                    "after it was listened to. It is not listened to again on this connection, "
                    "so a change of its tools is not heard of before the connection is made "
                    "again",
                    self.server,
                    soon,
                )
                return
            wait = min(FIRST_WAIT * 2 ** (soon - 1), LONGEST_WAIT)
            if await self._until(self._sleep(wait), lost):
                return
            # Left by the task that entered it, which is this one.
            await heard.aclose()
            try:
                async with asyncio.timeout(self._startup):
                    said = await self._listening(heard, client)
            except TimeoutError:
                logger.debug(
                    "the MCP server %s did not say within %g seconds that it listens again",
                    self.server,
                    self._startup,
                )
                said = None
            # Asked also where the server refuses now: that shows whether the connection
            # broke off with its word, and what changed while nobody listened.
            await self._relist(client)

    @staticmethod
    async def _until(doing: Awaitable[None], lost: asyncio.Future[Any]) -> bool:
        """Do something for as long as the connection lasts, and tell whether it was lost.

        What is being done when the connection is lost is broken off.
        """
        task = asyncio.ensure_future(doing)
        try:
            await asyncio.wait({task, lost}, return_when=asyncio.FIRST_COMPLETED)
        finally:
            task.cancel()
            await asyncio.wait([task])
        return lost.done()

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

    async def _watch(self, client: Client, said: AsyncIterator[object]) -> None:
        """List the tools anew whenever the server says that they changed."""
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
                # No list, because there is no connection. That is told by the keeper, once
                # for everybody who found it so.
                self._gone(client)
                return
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
            # What is kept out is taken out before the failure is put on one line: a text
            # with a line break or with several blanks in it would not be found afterwards.
            said = " ".join(self._blotted(str(error)).split()) or type(error).__name__
        # And once more on the one line, where a failure wrote the blanks of a text otherwise.
        said = self._blotted(said)
        return said if len(said) <= _SAID else f"{said[:_SAID].rstrip()} [...]"

    def _blotted(self, text: str) -> str:
        """Return a text without what is to be kept out of it."""
        # The longest first. A text that is part of a longer one would leave the rest of it.
        for secret in sorted(self._kept_out, key=len, reverse=True):
            text = text.replace(secret, "[...]")
        return text
