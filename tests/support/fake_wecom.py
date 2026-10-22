"""WeCom for tests: the other end of the long connection of a bot, which a test plays.

A test puts in what WeCom sends, and reads what the bot sent. A socket that answers answers each
frame of the bot as WeCom does, with its ``req_id`` and an errcode: 0, or the one that the test
names for a command. A file that is sent to the bot is encrypted here as WeCom encrypts it.
"""

import asyncio
import base64
import json
import threading
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable, Mapping
from contextlib import asynccontextmanager
from typing import Any, Literal

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

from columbina.wecom.link import Socket

__all__ = [
    "BOT_ID",
    "KEY",
    "OWNER",
    "PLACE",
    "SECRET",
    "STRANGER",
    "FakeSocket",
    "Opened",
    "answered",
    "encrypted",
    "event",
    "message",
    "serving",
]

PLACE = "wss://openws.work.weixin.qq.com"
BOT_ID = "aibTestBot0123"
SECRET = "wecom-test-secret-0123456789abcdef"
OWNER = "zhangsan"
STRANGER = "lisi"
KEY = base64.b64encode(bytes(range(32))).decode().rstrip("=")
"""The key of a file that is sent, as WeCom gives it: Base64, here without its padding."""


class FakeSocket:
    """A WebSocket whose incoming frames a test puts in, and whose sent frames it reads.

    Args:
        answering: Whether every frame of the bot is answered as it comes.
        refusing: The errcode with which frames of a command are answered, where not 0.

    Attributes:
        queued: The errcodes with which the next frames of a command are answered, one each,
            before those of ``refusing``.
    """

    def __init__(
        self, *, answering: bool = True, refusing: Mapping[str, int] | None = None
    ) -> None:
        self.incoming: asyncio.Queue[str | BaseException | None] = asyncio.Queue()
        self.sent: list[dict[str, Any]] = []
        self.sent_one = asyncio.Event()
        self.answering = answering
        self.refusing = dict(refusing or {})
        self.queued: dict[str, list[int]] = {}
        self.failing: BaseException | None = None
        self.closing: BaseException | None = None
        self.closed = 0

    async def send(self, message: str) -> None:
        if self.failing is not None:
            raise self.failing
        frame: dict[str, Any] = json.loads(message)
        self.sent.append(frame)
        self.sent_one.set()
        if self.answering:
            waiting = self.queued.get(frame["cmd"])
            errcode = waiting.pop(0) if waiting else self.refusing.get(frame["cmd"], 0)
            self.answer(frame, errcode)

    def answer(self, frame: Mapping[str, Any], errcode: int = 0, errmsg: str = "ok") -> None:
        """Answer a frame of the bot as WeCom does: with its ``req_id``, and no command."""
        req_id = frame["headers"]["req_id"]
        self.tell({"headers": {"req_id": req_id}, "errcode": errcode, "errmsg": errmsg})

    def tell(self, frame: Mapping[str, Any]) -> None:
        """Send the bot a frame."""
        self.incoming.put_nowait(json.dumps(frame, ensure_ascii=False))

    async def close(self) -> None:
        self.closed += 1
        if self.closing is not None:
            raise self.closing

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

    def of(self, cmd: str) -> list[dict[str, Any]]:
        """The frames of a command that the bot sent."""
        return [frame for frame in self.sent if frame.get("cmd") == cmd]


class Opened:
    """Opens the sockets of a test, one after another, and notes how each was asked for.

    The last one is opened again where the bot asks for more than there are.
    """

    def __init__(self, *outcomes: FakeSocket | BaseException) -> None:
        self.outcomes = list(outcomes)
        self.asked: list[tuple[str, str | Literal[True] | None]] = []

    def __call__(self, url: str, proxy: str | Literal[True] | None) -> Any:
        self.asked.append((url, proxy))
        outcome = self.outcomes.pop(0) if len(self.outcomes) > 1 else self.outcomes[0]

        @asynccontextmanager
        async def opened() -> AsyncGenerator[Socket]:
            if isinstance(outcome, BaseException):
                raise outcome
            yield outcome

        return opened()


def message(
    text: str | None = None,
    *,
    message_id: str,
    sender: str = OWNER,
    chattype: str = "single",
    kind: str = "text",
    content: Mapping[str, Any] | None = None,
    **more: Any,
) -> dict[str, Any]:
    """A frame that brings a message to the bot, answered under ``req-`` and its id."""
    body: dict[str, Any] = {
        "msgid": message_id,
        "aibotid": BOT_ID,
        "chattype": chattype,
        "from": {"userid": sender},
        "msgtype": kind,
        kind: dict(content) if content is not None else {"content": text or ""},
        **more,
    }
    return {"cmd": "aibot_msg_callback", "headers": {"req_id": f"req-{message_id}"}, "body": body}


def event(kind: str, *, req_id: str = "req-event", sender: str = OWNER) -> dict[str, Any]:
    """A frame that brings an event to the bot."""
    body = {
        "msgid": f"event-{kind}",
        "aibotid": BOT_ID,
        "chattype": "single",
        "from": {"userid": sender},
        "msgtype": "event",
        "event": {"eventtype": kind},
    }
    return {"cmd": "aibot_event_callback", "headers": {"req_id": req_id}, "body": body}


def encrypted(content: bytes, key: str = KEY) -> bytes:
    """Encrypt a file as WeCom does: AES-256 in CBC, filled up to 32 bytes as PKCS #7 does."""
    raw = base64.b64decode(key + "=" * (-len(key) % 4))
    filled = 32 - len(content) % 32
    sealed = Cipher(algorithms.AES(raw), modes.CBC(raw[:16])).encryptor()
    return sealed.update(content + bytes([filled]) * filled) + sealed.finalize()


def serving(answer: Callable[[Any], Awaitable[None]]) -> tuple[str, Callable[[], None]]:
    """Serve WebSockets on this machine, on a loop of its own, and return where and a stop."""
    from websockets.asyncio.server import serve

    ready = threading.Event()
    stopping: list[Callable[[], None]] = []
    where: list[str] = []

    def run() -> None:
        async def main() -> None:
            loop = asyncio.get_running_loop()
            done = asyncio.Event()
            async with serve(answer, "127.0.0.1", 0) as server:
                port = next(iter(server.sockets)).getsockname()[1]
                where.append(f"ws://127.0.0.1:{port}")

                def stop() -> None:
                    loop.call_soon_threadsafe(done.set)

                stopping.append(stop)
                ready.set()
                await done.wait()

        # What is still pending is ended before the loop closes, and not in another test.
        with asyncio.Runner(loop_factory=asyncio.SelectorEventLoop) as runner:
            runner.run(main())

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    ready.wait(5)

    def stop() -> None:
        stopping[0]()
        thread.join(5)

    return where[0], stop


def answered(frame: dict[str, Any]) -> str:
    return json.dumps({"headers": {"req_id": frame["headers"]["req_id"]}, "errcode": 0})
