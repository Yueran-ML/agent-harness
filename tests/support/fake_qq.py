"""QQ for tests: the gateway and the API of a bot, which a test plays.

The socket greets the bot as the gateway of QQ does, takes its sign-in, says that it is ready,
and answers its heartbeats. A test puts in what QQ sends, and reads what the bot sent. The API
hands out tokens for the AppID and the AppSecret of the tests, gives the address of the gateway,
and keeps the answers that the bot sends.
"""

import asyncio
import json
from collections.abc import AsyncGenerator, AsyncIterator, Mapping
from contextlib import asynccontextmanager
from typing import Any

import httpx

from columbina.qq.link import GROUP, PRIVATE, Socket

__all__ = [
    "APP_ID",
    "GATEWAY",
    "OWNER",
    "SECRET",
    "STRANGER",
    "TOKEN",
    "FakeApi",
    "FakeSocket",
    "Opened",
    "in_group",
    "private",
]

APP_ID = "102030405"
SECRET = "qq-test-secret-0123456789abcdef"
TOKEN = "qq-token-abcdef0123456789"
GATEWAY = "wss://api.sgroup.qq.com/websocket"
OWNER = "OWNER0123456789ABCDEF"
STRANGER = "MEMBER0123456789ABCDEF"


class FakeSocket:
    """The gateway of QQ, for one connection.

    Args:
        ready: Whether the sign-in is answered with READY, or the session that was taken up
            with RESUMED.
        interval: The time between two heartbeats that QQ asks for, in milliseconds.
    """

    def __init__(self, *, ready: bool = True, interval: int = 41250) -> None:
        self.incoming: asyncio.Queue[str | BaseException | None] = asyncio.Queue()
        self.sent: list[dict[str, Any]] = []
        self.ready = ready
        self.closed = 0
        self.number = 0
        self.tell({"op": 10, "d": {"heartbeat_interval": interval}})

    async def send(self, message: str) -> None:
        frame: dict[str, Any] = json.loads(message)
        self.sent.append(frame)
        op = frame.get("op")
        if op == 2 and self.ready:
            self.dispatch("READY", {"session_id": "session-1", "version": 1})
        elif op == 6 and self.ready:
            self.dispatch("RESUMED", {})
        elif op == 1:
            self.tell({"op": 11})

    def tell(self, frame: Mapping[str, Any]) -> None:
        """Send the bot a frame."""
        self.incoming.put_nowait(json.dumps(frame, ensure_ascii=False))

    def dispatch(self, kind: str, data: Mapping[str, Any], event_id: str | None = None) -> None:
        """Send the bot an event, with the next number."""
        self.number += 1
        frame: dict[str, Any] = {"op": 0, "s": self.number, "t": kind, "d": dict(data)}
        if event_id is not None:
            frame["id"] = event_id
        self.tell(frame)

    def end(self, error: BaseException | None = None) -> None:
        """Close the connection from the side of QQ, with an error or as it was meant to."""
        self.incoming.put_nowait(error)

    async def close(self) -> None:
        self.closed += 1

    def __aiter__(self) -> AsyncIterator[str]:
        return self._messages()

    async def _messages(self) -> AsyncIterator[str]:
        while True:
            message = await self.incoming.get()
            if message is None:
                return
            if isinstance(message, BaseException):
                raise message
            yield message

    def of(self, op: int) -> list[dict[str, Any]]:
        """The frames of an opcode that the bot sent."""
        return [frame for frame in self.sent if frame.get("op") == op]


class Opened:
    """Opens the sockets of a test, one after another, and notes the addresses.

    The last one is opened again where the bot asks for more than there are.
    """

    def __init__(self, *outcomes: FakeSocket | BaseException) -> None:
        self.outcomes: list[FakeSocket | BaseException] = list(outcomes)
        self.asked: list[str] = []

    def __call__(self, url: str) -> Any:
        self.asked.append(url)
        outcome = self.outcomes.pop(0) if len(self.outcomes) > 1 else self.outcomes[0]

        @asynccontextmanager
        async def opened() -> AsyncGenerator[Socket]:
            if isinstance(outcome, BaseException):
                raise outcome
            yield outcome

        return opened()


class FakeApi:
    """The HTTP side of QQ: tokens, the gateway, and the answers of the bot.

    Attributes:
        answers: What the bot answered: the path, and the body of each answer.
        refusing: Answers with this status and body to every request for an answer, where set.
        lasts: How long a token holds, in seconds.
    """

    def __init__(self) -> None:
        self.answers: list[tuple[str, dict[str, Any]]] = []
        self.tokens = 0
        self.refusing: tuple[int, dict[str, Any]] | None = None
        self.lasts = 7200
        self.requests: list[httpx.Request] = []
        self.transport = httpx.MockTransport(self._handle)

    def _handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if request.url.path == "/app/getAppAccessToken":
            body = json.loads(request.content)
            if body != {"appId": APP_ID, "clientSecret": SECRET}:
                return httpx.Response(200, json={"code": 100016, "message": "invalid appid"})
            self.tokens += 1
            return httpx.Response(200, json={"access_token": TOKEN, "expires_in": str(self.lasts)})
        if request.headers.get("Authorization") != f"QQBot {TOKEN}":
            return httpx.Response(401, json={"code": 11241, "message": "token invalid"})
        if request.url.path == "/gateway":
            return httpx.Response(200, json={"url": GATEWAY})
        if self.refusing is not None:
            return httpx.Response(self.refusing[0], json=self.refusing[1])
        self.answers.append((request.url.path, json.loads(request.content)))
        return httpx.Response(200, json={"id": f"answer-{len(self.answers)}"})

    def said(self) -> list[tuple[str, str, int]]:
        """What the bot answered: to which message, what, and as which answer to it."""
        return [(body["msg_id"], body["content"], body["msg_seq"]) for _, body in self.answers]


def private(text: str, message_id: str, sender: str = OWNER) -> tuple[str, dict[str, Any]]:
    """An event of a message to the bot in a chat of two."""
    return PRIVATE, {
        "id": message_id,
        "author": {"user_openid": sender},
        "content": text,
        "timestamp": "2026-10-24T20:00:00+08:00",
    }


def in_group(
    text: str, message_id: str, *, group: str = "GROUP01", sender: str = STRANGER
) -> tuple[str, dict[str, Any]]:
    """An event of a message in a group that mentions the bot."""
    return GROUP, {
        "id": message_id,
        "group_openid": group,
        "author": {"member_openid": sender},
        "content": f" {text}",
        "timestamp": "2026-10-24T20:00:00+08:00",
    }
