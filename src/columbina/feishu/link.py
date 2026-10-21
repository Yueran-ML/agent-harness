"""The long connection to Feishu: how the assistant hears what is written to its bot.

Feishu tells an app of an event in one of two ways. One sends it to an address of the app on
the internet, which a program on a person's machine does not have. The other is this one: the
app asks Feishu for the address of a connection, with its App ID and App Secret, opens a
WebSocket there, and is sent its events over it. Nothing on this machine is reached from
outside.

The connection is kept going from both ends. This side sends a ping now and then, and Feishu
answers with a pong, in which it may say how often to ping and how to come back after the
connection was lost. An event comes in a frame of data, or in several where it is large, and
each frame of an event is answered with itself and a payload that says it was taken. An
event that is not answered is sent again later.

A connection that is lost is opened again, at the pace that Feishu says: after a pause drawn
at random the first time, so that apps which lost theirs at once do not all come back at once,
and after a fixed pause each time a try fails. A connection that Feishu refuses for a reason
that waiting does not change, such as an App Secret that is wrong, is not tried again: the
error says why.

Each of the two platforms gives a token to an app of the other as well, but the address of a
connection only to an app of its own. So the address is where an app of Lark that was set up
as one of Feishu, or the other way round, is found: before it is kept, and when it runs.

What is written about the connection names the host it goes to, and never the address: the
address that Feishu gives holds what lets the connection in.
"""

import asyncio
import json
import logging
import random
import time
from collections.abc import AsyncIterator, Callable, Mapping
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass, field, replace
from http import HTTPStatus
from typing import Any, Literal, Protocol, cast
from urllib.parse import parse_qs, urlsplit

import httpx

from columbina.feishu.app import Domain, FeishuError, Host
from columbina.feishu.frames import CONTROL, DATA, Frame, FrameError, decoded, encoded
from columbina.llm.credentials import spellings
from columbina.net import proxy_for

__all__ = ["ENDPOINT", "Link", "Opener", "Rhythm", "Socket", "platform_checked"]

logger = logging.getLogger(__name__)

ENDPOINT = "/callback/ws/endpoint"
"""Where the address of a connection is asked for, on the host of the API."""

_BUSY = (1, 1000040343)
"""The codes with which Feishu says that it cannot give an address now, and may later."""

_ELSEWHERE = 1000040351
"""The code with which a platform says that the app is not one of its own, with the words
"Incorrect domain name": an app of Lark asked about at Feishu, or one of Feishu at Lark."""

_NAMES: dict[Domain, str] = {"feishu": "Feishu", "lark": "Lark"}

_FORBIDDEN = "403"
_AUTH_FAILED = "514"
_TOO_MANY = "1000040350"
"""What Feishu says in the headers of a refused connection: that it is forbidden, that the
sign-in failed, and, of the latter, that the app has as many connections as it may."""

_LARGEST = 16 * 1024 * 1024
"""How large a message on the connection may be, in bytes."""

_PIECES_LAST = 5.0
"""For how many seconds the pieces of an event are kept while the others are waited for."""

_TAKEN = json.dumps({"code": 200}).encode()
_FAILED = json.dumps({"code": 500}).encode()

_PROXY_VARIABLES = ("https_proxy", "http_proxy", "all_proxy")


class Socket(Protocol):
    """The side of a WebSocket that this module uses."""

    async def send(self, message: bytes) -> None:
        """Send a message of bytes."""
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


_QUIET = logging.getLogger("columbina.feishu.websockets")
"""What the library of WebSockets writes about a connection. What it writes below warnings
names the address, so that is not written, whatever the rest of the log writes."""
_QUIET.setLevel(logging.WARNING)


def _opened(url: str, proxy: str | Literal[True] | None) -> AbstractAsyncContextManager[Socket]:
    """Open a WebSocket with the library that comes with ``columbina[feishu]``."""
    # Only an assistant in Feishu needs it, and it comes with columbina[feishu].
    from websockets.asyncio.client import connect

    return cast(
        "AbstractAsyncContextManager[Socket]",
        connect(url, proxy=proxy, max_size=_LARGEST, open_timeout=20, logger=_QUIET),
    )


def _seconds(value: object) -> float | None:
    """Return a number of seconds that Feishu gave, or ``None`` where it gave none that holds."""
    if isinstance(value, bool) or not isinstance(value, int | float) or value < 0:
        return None
    return float(value)


@dataclass(slots=True)
class Rhythm:
    """How the connection is kept, as Feishu says.

    Attributes:
        ping: How many seconds go between two pings.
        nonce: The most seconds of the pause before the connection is opened again, the first
            time after it was lost or could not be opened.
        interval: The seconds of each pause after that.
        attempts: How many tries in a row may fail, or -1 for any number.
    """

    ping: float = 120.0
    nonce: float = 30.0
    interval: float = 120.0
    attempts: int = -1

    def take(self, said: object) -> None:
        """Take what Feishu said of the rhythm, as far as it said something that holds."""
        if not isinstance(said, dict):
            return
        given = cast("dict[str, object]", said)
        ping = _seconds(given.get("PingInterval"))
        nonce = _seconds(given.get("ReconnectNonce"))
        interval = _seconds(given.get("ReconnectInterval"))
        attempts = given.get("ReconnectCount")
        if ping is not None and ping > 0:
            self.ping = ping
        if nonce is not None:
            self.nonce = nonce
        if interval is not None:
            self.interval = interval
        if isinstance(attempts, int) and not isinstance(attempts, bool) and attempts >= -1:
            self.attempts = attempts


class _Passing(Exception):  # noqa: N818 - a failure that may pass, and no error of a caller
    """The connection could not be opened, or was lost. Another try may do."""


@dataclass(frozen=True, slots=True)
class _Address:
    url: str
    service: int


@dataclass(slots=True)
class _Pieces:
    parts: list[bytes | None]
    until: float


def _refusal(error: BaseException, said: Callable[[str], str]) -> FeishuError | None:
    """Return why Feishu refused to open a connection for good, or ``None`` if it may yet.

    Feishu says why in headers of the answer to the request that opens a WebSocket. A
    connection that is forbidden, or one more than the app may have, is refused for good.

    Args:
        error: What opening the WebSocket raised.
        said: Takes out of a text what is not to be said.
    """
    response = getattr(error, "response", None)
    headers = getattr(response, "headers", None)
    if headers is None:
        return None
    named = cast("Mapping[str, str]", headers)
    status, message = named.get("handshake-status"), named.get("handshake-msg") or ""
    if status == _FORBIDDEN or (
        status == _AUTH_FAILED and named.get("handshake-autherrcode") == _TOO_MANY
    ):
        return FeishuError(
            f"Feishu refuses the connection: {said(message) or status}. Another program may hold "
            "the connection of this app already",
            code="REFUSED",
        )
    return None


@dataclass(slots=True)
class Link:
    """The long connection of one app.

    Attributes:
        host: Where the app is.
        app_id: Its App ID.
        secret: Its App Secret.
        http: The client that the address of a connection is asked for with.
        heard: Is given each event, as the object of JSON that Feishu sent. It is called on
            the loop of the connection and should wait for nothing: the event is answered when
            it returns, and Feishu waits for that for a few seconds only.
        told: Is told how the connection stands, ``"connecting"`` or ``"connected"``, and why
            the last try failed or the connection was lost, until it is open again.
        env: The environment, which may name a proxy.
        opener: Opens a WebSocket.
        sleep: Waits, for the pauses.
        draw: Draws a number from 0 to 1, for the first pause.
        clock: Tells the time in seconds, for the pieces of an event and how long one took.
        rhythm: How the connection is kept. Feishu says more of it as it goes.
    """

    host: Host
    app_id: str
    secret: str = field(repr=False)
    http: httpx.AsyncClient
    heard: Callable[[dict[str, Any]], None]
    told: Callable[[str, str | None], None]
    env: Mapping[str, str] = field(default_factory=dict[str, str])
    opener: Opener = _opened
    sleep: Callable[[float], Any] = asyncio.sleep
    draw: Callable[[], float] = random.random
    clock: Callable[[], float] = time.monotonic
    rhythm: Rhythm = field(default_factory=Rhythm)
    _pieces: dict[str, _Pieces] = field(default_factory=dict[str, _Pieces])

    async def run(self) -> None:
        """Keep the connection open until this is cancelled.

        Raises:
            FeishuError: Feishu refuses the connection for a reason that waiting does not
                change, or could not be reached as many times in a row as it allows.
        """
        failed = 0
        self.told("connecting", None)
        while True:
            try:
                address = await self._address()
                async with self.opener(address.url, self._proxy(address.url)) as socket:
                    failed = 0
                    logger.info("connected to Feishu at %s", urlsplit(address.url).hostname)
                    self.told("connected", None)
                    await self._kept(socket, address.service)
                why = "the connection was closed"
            except _Passing as error:
                why = str(error)
            except FeishuError:
                raise
            except Exception as error:
                refused = _refusal(error, self._said)
                if refused is not None:
                    raise refused from None
                why = self._said(error)
            failed += 1
            logger.warning("the connection to Feishu is not open: %s", why)
            self.told("connecting", why)
            if 0 <= self.rhythm.attempts < failed:
                raise FeishuError(
                    f"Feishu could not be reached {failed} times in a row: {why}",
                    code="UNREACHABLE",
                )
            first = failed == 1
            await self.sleep(self.draw() * self.rhythm.nonce if first else self.rhythm.interval)

    def _said(self, error: BaseException | str) -> str:
        """Say what went wrong, with the App Secret taken out wherever it is spelled."""
        text = error if isinstance(error, str) else str(error) or type(error).__name__
        for spelling in spellings(self.secret):
            text = text.replace(spelling, "[App Secret]")
        return text

    def _proxy(self, url: str) -> str | Literal[True] | None:
        """Return the proxy that the WebSocket goes through: as for HTTPS, where it is ``wss``."""
        if any(name.lower() in _PROXY_VARIABLES and value for name, value in self.env.items()):
            return True
        parts = urlsplit(url)
        scheme = {"wss": "https", "ws": "http"}.get(parts.scheme, parts.scheme)
        return proxy_for(parts._replace(scheme=scheme).geturl(), self.env)

    async def _address(self) -> _Address:
        """Ask Feishu for the address of a connection.

        Raises:
            _Passing: Feishu could not be asked, or cannot give one now.
            FeishuError: Feishu gives none to this app.
        """
        try:
            response = await self.http.post(
                f"{self.host.api}{ENDPOINT}",
                json={"AppID": self.app_id, "AppSecret": self.secret},
                headers={"locale": "zh"},
            )
        except httpx.TimeoutException:
            raise _Passing("Feishu did not answer in time") from None
        except httpx.RequestError as error:
            raise _Passing(f"Feishu could not be reached: {self._said(error)}") from None
        if response.status_code != HTTPStatus.OK:
            raise _Passing(f"Feishu answered with HTTP {response.status_code}")
        try:
            said: object = response.json()
        except ValueError:
            said = None
        if not isinstance(said, dict):
            raise _Passing("Feishu answered with something that is no object of JSON")
        given = cast("dict[str, object]", said)
        code, message = given.get("code"), self._said(str(given.get("msg") or ""))
        if code in _BUSY:
            raise _Passing(f"Feishu is busy: {message or code}")
        if code == _ELSEWHERE:
            asked = _NAMES[self.host.domain]
            other = _NAMES["lark" if self.host.domain == "feishu" else "feishu"]
            why = f"{message}, code {code}" if message else f"code {code}"
            raise FeishuError(
                f"{asked} does not know this app, which is one of {other} ({why}). Set it up as "
                f"an app of {other}",
                code="WRONG_PLATFORM",
            )
        if code != 0:
            raise FeishuError(
                f"Feishu opens no connection for this app: {message or 'it gave no reason'} "
                f"(code {code}). See that the App ID and the App Secret are those of the app",
                code="REFUSED",
            )
        data = given.get("data")
        found = cast("dict[str, object]", data) if isinstance(data, dict) else {}
        url = found.get("URL")
        if not isinstance(url, str) or urlsplit(url).scheme not in ("ws", "wss"):
            raise _Passing("Feishu gave no address for the connection")
        self.rhythm.take(found.get("ClientConfig"))
        service = parse_qs(urlsplit(url).query).get("service_id", [""])[0]
        return _Address(url, int(service) if service.isdigit() else 0)

    async def _kept(self, socket: Socket, service: int) -> None:
        """Keep a connection until it is closed: ping, and answer what comes in.

        Raises:
            _Passing: The connection broke off.
        """
        pinging = asyncio.ensure_future(self._pinging(socket, service))
        try:
            async for message in socket:
                if isinstance(message, bytes):
                    await self._take(socket, message)
        except asyncio.CancelledError:
            # The assistant stops. Feishu is told that the connection ended as it was meant to,
            # and not that something went wrong, which is what the library says otherwise.
            try:
                await socket.close()
            except Exception as error:
                logger.info("the connection to Feishu could not be closed: %s", self._said(error))
            raise
        except Exception as error:
            raise _Passing(f"the connection broke off: {self._said(error)}") from None
        finally:
            pinging.cancel()
            await asyncio.gather(pinging, return_exceptions=True)

    async def _pinging(self, socket: Socket, service: int) -> None:
        ping = encoded(Frame(service=service, method=CONTROL, headers=(("type", "ping"),)))
        while True:
            try:
                await socket.send(ping)
            except Exception as error:
                logger.warning("a ping to Feishu could not be sent: %s", self._said(error))
            await self.sleep(self.rhythm.ping)

    async def _take(self, socket: Socket, message: bytes) -> None:
        """Take a frame that came in, and answer it where it is one of an event."""
        try:
            frame = decoded(message)
        except FrameError as error:
            logger.warning("Feishu sent something that is no frame: %s", error)
            return
        kind = frame.header("type")
        if frame.method == CONTROL:
            if kind == "pong" and frame.payload:
                try:
                    self.rhythm.take(json.loads(frame.payload))
                except ValueError:
                    logger.warning("a pong of Feishu holds no JSON")
            return
        if frame.method != DATA or kind != "event":
            # A card that was clicked, or something newer than this program, is not answered.
            return
        payload = self._whole(frame)
        if payload is None:
            return
        began = self.clock()
        answer = _TAKEN
        try:
            event: object = json.loads(payload)
            if not isinstance(event, dict):
                raise ValueError("the event is no object of JSON")
            self.heard(cast("dict[str, Any]", event))
        except Exception as error:
            logger.error("an event of Feishu could not be taken", exc_info=error)
            answer = _FAILED
        taken = round((self.clock() - began) * 1000)
        reply = replace(frame.with_header("biz_rt", str(taken)), payload=answer)
        try:
            await socket.send(encoded(reply))
        except Exception as error:
            logger.warning("an event of Feishu could not be answered: %s", self._said(error))

    def _whole(self, frame: Frame) -> bytes | None:
        """Return the payload of an event, or ``None`` while pieces of it are missing."""
        payload = frame.payload or b""
        count, place, key = frame.header("sum"), frame.header("seq"), frame.header("message_id")
        if count is None or not count.isdigit() or int(count) <= 1:
            return payload
        if key is None or place is None or not place.isdigit() or int(place) >= int(count):
            logger.warning("a piece of an event of Feishu does not say where it goes")
            return None
        now = self.clock()
        for gone in [name for name, held in self._pieces.items() if held.until < now]:
            del self._pieces[gone]
        held = self._pieces.get(key)
        if held is None or len(held.parts) != int(count):
            held = self._pieces[key] = _Pieces([None] * int(count), now)
        held.parts[int(place)] = payload
        held.until = now + _PIECES_LAST
        if any(part is None for part in held.parts):
            return None
        del self._pieces[key]
        return b"".join(part for part in held.parts if part is not None)


async def platform_checked(host: Host, app_id: str, secret: str, http: httpx.AsyncClient) -> None:
    """Say whether an app is one of the other platform than the one that it is to be kept in.

    The platform is asked for the address of a connection, which it gives only to an app of its
    own. Nothing else is said here: an address that cannot be had now, or that is refused for
    another reason, is said when the assistant starts.

    Raises:
        FeishuError: The app is one of the other platform. Its code is ``WRONG_PLATFORM``.
    """
    link = Link(host, app_id, secret, http, heard=lambda event: None, told=lambda state, why: None)
    try:
        # The address is asked for as the assistant asks for it, and is not used.
        await link._address()  # pyright: ignore[reportPrivateUsage]
    except _Passing:
        return
    except FeishuError as error:
        if error.code == "WRONG_PLATFORM":
            raise
