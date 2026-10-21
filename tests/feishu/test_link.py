"""The long connection to Feishu: ``columbina.feishu.link``."""

import asyncio
import json
import logging
import threading
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable, Mapping
from contextlib import asynccontextmanager
from typing import Any, Literal

import httpx
import pytest

from columbina.feishu import link as link_module
from columbina.feishu.app import FeishuError, Host
from columbina.feishu.frames import CONTROL, DATA, Frame, decoded, encoded
from columbina.feishu.link import ENDPOINT, Link, Rhythm, Socket, platform_checked
from columbina.net import proxy_for

HOST = Host("feishu", "https://open.feishu.cn")
LARK = Host("lark", "https://open.larksuite.com")
SECRET = "s3cr3t-0123456789abcdef"
ADDRESS = "wss://msg-frontier.feishu.cn/ws/v2?device_id=d1&service_id=4321&access_key=k"
LONG = 3600.0
"""The pings of a test wait this long, which no test waits for."""


def endpoint(url: str = ADDRESS, code: int = 0, config: object = None, msg: str = "") -> object:
    data: dict[str, object] = {"URL": url}
    if config is not None:
        data["ClientConfig"] = config
    return {"code": code, "msg": msg, "data": data}


class Feishu:
    """The open platform, as far as a connection asks it: answers in a row, then the last again."""

    def __init__(self, *answers: Callable[[httpx.Request], httpx.Response]) -> None:
        self.answers = list(answers) or [lambda request: httpx.Response(200, json=endpoint())]
        self.asked: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.asked.append(request)
        answer = self.answers.pop(0) if len(self.answers) > 1 else self.answers[0]
        return answer(request)


class FakeSocket:
    """A WebSocket whose messages a test puts in, and whose sent messages it reads."""

    def __init__(self) -> None:
        self.incoming: asyncio.Queue[bytes | str | BaseException | None] = asyncio.Queue()
        self.sent: list[bytes] = []
        self.sent_one = asyncio.Event()
        self.failing: BaseException | None = None
        self.closed = 0
        self.closing: BaseException | None = None

    async def send(self, message: bytes) -> None:
        if self.failing is not None:
            raise self.failing
        self.sent.append(message)
        self.sent_one.set()

    async def close(self) -> None:
        self.closed += 1
        if self.closing is not None:
            raise self.closing

    def __aiter__(self) -> AsyncIterator[bytes | str]:
        return self._messages()

    async def _messages(self) -> AsyncIterator[bytes | str]:
        while True:
            message = await self.incoming.get()
            if message is None:
                return
            if isinstance(message, BaseException):
                raise message
            yield message

    def frames(self) -> list[Frame]:
        return [decoded(message) for message in self.sent]


class Opened:
    """Opens the sockets of a test, one after another, and notes how each was asked for."""

    def __init__(self, *outcomes: FakeSocket | BaseException) -> None:
        self.outcomes = list(outcomes)
        self.asked: list[tuple[str, str | Literal[True] | None]] = []

    def __call__(self, url: str, proxy: str | Literal[True] | None) -> Any:
        self.asked.append((url, proxy))
        outcome = self.outcomes.pop(0)

        @asynccontextmanager
        async def opened() -> AsyncGenerator[Socket]:
            if isinstance(outcome, BaseException):
                raise outcome
            yield outcome

        return opened()


class Pauses:
    """Waits as a connection asks: a ping waits for good, and every other pause not at all."""

    def __init__(self) -> None:
        self.taken: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.taken.append(seconds)
        if seconds == LONG:
            await asyncio.Event().wait()
        await asyncio.sleep(0)


class Clock:
    def __init__(self) -> None:
        self.now = 100.0

    def __call__(self) -> float:
        return self.now


class Run:
    """A link at work in a test: what it heard, what it was told, and its task."""

    def __init__(
        self,
        feishu: Feishu,
        opened: Opened,
        *,
        env: dict[str, str] | None = None,
        heard: Callable[[dict[str, Any]], None] | None = None,
        host: Host = HOST,
    ) -> None:
        self.heard: list[dict[str, Any]] = []
        self.told: list[tuple[str, str | None]] = []
        self.pauses = Pauses()
        self.clock = Clock()
        self.http = httpx.AsyncClient(transport=httpx.MockTransport(feishu))
        self.link = Link(
            host=host,
            app_id="cli_a1b2c3",
            secret=SECRET,
            http=self.http,
            heard=heard or self.heard.append,
            told=lambda state, why: self.told.append((state, why)),
            env=env or {},
            opener=opened,
            sleep=self.pauses,
            draw=lambda: 0.5,
            clock=self.clock,
            rhythm=Rhythm(ping=LONG),
        )
        self.task: asyncio.Task[None] | None = None

    def start(self) -> None:
        self.task = asyncio.ensure_future(self.link.run())

    async def stop(self) -> None:
        assert self.task is not None
        self.task.cancel()
        await asyncio.gather(self.task, return_exceptions=True)
        await self.http.aclose()


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    """Wait for something that the link does in its own time."""
    async with asyncio.timeout(seconds):
        # What is waited for is done by the link or a fake of a test, which set no event.
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.001)


def event_frame(payload: bytes, **headers: str) -> Frame:
    named = {"type": "event", "message_id": "m1", "sum": "1", "seq": "0", "trace_id": "t1"}
    named.update(headers)
    return Frame(
        seq_id=5, log_id=9, service=4321, method=DATA, headers=tuple(named.items()), payload=payload
    )


@pytest.fixture
def socket() -> FakeSocket:
    return FakeSocket()


async def connected(socket: FakeSocket, *, feishu: Feishu | None = None, **given: Any) -> Run:
    run = Run(feishu or Feishu(), Opened(socket), **given)
    run.start()
    await until(lambda: ("connected", None) in run.told)
    return run


# Opening the connection


async def test_the_address_is_asked_for_with_the_app_and_then_opened(
    socket: FakeSocket, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO, logger="columbina.feishu.link")
    feishu = Feishu()
    opened = Opened(socket)
    run = Run(feishu, opened)
    run.start()
    await until(lambda: ("connected", None) in run.told)

    asked = feishu.asked[0]
    assert (asked.method, str(asked.url)) == ("POST", f"https://open.feishu.cn{ENDPOINT}")
    assert json.loads(asked.content) == {"AppID": "cli_a1b2c3", "AppSecret": SECRET}
    assert asked.headers["locale"] == "zh"
    assert opened.asked == [(ADDRESS, None)]
    assert run.told == [("connecting", None), ("connected", None)]
    # The address holds what lets the connection in. Only its host is written down.
    assert "connected to Feishu at msg-frontier.feishu.cn" in caplog.text
    assert "access_key" not in caplog.text
    await run.stop()


async def test_a_ping_names_the_service_of_the_address(socket: FakeSocket) -> None:
    run = await connected(socket)
    await socket.sent_one.wait()

    assert socket.frames()[0] == Frame(service=4321, method=CONTROL, headers=(("type", "ping"),))
    assert run.pauses.taken == [LONG]
    await run.stop()


async def test_an_address_without_a_service_pings_service_zero(socket: FakeSocket) -> None:
    address = "wss://msg-frontier.feishu.cn/ws/v2?device_id=d1"
    feishu = Feishu(lambda request: httpx.Response(200, json=endpoint(address)))
    run = await connected(socket, feishu=feishu)
    await socket.sent_one.wait()

    assert socket.frames()[0].service == 0
    await run.stop()


async def test_the_rhythm_that_feishu_gives_with_the_address_is_taken(socket: FakeSocket) -> None:
    config = {
        "PingInterval": 90,
        "ReconnectNonce": 10,
        "ReconnectInterval": 60,
        "ReconnectCount": 5,
    }
    feishu = Feishu(lambda request: httpx.Response(200, json=endpoint(config=config)))
    run = await connected(socket, feishu=feishu)

    assert run.link.rhythm == Rhythm(ping=90.0, nonce=10.0, interval=60.0, attempts=5)
    await run.stop()


@pytest.mark.parametrize(
    ("said", "kept"),
    [
        ({"PingInterval": True, "ReconnectNonce": -1, "ReconnectInterval": "60"}, Rhythm()),
        ({"PingInterval": 0, "ReconnectCount": -2}, Rhythm()),
        ({"ReconnectCount": False}, Rhythm()),
        ([1, 2], Rhythm()),
        (None, Rhythm()),
        (
            {"ReconnectNonce": 0, "ReconnectInterval": 0.5, "ReconnectCount": -1},
            Rhythm(nonce=0.0, interval=0.5, attempts=-1),
        ),
    ],
)
def test_only_a_rhythm_that_holds_is_taken(said: object, kept: Rhythm) -> None:
    rhythm = Rhythm()

    rhythm.take(said)

    assert rhythm == kept


async def test_a_pong_says_how_often_to_ping(socket: FakeSocket) -> None:
    run = await connected(socket)
    pong = Frame(
        service=4321, method=CONTROL, headers=(("type", "pong"),), payload=b'{"PingInterval": 30}'
    )
    await socket.incoming.put(encoded(pong))
    await until(lambda: run.link.rhythm.ping == 30.0)
    await run.stop()


async def test_a_pong_without_a_rhythm_or_with_no_json_changes_nothing(
    socket: FakeSocket, caplog: pytest.LogCaptureFixture
) -> None:
    run = await connected(socket)
    for payload in (None, b"", b"not json"):
        await socket.incoming.put(
            encoded(Frame(method=CONTROL, headers=(("type", "pong"),), payload=payload))
        )
    await socket.incoming.put(encoded(Frame(method=CONTROL, headers=(("type", "ping"),))))
    await until(lambda: "a pong of Feishu holds no JSON" in caplog.text)

    assert run.link.rhythm.ping == LONG
    assert len(socket.sent) == 1
    await run.stop()


async def test_the_proxy_of_the_environment_is_left_to_the_library(socket: FakeSocket) -> None:
    opened = Opened(socket)
    run = Run(Feishu(), opened, env={"HTTPS_PROXY": "http://10.0.0.1:3128"})
    run.start()
    await until(lambda: bool(opened.asked))

    assert opened.asked == [(ADDRESS, True)]
    await run.stop()


CLASH = {"https": "http://127.0.0.1:7890", "http": "http://127.0.0.1:7891"}
"""What a tool such as Clash leaves in the settings of the system: no proxy for wss or ws."""


@pytest.mark.parametrize(
    ("address", "proxy"),
    [
        (ADDRESS, "http://127.0.0.1:7890"),
        ("ws://127.0.0.1:8123/ws?service_id=1", None),
        ("ws://msg.example/ws?service_id=1", "http://127.0.0.1:7891"),
    ],
)
async def test_a_websocket_goes_through_the_proxy_of_the_system_for_its_kind_of_http(
    socket: FakeSocket, monkeypatch: pytest.MonkeyPatch, address: str, proxy: str | None
) -> None:
    def system(url: str, env: Mapping[str, str]) -> str | None:
        return proxy_for(url, env, system=lambda: CLASH, bypassed=lambda host: False)

    monkeypatch.setattr(link_module, "proxy_for", system)
    opened = Opened(socket)
    feishu = Feishu(lambda request: httpx.Response(200, json=endpoint(address)))
    run = Run(feishu, opened, env={"HTTPS_PROXY": ""})
    run.start()
    await until(lambda: bool(opened.asked))

    assert opened.asked == [(address, proxy)]
    await run.stop()


# Events


async def test_an_event_is_heard_and_answered_with_its_frame(socket: FakeSocket) -> None:
    run = await connected(socket)
    frame = event_frame(b'{"header": {"event_type": "im.message.receive_v1"}}')

    await socket.incoming.put(encoded(frame))
    await until(lambda: len(socket.sent) == 2)

    assert run.heard == [{"header": {"event_type": "im.message.receive_v1"}}]
    answer = socket.frames()[1]
    assert answer == Frame(
        seq_id=5,
        log_id=9,
        service=4321,
        method=DATA,
        headers=(*frame.headers, ("biz_rt", "0")),
        payload=b'{"code": 200}',
    )
    await run.stop()


async def test_how_long_an_event_took_is_said_in_milliseconds(socket: FakeSocket) -> None:
    run: Run | None = None

    def slow(event: dict[str, Any]) -> None:
        assert run is not None
        run.clock.now += 0.25

    run = await connected(socket, heard=slow)
    await socket.incoming.put(encoded(event_frame(b"{}")))
    await until(lambda: len(socket.sent) == 2)

    assert socket.frames()[1].header("biz_rt") == "250"
    await run.stop()


@pytest.mark.parametrize("payload", [b"[1, 2]", b"not json"])
async def test_an_event_that_is_no_object_is_answered_as_failed(
    socket: FakeSocket, payload: bytes, caplog: pytest.LogCaptureFixture
) -> None:
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(payload)))
    await until(lambda: len(socket.sent) == 2)

    assert run.heard == []
    assert socket.frames()[1].payload == b'{"code": 500}'
    assert "an event of Feishu could not be taken" in caplog.text
    await run.stop()


async def test_an_event_that_cannot_be_taken_is_answered_as_failed(socket: FakeSocket) -> None:
    def failing(event: dict[str, Any]) -> None:
        raise RuntimeError("no room")

    run = await connected(socket, heard=failing)
    await socket.incoming.put(encoded(event_frame(b"{}")))
    await until(lambda: len(socket.sent) == 2)

    assert socket.frames()[1].payload == b'{"code": 500}'
    await run.stop()


async def test_an_event_in_pieces_is_heard_once_whole(socket: FakeSocket) -> None:
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(b'"b"}', sum="2", seq="1")))
    await socket.incoming.put(encoded(event_frame(b'{"a": ', sum="2", seq="0")))
    await until(lambda: len(run.heard) == 1)

    assert run.heard == [{"a": "b"}]
    # Only the piece that makes it whole is answered, as Feishu's own client does.
    assert len(socket.sent) == 2
    await run.stop()


async def test_the_pieces_of_an_event_are_kept_a_few_seconds(socket: FakeSocket) -> None:
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(b'{"a": ', sum="2", seq="0")))
    await until(lambda: "m1" in run.link._pieces)  # pyright: ignore[reportPrivateUsage]
    run.clock.now += 6.0
    await socket.incoming.put(encoded(event_frame(b'"x"}', sum="2", seq="1", message_id="m2")))
    await socket.incoming.put(encoded(event_frame(b'"b"}', sum="2", seq="1")))
    await socket.incoming.put(encoded(event_frame(b"{}")))
    await until(lambda: len(run.heard) == 1)

    assert run.heard == [{}]
    assert "m1" in run.link._pieces  # pyright: ignore[reportPrivateUsage]
    await run.stop()


async def test_pieces_that_say_another_count_begin_anew(socket: FakeSocket) -> None:
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(b"x", sum="3", seq="0")))
    await socket.incoming.put(encoded(event_frame(b'{"c": ', sum="2", seq="0")))
    await socket.incoming.put(encoded(event_frame(b"1}", sum="2", seq="1")))
    await until(lambda: len(run.heard) == 1)

    assert run.heard == [{"c": 1}]
    await run.stop()


@pytest.mark.parametrize(
    "headers",
    [{"seq": "2"}, {"seq": "first"}, {"message_id": None}, {"seq": None}],
    ids=["past the count", "no number", "no message", "no place"],
)
async def test_a_piece_that_does_not_say_where_it_goes_is_passed_over(
    socket: FakeSocket, headers: dict[str, str | None], caplog: pytest.LogCaptureFixture
) -> None:
    run = await connected(socket)
    named = {"type": "event", "message_id": "m1", "sum": "2", "seq": "0"}
    named.update({key: value for key, value in headers.items() if value is not None})
    for key in [key for key, value in headers.items() if value is None]:
        del named[key]
    piece = Frame(method=DATA, headers=tuple(named.items()), payload=b"{}")
    await socket.incoming.put(encoded(piece))
    await until(lambda: "does not say where it goes" in caplog.text)

    assert run.heard == []
    assert len(socket.sent) == 1
    await run.stop()


@pytest.mark.parametrize("count", [None, "1", "many"])
async def test_an_event_that_is_one_piece_needs_no_count(
    socket: FakeSocket, count: str | None
) -> None:
    run = await connected(socket)
    named = [("type", "event"), ("message_id", "m1")]
    if count is not None:
        named.append(("sum", count))
    await socket.incoming.put(encoded(Frame(method=DATA, headers=tuple(named))))
    await until(lambda: len(socket.sent) == 2)

    assert socket.frames()[1].payload == b'{"code": 500}'
    await run.stop()


async def test_what_is_no_event_is_not_answered(
    socket: FakeSocket, caplog: pytest.LogCaptureFixture
) -> None:
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(b"{}", type="card")))
    await socket.incoming.put(encoded(Frame(method=7, headers=(("type", "event"),))))
    await socket.incoming.put("a message of text")
    await socket.incoming.put(b"\x0b")
    await until(lambda: "Feishu sent something that is no frame" in caplog.text)

    assert run.heard == []
    assert len(socket.sent) == 1
    await run.stop()


async def test_a_ping_or_an_answer_that_cannot_be_sent_is_let_go(
    socket: FakeSocket, caplog: pytest.LogCaptureFixture
) -> None:
    socket.failing = OSError(f"gone, with {SECRET} in it")
    run = await connected(socket)
    await socket.incoming.put(encoded(event_frame(b"{}")))
    await until(lambda: "could not be answered" in caplog.text)

    assert "a ping to Feishu could not be sent" in caplog.text
    assert run.heard == [{}]
    assert SECRET not in caplog.text
    await run.stop()


# Losing the connection, and opening it again


async def test_a_connection_that_is_closed_is_opened_again(socket: FakeSocket) -> None:
    again = FakeSocket()
    feishu = Feishu()
    run = Run(feishu, Opened(socket, again))
    run.start()
    await until(lambda: ("connected", None) in run.told)

    await socket.incoming.put(None)
    await until(lambda: run.told.count(("connected", None)) == 2)

    assert run.told == [
        ("connecting", None),
        ("connected", None),
        ("connecting", "the connection was closed"),
        ("connected", None),
    ]
    assert len(feishu.asked) == 2
    # The first pause is drawn at random, at most as long as Feishu says.
    assert run.pauses.taken == [LONG, 15.0, LONG]
    await run.stop()


async def test_a_connection_that_breaks_off_is_opened_again(socket: FakeSocket) -> None:
    run = Run(Feishu(), Opened(socket, FakeSocket()))
    run.start()
    await until(lambda: ("connected", None) in run.told)

    await socket.incoming.put(ConnectionResetError(f"reset, {SECRET}"))
    await until(lambda: run.told.count(("connected", None)) == 2)

    lost = run.told[2]
    assert lost[0] == "connecting"
    assert lost[1] is not None
    assert lost[1].startswith("the connection broke off: reset, [App Secret]")
    await run.stop()


def failing_with(error: Exception) -> Callable[[httpx.Request], httpx.Response]:
    def answer(request: httpx.Request) -> httpx.Response:
        raise error

    return answer


def answering(status: int, body: bytes) -> Callable[[httpx.Request], httpx.Response]:
    return lambda request: httpx.Response(status, content=body)


@pytest.mark.parametrize(
    ("answer", "why"),
    [
        (failing_with(httpx.ConnectTimeout("slow")), "Feishu did not answer in time"),
        (
            failing_with(httpx.ConnectError(f"refused {SECRET}")),
            "Feishu could not be reached: refused [App Secret]",
        ),
        (answering(502, b"{}"), "Feishu answered with HTTP 502"),
        (answering(200, b"<html>"), "Feishu answered with something that is no object of JSON"),
        (answering(200, b"[]"), "Feishu answered with something that is no object of JSON"),
        (answering(200, b'{"code": 1, "msg": "system busy"}'), "Feishu is busy: system busy"),
        (answering(200, b'{"code": 1000040343}'), "Feishu is busy: 1000040343"),
        (
            answering(200, b'{"code": 0, "data": {"URL": "https://x"}}'),
            "Feishu gave no address for the connection",
        ),
        (answering(200, b'{"code": 0, "data": []}'), "Feishu gave no address for the connection"),
    ],
)
async def test_an_address_that_cannot_be_had_now_is_asked_for_again(
    socket: FakeSocket, answer: Callable[[httpx.Request], httpx.Response], why: str
) -> None:
    feishu = Feishu(answer, answer, lambda request: httpx.Response(200, json=endpoint()))
    run = Run(feishu, Opened(socket))
    run.start()
    await until(lambda: ("connected", None) in run.told)

    assert run.told == [
        ("connecting", None),
        ("connecting", why),
        ("connecting", why),
        ("connected", None),
    ]
    # A pause drawn at random first, then the pause that Feishu says.
    assert run.pauses.taken[:2] == [15.0, 120.0]
    await run.stop()


async def test_an_app_that_feishu_gives_no_connection_is_not_tried_again() -> None:
    said = {"code": 1000040344, "msg": f"app secret {SECRET} is invalid"}
    feishu = Feishu(lambda request: httpx.Response(200, json=said))
    run = Run(feishu, Opened())

    with pytest.raises(FeishuError) as refused:
        await run.link.run()

    assert refused.value.code == "REFUSED"
    assert "(code 1000040344)" in str(refused.value)
    assert SECRET not in str(refused.value)
    assert len(feishu.asked) == 1
    await run.http.aclose()


async def test_a_refusal_without_a_reason_says_so() -> None:
    feishu = Feishu(lambda request: httpx.Response(200, json={"code": 10003}))
    run = Run(feishu, Opened())

    with pytest.raises(FeishuError, match="it gave no reason"):
        await run.link.run()
    await run.http.aclose()


ELSEWHERE = {"code": 1000040351, "msg": "Incorrect domain name"}
"""What a platform answers when it is asked for a connection of an app of the other one."""


@pytest.mark.parametrize(
    ("host", "said"),
    [
        pytest.param(
            HOST,
            "Feishu does not know this app, which is one of Lark (Incorrect domain name, code "
            "1000040351). Set it up as an app of Lark",
            id="an app of lark asked about at feishu",
        ),
        pytest.param(
            LARK,
            "Lark does not know this app, which is one of Feishu (Incorrect domain name, code "
            "1000040351). Set it up as an app of Feishu",
            id="an app of feishu asked about at lark",
        ),
    ],
)
async def test_an_app_of_the_other_platform_is_said_to_be_one_and_not_tried_again(
    host: Host, said: str
) -> None:
    feishu = Feishu(lambda request: httpx.Response(200, json=ELSEWHERE))
    run = Run(feishu, Opened(), host=host)

    with pytest.raises(FeishuError) as refused:
        await run.link.run()

    assert refused.value.code == "WRONG_PLATFORM"
    assert str(refused.value) == said
    assert [str(request.url) for request in feishu.asked] == [f"{host.api}{ENDPOINT}"]
    await run.http.aclose()


async def test_an_app_of_the_other_platform_without_words_is_named_by_the_code() -> None:
    feishu = Feishu(lambda request: httpx.Response(200, json={"code": 1000040351}))
    run = Run(feishu, Opened())

    with pytest.raises(FeishuError) as refused:
        await run.link.run()

    assert "which is one of Lark (code 1000040351). Set it up" in str(refused.value)
    await run.http.aclose()


async def test_an_app_of_the_other_platform_is_found_before_it_is_kept() -> None:
    feishu = Feishu(lambda request: httpx.Response(200, json=ELSEWHERE))

    async with httpx.AsyncClient(transport=httpx.MockTransport(feishu)) as http:
        with pytest.raises(FeishuError) as refused:
            await platform_checked(HOST, "cli_a1b2c3", SECRET, http)

    assert refused.value.code == "WRONG_PLATFORM"
    assert str(refused.value).endswith("Set it up as an app of Lark")


@pytest.mark.parametrize(
    "answer",
    [
        pytest.param(answering(200, json.dumps(endpoint()).encode()), id="given"),
        pytest.param(answering(200, b'{"code": 1, "msg": "system busy"}'), id="busy"),
        pytest.param(answering(502, b"{}"), id="no answer of json"),
        pytest.param(failing_with(httpx.ConnectError("refused")), id="not reached"),
        pytest.param(answering(200, b'{"code": 1000040344}'), id="refused otherwise"),
    ],
)
async def test_an_app_that_its_platform_knows_or_cannot_tell_about_now_is_taken(
    answer: Callable[[httpx.Request], httpx.Response],
) -> None:
    feishu = Feishu(answer)

    async with httpx.AsyncClient(transport=httpx.MockTransport(feishu)) as http:
        await platform_checked(HOST, "cli_a1b2c3", SECRET, http)

    (asked,) = feishu.asked
    assert str(asked.url) == f"{HOST.api}{ENDPOINT}"
    assert json.loads(asked.content) == {"AppID": "cli_a1b2c3", "AppSecret": SECRET}


class RefusedError(Exception):
    """What the library raises where Feishu does not let a WebSocket be opened."""

    def __init__(self, headers: dict[str, str] | None) -> None:
        super().__init__("server rejected WebSocket connection: HTTP 400")
        self.response = None if headers is None else type("Answer", (), {"headers": headers})()


@pytest.mark.parametrize(
    "headers",
    [
        {"handshake-status": "403", "handshake-msg": "forbidden"},
        {
            "handshake-status": "514",
            "handshake-msg": "too many",
            "handshake-autherrcode": "1000040350",
        },
        {"handshake-status": "403"},
    ],
)
async def test_a_connection_that_is_refused_for_good_is_not_tried_again(
    headers: dict[str, str],
) -> None:
    run = Run(Feishu(), Opened(RefusedError(headers)))

    with pytest.raises(FeishuError) as refused:
        await run.link.run()

    assert refused.value.code == "REFUSED"
    assert (headers.get("handshake-msg") or "403") in str(refused.value)
    await run.http.aclose()


async def test_a_refusal_that_repeats_the_secret_does_not_say_it() -> None:
    headers = {"handshake-status": "403", "handshake-msg": f"no app with {SECRET}"}
    run = Run(Feishu(), Opened(RefusedError(headers)))

    with pytest.raises(FeishuError) as refused:
        await run.link.run()

    assert SECRET not in str(refused.value)
    assert "no app with [App Secret]" in str(refused.value)
    await run.http.aclose()


@pytest.mark.parametrize(
    "headers",
    [
        {"handshake-status": "514", "handshake-autherrcode": "1"},
        {"handshake-status": "500"},
        None,
    ],
)
async def test_a_connection_that_is_refused_for_now_is_tried_again(
    socket: FakeSocket, headers: dict[str, str] | None
) -> None:
    run = Run(Feishu(), Opened(RefusedError(headers), socket))
    run.start()
    await until(lambda: ("connected", None) in run.told)

    assert run.told[1] == ("connecting", "server rejected WebSocket connection: HTTP 400")
    await run.stop()


async def test_an_error_without_words_is_named_by_its_kind(socket: FakeSocket) -> None:
    run = Run(Feishu(), Opened(TimeoutError(), socket))
    run.start()
    await until(lambda: ("connected", None) in run.told)

    assert run.told[1] == ("connecting", "TimeoutError")
    await run.stop()


async def test_the_tries_that_failed_are_counted_anew_once_a_connection_was_open(
    socket: FakeSocket,
) -> None:
    # Feishu allows one try that fails, and no second one in a row.
    closing = FakeSocket()
    closing.incoming.put_nowait(None)
    feishu = Feishu(
        lambda request: httpx.Response(200, json=endpoint(config={"ReconnectCount": 1}))
    )
    run = Run(feishu, Opened(OSError("no way"), closing, socket))
    run.start()
    await until(lambda: run.told.count(("connected", None)) == 2)

    assert run.told == [
        ("connecting", None),
        ("connecting", "no way"),
        ("connected", None),
        ("connecting", "the connection was closed"),
        ("connected", None),
    ]
    assert run.task is not None
    assert not run.task.done()
    await run.stop()


async def test_feishu_that_cannot_be_reached_as_often_as_it_allows_is_given_up_on() -> None:
    config = {"ReconnectCount": 1}
    feishu = Feishu(
        lambda request: httpx.Response(200, json=endpoint(config=config)),
        lambda request: httpx.Response(502),
    )
    run = Run(feishu, Opened(OSError("no way")))

    with pytest.raises(FeishuError) as given_up:
        await run.link.run()

    assert given_up.value.code == "UNREACHABLE"
    assert (
        str(given_up.value)
        == "Feishu could not be reached 2 times in a row: Feishu answered with HTTP 502"
    )
    await run.http.aclose()


# The library of WebSockets, with a server on this machine


def serving(
    answer: Callable[[Any], Awaitable[None]],
) -> tuple[str, Callable[[], None]]:
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
                where.append(f"ws://127.0.0.1:{port}/ws?service_id=7")

                def stop() -> None:
                    loop.call_soon_threadsafe(done.set)

                stopping.append(stop)
                ready.set()
                await done.wait()

        # The runner ends what is still pending before it closes its loop, such as a connection
        # that came in while the server closed. Left pending, it is destroyed later, in another
        # test, which then fails.
        with asyncio.Runner(loop_factory=asyncio.SelectorEventLoop) as runner:
            runner.run(main())

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    ready.wait(5)

    def stop() -> None:
        stopping[0]()
        thread.join(5)

    return where[0], stop


async def test_a_connection_that_is_stopped_is_closed_as_one_that_ended_well(
    socket: FakeSocket,
) -> None:
    run = await connected(socket)

    await run.stop()

    assert socket.closed == 1


async def test_a_connection_that_cannot_be_closed_is_let_go(
    socket: FakeSocket, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    socket.closing = OSError(f"gone with {SECRET}")
    run = await connected(socket)

    await run.stop()

    assert run.task is not None
    assert run.task.cancelled()
    assert "the connection to Feishu could not be closed: gone with [App Secret]" in caplog.text
    assert SECRET not in caplog.text


async def test_the_library_of_websockets_closes_a_stopped_connection_as_one_that_ended_well() -> (
    None
):
    codes: list[int | None] = []
    connected_now = threading.Event()

    async def feishu(connection: Any) -> None:
        connected_now.set()
        try:
            async for _ in connection:
                pass
        finally:
            codes.append(connection.close_code)

    address, stop = serving(feishu)
    told: list[tuple[str, str | None]] = []
    http = httpx.AsyncClient(
        transport=httpx.MockTransport(lambda request: httpx.Response(200, json=endpoint(address)))
    )
    link = Link(
        HOST,
        "cli_a1b2c3",
        SECRET,
        http,
        lambda event: None,
        lambda state, why: told.append((state, why)),
        sleep=Pauses(),
        rhythm=Rhythm(ping=LONG),
    )
    task = asyncio.ensure_future(link.run())
    try:
        await until(lambda: ("connected", None) in told, 10)
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await until(lambda: bool(codes), 10)
    finally:
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await http.aclose()
        stop()

    assert connected_now.is_set()
    assert codes == [1000]


async def test_the_library_of_websockets_carries_an_event_and_its_answer() -> None:
    answered: list[bytes] = []

    async def feishu(connection: Any) -> None:
        await connection.send(encoded(event_frame(b'{"hello": "there"}')))
        async for message in connection:
            frame = decoded(message)
            if frame.method == DATA:
                answered.append(message)
                await connection.close()

    address, stop = serving(feishu)
    heard: list[dict[str, Any]] = []
    told: list[tuple[str, str | None]] = []
    http = httpx.AsyncClient(
        transport=httpx.MockTransport(lambda request: httpx.Response(200, json=endpoint(address)))
    )
    link = Link(
        HOST,
        "cli_a1b2c3",
        SECRET,
        http,
        heard.append,
        lambda state, why: told.append((state, why)),
        sleep=Pauses(),
        rhythm=Rhythm(ping=LONG),
    )
    task = asyncio.ensure_future(link.run())
    try:
        await until(lambda: ("connecting", "the connection was closed") in told, 10)
    finally:
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await http.aclose()
        stop()

    assert heard == [{"hello": "there"}]
    assert decoded(answered[0]).payload == b'{"code": 200}'
