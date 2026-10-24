"""The gateway of QQ: ``columbina.qq.link``."""

import asyncio
import json
import logging
from collections.abc import Callable
from typing import Any

import httpx
import pytest

from columbina.qq import link as link_module
from columbina.qq.app import Api, QqError
from columbina.qq.link import GROUP, INTENTS, Link
from support.fake_qq import APP_ID, GATEWAY, SECRET, TOKEN, FakeApi, FakeSocket, Opened


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    async with asyncio.timeout(seconds):
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.002)


class Beats:
    """The time between two heartbeats, which passes when the test says so."""

    def __init__(self) -> None:
        self.asked: list[float] = []
        self.go = asyncio.Event()

    async def __call__(self, seconds: float) -> None:
        self.asked.append(seconds)
        await self.go.wait()
        self.go.clear()


class Closed(Exception):  # noqa: N818 - what the library raises, played here
    """What the library of WebSockets raises where the other side closed with a code."""

    def __init__(self, code: int) -> None:
        super().__init__(f"received {code}")
        self.rcvd = type("Close", (), {"code": code})()


class Rig:
    def __init__(self, *sockets: FakeSocket | BaseException, api: Api | None = None) -> None:
        self.fake = FakeApi()
        self.api = api or Api(APP_ID, SECRET, {}, transport=self.fake.transport)
        self.opened = Opened(*sockets)
        self.heard: list[tuple[str, dict[str, Any], str | None]] = []
        self.told: list[tuple[str, str | None]] = []
        self.pauses: list[float] = []
        self.beats = Beats()

        async def paused(seconds: float) -> None:
            self.pauses.append(seconds)
            await asyncio.sleep(0)

        self.link = Link(
            self.api,
            heard=lambda kind, data, event_id: self.heard.append((kind, data, event_id)),
            told=lambda state, why: self.told.append((state, why)),
            opener=self.opened,
            sleep=paused,
            ticking=self.beats,
        )
        self.task: asyncio.Task[None] | None = None

    def start(self) -> None:
        self.task = asyncio.ensure_future(self.link.run())

    async def stop(self) -> None:
        assert self.task is not None
        self.task.cancel()
        await asyncio.gather(self.task, return_exceptions=True)

    def connected(self) -> bool:
        return bool(self.told) and self.told[-1][0] == "connected"


async def test_the_bot_signs_in_and_hears_what_is_written_to_it() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)

    socket.dispatch(GROUP, {"id": "m1", "content": " hi"}, event_id="e1")
    await until(lambda: bool(rig.heard))

    assert rig.opened.asked == [GATEWAY]
    (identify,) = socket.of(2)
    assert identify["d"] == {"token": f"QQBot {TOKEN}", "intents": INTENTS, "shard": [0, 1]}
    assert rig.heard == [(GROUP, {"id": "m1", "content": " hi"}, "e1")]
    assert rig.told == [("connecting", None), ("connected", None)]
    await rig.stop()
    assert socket.closed == 1


async def test_a_heartbeat_goes_out_as_often_as_qq_asks_with_the_last_number() -> None:
    socket = FakeSocket(interval=1500)
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)
    socket.dispatch("SOMETHING", {"x": 1})
    await until(lambda: bool(rig.heard))

    rig.beats.go.set()
    await until(lambda: len(socket.of(1)) == 1)

    assert rig.beats.asked[0] == 1.5
    # READY was the first, and this the second.
    assert socket.of(1)[0]["d"] == 2
    # QQ may ask for one too.
    socket.tell({"op": 1})
    await until(lambda: len(socket.of(1)) == 2)
    await rig.stop()


async def test_a_greeting_without_an_interval_keeps_the_one_there_is() -> None:
    socket = FakeSocket()
    socket.incoming.get_nowait()
    socket.tell({"op": 10, "d": {}})
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)
    rig.beats.go.set()
    await until(lambda: bool(rig.beats.asked))

    assert rig.beats.asked[0] == 30.0
    await rig.stop()


async def test_a_connection_whose_heartbeats_go_unanswered_is_opened_again_and_takes_up_its_session(
    caplog: pytest.LogCaptureFixture,
) -> None:
    silent = FakeSocket()
    again = FakeSocket()
    rig = Rig(silent, again)
    rig.start()
    await until(rig.connected)
    silent.dispatch(GROUP, {"id": "m1"})
    await until(lambda: len(rig.heard) == 1)
    # QQ stops answering the heartbeats.
    silent.send = _deaf(silent)  # type: ignore[method-assign]
    for _ in range(3):
        rig.beats.go.set()
        await asyncio.sleep(0.01)
    # What QQ sends again while the session is taken up counts as well.
    again.dispatch(GROUP, {"id": "m2"})
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    (resume,) = again.of(6)
    assert resume["d"] == {"token": f"QQBot {TOKEN}", "session_id": "session-1", "seq": 2}
    assert again.of(2) == []
    await until(lambda: len(rig.heard) == 2)
    assert rig.told[2] == ("connecting", "QQ did not answer 2 heartbeats in a row")
    assert rig.pauses == [1.0]
    await rig.stop()


def _deaf(socket: FakeSocket) -> Callable[[str], Any]:
    async def send(message: str) -> None:
        socket.sent.append(json.loads(message))

    return send


@pytest.mark.parametrize(
    ("frame", "why", "anew"),
    [
        ({"op": 7}, "QQ asked for a new connection", False),
        ({"op": 9, "d": False}, "QQ said that the session is no longer valid", True),
    ],
    ids=["reconnect", "invalid"],
)
async def test_qq_may_ask_for_a_new_connection_or_a_new_session(
    frame: dict[str, Any], why: str, anew: bool
) -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    first.tell(frame)
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert ("connecting", why) in rig.told
    assert len(again.of(2 if anew else 6)) == 1
    await rig.stop()


async def test_a_session_that_qq_does_not_take_up_is_signed_in_anew() -> None:
    first = FakeSocket()
    refusing = FakeSocket(ready=False)
    again = FakeSocket()
    rig = Rig(first, refusing, again)
    rig.start()
    await until(rig.connected)
    first.tell({"op": 7})
    await until(lambda: len(refusing.of(6)) == 1)

    refusing.tell({"op": 9, "d": False})
    await until(lambda: len(rig.opened.asked) == 3 and rig.connected())

    assert len(again.of(2)) == 1
    await rig.stop()


async def test_what_comes_before_the_bot_is_ready_that_is_no_event_is_passed_over() -> None:
    socket = FakeSocket(ready=False)
    rig = Rig(socket)
    rig.start()
    await until(lambda: len(socket.of(2)) == 1)

    socket.tell({"op": 11})
    socket.dispatch("READY", {"session_id": 7})
    await until(rig.connected)
    await rig.stop()


async def test_a_connection_that_qq_does_not_greet_is_tried_again() -> None:
    rude = FakeSocket()
    rude.incoming.get_nowait()
    rude.tell({"op": 0, "t": "READY"})
    rig = Rig(rude, FakeSocket())
    rig.start()

    await until(rig.connected)

    assert rig.told[1] == ("connecting", "QQ did not greet the connection")
    await rig.stop()


async def test_a_connection_closed_before_the_bot_is_ready_is_tried_again_up_to_five_times() -> (
    None
):
    closing = FakeSocket(ready=False)
    rig = Rig(closing)

    async def closed_at_once(seconds: float) -> None:
        rig.pauses.append(seconds)
        closing.incoming = asyncio.Queue()
        closing.tell({"op": 10, "d": {"heartbeat_interval": 41250}})
        closing.end()

    rig.link.sleep = closed_at_once
    closing.end()

    with pytest.raises(QqError, match="QQ closed the connection 5 times in a row") as raised:
        await rig.link.run()

    assert raised.value.code == "REFUSED"
    assert rig.pauses == [1.0, 2.0, 4.0, 8.0]


async def test_the_pause_before_another_try_doubles_up_to_thirty_seconds() -> None:
    rig = Rig(ConnectionResetError("reset"))
    tries = 0

    async def paused(seconds: float) -> None:
        nonlocal tries
        rig.pauses.append(seconds)
        tries += 1
        if tries == 7:
            raise asyncio.CancelledError

    rig.link.sleep = paused

    with pytest.raises(asyncio.CancelledError):
        await rig.link.run()

    assert rig.pauses == [1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]
    assert rig.told[-1] == ("connecting", "reset")


@pytest.mark.parametrize(
    ("code", "why"), [(4914, "the bot is offline"), (4915, "the bot is banned")]
)
async def test_a_bot_that_qq_closes_for_good_is_not_tried_again(code: int, why: str) -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)

    socket.end(Closed(code))

    assert rig.task is not None
    with pytest.raises(QqError, match=why):
        await rig.task


async def test_a_token_that_qq_did_not_take_is_asked_for_anew() -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    first.end(Closed(4004))
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert rig.fake.tokens == 2
    assert ("connecting", "the connection was closed with the code 4004") in rig.told
    await rig.stop()


async def test_a_session_that_cannot_be_taken_up_is_signed_in_anew() -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    first.end(Closed(4009))
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert len(again.of(2)) == 1
    await rig.stop()


async def test_a_connection_that_breaks_off_without_a_code_says_how() -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    first.end(ConnectionResetError())
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert ("connecting", "ConnectionResetError") in rig.told
    await rig.stop()


async def test_a_connection_that_qq_closes_as_meant_is_opened_again() -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    first.end()
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert ("connecting", "the connection was closed") in rig.told
    await rig.stop()


async def test_a_gateway_that_cannot_be_reached_is_tried_again_and_one_that_refuses_is_not() -> (
    None
):
    fake = FakeApi()
    calls = 0

    def handle(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        if request.url.path == "/gateway":
            calls += 1
            if calls == 1:
                return httpx.Response(503, json={"message": "busy"})
            return httpx.Response(403, json={"code": 11298, "message": "no right"})
        return fake.transport.handle_request(request)

    api = Api(APP_ID, SECRET, {}, transport=httpx.MockTransport(handle))
    rig = Rig(FakeSocket(), api=api)

    with pytest.raises(QqError, match="no right"):
        await rig.link.run()

    assert "busy" in str(rig.told[1][1])


async def test_a_sign_in_that_qq_does_not_answer_in_time_is_tried_again(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(link_module, "_SIGN_IN_SECONDS", 0.01)
    rig = Rig(FakeSocket(ready=False), FakeSocket())
    rig.start()

    await until(rig.connected)

    assert rig.told[1] == ("connecting", "QQ did not answer the sign-in in time")
    await rig.stop()


async def test_a_heartbeat_that_cannot_be_sent_loses_the_connection() -> None:
    first, again = FakeSocket(), FakeSocket()
    rig = Rig(first, again)
    rig.start()
    await until(rig.connected)

    async def broken(message: str) -> None:
        raise OSError("the network is gone")

    first.send = broken
    rig.beats.go.set()
    await until(lambda: len(rig.opened.asked) == 2 and rig.connected())

    assert ("connecting", "a heartbeat could not be sent: the network is gone") in rig.told
    await rig.stop()


async def test_what_is_no_frame_or_cannot_be_taken_is_passed_over(
    caplog: pytest.LogCaptureFixture,
) -> None:
    socket = FakeSocket()
    rig = Rig(socket)

    def heard(kind: str, data: dict[str, Any], event_id: str | None) -> None:
        if data.get("id") == "bad":
            raise RuntimeError("cannot take it")
        rig.heard.append((kind, data, event_id))

    rig.link.heard = heard
    rig.start()
    await until(rig.connected)

    socket.incoming.put_nowait("not json")
    socket.incoming.put_nowait("[1, 2]")
    socket.tell({"op": 0, "s": 9, "d": {}})
    socket.dispatch(GROUP, {"id": "bad"})
    socket.dispatch(GROUP, {"id": "good"}, event_id=None)
    socket.incoming.put_nowait(b'{"op": 0, "s": 12, "t": "X", "d": {"id": "bytes"}}')  # type: ignore[arg-type]
    await until(lambda: len(rig.heard) == 2)

    assert [data["id"] for _, data, _ in rig.heard] == ["good", "bytes"]
    assert "QQ sent something that is no JSON" in caplog.text
    assert "QQ sent something that is no object of JSON" in caplog.text
    assert "what QQ sent could not be taken" in caplog.text
    await rig.stop()


async def test_a_connection_that_cannot_be_closed_when_it_stops_is_let_go(
    caplog: pytest.LogCaptureFixture,
) -> None:
    caplog.set_level(logging.INFO)
    socket = FakeSocket()

    async def broken() -> None:
        raise OSError("already gone")

    socket.close = broken
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)

    await rig.stop()

    assert "the connection to QQ could not be closed: already gone" in caplog.text


def test_a_websocket_is_opened_with_the_library_of_websockets() -> None:
    opened = link_module._opened("wss://example.test/websocket")  # pyright: ignore[reportPrivateUsage]

    assert opened is not None


async def test_what_is_no_frame_or_no_known_op_is_passed_over_on_the_way() -> None:
    socket = FakeSocket()
    first = socket.incoming.get_nowait()
    socket.incoming.put_nowait("not json")
    socket.incoming.put_nowait(first)
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)

    socket.tell({"op": 99})
    socket.dispatch(GROUP, {"id": "after"})
    await until(lambda: len(rig.heard) == 1)

    await rig.stop()


async def test_a_connection_closed_with_a_code_before_the_bot_is_ready_counts_as_refused() -> None:
    rig = Rig(Closed(4010))

    with pytest.raises(QqError, match="with the code 4010"):
        await rig.link.run()

    assert len(rig.pauses) == 4


def test_the_bot_asks_for_the_events_of_groups_and_of_chats_of_two() -> None:
    # The intent of QQ for both, by its number.
    assert INTENTS == 33554432


async def test_answered_heartbeats_keep_the_connection() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.start()
    await until(rig.connected)

    for beat in range(1, 5):
        rig.beats.go.set()
        await until(lambda beat=beat: len(socket.of(1)) == beat)
        await asyncio.sleep(0.01)

    assert len(rig.opened.asked) == 1
    await rig.stop()


async def test_a_connection_closed_with_a_code_once_the_bot_is_ready_is_tried_for_ever() -> None:
    sockets = [FakeSocket() for _ in range(7)]
    rig = Rig(*sockets)
    rig.start()

    for number in range(6):
        await until(lambda number=number: len(rig.opened.asked) == number + 1 and rig.connected())
        sockets[number].end(Closed(4009))
    await until(lambda: len(rig.opened.asked) == 7 and rig.connected())

    assert rig.task is not None
    assert not rig.task.done()
    await rig.stop()
