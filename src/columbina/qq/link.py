"""The gateway of QQ: the WebSocket on which the bot hears what is written to it.

QQ greets a connection with the time between two heartbeats. This side then signs in with the
access token of the bot and the kinds of event that it wants to hear, those of groups and of
chats of two, and QQ answers that the bot is ready, with a session. From then on QQ sends each
event with a number, and this side sends a heartbeat with the last number that it heard, as
often as QQ asked. Where two heartbeats in a row go unanswered, the connection is taken for
lost.

A connection that is lost is opened again after a pause that doubles each time from one second,
up to thirty, and the session is taken up where it was, so that no event is lost meanwhile.
QQ may say that a session is no longer valid: the bot then signs in anew. Where QQ closes the
connection before the bot is ready, five times in a row, or says that the bot is banned or
offline, it is not tried again, and the error says why.

What is written about the connection never holds the token.
"""

import asyncio
import json
import logging
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass, field
from typing import Any, Protocol, cast

from columbina.qq.app import Api, QqError

__all__ = ["GROUP", "INTENTS", "PRIVATE", "Link", "Opener", "Socket"]

logger = logging.getLogger(__name__)

INTENTS = 1 << 25
"""The kinds of event that the bot asks for: those of groups and of chats of two."""

GROUP = "GROUP_AT_MESSAGE_CREATE"
"""The event of a message in a group that mentions the bot."""

PRIVATE = "C2C_MESSAGE_CREATE"
"""The event of a message to the bot in a chat of two."""

_DISPATCH, _HEARTBEAT, _IDENTIFY, _RESUME, _RECONNECT, _INVALID, _HELLO, _ACK = (
    0,
    1,
    2,
    6,
    7,
    9,
    10,
    11,
)

_FOR_GOOD = {4914: "the bot is offline", 4915: "the bot is banned"}
"""The codes with which QQ closes a connection for a reason that waiting does not change."""

_SIGN_IN_SECONDS = 20.0
_MISSED_MOST = 2
_REFUSALS_MOST = 5
_FIRST_PAUSE = 1.0
_LONGEST_PAUSE = 30.0
_LARGEST = 16 * 1024 * 1024


class Socket(Protocol):
    """The side of a WebSocket that this module uses."""

    async def send(self, message: str) -> None:
        """Send a message of text."""
        ...

    async def close(self) -> None:
        """Close the connection as one that ended as it was meant to."""
        ...

    def __aiter__(self) -> AsyncIterator[bytes | str]:
        """Give the messages that come in, until the connection is closed."""
        ...


Opener = Callable[[str], AbstractAsyncContextManager[Socket]]
"""Opens a WebSocket to an address. The tests put sockets of their own in its place."""


_QUIET = logging.getLogger("columbina.qq.websockets")
_QUIET.setLevel(logging.WARNING)


def _opened(url: str) -> AbstractAsyncContextManager[Socket]:
    """Open a WebSocket with the library that comes with ``columbina[qq]``."""
    from websockets.asyncio.client import connect

    # The heartbeats that keep the connection are those of QQ, and not those of WebSocket.
    return cast(
        "AbstractAsyncContextManager[Socket]",
        connect(url, max_size=_LARGEST, open_timeout=20, ping_interval=None, logger=_QUIET),
    )


class _Passing(Exception):  # noqa: N818 - a failure that may pass, and no error of a caller
    """The connection could not be opened, or was lost. Another try may do."""


class _Refused(_Passing):
    """QQ closed the connection before the bot was ready."""


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def _closed_with(error: BaseException) -> int | None:
    """Return the code with which the other side closed a connection, where it gave one."""
    received = getattr(error, "rcvd", None)
    code = getattr(received, "code", None)
    return code if isinstance(code, int) else None


@dataclass(slots=True)
class Link:
    """The gateway of one bot.

    Attributes:
        api: The requests of the bot, which give the token and the address of the gateway.
        heard: Is given each event that is no matter of the connection: its kind, what it holds
            and its id. It is called on the loop of the connection and should wait for nothing.
        told: Is told how the connection stands, ``"connecting"`` or ``"connected"``, and why
            the last try failed or the connection was lost, until it is open again.
        opener: Opens a WebSocket.
        sleep: Waits, for the pauses before the connection is opened again.
        ticking: Waits, between two heartbeats.
    """

    api: Api
    heard: Callable[[str, dict[str, Any], str | None], None]
    told: Callable[[str, str | None], None]
    opener: Opener = _opened
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep
    ticking: Callable[[float], Awaitable[None]] = asyncio.sleep
    _session: str | None = None
    _number: int | None = None
    _missed: int = 0
    _every: float = 30.0
    _ready: bool = field(default=False)

    async def run(self) -> None:
        """Keep the connection open until this is cancelled.

        Raises:
            QqError: QQ refuses the bot for a reason that waiting does not change.
        """
        failed = refused = 0
        self.told("connecting", None)
        while True:
            self._ready = False
            try:
                url = await self.api.gateway()
                async with self.opener(url) as socket:
                    messages = aiter(socket)
                    await self._signed_in(socket, messages)
                    failed = refused = 0
                    logger.info("connected to QQ")
                    self.told("connected", None)
                    await self._kept(socket, messages)
                why = "the connection was closed"
            except QqError as error:
                if error.code != "UNREACHABLE":
                    raise
                why = str(error)
            except Exception as error:
                why, early = self._ended(error)
                if early:
                    refused += 1
                    if refused >= _REFUSALS_MOST:
                        raise QqError(
                            f"QQ closed the connection {refused} times in a row before the bot "
                            f"was ready: {why}",
                            code="REFUSED",
                        ) from None
            failed += 1
            logger.warning("the connection to QQ is not open: %s", why)
            self.told("connecting", why)
            await self.sleep(min(_FIRST_PAUSE * 2 ** min(failed - 1, 5), _LONGEST_PAUSE))

    def _ended(self, error: BaseException) -> tuple[str, bool]:
        """Say why a connection ended, and whether QQ closed it before the bot was ready.

        Raises:
            QqError: QQ closed it for a reason that waiting does not change.
        """
        if isinstance(error, _Passing):
            return str(error), isinstance(error, _Refused)
        code = _closed_with(error)
        if code is None:
            # The network, and not QQ: that is tried again however often it happens.
            return str(error) or type(error).__name__, False
        if code in _FOR_GOOD:
            raise QqError(
                f"QQ closed the connection: {_FOR_GOOD[code]} (code {code})", code="REFUSED"
            )
        if code == 4004:
            # The token was not taken: another one is asked for.
            self.api.forget_token()
        if code in (4006, 4007, 4009):
            # The session cannot be taken up: the bot signs in anew.
            self._session = self._number = None
        return f"the connection was closed with the code {code}", not self._ready

    async def _signed_in(self, socket: Socket, messages: AsyncIterator[bytes | str]) -> None:
        """Sign the bot in on a connection that was opened, or take up its session.

        Raises:
            _Passing: QQ did not answer in time, or the connection broke off.
        """
        try:
            async with asyncio.timeout(_SIGN_IN_SECONDS):
                hello = await self._next(messages)
                if hello.get("op") != _HELLO:
                    raise _Passing("QQ did not greet the connection")
                interval = _object(hello.get("d")).get("heartbeat_interval")
                if isinstance(interval, int | float) and interval > 0:
                    self._every = interval / 1000
                token = f"QQBot {await self.api.token()}"
                if self._session is not None:
                    resume = {"token": token, "session_id": self._session, "seq": self._number}
                    await socket.send(json.dumps({"op": _RESUME, "d": resume}))
                else:
                    identify = {"token": token, "intents": INTENTS, "shard": [0, 1]}
                    await socket.send(json.dumps({"op": _IDENTIFY, "d": identify}))
                while True:
                    frame = await self._next(messages)
                    if frame.get("op") == _INVALID:
                        self._session = self._number = None
                        raise _Passing("QQ did not take up the session: the bot signs in anew")
                    if frame.get("op") != _DISPATCH:
                        continue
                    kind = frame.get("t")
                    if kind == "READY":
                        session = _object(frame.get("d")).get("session_id")
                        self._session = session if isinstance(session, str) else None
                        self._number = self._numbered(frame)
                        break
                    if kind == "RESUMED":
                        break
                    # What QQ sends again while a session is taken up counts as well.
                    self._dispatch(frame)
        except TimeoutError:
            raise _Passing("QQ did not answer the sign-in in time") from None
        self._ready = True

    async def _next(self, messages: AsyncIterator[bytes | str]) -> dict[str, Any]:
        async for message in messages:
            frame = self._read(message)
            if frame is not None:
                return frame
        raise _Refused("the connection was closed before the bot was ready")

    async def _kept(self, socket: Socket, messages: AsyncIterator[bytes | str]) -> None:
        """Keep a connection until it is closed: beat, and take what comes in.

        Raises:
            _Passing: QQ asked for a new connection, or stopped answering the heartbeats.
        """
        self._missed = 0
        reading = asyncio.ensure_future(self._reading(socket, messages))
        beating = asyncio.ensure_future(self._beating(socket))
        try:
            done, _ = await asyncio.wait({reading, beating}, return_when=asyncio.FIRST_COMPLETED)
        except asyncio.CancelledError:
            try:
                await socket.close()
            except Exception as error:
                logger.info("the connection to QQ could not be closed: %s", error)
            raise
        finally:
            for task in (reading, beating):
                task.cancel()
            await asyncio.gather(reading, beating, return_exceptions=True)
        for task in done:
            task.result()

    async def _reading(self, socket: Socket, messages: AsyncIterator[bytes | str]) -> None:
        async for message in messages:
            frame = self._read(message)
            if frame is None:
                continue
            op = frame.get("op")
            if op == _DISPATCH:
                self._dispatch(frame)
            elif op == _ACK:
                self._missed = 0
            elif op == _HEARTBEAT:
                await self._beat(socket)
            elif op == _RECONNECT:
                raise _Passing("QQ asked for a new connection")
            elif op == _INVALID:
                self._session = self._number = None
                raise _Passing("QQ said that the session is no longer valid")

    async def _beating(self, socket: Socket) -> None:
        while True:
            await self.ticking(self._every)
            if self._missed >= _MISSED_MOST:
                raise _Passing(f"QQ did not answer {self._missed} heartbeats in a row")
            self._missed += 1
            await self._beat(socket)

    async def _beat(self, socket: Socket) -> None:
        try:
            await socket.send(json.dumps({"op": _HEARTBEAT, "d": self._number}))
        except Exception as error:
            raise _Passing(f"a heartbeat could not be sent: {error}") from None

    def _numbered(self, frame: dict[str, Any]) -> int | None:
        number = frame.get("s")
        return number if isinstance(number, int) else self._number

    def _dispatch(self, frame: dict[str, Any]) -> None:
        self._number = self._numbered(frame)
        kind, event_id = frame.get("t"), frame.get("id")
        if not isinstance(kind, str):
            return
        try:
            self.heard(
                kind, _object(frame.get("d")), event_id if isinstance(event_id, str) else None
            )
        except Exception as error:
            logger.error("what QQ sent could not be taken", exc_info=error)

    def _read(self, message: bytes | str) -> dict[str, Any] | None:
        """Return a frame as the object of JSON that it is, or ``None`` for one that is no such."""
        text = message.decode("utf-8", "replace") if isinstance(message, bytes) else message
        try:
            frame: object = json.loads(text)
        except ValueError:
            logger.warning("QQ sent something that is no JSON")
            return None
        if not isinstance(frame, dict):
            logger.warning("QQ sent something that is no object of JSON")
            return None
        return cast("dict[str, Any]", frame)
