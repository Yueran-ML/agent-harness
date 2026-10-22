"""The long connection to WeCom: ``columbina.wecom.link``."""

import asyncio
import json
import logging
from collections.abc import Callable
from typing import Any

import pytest

from columbina.net import proxy_for
from columbina.wecom import link as link_module
from columbina.wecom.app import WecomError
from columbina.wecom.link import EVENT, MESSAGE, RESPOND, SEND, Link, request_id
from support.fake_wecom import (
    BOT_ID,
    PLACE,
    SECRET,
    FakeSocket,
    Opened,
    answered,
    event,
    message,
    serving,
)


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    """Wait for something that the link does in its own time."""
    async with asyncio.timeout(seconds):
        # What is waited for is done by the link, which sets no event a test can see.
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.002)


class Pauses:
    """The pauses before a connection is opened again: noted, and not waited for."""

    def __init__(self) -> None:
        self.taken: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.taken.append(seconds)
        await asyncio.sleep(0)


class Ticks:
    """The time between two pings, which passes when a test says so."""

    def __init__(self) -> None:
        self.asked: list[float] = []
        self._passing: asyncio.Queue[None] = asyncio.Queue()

    async def __call__(self, seconds: float) -> None:
        self.asked.append(seconds)
        await self._passing.get()

    def tick(self) -> None:
        self._passing.put_nowait(None)


class Rig:
    """A link of a test, with what it heard and was told."""

    def __init__(
        self, *outcomes: FakeSocket | BaseException, env: dict[str, str] | None = None
    ) -> None:
        self.opened = Opened(*outcomes)
        self.pauses = Pauses()
        self.ticks = Ticks()
        self.heard: list[dict[str, Any]] = []
        self.told: list[tuple[str, str | None]] = []
        self.link = Link(
            PLACE,
            BOT_ID,
            SECRET,
            heard=self.heard.append,
            told=lambda state, why: self.told.append((state, why)),
            env=env or {},
            opener=self.opened,
            sleep=self.pauses,
            ticking=self.ticks,
        )
        self.task: asyncio.Task[None] | None = None

    def run(self) -> None:
        self.task = asyncio.ensure_future(self.link.run())

    async def connected(self, times: int = 1) -> None:
        await until(lambda: self.told.count(("connected", None)) >= times)

    async def ended(self) -> BaseException | None:
        assert self.task is not None
        await until(self.task.done)
        return self.task.exception()

    async def stop(self) -> None:
        assert self.task is not None
        self.task.cancel()
        await asyncio.gather(self.task, return_exceptions=True)


# Signing in


async def test_the_bot_signs_in_with_its_id_and_its_secret_and_is_connected() -> None:
    socket = FakeSocket()
    rig = Rig(socket)

    rig.run()
    await rig.connected()

    (signed,) = socket.sent
    assert signed["cmd"] == "aibot_subscribe"
    assert signed["headers"]["req_id"].startswith("aibot_subscribe_")
    assert signed["body"] == {"bot_id": BOT_ID, "secret": SECRET}
    assert rig.told == [("connecting", None), ("connected", None)]
    assert rig.opened.asked == [(PLACE, None)]
    await rig.stop()
    # WeCom is told that the connection ended as it was meant to.
    assert socket.closed == 1


async def test_a_bot_whose_id_and_secret_do_not_belong_together_is_not_tried_again() -> None:
    socket = FakeSocket(refusing={"aibot_subscribe": 853000})
    rig = Rig(socket)

    rig.run()
    error = await rig.ended()

    assert isinstance(error, WecomError)
    assert (error.code, error.number) == ("REFUSED", 853000)
    assert "does not take this Bot ID with this Secret (ok, errcode 853000)" in str(error)
    assert len(rig.opened.asked) == 1


async def test_a_bot_whose_api_is_not_the_long_connection_is_not_tried_again() -> None:
    rig = Rig(FakeSocket(refusing={"aibot_subscribe": 846601}))

    rig.run()
    error = await rig.ended()

    assert isinstance(error, WecomError)
    assert (error.code, error.number) == ("NOT_LONG", 846601)
    assert "the API of this bot is not set to the long connection" in str(error)


async def test_a_sign_in_that_is_refused_for_another_reason_is_tried_again_and_given_up() -> None:
    rig = Rig(FakeSocket(refusing={"aibot_subscribe": 846609}))

    rig.run()
    error = await rig.ended()

    assert isinstance(error, WecomError)
    assert error.code == "REFUSED"
    assert str(error) == (
        "WeCom refused the sign-in 5 times in a row: WeCom refused the sign-in: ok, errcode 846609"
    )
    assert len(rig.opened.asked) == 5
    assert rig.pauses.taken == [1.0, 2.0, 4.0, 8.0]


async def test_refusals_are_counted_anew_once_the_bot_is_signed_in() -> None:
    refusing = FakeSocket(refusing={"aibot_subscribe": 846609})
    taking = FakeSocket()
    lost = FakeSocket()
    rig = Rig(refusing, refusing, refusing, refusing, taking, refusing, lost)
    rig.run()
    await rig.connected()

    taking.incoming.put_nowait(None)
    await rig.connected(2)

    # Four refusals, then a sign-in: the refusal after it is the first, and the link goes on.
    assert rig.pauses.taken == [1.0, 2.0, 4.0, 8.0, 1.0, 2.0]
    assert rig.task is not None
    assert not rig.task.done()
    await rig.stop()


async def test_a_sign_in_that_is_not_answered_in_time_is_tried_again(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(link_module, "_SIGN_IN_SECONDS", 0.01)
    silent, taking = FakeSocket(answering=False), FakeSocket()
    rig = Rig(silent, taking)

    rig.run()
    await rig.connected()

    assert rig.told[1] == ("connecting", "WeCom did not answer the sign-in in time")
    await rig.stop()


async def test_a_connection_that_closes_before_the_sign_in_is_answered_is_opened_again() -> None:
    closing, taking = FakeSocket(answering=False), FakeSocket()
    closing.incoming.put_nowait(json.dumps({"cmd": "something newer"}))
    closing.incoming.put_nowait(None)
    rig = Rig(closing, taking)

    rig.run()
    await rig.connected()

    assert rig.told[1] == ("connecting", "the connection was closed before the bot was signed in")
    await rig.stop()


async def test_a_connection_that_cannot_be_opened_is_opened_again_after_pauses_that_double() -> (
    None
):
    failing = OSError(f"no route, with {SECRET} in it")
    rig = Rig(*[failing] * 7, FakeSocket())

    rig.run()
    await rig.connected()

    assert rig.pauses.taken == [1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]
    # What is said of it never holds the Secret.
    assert rig.told[1] == ("connecting", "no route, with [Secret] in it")
    await rig.stop()


async def test_a_connection_that_was_lost_is_opened_again() -> None:
    first, second = FakeSocket(), FakeSocket()
    rig = Rig(first, second)
    rig.run()
    await rig.connected()

    first.incoming.put_nowait(ConnectionResetError("reset"))
    await rig.connected(2)

    assert ("connecting", "the connection broke off: reset") in rig.told
    assert rig.pauses.taken == [1.0]
    await rig.stop()


async def test_a_connection_that_wecom_closes_is_opened_again() -> None:
    first, second = FakeSocket(), FakeSocket()
    rig = Rig(first, second)
    rig.run()
    await rig.connected()

    first.incoming.put_nowait(None)
    await rig.connected(2)

    assert ("connecting", "the connection was closed") in rig.told
    await rig.stop()


async def test_the_connection_goes_through_the_proxy_of_the_environment() -> None:
    named = Rig(FakeSocket(), env={"HTTPS_PROXY": "http://proxy.example:8080"})
    named.run()
    await named.connected()
    await named.stop()
    unnamed = Rig(FakeSocket(), env={})
    unnamed.run()
    await unnamed.connected()
    await unnamed.stop()

    assert named.opened.asked == [(PLACE, True)]
    assert unnamed.opened.asked == [(PLACE, proxy_for("https://openws.work.weixin.qq.com", {}))]


async def test_a_connection_that_cannot_be_closed_politely_is_left(
    caplog: pytest.LogCaptureFixture,
) -> None:
    caplog.set_level(logging.INFO, logger="columbina.wecom.link")
    socket = FakeSocket()
    socket.closing = OSError(f"gone, with {SECRET}")
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    await rig.stop()

    assert "the connection to WeCom could not be closed: gone, with [Secret]" in caplog.text


# What comes in


async def test_messages_and_events_are_heard_as_wecom_sent_them() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()
    said = message("你好", message_id="m1")
    entered = event("enter_chat")

    socket.tell(said)
    socket.tell(entered)
    await until(lambda: len(rig.heard) == 2)

    assert rig.heard == [said, entered]
    assert said["cmd"] == MESSAGE
    assert entered["cmd"] == EVENT
    await rig.stop()


async def test_characters_that_json_does_not_take_are_left_out() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    socket.incoming.put_nowait(
        '{"cmd": "aibot_msg_callback", "headers": {"req_id": "r\x0b1"}, '
        '"body": {"msgid": "m\x011"}}'
    )
    as_bytes: Any = b'{"cmd": "aibot_event_callback", "body": {}}'
    socket.incoming.put_nowait(as_bytes)
    await until(lambda: len(rig.heard) == 2)

    assert rig.heard == [
        {"cmd": MESSAGE, "headers": {"req_id": "r1"}, "body": {"msgid": "m1"}},
        {"cmd": EVENT, "body": {}},
    ]
    await rig.stop()


async def test_what_is_no_object_of_json_is_passed_over(caplog: pytest.LogCaptureFixture) -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    socket.incoming.put_nowait("not json")
    socket.incoming.put_nowait("[1, 2]")
    socket.tell({"headers": {"req_id": "nobody waits for this"}, "errcode": 0})
    socket.tell({"errcode": 0})
    socket.tell(message("你好", message_id="m1"))
    await until(lambda: len(rig.heard) == 1)

    assert "WeCom sent something that is no JSON" in caplog.text
    assert "WeCom sent something that is no object of JSON" in caplog.text
    await rig.stop()


async def test_a_frame_that_cannot_be_taken_is_logged_and_the_next_one_is_taken(
    caplog: pytest.LogCaptureFixture,
) -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    taken: list[dict[str, Any]] = []

    def heard(frame: dict[str, Any]) -> None:
        if not taken:
            taken.append(frame)
            raise RuntimeError("broken")
        taken.append(frame)

    rig.link.heard = heard
    rig.run()
    await rig.connected()

    socket.tell(message("one", message_id="m1"))
    socket.tell(message("two", message_id="m2"))
    await until(lambda: len(taken) == 2)

    assert "what WeCom sent could not be taken" in caplog.text
    await rig.stop()


async def test_a_bot_that_another_program_signed_in_with_stops_and_is_not_opened_again() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    socket.tell(event("disconnected_event"))
    error = await rig.ended()

    assert isinstance(error, WecomError)
    assert error.code == "TAKEN_OVER"
    assert "another program signed in with this bot" in str(error)
    assert rig.heard == []
    assert len(rig.opened.asked) == 1


# Pings


async def test_a_ping_goes_out_every_thirty_seconds_once_the_bot_is_signed_in() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    for count in (1, 2, 3):
        # Thirty seconds pass, and the answer to the ping comes long before the next.
        rig.ticks.tick()
        await until(lambda count=count: len(socket.of("ping")) == count)
        await asyncio.sleep(0.01)

    assert rig.ticks.asked[:3] == [30.0, 30.0, 30.0]
    assert all(ping["headers"]["req_id"].startswith("ping_") for ping in socket.of("ping"))
    assert "body" not in socket.of("ping")[0]
    # Each was answered: the connection holds.
    assert rig.told[-1] == ("connected", None)
    await rig.stop()


async def test_two_pings_in_a_row_without_an_answer_lose_the_connection() -> None:
    silent = FakeSocket(refusing={"ping": 846609})
    rig = Rig(silent, FakeSocket())
    rig.run()
    await rig.connected()

    # A ping answered with an error is answered no better than one not answered.
    rig.ticks.tick()
    rig.ticks.tick()
    await until(lambda: len(silent.of("ping")) == 2)
    rig.ticks.tick()
    await rig.connected(2)

    assert ("connecting", "WeCom did not answer 2 pings in a row") in rig.told
    await rig.stop()


async def test_a_ping_that_is_answered_counts_the_unanswered_ones_anew() -> None:
    socket = FakeSocket(answering=False)
    rig = Rig(socket)
    rig.run()
    await until(lambda: len(socket.sent) == 1)
    socket.answer(socket.sent[0])
    await rig.connected()

    rig.ticks.tick()
    await until(lambda: len(socket.of("ping")) == 1)
    rig.ticks.tick()
    await until(lambda: len(socket.of("ping")) == 2)
    socket.answer(socket.of("ping")[1])
    await asyncio.sleep(0.01)
    rig.ticks.tick()
    await until(lambda: len(socket.of("ping")) == 3)

    assert rig.told[-1] == ("connected", None)
    await rig.stop()


async def test_a_ping_that_cannot_be_sent_loses_the_connection() -> None:
    first, second = FakeSocket(), FakeSocket()
    rig = Rig(first, second)
    rig.run()
    await rig.connected()

    first.failing = OSError(f"broken pipe {SECRET}")
    rig.ticks.tick()
    await rig.connected(2)

    assert ("connecting", "a ping could not be sent: broken pipe [Secret]") in rig.told
    await rig.stop()


# What goes out


async def test_a_frame_goes_out_under_the_req_id_of_the_message_and_its_answer_comes_back() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()
    body = {"msgtype": "stream", "stream": {"id": "s1", "finish": True, "content": "好的"}}

    answer = await rig.link.call(RESPOND, body, "req-m1")

    assert socket.of(RESPOND) == [{"cmd": RESPOND, "headers": {"req_id": "req-m1"}, "body": body}]
    assert answer == {"headers": {"req_id": "req-m1"}, "errcode": 0, "errmsg": "ok"}
    await rig.stop()


async def test_a_frame_that_answers_no_message_has_a_req_id_of_its_own() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    await rig.link.call(SEND, {"chatid": "zhangsan"})

    (sent,) = socket.of(SEND)
    assert sent["headers"]["req_id"].startswith("aibot_send_msg_")
    assert request_id("x") != request_id("x")
    await rig.stop()


async def test_a_frame_that_wecom_refuses_says_why() -> None:
    socket = FakeSocket(refusing={RESPOND: 846604})
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    with pytest.raises(WecomError) as refused:
        await rig.link.call(RESPOND, {}, "req-m1")

    assert (refused.value.code, refused.value.number) == ("REFUSED", 846604)
    assert str(refused.value) == "WeCom refused it: ok (errcode 846604)"
    await rig.stop()


async def test_a_frame_that_is_refused_without_a_reason_says_so() -> None:
    socket = FakeSocket(answering=False)
    rig = Rig(socket)
    rig.run()
    await until(lambda: len(socket.sent) == 1)
    socket.answer(socket.sent[0])
    await rig.connected()

    calling = asyncio.ensure_future(rig.link.call(RESPOND, {}, "req-m1"))
    await until(lambda: len(socket.of(RESPOND)) == 1)
    socket.tell({"headers": {"req_id": "req-m1"}, "errcode": "odd"})

    with pytest.raises(WecomError) as refused:
        await calling
    assert (refused.value.code, refused.value.number) == ("REFUSED", None)
    assert str(refused.value) == "WeCom refused it: it gave no reason (errcode odd)"
    await rig.stop()


async def test_nothing_goes_out_while_the_bot_is_not_connected() -> None:
    rig = Rig(FakeSocket())

    with pytest.raises(WecomError) as unreached:
        await rig.link.call(SEND, {})

    assert unreached.value.code == "UNREACHABLE"
    assert str(unreached.value) == "the bot is not connected to WeCom now"


async def test_a_frame_that_wecom_does_not_answer_in_time_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(link_module, "_ANSWER_SECONDS", 0.01)
    socket = FakeSocket(answering=False)
    rig = Rig(socket)
    rig.run()
    await until(lambda: len(socket.sent) == 1)
    socket.answer(socket.sent[0])
    await rig.connected()

    with pytest.raises(WecomError) as late:
        await rig.link.call(RESPOND, {}, "req-m1")

    assert (late.value.code, str(late.value)) == ("UNREACHABLE", "WeCom did not answer in time")
    # A late answer finds nobody waiting for it.
    socket.answer(socket.of(RESPOND)[0])
    await asyncio.sleep(0.01)
    await rig.stop()


async def test_a_frame_that_cannot_be_sent_fails() -> None:
    socket = FakeSocket()
    rig = Rig(socket)
    rig.run()
    await rig.connected()

    socket.failing = OSError(f"broken {SECRET}")
    with pytest.raises(WecomError) as broken:
        await rig.link.call(RESPOND, {}, "req-m1")

    assert broken.value.code == "UNREACHABLE"
    assert str(broken.value) == "a frame could not be sent to WeCom: broken [Secret]"
    await rig.stop()


async def test_a_frame_whose_answer_does_not_come_before_the_connection_is_lost_fails() -> None:
    first, second = FakeSocket(answering=False), FakeSocket()
    rig = Rig(first, second)
    rig.run()
    await until(lambda: len(first.sent) == 1)
    first.answer(first.sent[0])
    await rig.connected()

    calling = asyncio.ensure_future(rig.link.call(RESPOND, {}, "req-m1"))
    await until(lambda: len(first.of(RESPOND)) == 1)
    first.incoming.put_nowait(None)

    with pytest.raises(WecomError) as lost:
        await calling
    assert lost.value.code == "UNREACHABLE"
    assert str(lost.value) == "the connection to WeCom was lost before WeCom answered"
    await rig.stop()


async def test_the_frames_of_one_message_go_out_one_after_another() -> None:
    socket = FakeSocket(answering=False)
    rig = Rig(socket)
    rig.run()
    await until(lambda: len(socket.sent) == 1)
    socket.answer(socket.sent[0])
    await rig.connected()

    first = asyncio.ensure_future(rig.link.call(RESPOND, {"part": 1}, "req-m1"))
    second = asyncio.ensure_future(rig.link.call(RESPOND, {"part": 2}, "req-m1"))
    other = asyncio.ensure_future(rig.link.call(RESPOND, {"part": 3}, "req-m2"))
    await until(lambda: len(socket.of(RESPOND)) == 2)
    await asyncio.sleep(0.01)

    # The second part of a message waits for the answer to the first, and another message does not.
    assert [frame["body"] for frame in socket.of(RESPOND)] == [{"part": 1}, {"part": 3}]
    socket.answer(socket.of(RESPOND)[0])
    await first
    await until(lambda: len(socket.of(RESPOND)) == 3)
    assert socket.of(RESPOND)[2]["body"] == {"part": 2}
    socket.answer(socket.of(RESPOND)[2])
    socket.answer(socket.of(RESPOND)[1])
    await asyncio.gather(second, other)
    assert rig.link._lines == {}  # pyright: ignore[reportPrivateUsage]
    await rig.stop()


# Asking once


async def test_a_bot_is_checked_by_signing_it_in_once() -> None:
    socket = FakeSocket()
    rig = Rig(socket)

    await rig.link.check()

    assert [frame["cmd"] for frame in socket.sent] == ["aibot_subscribe"]
    assert socket.closed == 1
    assert rig.told == []


@pytest.mark.parametrize(
    ("errcode", "code"), [(853000, "REFUSED"), (846601, "NOT_LONG"), (846609, "REFUSED")]
)
async def test_a_bot_that_wecom_refuses_is_said_to_be_refused(errcode: int, code: str) -> None:
    rig = Rig(FakeSocket(refusing={"aibot_subscribe": errcode}))

    with pytest.raises(WecomError) as refused:
        await rig.link.check()

    assert refused.value.code == code


async def test_a_check_that_wecom_cannot_be_asked_for_says_why(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(link_module, "_SIGN_IN_SECONDS", 0.01)
    unopened = Rig(OSError(f"no route {SECRET}"))
    silent = Rig(FakeSocket(answering=False))

    with pytest.raises(WecomError) as unreached:
        await unopened.link.check()
    with pytest.raises(WecomError) as unanswered:
        await silent.link.check()

    assert (unreached.value.code, str(unreached.value)) == (
        "UNREACHABLE",
        "WeCom could not be reached: no route [Secret]",
    )
    assert (unanswered.value.code, str(unanswered.value)) == (
        "UNREACHABLE",
        "WeCom could not be asked: WeCom did not answer the sign-in in time",
    )


# The library of WebSockets, with a server on this machine


async def test_the_library_of_websockets_carries_the_sign_in_a_message_and_an_answer() -> None:
    taken: list[dict[str, Any]] = []
    codes: list[int | None] = []

    async def wecom(connection: Any) -> None:
        try:
            async for text in connection:
                frame = json.loads(text)
                taken.append(frame)
                await connection.send(answered(frame))
                if frame["cmd"] == "aibot_subscribe":
                    await connection.send(json.dumps(message("你好", message_id="m1")))
        finally:
            codes.append(connection.close_code)

    place, stop = serving(wecom)
    heard: list[dict[str, Any]] = []
    told: list[tuple[str, str | None]] = []
    link = Link(
        place,
        BOT_ID,
        SECRET,
        heard=heard.append,
        told=lambda state, why: told.append((state, why)),
        sleep=Pauses(),
        ticking=Ticks(),
    )
    task = asyncio.ensure_future(link.run())
    try:
        await until(lambda: bool(heard), 10)
        answer = await link.call(RESPOND, {"msgtype": "markdown"}, "req-m1")
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await until(lambda: bool(codes), 10)
    finally:
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        stop()

    assert [frame["cmd"] for frame in taken] == ["aibot_subscribe", RESPOND]
    assert heard == [message("你好", message_id="m1")]
    assert answer == {"headers": {"req_id": "req-m1"}, "errcode": 0}
    # Stopped, the connection ended as it was meant to.
    assert codes == [1000]
