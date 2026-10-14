"""A connection to a server that is kept: made, lost, made again, and given up."""

import asyncio
import itertools
import logging
import re
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable, Iterator
from contextlib import asynccontextmanager, suppress
from functools import partial
from typing import Any, cast

import pytest
from mcp import Client, MCPError
from mcp.client.subscriptions import SubscriptionLost
from mcp.shared import jsonrpc_dispatcher
from mcp.types import CONNECTION_CLOSED, CallToolResult, ListToolsResult, Tool

from columbina.mcp.server import connection as connection_module
from columbina.mcp.server.connection import (
    ATTEMPTS,
    FIRST_WAIT,
    LONGEST_WAIT,
    STABLE_SECONDS,
    CallError,
    Connection,
)
from columbina.mcp.server.content import shown
from columbina.mcp.server.reach import kept_out, reached
from support.mcp_servers import INSTRUCTIONS, Cuttable, Hanging, Online, Served
from support.terminal import until

BASE = ["add", "nap", "grow", "grow_as_before", "stir"]
LOGGER = "columbina.mcp.server.connection"
LOST = (
    "the connection to the MCP server served was lost while the call was under way. It is not "
    "known how far the call got, and it was not made again"
)
CLOSED = "the connection to the MCP server served is closed"
AGAIN = "the connection to the MCP server served was lost. It is being made again"
TWO = {"a": 1, "b": 1}


def names(connection: Connection) -> list[str]:
    return [tool.name for tool in connection.tools]


def telling(act: Callable[[], object]) -> Callable[[], Awaitable[None]]:
    """What a connection calls when its tools are others, made of something that a test does."""

    async def told() -> None:
        act()

    return told


class Naps:
    """The waits of a keeper: how long each was to be, and when it ends if a test holds it.

    Args:
        held: Whether a wait lasts until the test ends it.
    """

    def __init__(self, held: bool = False) -> None:
        self.asked: list[float] = []
        self.held = held
        self.begun = asyncio.Event()
        self._end = asyncio.Event()

    async def __call__(self, seconds: float) -> None:
        self.asked.append(seconds)
        if self.held:
            self.begun.set()
            await self._end.wait()
            self._end.clear()
            self.begun.clear()

    def end(self) -> None:
        self._end.set()


class Clock:
    """A time that passes when a test says so."""

    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


@pytest.fixture
def served() -> Served:
    return Served()


@pytest.fixture
async def make(monkeypatch: pytest.MonkeyPatch) -> AsyncIterator[Callable[..., Connection]]:
    """Makes connections, and closes them when the test is over."""
    made: list[Connection] = []

    def one(*arguments: Any, **options: Any) -> Connection:
        connection = Connection(*arguments, **options)
        made.append(connection)
        return connection

    yield one
    # A connection lets go in no time here. Should closing ever be broken, one test says so,
    # and the others are not each held up for as long as a real server is given.
    monkeypatch.setattr(connection_module, "LEAVE_SECONDS", 1.0)
    for connection in made:
        await connection.close()


async def begun(connection: Connection) -> Connection:
    """Start a connection and wait until its first attempt is over."""
    connection.start()
    await asyncio.wait_for(connection.settled(), 5)
    return connection


async def lose(connection: Connection, transport: Cuttable, *, older: bool = False) -> None:
    """Cut a connection, and wait until its keeper has left it.

    Args:
        connection: The connection.
        transport: The way to its server.
        older: Whether the server speaks the older protocol. Nothing tells
            the keeper of such a one that it died, so a call is made that
            finds the connection closed and says so.
    """
    entered = transport.entered
    await transport.cut()
    if older:
        with pytest.raises(CallError):
            await connection.call("add", TWO, 5)
    await until(lambda: transport.left == entered)


# A connection that is made


async def test_a_connection_that_is_made_has_the_tools_and_what_the_server_says_of_itself(
    make: Callable[..., Connection], served: Served
) -> None:
    told: list[list[str]] = []
    connection = make(
        "served", lambda: served.server, on_tools=telling(lambda: told.append(names(connection)))
    )

    assert connection.state == "connecting"
    assert (connection.tools, connection.instructions, connection.failure) == ((), None, None)
    await begun(connection)

    assert connection.state == "connected"
    assert names(connection) == BASE
    assert connection.tools[0].input_schema["required"] == ["a", "b"]
    assert connection.instructions == INSTRUCTIONS
    assert connection.failure is None
    assert told == [BASE]


async def test_a_server_that_says_nothing_of_itself_has_no_instructions(
    make: Callable[..., Connection],
) -> None:
    connection = await begun(make("served", lambda: Served(instructions=None).server))

    assert connection.instructions is None


async def test_a_connection_is_made_once_however_often_it_is_started(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    connection = make("served", lambda: transport)

    connection.start()
    connection.start()
    await asyncio.wait_for(connection.settled(), 5)
    connection.start()
    await asyncio.sleep(0.05)

    assert transport.entered == 1


async def test_a_list_that_comes_in_pages_is_read_to_its_end(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    whole = Client.list_tools
    asked: list[str | None] = []

    async def in_pages(self: Client, *, cursor: str | None = None, **more: Any) -> ListToolsResult:
        asked.append(cursor)
        listed = await whole(self)
        if cursor is None:
            return listed.model_copy(update={"tools": listed.tools[:2], "next_cursor": "more"})
        return listed.model_copy(update={"tools": listed.tools[2:], "next_cursor": None})

    monkeypatch.setattr(Client, "list_tools", in_pages)

    connection = await begun(make("served", lambda: served.server))

    assert names(connection) == BASE
    assert asked == [None, "more"]


# Calls


async def test_a_call_gives_what_the_server_answered(
    make: Callable[..., Connection], served: Served
) -> None:
    connection = await begun(make("served", lambda: served.server))

    result = await connection.call("add", {"a": 2, "b": 3}, 5)

    assert (shown(result), result.is_error) == ("5", False)


async def test_a_tool_that_fails_is_a_result_and_no_failure_of_the_call(
    make: Callable[..., Connection], served: Served
) -> None:
    connection = await begun(make("served", lambda: served.server))

    wrong = await connection.call("add", {"a": "two"}, 5)
    unknown = await connection.call("no such tool", {}, 5)

    assert (wrong.is_error, unknown.is_error) == (True, True)
    assert "no such tool" in shown(unknown)


async def test_a_call_that_is_not_answered_in_time_fails_and_is_broken_off_at_the_server(
    make: Callable[..., Connection], served: Served
) -> None:
    connection = await begun(make("served", lambda: served.server))

    with pytest.raises(CallError) as refused:
        await connection.call("nap", {"seconds": 3}, 0.2)

    assert str(refused.value) == (
        "the MCP server served did not answer within 0.2 seconds. The call may have taken effect "
        "all the same"
    )
    assert refused.value.__cause__ is None
    await until(lambda: served.cancelled == 1)
    # The connection is as good as before.
    assert shown(await connection.call("add", TWO, 5)) == "2"
    assert connection.state == "connected"


class Unread:
    """The writing end of a way to a server. It takes things for as long as its server reads."""

    def __init__(self, write: Any, way: "Wedged") -> None:
        self._write = write
        self._way = way

    async def send(self, item: Any) -> None:
        if self._way.stuck:
            # As with a program that reads no more once its pipe is full.
            await asyncio.Future[None]()
        await self._write.send(item)

    async def aclose(self) -> None:
        await self._write.aclose()

    async def __aenter__(self) -> "Unread":
        await self._write.__aenter__()
        return self

    async def __aexit__(self, *raised: Any) -> bool | None:
        return await self._write.__aexit__(*raised)


class Wedged(Cuttable):
    """A way to a server that reads no more from when a test says so.

    Attributes:
        stuck: Whether the server has stopped reading.
    """

    def __init__(self, served: Served) -> None:
        super().__init__(served)
        self.stuck = False

    async def __aenter__(self) -> Any:
        read, write = await super().__aenter__()
        return read, Unread(write, self)


async def test_a_call_that_cannot_be_written_to_its_server_is_given_up_when_its_time_is_over(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The SDK gives the word that tells a server to stop five seconds to be written. That
    # word is not written here either, and the test does not wait so long for it.
    monkeypatch.setattr(jsonrpc_dispatcher, "_ABANDON_WRITE_TIMEOUT", 0.1)
    transport = Wedged(served)
    connection = await begun(make("served", lambda: transport))
    assert shown(await connection.call("add", TWO, 5)) == "2"
    transport.stuck = True

    with pytest.raises(CallError) as refused:
        # Without an end of its own, the call would wait for as long as the server lives.
        await asyncio.wait_for(connection.call("add", TWO, 0.2), 5)

    assert str(refused.value) == (
        "the MCP server served did not answer within 0.2 seconds. The call may have taken effect "
        "all the same"
    )
    assert refused.value.__cause__ is None
    # The connection is kept. Whether the server reads again is for the server to show.
    assert connection.state == "connected"
    transport.stuck = False
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_a_call_whose_caller_is_cancelled_is_broken_off_at_the_server(
    make: Callable[..., Connection], served: Served
) -> None:
    connection = await begun(make("served", lambda: served.server))
    calling = asyncio.ensure_future(connection.call("nap", {"seconds": 30}, 60))
    await until(lambda: served.napping == 1)

    calling.cancel()

    with pytest.raises(asyncio.CancelledError):
        await calling
    await until(lambda: served.cancelled == 1)
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_a_call_that_is_broken_off_inside_the_connection_is_a_failure_and_no_cancellation(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    connection = await begun(make("served", lambda: served.server))

    async def broken_off(self: Client, *arguments: Any, **more: Any) -> Any:
        raise asyncio.CancelledError

    monkeypatch.setattr(Client, "call_tool", broken_off)

    with pytest.raises(CallError) as refused:
        await connection.call("add", TWO, 5)

    assert str(refused.value) == (
        "the call was broken off inside the connection to the MCP server served, and not by "
        "whoever made it. It may have taken effect all the same"
    )


@pytest.mark.parametrize(
    ("error", "said"),
    [
        (
            MCPError(-32602, "the arguments\n  are of no use"),
            "the MCP server served answered the call with an error: the arguments are of no use",
        ),
        (
            ValueError("no such shape"),
            "what the MCP server served answered cannot be used: no such shape",
        ),
    ],
    ids=["an error of the protocol", "an answer of no known shape"],
)
async def test_a_call_that_the_server_does_not_answer_with_a_result_fails_in_words_of_its_own(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    error: Exception,
    said: str,
) -> None:
    connection = await begun(make("served", lambda: served.server))

    async def failing(self: Client, *arguments: Any, **more: Any) -> Any:
        raise error

    monkeypatch.setattr(Client, "call_tool", failing)

    with pytest.raises(CallError) as refused:
        await connection.call("add", TWO, 5)

    assert str(refused.value) == said
    # Neither is a connection that was lost.
    assert connection.state == "connected"


# A connection that is lost


@pytest.mark.parametrize("mode", ["auto", "legacy"])
async def test_a_call_that_waits_when_the_connection_is_lost_fails_and_the_connection_is_made_again(
    make: Callable[..., Connection], served: Served, mode: str, caplog: pytest.LogCaptureFixture
) -> None:
    transport = Cuttable(served)
    naps = Naps()
    told: list[list[str]] = []
    connection = await begun(
        make(
            "served",
            lambda: transport,
            mode=mode,
            sleep=naps,
            on_tools=telling(lambda: told.append([])),
        )
    )
    calling = asyncio.ensure_future(connection.call("nap", {"seconds": 30}, 60))
    await until(lambda: served.napping == 1)

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await transport.cut()

        with pytest.raises(CallError) as refused:
            await asyncio.wait_for(calling, 5)
        assert str(refused.value) == LOST
        await until(lambda: transport.entered == 2 and connection.state == "connected")
    # The user is told once what happened, however many found the connection closed: the
    # call, and whoever listened for a change of the tools.
    assert caplog.messages == [AGAIN]
    assert naps.asked == [FIRST_WAIT]
    assert transport.left == 1
    assert connection.failure is None
    assert names(connection) == BASE
    # The tools are the ones that were there, so nobody is told of them a second time.
    assert len(told) == 1
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_two_calls_that_find_one_connection_closed_have_it_made_again_once(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    naps = Naps()
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=naps))
    calling = [asyncio.ensure_future(connection.call("nap", {"seconds": 30}, 60)) for _ in range(2)]
    await until(lambda: served.napping == 2)

    await transport.cut()

    for call in calling:
        with pytest.raises(CallError, match="was lost while the call was under way"):
            await asyncio.wait_for(call, 5)
    await until(lambda: transport.entered == 2 and connection.state == "connected")
    await asyncio.sleep(0.05)
    assert (naps.asked, transport.entered) == ([FIRST_WAIT], 2)


async def test_a_server_of_2026_that_dies_while_nothing_is_called_is_connected_again(
    make: Callable[..., Connection], served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, sleep=Naps()))

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await transport.cut()

        # Its word of its tools broke off, and asking for them showed that it is gone.
        await until(lambda: transport.entered == 2 and connection.state == "connected")
    # A list that could not be had because the server is gone is the loss of the connection,
    # and it is told as that.
    assert caplog.messages == [AGAIN]


async def test_an_older_server_that_dies_while_nothing_is_called_is_missed_at_the_next_call(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=Naps()))

    await transport.cut()
    await asyncio.sleep(0.05)

    assert (connection.state, transport.entered) == ("connected", 1)
    with pytest.raises(CallError) as refused:
        await connection.call("add", TWO, 5)
    assert str(refused.value) == LOST
    await until(lambda: transport.entered == 2 and connection.state == "connected")


async def test_a_call_that_comes_once_a_connection_is_known_to_be_lost_waits_for_the_next_one(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=Naps()))
    await transport.cut()
    with pytest.raises(CallError) as refused:
        await connection.call("add", TWO, 5)
    assert str(refused.value) == LOST

    # The keeper has been told, and has not come to leave the old client yet. Whoever calls
    # now is not handed that client to fail with it a second time. The call waits for the
    # next connection, like one that comes while the connection is being made again.
    then = connection.state
    again = connection.call("add", {"a": 2, "b": 3}, 5)
    assert shown(await asyncio.wait_for(again, 5)) == "5"
    assert (then, transport.entered, connection.state) == ("reconnecting", 2, "connected")


async def test_while_a_connection_is_made_again_the_tools_stay_and_a_call_waits_for_it(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))

    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)

    assert connection.state == "reconnecting"
    assert connection.failure == "the connection was lost"
    assert names(connection) == BASE
    calling = asyncio.ensure_future(connection.call("add", {"a": 2, "b": 3}, 5))
    await asyncio.sleep(0.05)
    assert not calling.done()
    naps.end()
    assert shown(await asyncio.wait_for(calling, 5)) == "5"
    assert connection.state == "connected"


def listed_as(connection: Connection, name: str) -> Tool:
    """A tool of a server as the connection has it listed now."""
    (tool,) = (tool for tool in connection.tools if tool.name == name)
    return tool


async def test_a_call_that_waited_is_not_made_where_its_tool_is_listed_otherwise_by_then(
    make: Callable[..., Connection], served: Served
) -> None:
    served.with_a_reader()
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))
    reader = listed_as(connection, "peek")
    assert reader.annotations is not None
    assert reader.annotations.read_only_hint is True
    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)
    # Whoever let this call through went by what the server said of the tool: it only reads.
    calling = asyncio.ensure_future(connection.call("peek", {}, 5, listed=reader))
    await asyncio.sleep(0.05)
    assert not calling.done()

    # The server comes back with a tool of that name of which it says no such thing.
    served.with_a_reader_that_writes_now()
    naps.end()

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(calling, 5)
    assert str(refused.value) == (
        "the MCP server served no longer lists the tool peek as it did when this call was "
        "made, so the call was not made. Look at the tool as it is now, and call it again "
        "if that is still wanted"
    )
    assert served.peeked == 0
    # Asked for as it is listed now, it is called.
    assert listed_as(connection, "peek") != reader
    now = connection.call("peek", {}, 5, listed=listed_as(connection, "peek"))
    assert shown(await asyncio.wait_for(now, 5)) == "wrote"
    assert served.peeked == 1


async def test_a_call_that_waited_is_made_where_its_tool_is_listed_as_before(
    make: Callable[..., Connection], served: Served
) -> None:
    served.with_a_reader()
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))
    reader = listed_as(connection, "peek")
    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)
    calling = asyncio.ensure_future(connection.call("peek", {}, 5, listed=reader))
    await asyncio.sleep(0.05)

    # Another tool is there when the server comes back. This one is as it was.
    served.one_more()
    naps.end()

    assert shown(await asyncio.wait_for(calling, 5)) == "peeked"
    assert "more1" in names(connection)
    # The server listed it anew, and what it listed is the same tool.
    assert listed_as(connection, "peek") is not reader
    assert listed_as(connection, "peek") == reader


async def test_a_call_is_not_made_for_a_tool_that_its_server_no_longer_lists(
    make: Callable[..., Connection], served: Served
) -> None:
    served.with_a_reader()
    connection = await begun(make("served", lambda: Cuttable(served)))
    reader = listed_as(connection, "peek")

    # No connection was lost: the server said that its tools changed, and they were listed anew.
    served.server.remove_tool("peek")
    await connection.call("grow", {}, 5)
    await until(lambda: "peek" not in names(connection))

    with pytest.raises(CallError, match="no longer lists the tool peek as it did"):
        await connection.call("peek", {}, 5, listed=reader)
    assert served.peeked == 0


async def test_a_call_does_not_wait_for_a_connection_longer_than_one_may_take_to_be_made(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))
    # Set once the connection is there: a machine that is busy may take longer to make one.
    monkeypatch.setattr(connection, "_startup", 0.2)
    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(connection.call("add", TWO, 5), 5)

    assert str(refused.value) == (
        "the MCP server served is being connected again, and was not within 0.2 seconds"
    )


async def test_tools_that_are_others_after_a_connection_was_made_again_are_told(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    told: list[list[str]] = []
    connection = make(
        "served",
        lambda: transport,
        sleep=Naps(),
        on_tools=telling(lambda: told.append(names(connection))),
    )
    await begun(connection)

    served.one_more()
    await lose(connection, transport)
    await until(lambda: transport.entered == 2 and connection.state == "connected")

    assert told == [BASE, [*BASE, "more1"]]


async def test_leaving_a_connection_that_broke_may_raise_whatever_it_likes(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    transport.leaving = ExceptionGroup("unhandled errors in a TaskGroup", [OSError("the pipe")])
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))

    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)

    # What leaving raised is nothing that anybody is told of.
    assert (connection.state, connection.failure) == ("reconnecting", "the connection was lost")
    naps.end()
    await until(lambda: transport.entered == 2 and connection.state == "connected")
    assert connection.failure is None


async def test_what_leaving_a_connection_sets_in_motion_does_not_reach_the_next_attempt(
    make: Callable[..., Connection], served: Served
) -> None:
    class Lingering(Cuttable):
        async def __aexit__(self, *why: Any) -> None:
            await super().__aexit__(*why)
            if self.left == 1:
                # What it left behind breaks off, a moment later, the task that left it.
                task = asyncio.current_task()
                assert task is not None
                asyncio.get_running_loop().call_soon(task.cancel)

    transport = Lingering(served)
    naps = Naps()
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=naps))

    await lose(connection, transport, older=True)

    await until(lambda: transport.entered == 2 and connection.state == "connected")
    # The first attempt after the loss is the one that made the connection.
    assert naps.asked == [FIRST_WAIT]
    assert connection.failure is None


async def test_a_call_that_hears_late_that_its_connection_closed_does_not_end_the_next_one(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=Naps()))
    through = Client.call_tool
    heard = asyncio.Event()

    async def late(self: Client, name: str, *arguments: Any, **more: Any) -> Any:
        if name == "nap":
            # It is told only now of what happened to its connection a while ago.
            await heard.wait()
            raise MCPError(CONNECTION_CLOSED, "Connection closed")
        return await through(self, name, *arguments, **more)

    monkeypatch.setattr(Client, "call_tool", late)
    calling = asyncio.ensure_future(connection.call("nap", {"seconds": 0}, 60))
    await asyncio.sleep(0.05)
    await lose(connection, transport, older=True)
    await until(lambda: transport.entered == 2 and connection.state == "connected")

    heard.set()

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(calling, 5)
    assert str(refused.value) == LOST
    await asyncio.sleep(0.05)
    assert (transport.entered, connection.state) == (2, "connected")


async def test_a_connection_that_does_not_let_go_is_not_waited_for_for_good(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    class Clinging(Cuttable):
        async def __aexit__(self, *why: Any) -> None:
            try:
                # The first connection does not let go until it is made to.
                await asyncio.sleep(3600 if self.entered == 1 else 0)
            finally:
                await super().__aexit__(*why)

    monkeypatch.setattr(connection_module, "LEAVE_SECONDS", 0.1)
    transport = Clinging(served)
    connection = await begun(make("served", lambda: transport, mode="legacy", sleep=Naps()))

    await lose(connection, transport, older=True)

    await until(lambda: transport.entered == 2 and connection.state == "connected")
    assert shown(await connection.call("add", TWO, 5)) == "2"


# The waits, and giving up


def failing_after(first: Any, error: BaseException) -> Callable[[], Any]:
    """What makes a way to a server once, and fails every time after that."""
    made = [first]

    def target() -> Any:
        if made:
            return made.pop()
        raise error

    return target


async def test_the_waits_double_up_to_the_longest_and_after_ten_failures_the_tools_are_withdrawn(
    make: Callable[..., Connection], served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    transport = Cuttable(served)
    naps = Naps()
    told: list[list[str]] = []
    connection = make(
        "served",
        failing_after(transport, ConnectionRefusedError("nobody answers")),
        sleep=naps,
        on_tools=telling(lambda: told.append(names(connection))),
    )
    await begun(connection)

    with caplog.at_level(logging.ERROR, logger=LOGGER):
        await lose(connection, transport)
        await until(lambda: connection.state == "gave up")

    assert naps.asked == [0.5, 1, 2, 4, 8, 16, 30, 30, 30, 30]
    assert len(naps.asked) == ATTEMPTS
    assert max(naps.asked) == LONGEST_WAIT
    assert connection.tools == ()
    assert told == [BASE, []]
    assert connection.failure == "nobody answers"
    assert caplog.messages == [
        "the MCP server served is no longer tried after 11 failures in a row: nobody answers"
    ]
    with pytest.raises(CallError) as refused:
        await connection.call("add", TWO, 5)
    assert str(refused.value) == (
        "the MCP server served is not connected, and it is no longer tried: nobody answers"
    )


async def test_whoever_waits_for_a_server_that_is_given_up_meanwhile_is_told_so_at_once(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(
        make(
            "served",
            failing_after(transport, ConnectionRefusedError("nobody answers")),
            sleep=naps,
        )
    )
    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)
    calling = asyncio.ensure_future(connection.call("add", TWO, 5))
    await asyncio.sleep(0.05)
    assert not calling.done()

    naps.held = False
    naps.end()

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(calling, 5)
    assert str(refused.value) == (
        "the MCP server served is not connected, and it is no longer tried: nobody answers"
    )


async def test_a_server_that_was_never_connected_is_tried_as_often_and_then_given_up(
    make: Callable[..., Connection],
) -> None:
    naps = Naps(held=True)
    told: list[None] = []
    tried: list[None] = []

    def target() -> Any:
        tried.append(None)
        raise FileNotFoundError("no such program: mcp-nowhere")

    connection = make("served", target, sleep=naps, on_tools=telling(lambda: told.append(None)))

    await begun(connection)

    # The first attempt is over, and whoever waited for it goes on while the next ones are made.
    assert connection.state == "connecting"
    assert connection.failure == "no such program: mcp-nowhere"
    # The keeper goes on by itself from here: it waits, and is held in that wait.
    await asyncio.wait_for(naps.begun.wait(), 5)
    assert (len(tried), naps.asked) == (1, [FIRST_WAIT])
    naps.held = False
    naps.end()
    await until(lambda: connection.state == "gave up")
    assert len(tried) == ATTEMPTS + 1
    assert naps.asked == [0.5, 1, 2, 4, 8, 16, 30, 30, 30, 30]
    # Why it is given up stays to be read.
    assert connection.failure == "no such program: mcp-nowhere"
    # There were no tools, so nobody is told that there are none.
    assert told == []


async def test_a_server_that_is_reached_after_some_failures_has_its_tools_from_then_on(
    make: Callable[..., Connection], served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    transport = Cuttable(served)
    left = [3]

    def target() -> Any:
        if left[0]:
            left[0] -= 1
            raise ConnectionRefusedError("not yet")
        return transport

    naps = Naps()
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        connection = await begun(make("served", target, sleep=naps))

        await until(lambda: connection.state == "connected")

    assert naps.asked == [0.5, 1, 2]
    assert names(connection) == BASE
    assert connection.failure is None
    # An attempt that failed is told as that. No connection was lost here.
    assert caplog.messages == ["the MCP server served is not connected: not yet"] * 3


async def test_the_count_of_failures_begins_anew_once_a_connection_lasted_half_a_minute(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    naps = Naps()
    clock = Clock()
    connection = await begun(make("served", lambda: transport, sleep=naps, clock=clock))

    for lasted in (STABLE_SECONDS, STABLE_SECONDS - 0.1, STABLE_SECONDS - 0.1, STABLE_SECONDS):
        entered = transport.entered
        clock.now += lasted
        await lose(connection, transport)
        await until(lambda: transport.entered == entered + 1 and connection.state == "connected")  # noqa: B023

    # Half a minute, and the count begins anew. Less, and it goes on.
    assert naps.asked == [0.5, 1, 2, 0.5]


async def test_a_server_that_is_lost_again_and_again_is_given_up_and_no_longer_said_to_come_back(
    make: Callable[..., Connection], served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, sleep=Naps()))

    def there(for_the: int) -> bool:
        return transport.entered == for_the and connection.state == "connected"

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        for time in range(2, ATTEMPTS + 2):
            await lose(connection, transport)
            await until(partial(there, time))
        await lose(connection, transport)
        await until(lambda: connection.state == "gave up")

    assert transport.entered == ATTEMPTS + 1
    assert connection.tools == ()
    assert caplog.messages == [AGAIN] * ATTEMPTS + [
        "the MCP server served is no longer tried after 11 failures in a row: "
        "the connection was lost"
    ]


class Lingering(Cuttable):
    """A way to a server that takes its time to be left.

    Args:
        served: The server.
        clock: The time of the test.
        seconds: How much of that time passes while it is left.
    """

    def __init__(self, served: Served, clock: Clock, seconds: float) -> None:
        super().__init__(served)
        self._clock = clock
        self._seconds = seconds

    async def __aexit__(self, *raised: Any) -> None:
        self._clock.now += self._seconds
        await super().__aexit__(*raised)


async def test_the_time_that_it_takes_to_leave_a_lost_connection_is_no_time_that_it_lasted(
    make: Callable[..., Connection], served: Served
) -> None:
    naps = Naps()
    clock = Clock()
    transport = Lingering(served, clock, 6.0)
    connection = await begun(make("served", lambda: transport, sleep=naps, clock=clock))

    # The server lives for 25 seconds each time, and takes six more to be left.
    for _ in range(3):
        entered = transport.entered
        clock.now += STABLE_SECONDS - 5
        await lose(connection, transport)
        await until(lambda: transport.entered == entered + 1 and connection.state == "connected")  # noqa: B023

    # It never lasted half a minute. The waits go on growing, as they do for a server that
    # dies as often as it is started.
    assert naps.asked == [0.5, 1, 2]


async def test_an_attempt_that_takes_too_long_is_given_up_and_the_next_one_is_made(
    make: Callable[..., Connection],
) -> None:
    transport = Hanging()
    naps = Naps(held=True)
    connection = make("served", lambda: transport, sleep=naps, startup_seconds=0.1)

    await begun(connection)

    assert connection.state == "connecting"
    assert connection.failure == "no connection within 0.1 seconds"
    assert transport.entered == 1
    naps.end()
    await until(lambda: transport.entered == 2)


@pytest.mark.parametrize(
    ("error", "said"),
    [
        (
            ConnectionRefusedError("cannot reach https://host/mcp?key=s3cret-of-mine"),
            "cannot reach https://host/mcp?key=[...]",
        ),
        (RuntimeError(), "RuntimeError"),
        (OSError("on\n  two lines "), "on two lines"),
        (OSError("x" * 300), "x" * 300),
        (OSError("word " * 80), "word " * 59 + "word [...]"),
        (
            ExceptionGroup("unhandled errors in a TaskGroup", [OSError("no route"), ValueError()]),
            "no route; ValueError",
        ),
        (asyncio.CancelledError(), "it was broken off from inside"),
    ],
    ids=[
        "with something that is for no log",
        "without a word",
        "on more than one line",
        "as long as is passed on",
        "longer than is passed on",
        "several at once",
        "a cancellation that nobody asked for",
    ],
)
async def test_why_an_attempt_failed_is_said_in_one_line_and_without_what_is_kept_out(
    make: Callable[..., Connection], error: BaseException, said: str
) -> None:
    def target() -> Any:
        raise error

    connection = make(
        "served", target, sleep=Naps(held=True), kept_out=["s3cret", "s3cret-of-mine"]
    )

    await begun(connection)

    assert connection.failure == said
    assert connection.state == "connecting"


async def test_what_is_kept_out_is_not_told_however_a_failure_spells_it(
    make: Callable[..., Connection],
) -> None:
    def target() -> Any:
        # A line break in a header: the library says the value as bytes are written. Here it
        # stands once more as it is, and once with its blanks written otherwise.
        raise ValueError(
            "Illegal header value b'Bearer  abcdefgh\\n' and again Bearer  abcdefgh\n. "
            "Or Bearer\tabcdefgh"
        )

    kept = kept_out(None, {"Authorization": "Bearer  abcdefgh\n"})
    connection = make("served", target, sleep=Naps(held=True), kept_out=kept)

    await begun(connection)

    assert connection.failure == "Illegal header value b'[...]' and again [...]. Or [...]"


async def test_a_connection_that_breaks_with_an_error_is_told_once_and_with_the_reason(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    transport = Cuttable(served)
    whole = Connection._held  # pyright: ignore[reportPrivateUsage]
    burned: list[None] = []

    async def burning(self: Connection, *held: Any) -> None:
        # The first connection is there for a moment, and then something of the transport
        # raises into the task that holds it, as a failed request over HTTP does.
        if burned:
            await whole(self, *held)
            return
        burned.append(None)
        holding = asyncio.ensure_future(whole(self, *held))
        await until(lambda: self.state == "connected")
        holding.cancel()
        await asyncio.wait([holding])
        raise OSError("the wire burned")

    monkeypatch.setattr(Connection, "_held", burning)
    connection = make("served", lambda: transport, sleep=Naps())

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        connection.start()
        await until(lambda: transport.entered == 2 and connection.state == "connected")

    assert caplog.messages == [
        "the connection to the MCP server served was lost: the wire burned. It is being made again"
    ]


# Tools that change


async def test_tools_that_a_server_of_2026_says_have_changed_are_listed_anew(
    make: Callable[..., Connection], served: Served
) -> None:
    told: list[list[str]] = []
    connection = make(
        "served", lambda: served.server, on_tools=telling(lambda: told.append(names(connection)))
    )
    await begun(connection)

    assert shown(await connection.call("grow", {}, 5)) == "grown"

    await until(lambda: names(connection) == [*BASE, "more1"])
    assert told == [BASE, [*BASE, "more1"]]
    assert shown(await connection.call("more1", {}, 5)) == "more1"


@pytest.mark.parametrize(
    ("mode", "grow"), [("auto", "grow"), ("legacy", "grow_as_before")], ids=["of 2026", "older"]
)
async def test_tools_that_change_while_they_are_listed_for_the_first_time_are_listed_anew(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    mode: str,
    grow: str,
) -> None:
    whole = Client.list_tools
    first = [True]

    async def and_then_one_more(self: Client, **what: Any) -> ListToolsResult:
        listed = await whole(self)
        if first[0]:
            first[0] = False
            # The server has one tool more before anybody has read this list, and says so.
            await self.call_tool(grow, {})
        return listed

    monkeypatch.setattr(Client, "list_tools", and_then_one_more)
    told: list[list[str]] = []
    connection = make(
        "served",
        lambda: Cuttable(served),
        mode=mode,
        on_tools=telling(lambda: told.append(names(connection))),
    )

    await begun(connection)

    # The server said that its tools changed before anybody held the list. That is heard
    # all the same: the server was listened to before it was asked for its tools.
    await until(lambda: names(connection) == [*BASE, "more1"])
    assert told == [BASE, [*BASE, "more1"]]


async def test_tools_that_an_older_server_says_have_changed_are_listed_anew(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    whole = Client.list_tools
    asked: list[None] = []

    async def counted(self: Client, **what: Any) -> ListToolsResult:
        asked.append(None)
        return await whole(self)

    monkeypatch.setattr(Client, "list_tools", counted)
    transport = Cuttable(served)
    told: list[list[str]] = []
    connection = make(
        "served",
        lambda: transport,
        mode="legacy",
        on_tools=telling(lambda: told.append(names(connection))),
    )
    await begun(connection)

    await connection.call("grow_as_before", {}, 5)
    await until(lambda: names(connection) == [*BASE, "more1"])
    await connection.call("grow_as_before", {}, 5)
    await until(lambda: names(connection) == [*BASE, "more1", "more2"])

    assert told == [BASE, [*BASE, "more1"], [*BASE, "more1", "more2"]]
    # The list was asked for when the connection was made, and once for each time that the
    # server said that it changed.
    await asyncio.sleep(0.05)
    assert len(asked) == 3


async def test_what_an_older_server_says_of_something_else_does_not_have_the_tools_listed(
    make: Callable[..., Connection], served: Served
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, mode="legacy"))
    served.one_more()

    assert shown(await connection.call("stir", {}, 5)) == "stirred"
    await asyncio.sleep(0.05)

    # The tool that was added without a word is not known of yet.
    assert names(connection) == BASE
    await connection.call("grow_as_before", {}, 5)
    await until(lambda: names(connection) == [*BASE, "more1", "more2"])


async def test_a_list_that_cannot_be_had_anew_leaves_the_tools_as_they_were(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    transport = Cuttable(served)
    connection = await begun(make("served", lambda: transport, sleep=Naps()))
    whole = Client.list_tools
    broken = [True]

    async def refusing(self: Client, **what: Any) -> ListToolsResult:
        if broken[0]:
            raise MCPError(-32603, "the list is broken")
        return await whole(self)

    monkeypatch.setattr(Client, "list_tools", refusing)

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await connection.call("grow", {}, 5)
        await until(lambda: bool(caplog.messages))

    assert caplog.messages == [
        "the MCP server served did not list its tools anew, and the list that was there stays: "
        "the list is broken"
    ]
    assert names(connection) == BASE
    broken[0] = False
    await connection.call("grow", {}, 5)
    await until(lambda: names(connection) == [*BASE, "more1", "more2"])
    # A list that could not be had is no connection that was lost: this one was made once.
    await asyncio.sleep(0.05)
    assert (connection.state, transport.entered) == ("connected", 1)


async def test_a_list_that_does_not_come_in_time_leaves_the_tools_as_they_were(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    connection = await begun(make("served", lambda: served.server))
    # Set once the connection is there: a machine that is busy may take longer to make one.
    monkeypatch.setattr(connection, "_startup", 0.2)

    async def never(self: Client, **what: Any) -> ListToolsResult:
        await asyncio.sleep(3600)
        raise AssertionError

    monkeypatch.setattr(Client, "list_tools", never)

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await connection.call("grow", {}, 5)
        await until(lambda: bool(caplog.messages))

    assert caplog.messages == [
        "the MCP server served did not list its tools anew, and the list that was there stays: "
        "TimeoutError"
    ]
    assert names(connection) == BASE


async def test_a_list_that_names_a_tool_twice_is_not_used(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    whole = Client.list_tools
    doubled = [True]

    async def listing(self: Client, **what: Any) -> ListToolsResult:
        listed = await whole(self)
        if doubled[0]:
            return listed.model_copy(update={"tools": [*listed.tools, listed.tools[1]]})
        return listed

    monkeypatch.setattr(Client, "list_tools", listing)
    told: list[list[str]] = []
    connection = make(
        "served", lambda: served.server, on_tools=telling(lambda: told.append(names(connection)))
    )

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await begun(connection)

    # The connection is there, and it has no tools.
    twice = "the server lists the tool 'nap' twice, so its list is not used"
    assert (connection.state, connection.tools, connection.failure) == ("connected", (), twice)
    assert caplog.messages == [f"the MCP server served: {twice}"]
    assert told == []
    doubled[0] = False
    await connection.call("grow", {}, 5)
    await until(lambda: names(connection) == [*BASE, "more1"])
    # A list that can be used ends what was wrong with the one before it.
    assert connection.failure is None
    # And a list like that leaves a good one in place, too.
    doubled[0] = True
    await connection.call("grow", {}, 5)
    await until(lambda: connection.failure == twice)
    assert names(connection) == [*BASE, "more1"]
    assert told == [[*BASE, "more1"]]
    assert caplog.messages == [f"the MCP server served: {twice}"] * 2


async def test_a_list_that_is_as_it_was_ends_what_was_wrong_with_the_one_before_it(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    whole = Client.list_tools
    lists: list[ListToolsResult] = []

    async def listing(self: Client, **what: Any) -> ListToolsResult:
        lists.append(await whole(self))
        if len(lists) == 2:
            return lists[1].model_copy(update={"tools": [*lists[1].tools, lists[1].tools[1]]})
        # The first list again, whatever the server has by now.
        return lists[0]

    monkeypatch.setattr(Client, "list_tools", listing)
    told: list[list[str]] = []
    connection = make(
        "served", lambda: served.server, on_tools=telling(lambda: told.append(names(connection)))
    )
    await begun(connection)

    await connection.call("grow", {}, 5)
    await until(lambda: connection.failure is not None)
    assert connection.failure == "the server lists the tool 'nap' twice, so its list is not used"
    await connection.call("grow", {}, 5)
    await until(lambda: connection.failure is None)

    # Nobody was told of tools: they are the ones that were there.
    assert names(connection) == BASE
    assert told == [BASE]


class Words:
    """A server's word of its tools, as a test hands it out.

    It stands in the place of what the SDK listens with. Every time a server is listened to,
    the test says what it says then, and how its word ends.

    Attributes:
        entered: How often a server was listened to.
        left: How often the listening was left.
        refuses_from: The server refuses to be listened to from this time on, counted from 1.
        silent_from: The server does not answer whoever would listen to it from this time on.
    """

    def __init__(self) -> None:
        self.entered = 0
        self.left = 0
        self.refuses_from: int | None = None
        self.silent_from: int | None = None
        self._said: list[asyncio.Queue[object]] = []

    def over(self, monkeypatch: pytest.MonkeyPatch) -> "Words":
        """Have every client listen to this in the place of its server."""
        words = self

        @asynccontextmanager
        async def listening(client: Client, **what: Any) -> AsyncGenerator[AsyncIterator[str]]:
            words.entered += 1
            if words.refuses_from is not None and words.entered >= words.refuses_from:
                raise MCPError(-32601, "Method not found")
            if words.silent_from is not None and words.entered >= words.silent_from:
                await asyncio.sleep(3600)
            heard: asyncio.Queue[object] = asyncio.Queue()
            words._said.append(heard)

            async def said() -> AsyncIterator[str]:
                while True:
                    word = await heard.get()
                    if isinstance(word, BaseException):
                        raise word
                    if word is None:
                        return
                    yield "the tools changed"

            try:
                yield said()
            finally:
                words.left += 1

        monkeypatch.setattr(Client, "listen", listening)
        return self

    def say(self) -> None:
        """Have the server say that its tools changed."""
        self._said[-1].put_nowait("the tools changed")

    def end(self) -> None:
        """Have the server end its word in good order."""
        self._said[-1].put_nowait(None)

    def lose(self) -> None:
        """Have the word of the server break off."""
        self._said[-1].put_nowait(SubscriptionLost("the stream ended"))


def counted(monkeypatch: pytest.MonkeyPatch) -> list[None]:
    """Count how often a server is asked for its tools."""
    whole = Client.list_tools
    asked: list[None] = []

    async def and_counted(self: Client, **what: Any) -> ListToolsResult:
        asked.append(None)
        return await whole(self)

    monkeypatch.setattr(Client, "list_tools", and_counted)
    return asked


@pytest.mark.parametrize("how", [Words.end, Words.lose], ids=["in good order", "broken off"])
async def test_a_server_whose_word_of_its_tools_ends_is_listened_to_again(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    how: Callable[[Words], None],
) -> None:
    words = Words().over(monkeypatch)
    asked = counted(monkeypatch)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: served.server, sleep=naps))
    assert (words.entered, len(asked)) == (1, 1)

    how(words)

    # It is asked for its tools once more, which shows that the connection is there. Then
    # comes a wait, as before a connection is made again.
    await asyncio.wait_for(naps.begun.wait(), 5)
    assert (len(asked), naps.asked, connection.state) == (2, [FIRST_WAIT], "connected")
    assert (words.entered, words.left) == (1, 0)
    # A tool changes while nobody listens.
    served.one_more()
    naps.end()

    # The server is listened to again, and asked for its tools after that: it says nothing
    # of what changed before it was listened to.
    await until(lambda: names(connection) == [*BASE, "more1"])
    assert (words.entered, words.left, len(asked)) == (2, 1, 3)
    # What it says from now on is heard.
    served.one_more()
    words.say()
    await until(lambda: names(connection) == [*BASE, "more1", "more2"])
    assert connection.failure is None


async def test_a_server_whose_word_ends_as_often_as_it_is_given_is_no_longer_listened_to(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    words = Words().over(monkeypatch)
    asked = counted(monkeypatch)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: served.server, sleep=naps))

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        for ended in range(1, ATTEMPTS + 1):
            words.end()
            await asyncio.wait_for(naps.begun.wait(), 5)
            assert words.entered == ended
            naps.end()
            await until(lambda at=ended: words.entered == at + 1)
        # The waits double up to the longest, as they do between attempts to connect.
        assert naps.asked == [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0, 30.0, 30.0]
        assert caplog.messages == []
        words.end()
        await until(lambda: len(caplog.messages) == 1)
        await asyncio.sleep(0.05)

    # Eleven times in a row its word ended soon after it was given. It is not asked again.
    assert caplog.messages == [
        "the MCP server served ended its word of its tools 11 times in a row, each soon after "
        "it was listened to. It is not listened to again on this connection, so a change of "
        "its tools is not heard of before the connection is made again"
    ]
    assert (words.entered, len(naps.asked)) == (ATTEMPTS + 1, ATTEMPTS)
    # The connection is none the worse for it.
    assert connection.state == "connected"
    assert shown(await connection.call("add", TWO, 5)) == "2"
    # Once when it was made, once after every time the word ended, once after every time
    # that the server was listened to again.
    assert len(asked) == 1 + (ATTEMPTS + 1) + ATTEMPTS


async def test_a_word_of_the_tools_that_lasted_does_not_count_among_those_that_ended_soon(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    words = Words().over(monkeypatch)
    naps = Naps()
    clock = Clock()
    connection = await begun(make("served", lambda: served.server, sleep=naps, clock=clock))

    async def ends(after: float) -> None:
        entered = words.entered
        clock.now += after
        words.end()
        await until(lambda: words.entered == entered + 1)

    await ends(1.0)
    await ends(1.0)
    assert naps.asked == [0.5, 1.0]
    # Half a minute, and the count begins anew.
    await ends(STABLE_SECONDS)
    assert naps.asked == [0.5, 1.0, 0.5]
    await ends(STABLE_SECONDS - 0.1)
    assert naps.asked == [0.5, 1.0, 0.5, 1.0]
    assert connection.state == "connected"


async def test_a_server_that_will_not_be_listened_to_again_is_none_the_worse_for_it(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    words = Words().over(monkeypatch)
    words.refuses_from = 2
    asked = counted(monkeypatch)
    naps = Naps()
    connection = await begun(make("served", lambda: served.server, sleep=naps))

    words.lose()

    await until(lambda: words.entered == 2)
    await until(lambda: len(asked) == 3)
    await asyncio.sleep(0.05)
    # It refused, and is not asked a third time. Its tools were asked for once more all the
    # same: that shows whether the connection broke off with its word.
    assert (words.entered, len(asked), naps.asked) == (2, 3, [FIRST_WAIT])
    assert connection.state == "connected"
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_a_server_that_does_not_answer_when_listened_to_again_is_not_waited_for_for_good(
    make: Callable[..., Connection],
    served: Served,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    words = Words().over(monkeypatch)
    words.silent_from = 2
    asked = counted(monkeypatch)
    connection = await begun(make("served", lambda: served.server, sleep=Naps()))
    # Set once the connection is there: a machine that is busy may take longer to make one.
    monkeypatch.setattr(connection, "_startup", 0.2)

    with caplog.at_level(logging.DEBUG, logger=LOGGER):
        words.lose()
        await until(lambda: len(asked) == 3)
        await asyncio.sleep(0.05)

    # It had as long as an attempt to connect has. Then its tools were asked for, which
    # showed that the connection is there, and nothing more is expected of its word.
    assert "the MCP server served did not say within 0.2 seconds that it listens again" in (
        caplog.messages
    )
    assert (words.entered, len(asked), connection.state) == (2, 3, "connected")
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_a_connection_that_is_lost_while_its_server_is_to_be_listened_to_again_is_made_again(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    words = Words().over(monkeypatch)
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, mode="auto", sleep=naps))
    words.lose()
    await asyncio.wait_for(naps.begun.wait(), 5)
    assert naps.asked == [FIRST_WAIT]

    # The wait before the server is listened to again is under way when a call finds the
    # connection closed. Nobody waits that wait out: the connection is left at once.
    await transport.cut()
    with pytest.raises(CallError):
        await connection.call("add", TWO, 5)
    await until(lambda: transport.left == 1)

    assert words.entered == 1
    # The wait that follows is the one before the connection is made again.
    await asyncio.wait_for(naps.begun.wait(), 5)
    naps.end()
    await until(lambda: transport.entered == 2 and connection.state == "connected")
    assert words.entered == 2


async def test_a_connection_is_closed_at_once_while_its_server_is_to_be_listened_to_again(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    words = Words().over(monkeypatch)
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(make("served", lambda: transport, sleep=naps))
    words.end()
    await asyncio.wait_for(naps.begun.wait(), 5)

    await asyncio.wait_for(connection.close(), 5)

    assert (connection.state, transport.left, words.entered, words.left) == ("closed", 1, 1, 1)


async def test_a_server_that_does_not_say_when_its_tools_change_is_none_the_worse_for_it(
    make: Callable[..., Connection], served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    whole = Client.list_tools
    asked: list[None] = []

    async def counted(self: Client, **what: Any) -> ListToolsResult:
        asked.append(None)
        return await whole(self)

    @asynccontextmanager
    async def refused(self: Client, **what: Any) -> AsyncGenerator[None]:
        raise MCPError(-32601, "Method not found")
        yield

    monkeypatch.setattr(Client, "list_tools", counted)
    monkeypatch.setattr(Client, "listen", refused)

    connection = await begun(make("served", lambda: served.server))
    await asyncio.sleep(0.05)

    assert connection.state == "connected"
    assert shown(await connection.call("add", TWO, 5)) == "2"
    # Its tools were asked for once. Nothing will say that they changed, so nothing asks again.
    assert len(asked) == 1


async def test_whoever_is_told_of_the_tools_and_fails_does_not_end_the_connection(
    make: Callable[..., Connection], served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    async def failing() -> None:
        raise RuntimeError("could not take the tools")

    with caplog.at_level(logging.ERROR, logger=LOGGER):
        connection = await begun(make("served", lambda: served.server, on_tools=failing))

    assert caplog.messages == ["the MCP server served: what was to be told of its tools failed"]
    assert connection.state == "connected"
    assert shown(await connection.call("add", TWO, 5)) == "2"


async def test_the_first_attempt_is_over_only_when_whoever_is_told_of_the_tools_has_them(
    make: Callable[..., Connection], served: Served
) -> None:
    taken = asyncio.Event()
    asked: list[list[str]] = []

    async def taking() -> None:
        asked.append(names(connection))
        await taken.wait()

    connection = make("served", lambda: served.server, on_tools=taking)
    connection.start()
    settling = asyncio.ensure_future(connection.settled())
    await until(lambda: bool(asked))
    await asyncio.sleep(0.05)

    # The tools are there to be taken, and nobody is told yet that the attempt is over.
    assert asked == [BASE]
    assert not settling.done()
    taken.set()
    await asyncio.wait_for(settling, 5)
    assert connection.state == "connected"


# A connection that is closed


async def test_a_connection_that_is_closed_is_left_and_nobody_is_kept_waiting(
    served: Served,
) -> None:
    transport = Cuttable(served)
    connection = await begun(Connection("served", lambda: transport))
    calling = asyncio.ensure_future(connection.call("nap", {"seconds": 30}, 60))
    await until(lambda: served.napping == 1)

    await connection.close()

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(calling, 5)
    assert str(refused.value) == LOST
    assert connection.state == "closed"
    assert (transport.entered, transport.left) == (1, 1)
    with pytest.raises(CallError) as after:
        await connection.call("add", TWO, 5)
    assert str(after.value) == CLOSED
    # Closed is closed: it is not started again, and closing it once more does nothing.
    connection.start()
    await asyncio.sleep(0.05)
    await connection.close()
    assert (connection.state, transport.entered) == ("closed", 1)


async def test_a_connection_can_be_closed_before_it_was_started() -> None:
    connection = Connection("served", Hanging)

    await connection.close()

    assert connection.state == "closed"


async def test_whoever_waits_for_a_connection_that_is_closed_meanwhile_is_told_so(
    served: Served,
) -> None:
    transport = Cuttable(served)
    naps = Naps(held=True)
    connection = await begun(Connection("served", lambda: transport, sleep=naps))
    await lose(connection, transport)
    await asyncio.wait_for(naps.begun.wait(), 5)
    calling = asyncio.ensure_future(connection.call("add", TWO, 5))
    await asyncio.sleep(0.05)
    assert not calling.done()

    await connection.close()

    with pytest.raises(CallError) as refused:
        await asyncio.wait_for(calling, 5)
    assert str(refused.value) == CLOSED
    assert (transport.entered, transport.left) == (1, 1)


async def test_a_connection_that_is_closed_while_it_is_made_is_not_made(
    served: Served,
) -> None:
    transport = Hanging()
    connection = Connection("served", lambda: transport)
    connection.start()
    await until(lambda: transport.entered == 1)

    await asyncio.wait_for(connection.close(), 5)

    assert connection.state == "closed"
    assert connection.failure is None


async def test_closing_does_not_wait_for_good_for_a_connection_that_does_not_let_go(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    released = asyncio.Event()

    class Clinging(Cuttable):
        async def __aexit__(self, *why: Any) -> None:
            # It does not let go for all that it is asked to.
            while not released.is_set():
                with suppress(asyncio.CancelledError):
                    await released.wait()
            await super().__aexit__(*why)

    monkeypatch.setattr(connection_module, "LEAVE_SECONDS", 0.1)
    transport = Clinging(served)
    connection = await begun(Connection("served", lambda: transport))

    try:
        with caplog.at_level(logging.WARNING, logger=LOGGER):
            await asyncio.wait_for(connection.close(), 5)

        assert caplog.messages == ["the MCP server served did not let go within 0.1 seconds"]
        assert connection.state == "closed"
    finally:
        # Whatever was found, the connection lets go now. One that never did would keep the
        # run of the tests from ending.
        released.set()
    await until(lambda: transport.left == 1)


async def test_closing_sees_a_connection_left_that_lets_go_only_when_its_time_is_up(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    class Slow(Cuttable):
        async def __aexit__(self, *why: Any) -> None:
            try:
                # It would take an hour. Leaving has a limit, and the limit ends this.
                await asyncio.sleep(3600)
            finally:
                await super().__aexit__(*why)

    monkeypatch.setattr(connection_module, "LEAVE_SECONDS", 0.1)
    transport = Slow(served)
    connection = await begun(Connection("served", lambda: transport))

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        await asyncio.wait_for(connection.close(), 5)

    # Whoever closes waits a moment longer than leaving may take, and so sees the limit kept:
    # the connection is left when closing comes back, and nothing is said of one that clings.
    assert (transport.left, connection.state) == (1, "closed")
    assert caplog.messages == []


async def test_closing_again_waits_again_for_a_connection_that_did_not_let_go(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    released = asyncio.Event()

    class Clinging(Cuttable):
        async def __aexit__(self, *why: Any) -> None:
            while not released.is_set():
                with suppress(asyncio.CancelledError):
                    await released.wait()
            await super().__aexit__(*why)

    monkeypatch.setattr(connection_module, "LEAVE_SECONDS", 0.1)
    transport = Clinging(served)
    connection = await begun(Connection("served", lambda: transport))
    said = "the MCP server served did not let go within 0.1 seconds"

    try:
        with caplog.at_level(logging.WARNING, logger=LOGGER):
            await asyncio.wait_for(connection.close(), 5)
            # It is still held, and whoever closes it a second time is not told otherwise
            # at once: the wait is made again.
            await asyncio.wait_for(connection.close(), 5)
            assert (caplog.messages, transport.left) == ([said, said], 0)
            released.set()
            await asyncio.wait_for(connection.close(), 5)
        # This time it let go, and closing came back when it had.
        assert (caplog.messages, transport.left) == ([said, said], 1)
        # There is nothing left to wait for.
        await asyncio.wait_for(connection.close(), 5)
        assert (caplog.messages, connection.state) == ([said, said], "closed")
    finally:
        released.set()
    await until(lambda: transport.left == 1)


# A server at an address, and a session that it ended

ENDED = (
    "the connection to the MCP server served was lost: the server no longer knew the session. "
    "It is being made again"
)


SESSIONS = re.compile("[0-9a-f]{32}")
"""What a session of the server of the SDK is called."""


@pytest.fixture
def online(served: Served, monkeypatch: pytest.MonkeyPatch) -> Iterator[Online]:
    """The server at an address of this machine, with a tool that tells its session."""
    # What is next door is reached directly, whatever proxy this machine names.
    for name in ("ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY"):
        monkeypatch.delenv(name, raising=False)
        monkeypatch.delenv(name.lower(), raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")
    monkeypatch.setenv("no_proxy", "127.0.0.1")
    served.with_its_session_told()
    with Online(served) as online:
        yield online


@pytest.fixture
def reaching(online: Online, make: Callable[..., Connection]) -> Callable[..., Connection]:
    """Makes connections to the server at its address.

    The server is asked for before the maker of connections, so the connections are closed
    while the server still answers.
    """

    def one(path: str = "", **options: Any) -> Connection:
        # As to a server from before 2026, which gives every connection a session.
        options.setdefault("mode", "legacy")
        return make("served", lambda: reached(online.url + path, {}), **options)

    return one


def adding(a: int) -> Callable[[dict[str, Any]], bool]:
    """Picks out the calls of the tool that adds whose first number is this one."""

    def picked(asked: dict[str, Any]) -> bool:
        params: dict[str, Any] = asked.get("params") or {}
        arguments: dict[str, Any] = params.get("arguments") or {}
        return params.get("name") == "add" and arguments.get("a") == a

    return picked


async def ended(connection: Connection, online: Online) -> str:
    """End the session of a connection on the server, and return what it was called."""
    session = shown(await connection.call("session", {}, 5))
    await asyncio.to_thread(online.end, session)
    return session


async def test_a_server_at_an_address_is_connected_to_and_called(
    reaching: Callable[..., Connection],
) -> None:
    connection = await begun(reaching())

    assert (connection.state, names(connection)) == ("connected", [*BASE, "session"])
    assert shown(await connection.call("add", TWO, 5)) == "2"
    # It gave the connection a session, as servers did before 2026.
    told = await connection.call("session", {}, 5)
    assert (told.is_error, SESSIONS.fullmatch(shown(told)) is not None) == (False, True)


async def test_a_call_of_a_tool_that_only_reads_is_made_again_over_a_new_session(
    reaching: Callable[..., Connection], online: Online, caplog: pytest.LogCaptureFixture
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    first = await ended(connection, online)

    with caplog.at_level(logging.WARNING):
        result = await connection.call("add", TWO, 5, reads=True)

    # By the protocol the server took nothing of the call, and a tool that only reads may be
    # called twice in any case.
    assert shown(result) == "2"
    assert (connection.state, connection.failure) == ("connected", None)
    assert shown(await connection.call("session", {}, 5)) not in ("", first)
    # Leaving the old connection, the SDK could not end a session that was gone. Nothing
    # of that is said: it is no failure, and nobody could do anything about it.
    assert (caplog.messages, naps.asked) == ([ENDED], [FIRST_WAIT])


async def test_a_call_that_may_change_something_is_not_made_again_by_itself(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    await ended(connection, online)

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5)

    # A server that is worse than the protocol may have done something all the same, so
    # whoever made the call decides whether to make it again.
    assert str(failed.value) == (
        "the MCP server served turned the call away as one that names a session it knows "
        "nothing of, and said: Session not found. The connection is being made again. Call "
        "the tool again if that is still wanted"
    )
    await until(lambda: connection.state == "connected")
    assert shown(await connection.call("add", TWO, 5)) == "2"
    assert naps.asked == [FIRST_WAIT]


async def test_a_call_that_a_server_sends_on_and_then_turns_away_is_made_over_a_new_session(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    # The server answers at its path without a slash at the end, and sends whatever comes
    # to the path with one on to there. Every request of this connection goes that way.
    connection = await begun(reaching("/", sleep=naps))
    first = await ended(connection, online)

    # What was sent on is a request of its own, made from the first one. It is a call all
    # the same, and it was turned away for want of its session.
    assert shown(await connection.call("add", TWO, 5, reads=True)) == "2"
    assert (connection.state, naps.asked) == ("connected", [FIRST_WAIT])
    assert shown(await connection.call("session", {}, 5)) not in ("", first)


async def test_a_call_that_finds_no_session_over_the_new_connection_either_is_not_made_again(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    # Every call from now on is answered as one that names a session nobody knows.
    online.turn_away("tools/call")

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5, reads=True)

    assert str(failed.value) == (
        "the MCP server served turned the call away as one that names a session it knows "
        "nothing of, also over a connection that was made anew for it, and said: Session not "
        "found. The call was not made"
    )
    # Over the connection that was there and over the next one, and over no third.
    assert online.turned_away == 2
    # The keeper was told both times, and makes the connection once more. A server that
    # ends sessions is alive all the same, so the waits do not grow, and it is not given up.
    await until(lambda: connection.state == "connected" and len(naps.asked) == 2)
    assert (naps.asked, online.turned_away) == ([FIRST_WAIT, FIRST_WAIT], 2)


async def test_a_server_that_says_so_with_the_code_of_a_closed_connection_is_understood(
    reaching: Callable[..., Connection], online: Online
) -> None:
    connection = await begun(reaching(sleep=Naps()))
    # The code is the one that the SDK gives a connection that is closed. What the status
    # says is the more.
    online.turn_away("tools/call", code=-32000)

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5)

    assert str(failed.value).startswith(
        "the MCP server served turned the call away as one that names a session it knows "
        "nothing of, and said: Session not found."
    )


async def test_calls_under_way_when_a_session_is_found_gone_hear_their_own_answer(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    await ended(connection, online)
    # The server is busy with one of the two, and answers it a moment after the other.
    online.slow(0.3, adding(2))

    results = await asyncio.gather(
        connection.call("add", TWO, 5, reads=True),
        connection.call("add", {"a": 2, "b": 3}, 5, reads=True),
    )

    # The keeper waited until the second had heard that its session was gone as well, and
    # did not break it off: both were made again over one new connection.
    assert [shown(result) for result in results] == ["2", "5"]
    assert naps.asked == [FIRST_WAIT]


async def test_a_call_that_stays_under_way_is_waited_for_a_moment_and_no_longer(
    reaching: Callable[..., Connection], online: Online, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(connection_module, "HEAR_SECONDS", 0.2)
    connection = await begun(reaching(sleep=Naps(), startup_seconds=2))
    await ended(connection, online)
    # The server holds the second call, as it holds a call that it took before the session
    # ended and is still at.
    online.slow(3600, adding(2))
    held = asyncio.ensure_future(connection.call("add", {"a": 2, "b": 3}, 30, reads=True))
    await until(lambda: online.waiting == 1)

    result = await connection.call("add", TWO, 5, reads=True)

    assert shown(result) == "2"
    with pytest.raises(CallError) as failed:
        await asyncio.wait_for(held, 5)
    assert str(failed.value) == LOST


async def test_what_one_call_heard_is_not_taken_for_what_another_heard(
    reaching: Callable[..., Connection], online: Online
) -> None:
    connection = await begun(reaching(sleep=Naps()))
    await ended(connection, online)
    # The server is busy with the second call, and answers it with an error of its own a
    # moment after it turned the first away.
    online.slow(0.3, adding(2))
    online.turn_away("tools/call", status=500, said="The server is busy", only=adding(2))

    first, second = await asyncio.gather(
        connection.call("add", TWO, 5, reads=True),
        connection.call("add", {"a": 2, "b": 3}, 5, reads=True),
        return_exceptions=True,
    )

    assert shown(cast("CallToolResult", first)) == "2"
    assert str(second) == (
        "the MCP server served answered the call with an error: The server is busy"
    )
    # The second was made once: it was not taken for one that was turned away.
    assert online.turned_away == 1


async def test_a_call_made_again_that_the_server_answers_with_an_error_says_that_error(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    await ended(connection, online)
    # The first call reaches the server, which knows nothing of its session. The one after
    # it, over the new connection, is answered with an error of the server's own.
    seen = itertools.count()
    online.turn_away(
        "tools/call", status=500, said="The server is busy", only=lambda asked: next(seen) > 0
    )

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5, reads=True)

    assert str(failed.value) == (
        "the MCP server served answered the call with an error: The server is busy"
    )
    assert (online.turned_away, naps.asked) == (1, [FIRST_WAIT])


async def test_a_call_made_again_is_not_made_where_the_server_lists_its_tool_otherwise_by_then(
    served: Served, reaching: Callable[..., Connection], online: Online
) -> None:
    served.with_a_reader()
    connection = await begun(reaching(sleep=Naps()))
    (peek,) = [tool for tool in connection.tools if tool.name == "peek"]
    await ended(connection, online)
    # The server comes back with the tool, and no longer says that it only reads.
    served.with_a_reader_that_writes_now()

    with pytest.raises(CallError) as failed:
        await connection.call("peek", {}, 5, listed=peek, reads=True)

    assert str(failed.value) == (
        "the MCP server served no longer lists the tool peek as it did when this call was "
        "made, so the call was not made. Look at the tool as it is now, and call it again if "
        "that is still wanted"
    )
    assert served.peeked == 0


async def test_a_call_that_waits_to_be_made_again_is_answered_when_the_connection_is_closed(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps(held=True)
    connection = await begun(reaching(sleep=naps))
    await ended(connection, online)

    calling = asyncio.ensure_future(connection.call("add", TWO, 5, reads=True))
    # The keeper waits before it connects again, and the call waits for it.
    await asyncio.wait_for(naps.begun.wait(), 5)
    await connection.close()

    with pytest.raises(CallError) as failed:
        await asyncio.wait_for(calling, 5)
    assert str(failed.value) == CLOSED


async def test_an_error_that_a_server_answers_a_call_with_ends_no_session(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    connection = await begun(reaching(sleep=naps))
    first = shown(await connection.call("session", {}, 5))
    online.turn_away("tools/call", status=500, said="The server is busy")

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5)

    assert str(failed.value) == (
        "the MCP server served answered the call with an error: The server is busy"
    )
    online.turn_away(None)
    # The call was not made a second time, and the connection is the one that it was.
    assert (online.turned_away, naps.asked, connection.state) == (1, [], "connected")
    assert shown(await connection.call("session", {}, 5)) == first


async def test_a_call_that_is_turned_away_where_there_is_no_session_is_an_error_like_any_other(
    reaching: Callable[..., Connection], online: Online
) -> None:
    naps = Naps()
    # A server of 2026 gives no session, so there is none that it could have ended.
    connection = await begun(reaching(mode="auto", sleep=naps))
    assert shown(await connection.call("session", {}, 5)) == ""
    online.turn_away("tools/call")

    with pytest.raises(CallError) as failed:
        await connection.call("add", TWO, 5)

    assert str(failed.value) == (
        "the MCP server served answered the call with an error: Session not found"
    )
    assert (online.turned_away, naps.asked, connection.state) == (1, [], "connected")


def test_a_call_that_is_turned_away_while_nobody_calls_is_nobodys() -> None:
    # A way to a server says this where it sees how the server answers. Outside of a call
    # there is nobody to tell, and nothing comes of it.
    connection_module.turned_away()
