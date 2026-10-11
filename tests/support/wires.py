"""The far end of a wire, in a test: what an interface would send, and what it would be sent.

A service process speaks to an interface over a pipe. A test of what it says
needs no pipe. It holds the far end here: it puts lines in, and looks at the
messages that came out, each as it was read from its line.
"""

import asyncio
import itertools
import json
from collections import Counter
from collections.abc import AsyncIterator, Callable
from typing import Any, cast

from columbina.apps.serve.rpc import on_the_wire

__all__ = ["Far"]

_END = object()


class Far:
    """The interface at the far end of a wire that is no pipe.

    Attributes:
        heard: Every message that the service process sent, in the order in
            which it was sent.
        closed: Whether the service process has closed the wire.
        on_sent: Called with every message at the moment it is sent, if a
            test wants to do something at exactly that moment.
    """

    def __init__(self) -> None:
        self._lines: asyncio.Queue[object] = asyncio.Queue()
        self._news = asyncio.Event()
        self._ids = itertools.count(1)
        # How many requests went out under each id.
        self._requests: Counter[object] = Counter()
        self.heard: list[dict[str, Any]] = []
        self.closed = False
        self.on_sent: Callable[[dict[str, Any]], None] | None = None

    # The wire, as the service process holds it

    async def lines(self) -> AsyncIterator[bytes | None]:
        while True:
            line = await self._lines.get()
            if line is _END:
                return
            assert line is None or isinstance(line, bytes)
            yield line

    def send(self, line: str) -> None:
        # What goes over a real wire goes through here too: a line that could not be
        # written, or that is more than one line, fails the test that had it sent.
        sent = on_the_wire(line)
        assert sent.count(b"\n") == 1
        message = json.loads(sent)
        self.heard.append(message)
        self._news.set()
        if self.on_sent is not None:
            self.on_sent(message)

    async def close(self) -> None:
        self.closed = True

    # The interface, as a test plays it

    def write(self, line: bytes | None) -> None:
        """Put a line on the wire as it is. ``None`` is a line that was too long to be taken."""
        self._count(line)
        self._lines.put_nowait(line)

    def _count(self, line: bytes | None) -> None:
        """Count a line as a request under its id, if it is one as far as that can be told."""
        if line is None:
            return
        try:
            read: object = json.loads(line)
        except (ValueError, RecursionError):
            return
        if isinstance(read, dict) and "method" in read:
            id = cast("dict[str, object]", read).get("id")
            if isinstance(id, (int, str)) and not isinstance(id, bool):
                self._requests[id] += 1

    def hang_up(self) -> None:
        """Close this end, as an interface does that goes away."""
        self._lines.put_nowait(_END)

    def say(self, message: dict[str, Any]) -> None:
        """Send a message."""
        self.write(json.dumps(message, ensure_ascii=False).encode())

    def ask(self, method: str, params: dict[str, Any] | None = None) -> int:
        """Send a request, and return the id that its answer will name."""
        request: dict[str, Any] = {"jsonrpc": "2.0", "id": next(self._ids), "method": method}
        if params is not None:
            request["params"] = params
        self.say(request)
        return request["id"]

    async def until(self, holds: Callable[[], bool], seconds: float = 5.0) -> None:
        """Wait until something holds that a message of the service process brings about."""
        async with asyncio.timeout(seconds):
            while not holds():
                self._news.clear()
                await self._news.wait()

    def answers(self, id: int) -> list[dict[str, Any]]:
        """The answers that came to a request so far. There should never be two."""
        return [
            message for message in self.heard if "method" not in message and message["id"] == id
        ]

    def answered_too_often(self) -> list[object]:
        """The ids that more answers came to than requests went out under.

        A request is answered once, and an answer of this side is answered
        not at all. What is named here broke one of the two.
        """
        answers = Counter(
            message["id"]
            for message in self.heard
            if "method" not in message and message.get("id") is not None
        )
        return [id for id, times in answers.items() if times > self._requests[id]]

    async def answer(self, id: int) -> dict[str, Any]:
        """Wait for the answer to a request, and return the message."""
        await self.until(lambda: bool(self.answers(id)))
        return self.answers(id)[0]

    async def result(self, method: str, params: dict[str, Any] | None = None) -> Any:
        """Ask for something, and return what it gave."""
        answer = await self.answer(self.ask(method, params))
        assert "error" not in answer, answer
        return answer["result"]

    async def refusal(self, method: str, params: dict[str, Any] | None = None) -> tuple[int, str]:
        """Ask for something that is refused, and return the code and the sentence."""
        answer = await self.answer(self.ask(method, params))
        assert "result" not in answer, answer
        return answer["error"]["code"], answer["error"]["message"]

    def told(self, method: str) -> list[dict[str, Any]]:
        """What the notifications of one kind said so far."""
        return [
            message["params"]
            for message in self.heard
            if message.get("method") == method and "id" not in message
        ]

    def asked(self) -> list[dict[str, Any]]:
        """The requests that the service process made so far, as messages."""
        return [message for message in self.heard if "method" in message and "id" in message]

    def reply(self, id: int, result: Any) -> None:
        """Answer a request of the service process."""
        self.say({"jsonrpc": "2.0", "id": id, "result": result})

    def fail(self, id: int, code: int = -32603, message: str = "It broke.") -> None:
        """Answer a request of the service process with an error."""
        self.say({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
