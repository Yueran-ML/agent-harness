"""Feishu for the tests: answers by method and path, and notes what it was asked.

A test says what each request gets, by its method and path, as one answer or a row of them,
the last of which is given again. A token is given for every request for one. What nothing
answers gets an error of Feishu, so that a test that forgets an answer fails loudly.
"""

import json
from collections.abc import Callable
from typing import Any

import httpx

from columbina.feishu.app import Host
from columbina.feishu.client import TOKEN, Feishu

__all__ = ["HOST", "SECRET", "FakeFeishu", "Pauses", "Reply", "done", "feishu_with", "refused"]

HOST = Host("feishu", "https://open.feishu.cn")
SECRET = "s3cr3t-0123456789abcdef"

Reply = httpx.Response | Callable[[httpx.Request], httpx.Response]


def done(data: object = None) -> httpx.Response:
    """An answer of Feishu that did what was asked, with this data."""
    body: dict[str, object] = {"code": 0, "msg": "success"}
    if data is not None:
        body["data"] = data
    return httpx.Response(200, json=body)


def refused(code: int, msg: str = "", status: int = 400) -> httpx.Response:
    """An answer of Feishu that did not do what was asked."""
    return httpx.Response(status, json={"code": code, "msg": msg})


class FakeFeishu:
    """The open platform of Feishu, as far as a test has it answer."""

    def __init__(self, otherwise: Callable[[httpx.Request], httpx.Response] | None = None) -> None:
        self.answers: dict[tuple[str, str], list[Reply]] = {}
        self.asked: list[httpx.Request] = []
        self.tokens = 0
        self.otherwise = otherwise

    def on(self, method: str, path: str, *replies: Reply) -> None:
        """Have requests to a path get these answers, one after another, the last again."""
        self.answers[(method, path)] = list(replies)

    def __call__(self, request: httpx.Request) -> httpx.Response:
        if request.url.path == TOKEN:
            self.tokens += 1
            return httpx.Response(
                200,
                json={"code": 0, "tenant_access_token": f"t-{self.tokens}", "expire": 7200},
            )
        self.asked.append(request)
        replies = self.answers.get((request.method, request.url.path))
        if not replies and self.otherwise is not None:
            return self.otherwise(request)
        if not replies:
            return refused(99999999, f"nothing answers {request.method} {request.url.path}")
        reply = replies.pop(0) if len(replies) > 1 else replies[0]
        return reply(request) if callable(reply) else reply

    def bodies(self, method: str, path: str) -> list[Any]:
        """The JSON bodies of the requests that went to a path."""
        return [
            json.loads(request.content)
            for request in self.asked
            if (request.method, request.url.path) == (method, path)
        ]

    def paths(self) -> list[str]:
        """The method and path of each request, in their order, without those for tokens."""
        return [f"{request.method} {request.url.path}" for request in self.asked]


class Pauses:
    """The pauses that a client was asked to make, which pass at once."""

    def __init__(self) -> None:
        self.taken: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.taken.append(seconds)


def feishu_with(fake: FakeFeishu, pauses: Pauses | None = None) -> Feishu:
    """A client of the REST API that asks the fake, and whose pauses pass at once."""
    return Feishu(
        HOST,
        "cli_a1b2c3",
        SECRET,
        {},
        transport=httpx.MockTransport(fake),
        sleep=pauses or Pauses(),
    )
