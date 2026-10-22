"""The long connection to WeCom: how the assistant hears its bot, and answers through it.

A smart bot whose API is set to the long connection opens a WebSocket at the address of WeCom
and signs in on it with a frame that holds its Bot ID and its Secret. WeCom then sends over it
what is written to the bot, and takes the answers over it too. Every frame is an object of
JSON. One that WeCom sends of itself names a command in ``cmd``; one that answers a frame of
this side has the ``req_id`` of that frame in its headers, with ``errcode`` and ``errmsg``.

The connection is kept going from this side: once the bot has signed in, a ``ping`` goes out
every thirty seconds, and WeCom answers each. Where two pings in a row go unanswered, the
connection is taken for lost. A connection that is lost, or that could not be opened, is
opened again after a pause that doubles each time from one second, up to thirty. WeCom refuses
a Bot ID and a Secret that do not belong together, and a bot whose API is not set to the long
connection, for good: those are not tried again, and the error says why. A bot has one
connection at a time. Where another program signs in with it, WeCom tells this one so and
closes it, and this one is not opened again either.

WeCom answers a frame with its ``req_id`` alone. So the frames that answer one message, which
all go out with the ``req_id`` of the message, are sent one after another, each once the one
before it was answered.

What is written about the connection never holds the Secret.
"""

import asyncio
import json
import logging
import re
import secrets
import time
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass, field
from typing import Any, Literal, Protocol, cast
from urllib.parse import urlsplit

from columbina.llm.credentials import spellings
from columbina.net import proxy_for
from columbina.wecom.app import WecomError

__all__ = [
    "EVENT",
    "MESSAGE",
    "RESPOND",
    "SEND",
    "WELCOME",
    "Link",
    "Opener",
    "Socket",
    "request_id",
]

logger = logging.getLogger(__name__)

SUBSCRIBE = "aibot_subscribe"
PING = "ping"
MESSAGE = "aibot_msg_callback"
"""The command of a frame that brings a message to the bot."""
EVENT = "aibot_event_callback"
"""The command of a frame that brings an event, such as a person opening the chat."""
RESPOND = "aibot_respond_msg"
"""The command that answers a message, under its ``req_id``."""
WELCOME = "aibot_respond_welcome_msg"
"""The command that greets a person who opened the chat, under the ``req_id`` of the event."""
SEND = "aibot_send_msg"
"""The command that sends a message that answers no message of the moment."""

_TAKEN_OVER = "disconnected_event"
"""The event with which WeCom says that another connection of the bot took the place of this one."""

_WRONG = 853000
"""The errcode with which WeCom says that the Bot ID or the Secret is not right."""

_NOT_LONG = 846601
"""The errcode with which WeCom says that the API of the bot is not set to the long connection."""

_PING_SECONDS = 30.0
_MISSED_MOST = 2
"""How many pings may go unanswered in a row before the connection is taken for lost."""

_SIGN_IN_SECONDS = 20.0
"""How long WeCom is given to answer the sign-in."""

_ANSWER_SECONDS = 10.0
"""How long WeCom is given to answer a frame that answers a message or sends one."""

_REFUSALS_MOST = 5
"""How many sign-ins WeCom may refuse in a row, for a reason that may pass, before none is tried."""

_FIRST_PAUSE = 1.0
_LONGEST_PAUSE = 30.0

_LARGEST = 16 * 1024 * 1024
"""How large a message on the connection may be, in bytes."""

_CONTROL = re.compile(r"[\x00-\x08\x0b-\x0d\x0e-\x1f]")
"""The characters that WeCom may put into a frame, which JSON does not take: they are left out."""

_PROXY_VARIABLES = ("https_proxy", "http_proxy", "all_proxy")


def request_id(prefix: str) -> str:
    """Return a ``req_id`` of this side: what it is for, the time, and something drawn at random."""
    return f"{prefix}_{round(time.time() * 1000)}_{secrets.token_hex(4)}"


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


Opener = Callable[[str, str | Literal[True] | None], AbstractAsyncContextManager[Socket]]
"""Opens a WebSocket to an address, through a proxy: ``True`` for the one that the environment
names, ``None`` for none. The tests put sockets of their own in its place."""


_QUIET = logging.getLogger("columbina.wecom.websockets")
"""What the library of WebSockets writes about a connection, at warnings and above."""
_QUIET.setLevel(logging.WARNING)


def _opened(url: str, proxy: str | Literal[True] | None) -> AbstractAsyncContextManager[Socket]:
    """Open a WebSocket with the library that comes with ``columbina[wecom]``."""
    # Only an assistant in WeCom needs it, and it comes with columbina[wecom].
    from websockets.asyncio.client import connect

    # The pings that keep the connection are frames of WeCom, and not those of WebSocket.
    return cast(
        "AbstractAsyncContextManager[Socket]",
        connect(
            url, proxy=proxy, max_size=_LARGEST, open_timeout=20, ping_interval=None, logger=_QUIET
        ),
    )


class _Passing(Exception):  # noqa: N818 - a failure that may pass, and no error of a caller
    """The connection could not be opened, or was lost. Another try may do."""


class _Refused(_Passing):
    """WeCom refused the sign-in for a reason that may pass."""


@dataclass(slots=True)
class _Line:
    """The frames that go out under one ``req_id``, one after another."""

    lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    users: int = 0


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def _frame(cmd: str, req_id: str, body: Mapping[str, Any] | None = None) -> str:
    sent: dict[str, Any] = {"cmd": cmd, "headers": {"req_id": req_id}}
    if body is not None:
        sent["body"] = body
    return json.dumps(sent, ensure_ascii=False)


def _heeded(answer: asyncio.Future[dict[str, Any]]) -> None:
    """Take note of how an answer ended. A wait that ran out cancelled it."""
    if not answer.cancelled():
        answer.exception()


def _req_id(frame: Mapping[str, Any]) -> str | None:
    found = _object(frame.get("headers")).get("req_id")
    return found if isinstance(found, str) else None


@dataclass(slots=True)
class Link:
    """The long connection of one bot.

    Attributes:
        place: Where the connection is opened.
        bot_id: The Bot ID of the bot.
        secret: Its Secret.
        heard: Is given each frame that brings a message or an event, as the object of JSON
            that WeCom sent. It is called on the loop of the connection and should wait for
            nothing.
        told: Is told how the connection stands, ``"connecting"`` or ``"connected"``, and why
            the last try failed or the connection was lost, until it is open again.
        env: The environment, which may name a proxy.
        opener: Opens a WebSocket.
        sleep: Waits, for the pauses before the connection is opened again.
        ticking: Waits, between two pings.
    """

    place: str
    bot_id: str
    secret: str = field(repr=False)
    heard: Callable[[dict[str, Any]], None]
    told: Callable[[str, str | None], None]
    env: Mapping[str, str] = field(default_factory=dict[str, str])
    opener: Opener = _opened
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep
    ticking: Callable[[float], Awaitable[None]] = asyncio.sleep
    _socket: Socket | None = None
    _missed: int = 0
    _answers: dict[str, asyncio.Future[dict[str, Any]]] = field(
        default_factory=dict[str, "asyncio.Future[dict[str, Any]]"]
    )
    _lines: dict[str, _Line] = field(default_factory=dict[str, _Line])

    async def run(self) -> None:
        """Keep the connection open until this is cancelled.

        Raises:
            WecomError: WeCom refuses the bot for a reason that waiting does not change, or
                another program signed in with it.
        """
        failed = refused = 0
        self.told("connecting", None)
        while True:
            try:
                async with self.opener(self.place, self._proxy()) as socket:
                    messages = aiter(socket)
                    await self._signed_in(socket, messages)
                    failed = refused = 0
                    logger.info("connected to WeCom at %s", urlsplit(self.place).hostname)
                    self.told("connected", None)
                    await self._kept(socket, messages)
                why = "the connection was closed"
            except _Refused as error:
                refused += 1
                if refused >= _REFUSALS_MOST:
                    raise WecomError(
                        f"WeCom refused the sign-in {refused} times in a row: {error}",
                        code="REFUSED",
                    ) from None
                why = str(error)
            except _Passing as error:
                why = str(error)
            except WecomError:
                raise
            except Exception as error:
                why = self._said(error)
            failed += 1
            logger.warning("the connection to WeCom is not open: %s", why)
            self.told("connecting", why)
            await self.sleep(min(_FIRST_PAUSE * 2 ** (failed - 1), _LONGEST_PAUSE))

    async def check(self) -> None:
        """Open a connection, sign in on it, and close it: whether WeCom takes the bot.

        Raises:
            WecomError: WeCom refused the bot, or could not be reached.
        """
        try:
            async with self.opener(self.place, self._proxy()) as socket:
                await self._signed_in(socket, aiter(socket))
                await socket.close()
        except _Refused as error:
            raise WecomError(str(error), code="REFUSED") from None
        except _Passing as error:
            raise WecomError(f"WeCom could not be asked: {error}", code="UNREACHABLE") from None
        except WecomError:
            raise
        except Exception as error:
            raise WecomError(
                f"WeCom could not be reached: {self._said(error)}", code="UNREACHABLE"
            ) from None

    async def call(
        self, cmd: str, body: Mapping[str, Any], req_id: str | None = None
    ) -> dict[str, Any]:
        """Send a frame, and return the frame with which WeCom answered it.

        Args:
            cmd: The command.
            body: What it holds.
            req_id: The ``req_id`` of the message that it answers. Without one, it has one of its
                own.

        Raises:
            WecomError: The bot is not connected now, the connection was lost before WeCom
                answered, WeCom did not answer in time, or it refused the frame. Then the code
                is ``REFUSED`` and the number the errcode.
        """
        named = req_id or request_id(cmd)
        line = self._lines.setdefault(named, _Line())
        line.users += 1
        try:
            async with line.lock:
                return await self._answered(cmd, named, body)
        finally:
            line.users -= 1
            if line.users == 0:
                del self._lines[named]

    async def _answered(self, cmd: str, req_id: str, body: Mapping[str, Any]) -> dict[str, Any]:
        socket = self._socket
        if socket is None:
            raise WecomError("the bot is not connected to WeCom now", code="UNREACHABLE")
        answer: asyncio.Future[dict[str, Any]] = asyncio.get_running_loop().create_future()
        # An answer that ends where nobody waits for it, as where sending failed, is no error.
        answer.add_done_callback(_heeded)
        self._answers[req_id] = answer
        try:
            await socket.send(_frame(cmd, req_id, body))
            async with asyncio.timeout(_ANSWER_SECONDS):
                frame = await answer
        except TimeoutError:
            raise WecomError("WeCom did not answer in time", code="UNREACHABLE") from None
        except WecomError:
            raise
        except Exception as error:
            raise WecomError(
                f"a frame could not be sent to WeCom: {self._said(error)}", code="UNREACHABLE"
            ) from None
        finally:
            if self._answers.get(req_id) is answer:
                del self._answers[req_id]
        number = frame.get("errcode")
        if number != 0:
            message = self._said(str(frame.get("errmsg") or ""))
            raise WecomError(
                f"WeCom refused it: {message or 'it gave no reason'} (errcode {number})",
                code="REFUSED",
                number=number if isinstance(number, int) else None,
            )
        return frame

    def _said(self, error: BaseException | str) -> str:
        """Say what went wrong, with the Secret taken out wherever it is spelled."""
        text = error if isinstance(error, str) else str(error) or type(error).__name__
        for spelling in spellings(self.secret):
            text = text.replace(spelling, "[Secret]")
        return text

    def _proxy(self) -> str | Literal[True] | None:
        """Return the proxy that the WebSocket goes through: as for HTTPS, where it is ``wss``."""
        if any(name.lower() in _PROXY_VARIABLES and value for name, value in self.env.items()):
            return True
        parts = urlsplit(self.place)
        scheme = {"wss": "https", "ws": "http"}.get(parts.scheme, parts.scheme)
        return proxy_for(parts._replace(scheme=scheme).geturl(), self.env)

    async def _signed_in(self, socket: Socket, messages: AsyncIterator[bytes | str]) -> None:
        """Sign the bot in on a connection that was opened, and wait until WeCom answers.

        Raises:
            WecomError: WeCom refuses the bot for good.
            _Refused: WeCom refused the sign-in for another reason.
            _Passing: WeCom did not answer in time, or the connection broke off.
        """
        asked = request_id(SUBSCRIBE)
        await socket.send(_frame(SUBSCRIBE, asked, {"bot_id": self.bot_id, "secret": self.secret}))
        try:
            async with asyncio.timeout(_SIGN_IN_SECONDS):
                async for message in messages:
                    frame = self._read(message)
                    if frame is not None and _req_id(frame) == asked:
                        break
                else:
                    raise _Passing("the connection was closed before the bot was signed in")
        except TimeoutError:
            raise _Passing("WeCom did not answer the sign-in in time") from None
        number = frame.get("errcode")
        if number == 0:
            return
        message = self._said(str(frame.get("errmsg") or ""))
        why = f"{message or 'it gave no reason'}, errcode {number}"
        if number == _WRONG:
            raise WecomError(
                f"WeCom does not take this Bot ID with this Secret ({why}). See that both are "
                "those that the page of the bot shows under the API in the long connection",
                code="REFUSED",
                number=number,
            )
        if number == _NOT_LONG:
            raise WecomError(
                f"the API of this bot is not set to the long connection ({why}). Set it so on the "
                "page of the bot in the admin console of WeCom",
                code="NOT_LONG",
                number=number,
            )
        raise _Refused(f"WeCom refused the sign-in: {why}")

    async def _kept(self, socket: Socket, messages: AsyncIterator[bytes | str]) -> None:
        """Keep a connection until it is closed: ping, and take what comes in.

        Raises:
            WecomError: Another program signed in with the bot.
            _Passing: The connection broke off, or WeCom stopped answering the pings.
        """
        self._socket, self._missed = socket, 0
        reading = asyncio.ensure_future(self._reading(messages))
        pinging = asyncio.ensure_future(self._pinging(socket))
        try:
            done, _ = await asyncio.wait({reading, pinging}, return_when=asyncio.FIRST_COMPLETED)
        except asyncio.CancelledError:
            # The assistant stops. WeCom is told that the connection ended as it was meant to.
            try:
                await socket.close()
            except Exception as error:
                logger.info("the connection to WeCom could not be closed: %s", self._said(error))
            raise
        finally:
            self._socket = None
            for task in (reading, pinging):
                task.cancel()
            await asyncio.gather(reading, pinging, return_exceptions=True)
            self._lost()
        for task in done:
            task.result()

    async def _reading(self, messages: AsyncIterator[bytes | str]) -> None:
        try:
            async for message in messages:
                frame = self._read(message)
                if frame is not None:
                    self._take(frame)
        except WecomError:
            raise
        except Exception as error:
            raise _Passing(f"the connection broke off: {self._said(error)}") from None

    async def _pinging(self, socket: Socket) -> None:
        while True:
            await self.ticking(_PING_SECONDS)
            if self._missed >= _MISSED_MOST:
                raise _Passing(f"WeCom did not answer {self._missed} pings in a row")
            self._missed += 1
            try:
                await socket.send(_frame(PING, request_id(PING)))
            except Exception as error:
                raise _Passing(f"a ping could not be sent: {self._said(error)}") from None

    def _read(self, message: bytes | str) -> dict[str, Any] | None:
        """Return a frame as the object of JSON that it is, or ``None`` for one that is no such."""
        text = message.decode("utf-8", "replace") if isinstance(message, bytes) else message
        try:
            frame: object = json.loads(_CONTROL.sub("", text))
        except ValueError:
            logger.warning("WeCom sent something that is no JSON")
            return None
        if not isinstance(frame, dict):
            logger.warning("WeCom sent something that is no object of JSON")
            return None
        return cast("dict[str, Any]", frame)

    def _take(self, frame: dict[str, Any]) -> None:
        """Take a frame that came in.

        Raises:
            WecomError: It says that another program signed in with the bot.
        """
        cmd = frame.get("cmd")
        if cmd in (MESSAGE, EVENT):
            event = _object(_object(frame.get("body")).get("event"))
            if cmd == EVENT and event.get("eventtype") == _TAKEN_OVER:
                raise WecomError(
                    "another program signed in with this bot, and WeCom closed the connection of "
                    "this one: a bot has one connection at a time",
                    code="TAKEN_OVER",
                )
            try:
                self.heard(frame)
            except Exception as error:
                logger.error("what WeCom sent could not be taken", exc_info=error)
            return
        req_id = _req_id(frame)
        if req_id is None:
            return
        if req_id.startswith(f"{PING}_"):
            if frame.get("errcode") == 0:
                self._missed = 0
            return
        answer = self._answers.pop(req_id, None)
        if answer is not None and not answer.done():
            answer.set_result(frame)

    def _lost(self) -> None:
        """Tell whoever waits for an answer that none will come on this connection."""
        answers, self._answers = self._answers, {}
        for answer in answers.values():
            # An answer that came was taken out of these before it was given.
            answer.set_exception(
                WecomError(
                    "the connection to WeCom was lost before WeCom answered", code="UNREACHABLE"
                )
            )
