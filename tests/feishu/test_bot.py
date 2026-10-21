"""The assistant in Feishu: ``columbina.feishu.bot``."""

import asyncio
import importlib.util
import json
import logging
from collections.abc import Callable
from pathlib import Path
from typing import Any

import httpx
import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
import columbina.skills
from columbina.agent import InboxItem
from columbina.agent.inbox import enqueue
from columbina.apps import FRONTEND, common
from columbina.feishu import bot as bot_module
from columbina.feishu import store
from columbina.feishu.app import FeishuError, Host
from columbina.feishu.bot import MOVED, SEEN, Assistant
from columbina.feishu.client import Feishu
from columbina.llm import LlmFailure, TextBlock, UserMessage
from columbina.llm.adapters.scripted import Reply
from columbina.permissions import PERMISSIONS
from columbina.session import SessionError
from columbina.session.store import SESSION_FILE
from support.fake_feishu import SECRET, FakeFeishu, done, refused
from support.links import link_directory
from support.loop import MODEL, Harness

OWNER = "ou_owner"
STRANGER = "ou_stranger"
FEISHU = Host("feishu", "https://open.feishu.cn")


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    """Wait for something that the assistant does in its own time."""
    async with asyncio.timeout(seconds):
        # What is waited for is done by the assistant, which sets no event a test can see.
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.002)


class FakeLink:
    """The long connection, as far as the assistant uses it: a test gives it the events."""

    def __init__(
        self,
        host: Host,
        app_id: str,
        secret: str,
        http: httpx.AsyncClient,
        *,
        heard: Callable[[dict[str, Any]], None],
        told: Callable[[str, str | None], None],
        env: object,
    ) -> None:
        self.heard = heard
        self.told = told
        self.app_id = app_id
        self.ended = asyncio.Event()
        self.failing: FeishuError | None = None

    async def run(self) -> None:
        self.told("connecting", None)
        self.told("connected", None)
        await self.ended.wait()
        if self.failing is not None:
            raise self.failing


def event(
    text: str | None = None,
    *,
    message_id: str,
    sender: str = OWNER,
    chat_type: str = "p2p",
    sender_type: str = "user",
    kind: str = "text",
    content: object = None,
    event_type: str = "im.message.receive_v1",
) -> dict[str, Any]:
    said = content if content is not None else {"text": text or ""}
    return {
        "schema": "2.0",
        "header": {"event_id": f"e-{message_id}", "event_type": event_type},
        "event": {
            "sender": {"sender_id": {"open_id": sender}, "sender_type": sender_type},
            "message": {
                "message_id": message_id,
                "chat_id": "oc_1",
                "chat_type": chat_type,
                "message_type": kind,
                "content": json.dumps(said, ensure_ascii=False),
            },
        },
    }


class Rig:
    """An assistant at work in a test, with Feishu, its long connection and a scripted model."""

    def __init__(self, harness: Harness, home: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        self.harness = harness
        self.home = home
        self.folder = home / store.FOLDER
        # The directory of the assistant, apart from the home of the program.
        self.cwd = home.parent / "Columbina" / "feishu"
        self.files: dict[str, httpx.Response] = {}
        self.reactions = 0
        self.react: Callable[[httpx.Request], httpx.Response] | None = None
        self.send: Callable[[httpx.Request], httpx.Response] | None = None
        self.feishu = FakeFeishu(self._answer)
        self.links: list[FakeLink] = []
        self.task: asyncio.Task[int] | None = None
        self.assistant: Assistant | None = None
        self.hanging: list[str] = []
        self.hung = 0

        async def answered(request: httpx.Request) -> httpx.Response:
            if any(part in request.url.path for part in self.hanging):
                # Feishu that takes a request and never answers it.
                self.hung += 1
                await asyncio.Event().wait()
            return self.feishu(request)

        transport = httpx.MockTransport(answered)

        async def no_pause(seconds: float) -> None:
            return None

        def client(host: Host, app_id: str, secret: str, env: object) -> Feishu:
            return Feishu(host, app_id, secret, {}, transport=transport, sleep=no_pause)

        def linked(*args: Any, **kwargs: Any) -> FakeLink:
            made = FakeLink(*args, **kwargs)
            self.links.append(made)
            return made

        monkeypatch.setattr(bot_module, "Feishu", client)
        monkeypatch.setattr(bot_module, "Link", linked)

    def _answer(self, request: httpx.Request) -> httpx.Response:
        path = request.url.path
        if path.endswith("/reply") or path == "/open-apis/im/v1/messages":
            return self.send(request) if self.send else done({"message_id": "om_answer"})
        if path.endswith("/reactions") and request.method == "POST":
            if self.react is not None:
                return self.react(request)
            self.reactions += 1
            return done({"reaction_id": f"r{self.reactions}"})
        if "/reactions/" in path and request.method == "DELETE":
            return done({})
        if "/resources/" in path:
            key = path.rsplit("/", 1)[-1]
            return self.files.get(key, refused(234003, "no such file"))
        return refused(99999999, f"nothing answers {request.method} {path}")

    def set_up(self, **changes: object) -> None:
        settings = store.Settings(domain="feishu", app_id="cli_a1b2c3d4").model_copy(update=changes)
        store.keep(self.home, settings, SECRET, FEISHU)

    async def start(self, **config: object) -> Assistant:
        await self.harness.ctx.plugin(
            bot_module,
            {"model": MODEL.model_dump(), "cwd": self.cwd, "home": self.home, **config},
        )
        assistant = self.harness.ctx.use(FRONTEND)
        assert isinstance(assistant, Assistant)
        self.assistant = assistant
        self.task = asyncio.ensure_future(assistant.run())
        await until(lambda: bool(self.links) or (self.task is not None and self.task.done()))
        return assistant

    @property
    def link(self) -> FakeLink:
        return self.links[-1]

    def say(self, text: str, message_id: str, sender: str = OWNER) -> None:
        self.link.heard(event(text, message_id=message_id, sender=sender))

    def replies(self) -> list[tuple[str, str]]:
        """What the assistant sent, as the message it replied to and the Markdown it said."""
        said: list[tuple[str, str]] = []
        for request in self.feishu.asked:
            path = request.url.path
            if path.endswith("/reply") or path == "/open-apis/im/v1/messages":
                body = json.loads(request.content)
                post = json.loads(body["content"])
                language = next(iter(post))
                where = path.split("/")[-2] if path.endswith("/reply") else "owner"
                said.append((where, post[language]["content"][0][0]["text"]))
        return said

    def requests(self, method: str, part: str) -> list[httpx.Request]:
        return [
            request
            for request in self.feishu.asked
            if request.method == method and part in request.url.path
        ]

    async def stop(self) -> int | BaseException:
        assert self.task is not None
        self.task.cancel()
        results = await asyncio.gather(self.task, return_exceptions=True)
        return results[0]


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


@pytest.fixture
def rig(harness: Harness, home: Path, monkeypatch: pytest.MonkeyPatch) -> Rig:
    return Rig(harness, home, monkeypatch)


async def paired(rig: Rig, **config: object) -> Assistant:
    rig.set_up(owner=OWNER)
    return await rig.start(**config)


# Starting


async def test_nothing_set_up_is_said_and_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "no app of Feishu is set up" in capsys.readouterr().err


async def test_an_app_without_its_secret_is_not_set_up(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    store.forget(rig.home)
    store.save(rig.home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"))

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "no app of Feishu is set up" in capsys.readouterr().err


async def test_an_app_that_cannot_be_read_is_said_to_be_so(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    (rig.home / store.SETTINGS).mkdir(parents=True)

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "cannot be read" in capsys.readouterr().err


async def test_the_package_of_websockets_is_asked_for_where_it_is_missing(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)

    def missing(name: str, package: str | None = None) -> None:
        return None

    monkeypatch.setattr(importlib.util, "find_spec", missing)

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "pip install columbina[feishu]" in capsys.readouterr().err


async def test_a_second_assistant_does_not_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    rig.folder.mkdir(parents=True)
    held = store_lock(rig.folder)

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "only one may run" in capsys.readouterr().err
    held.release()


def store_lock(folder: Path) -> Any:
    from columbina.session.lock import acquire

    held = acquire(folder / store.LOCK)
    assert held is not None
    return held


async def test_a_directory_that_cannot_be_used_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    rig.folder.write_text("a file where the directory would be", encoding="utf-8")

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


async def test_a_session_that_cannot_be_opened_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER, session_id="../outside")

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 1
    assert "the session of the assistant cannot be opened" in capsys.readouterr().err
    assert store.status(rig.home) is not None


async def test_a_running_assistant_says_how_it_is_doing(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    await paired(rig)

    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.paired, told.code) == ("connected", True, None)
    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.session_id is not None
    assert await rig.stop() is not None
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "stopped"
    # A person at a terminal reads what is said for errors, and no lines of JSON.
    assert capsys.readouterr().out == ""


async def test_feishu_that_refuses_the_app_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    await paired(rig)
    rig.link.failing = FeishuError("Feishu opens no connection for this app", code="REFUSED")
    rig.link.ended.set()

    assert rig.task is not None
    assert await rig.task == 1
    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.error) == ("failed", "Feishu opens no connection for this app")
    assert "Feishu opens no connection" in capsys.readouterr().err


async def test_a_lost_connection_is_told(rig: Rig) -> None:
    assistant = await paired(rig)

    rig.link.told("connecting", "the connection was closed")

    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.error) == ("connecting", "the connection was closed")
    del assistant
    await rig.stop()


# For another program


@pytest.fixture
def input_closes(monkeypatch: pytest.MonkeyPatch) -> asyncio.Event:
    """The end of the standard input, which the program that runs the assistant brings about."""
    closed = asyncio.Event()
    monkeypatch.setattr(bot_module, "when_the_input_closes", lambda: closed)
    return closed


def lines_of(out: str) -> list[dict[str, Any]]:
    return [json.loads(line) for line in out.splitlines()]


async def test_a_program_that_runs_the_assistant_hears_how_it_is_doing_and_stops_it(
    rig: Rig, input_closes: asyncio.Event, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    await rig.start(json_lines=True)
    await until(lambda: getattr(store.status(rig.home), "state", None) == "connected")

    input_closes.set()

    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0
    said = lines_of(capsys.readouterr().out)
    assert [line["state"] for line in said] == ["connecting", "connecting", "connected", "stopped"]
    code = said[0]["code"]
    assert isinstance(code, str)
    assert len(code) == 6
    assert code.isdigit()
    assert {(line["paired"], line["error"]) for line in said} == {(False, None)}
    # Each line says which app it is of, so that a program knows when the app was changed.
    assert {(line["domain"], line["app_id"]) for line in said} == {("feishu", "cli_a1b2c3d4")}
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "stopped"


async def test_a_program_hears_why_feishu_refused_the_app(
    rig: Rig, input_closes: asyncio.Event, capsys: pytest.CaptureFixture[str]
) -> None:
    await paired(rig, json_lines=True)
    rig.link.failing = FeishuError("Feishu opens no connection for this app", code="REFUSED")
    rig.link.ended.set()

    assert rig.task is not None
    assert await rig.task == 1
    last = lines_of(capsys.readouterr().out)[-1]
    assert last == {
        "state": "failed",
        "paired": True,
        "code": None,
        "error": "Feishu opens no connection for this app",
        "domain": "feishu",
        "app_id": "cli_a1b2c3d4",
    }


async def test_a_program_that_went_away_does_not_stop_the_assistant(
    rig: Rig,
    input_closes: asyncio.Event,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    class Gone:
        def write(self, text: str) -> int:
            raise BrokenPipeError("the pipe is closed")

        def flush(self) -> None:
            raise BrokenPipeError("the pipe is closed")

    caplog.set_level(logging.INFO)
    monkeypatch.setattr("sys.stdout", Gone())
    await paired(rig, json_lines=True)
    await until(lambda: getattr(store.status(rig.home), "state", None) == "connected")

    input_closes.set()

    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0
    assert "how the assistant is doing cannot be said: the pipe is closed" in caplog.text


# Pairing


async def test_whoever_sends_the_code_first_is_the_owner(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    assistant = await rig.start()
    told = store.status(rig.home)
    assert told is not None
    assert told.code is not None
    assert f"send its bot this code in Feishu: {told.code}" in capsys.readouterr().err

    rig.say("123", "om_1", STRANGER)
    rig.say("456", "om_2", STRANGER)
    rig.say(told.code, "om_3", OWNER)
    await until(lambda: len(rig.replies()) == 2)

    assert rig.replies()[0] == (
        "om_1",
        "这是一个私人助手。请发送配对码，配对码在 Columbina 里能看到。",
    )
    assert rig.replies()[1][0] == "om_3"
    assert rig.replies()[1][1].startswith("配对成功")
    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.owner == OWNER
    after = store.status(rig.home)
    assert after is not None
    assert (after.paired, after.code) == (True, None)
    del assistant
    await rig.stop()


async def test_a_code_that_is_too_old_is_replaced(rig: Rig) -> None:
    rig.set_up()
    assistant = await rig.start(pairing_minutes=1)
    old = store.status(rig.home)
    assert old is not None
    assistant._now = lambda: 10**12  # pyright: ignore[reportPrivateUsage]

    rig.say(old.code or "", "om_1", OWNER)
    await until(lambda: len(rig.replies()) == 1)

    new = store.status(rig.home)
    assert new is not None
    assert new.code != old.code
    assert rig.replies()[0][1].startswith("这是一个私人助手")
    await rig.stop()


async def test_too_many_wrong_codes_give_the_code_up(rig: Rig) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None

    for number in range(21):
        rig.say("000000" if told.code != "000000" else "111111", f"om_{number}", f"ou_{number}")
    await until(lambda: (store.status(rig.home) or told).code is None)
    rig.say(told.code or "", "om_last", OWNER)
    await asyncio.sleep(0.05)

    after = store.status(rig.home)
    assert after is not None
    assert after.error is not None
    assert "given up on" in after.error
    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.owner is None
    await rig.stop()


async def test_one_sender_that_sends_wrong_codes_again_and_again_gives_the_code_up(
    rig: Rig,
) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    wrong = "000000" if told.code != "000000" else "111111"

    for number in range(4):
        rig.say(wrong, f"om_{number}", STRANGER)
    await until(lambda: len(rig.replies()) == 1)
    await asyncio.sleep(0.05)
    still = store.status(rig.home)
    assert still is not None
    assert still.code == told.code
    rig.say(wrong, "om_4", STRANGER)
    await until(lambda: (store.status(rig.home) or told).code is None)
    rig.say(told.code or "", "om_5", STRANGER)
    await asyncio.sleep(0.05)

    after = store.status(rig.home)
    assert after is not None
    assert after.error is not None
    assert "given up on" in after.error
    # The fifth wrong code gives it up, and the right one does nothing after that.
    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.owner is None
    # The one that was asked for the code once is not asked again.
    assert rig.replies() == [("om_0", rig.replies()[0][1])]
    await rig.stop()


async def test_the_right_code_after_four_wrong_ones_pairs(rig: Rig) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    wrong = "000000" if told.code != "000000" else "111111"

    for number in range(4):
        rig.say(wrong, f"om_{number}")
    rig.say(told.code or "", "om_4")
    await until(lambda: len(rig.replies()) == 2)

    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.owner == OWNER
    await rig.stop()


async def test_a_pairing_that_cannot_be_written_down_is_not_made(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    assert told.code is not None
    kept = store.save

    def unwritable(home: Path, chosen: store.Settings) -> None:
        raise FeishuError("no room", code="STORE")

    monkeypatch.setattr(store, "save", unwritable)
    rig.say(told.code, "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "配对没能记下来：no room。请过一会儿再发一次配对码。")]
    still = store.status(rig.home)
    assert still is not None
    assert (still.paired, still.code) == (False, told.code)
    monkeypatch.setattr(store, "save", kept)
    rig.say(told.code, "om_2")
    await until(lambda: len(rig.replies()) == 2)
    settings = store.settings(rig.home)
    assert settings is not None
    assert settings.owner == OWNER
    await rig.stop()


# The conversation


async def test_the_owner_is_answered_with_a_reply_to_the_message(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="## 你好\n\n- 一"))
    await paired(rig)

    rig.say("你好", "om_1")
    await until(lambda: bool(rig.replies()))
    await until(lambda: bool(rig.requests("DELETE", "/reactions/")))

    assert rig.replies() == [("om_1", "## 你好\n\n- 一")]
    added = rig.requests("POST", "/open-apis/im/v1/messages/om_1/reactions")
    assert json.loads(added[0].content) == {"reaction_type": {"emoji_type": "Typing"}}
    assert rig.requests("DELETE", "/open-apis/im/v1/messages/om_1/reactions/r1")
    await rig.stop()


async def test_each_message_is_answered_in_a_turn_of_its_own(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="first", hold=True), Reply(text="second"))
    await paired(rig)

    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "om_2")
    rig.harness.model.release()
    await until(lambda: len(rig.replies()) == 2)

    assert rig.replies() == [("om_1", "first"), ("om_2", "second")]
    await rig.stop()


@pytest.mark.parametrize(
    "heard",
    [
        event("hi", message_id="om_g", chat_type="group"),
        event("hi", message_id="om_b", sender_type="bot"),
        event("hi", message_id="om_s", sender=STRANGER),
        event("hi", message_id="om_e", event_type="im.message.read_v1"),
        event("   ", message_id="om_blank"),
        {"header": {"event_type": "im.message.receive_v1"}, "event": {"message": {}}},
        event(content={"text": "hi"}, message_id="om_n", sender=""),
    ],
)
async def test_what_is_not_the_owner_writing_to_the_bot_is_passed_over(
    rig: Rig, heard: dict[str, Any]
) -> None:
    await paired(rig)

    rig.link.heard(heard)
    await asyncio.sleep(0.05)

    assert rig.feishu.asked == []
    await rig.stop()


async def test_a_message_that_comes_again_is_answered_once_also_after_a_restart(
    rig: Rig,
) -> None:
    rig.harness.model.add(Reply(text="once"))
    await paired(rig)
    rig.say("hi", "om_1")
    await until(lambda: len(rig.replies()) == 1)

    rig.say("hi", "om_1")
    await asyncio.sleep(0.05)

    assert len(rig.replies()) == 1
    assert json.loads((rig.folder / SEEN).read_text(encoding="utf-8")) == ["om_1"]
    await rig.stop()


async def test_messages_that_were_taken_are_known_from_their_file(rig: Rig) -> None:
    rig.folder.mkdir(parents=True)
    (rig.folder / SEEN).write_text(json.dumps(["om_1", 3, "om_2"]), encoding="utf-8")
    await paired(rig)

    rig.say("hi", "om_2")
    await asyncio.sleep(0.05)

    assert rig.feishu.asked == []
    await rig.stop()


async def test_a_file_of_taken_messages_that_cannot_be_read_or_written_is_let_go(
    rig: Rig, caplog: pytest.LogCaptureFixture
) -> None:
    rig.harness.model.add(Reply(text="still"))
    rig.folder.mkdir(parents=True)
    (rig.folder / SEEN).mkdir()
    await paired(rig)

    rig.say("hi", "om_1")
    await until(lambda: len(rig.replies()) == 1)

    assert "which messages were taken cannot be written down" in caplog.text
    # The message is answered all the same, and what that means is said.
    told = store.status(rig.home)
    assert told is not None
    assert told.error is not None
    assert "may be answered twice" in told.error
    await rig.stop()


async def test_a_new_session_that_cannot_be_opened_is_said_and_can_be_tried_again(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.harness.model.add(Reply(text="here again"))
    await paired(rig)
    opened = common.open_agent

    async def failing(*given: Any) -> Any:
        raise SessionError("the store is full")

    monkeypatch.setattr("columbina.feishu.bot.open_agent", failing)
    rig.say("/new", "om_1")
    await until(lambda: bool(rig.replies()))
    rig.say("hi", "om_2")
    await until(lambda: len(rig.replies()) == 2)
    monkeypatch.setattr("columbina.feishu.bot.open_agent", opened)
    rig.say("/new", "om_3")
    rig.say("hi", "om_4")
    await until(lambda: len(rig.replies()) == 4)

    assert rig.replies() == [
        ("om_1", "没能开始新对话：the store is full。可以再发一次 /new 试试。"),
        ("om_2", "现在没有可用的对话。发 /new 开始一个新对话。"),
        ("om_3", "已开始新对话。"),
        ("om_4", "here again"),
    ]
    await rig.stop()


async def test_an_answer_that_goes_out_again_clears_what_went_wrong(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="lost"))
    rig.harness.model.add(Reply(text="found"))
    await paired(rig)
    rig.send = lambda request: refused(230002, "the bot is not in the chat")

    rig.say("hi", "om_1")
    await until(
        lambda: (getattr(store.status(rig.home), "error", None) or "").startswith("an answer")
    )
    rig.send = None
    rig.say("again", "om_2")
    await until(lambda: getattr(store.status(rig.home), "error", "") is None)
    await rig.stop()


async def test_a_stop_is_taken_while_an_answer_waits_for_feishu(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.hanging.append("/reply")

    rig.say("/help", "om_2")
    await until(lambda: rig.hung == 1)
    rig.say("/stop", "om_3")
    await until(lambda: rig.hung == 2)

    live = rig.harness.agents.live
    await until(lambda: all(one.at_rest for one in live.values()))
    await asyncio.wait_for(rig.stop(), timeout=10)


@pytest.mark.parametrize(
    ("reply", "said"),
    [
        (
            Reply(failure=LlmFailure(message="the key is wrong", code="AUTH", status=401)),
            "这次没能回答",
        ),
        (Reply(text="   "), "（这次没有写出回答。）"),
    ],
)
async def test_a_turn_that_did_not_end_well_says_so(rig: Rig, reply: Reply, said: str) -> None:
    rig.harness.model.add(reply)
    await paired(rig)

    rig.say("hi", "om_1")
    await until(lambda: bool(rig.replies()))

    assert said in rig.replies()[0][1]
    await rig.stop()


async def test_a_long_answer_is_sent_in_parts(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="\n".join(f"第 {number} 行的内容" for number in range(4000))))
    await paired(rig)

    rig.say("hi", "om_1")
    await until(lambda: len(rig.replies()) > 1 and bool(rig.requests("DELETE", "/reactions/")))

    keys = [json.loads(request.content)["uuid"] for request in rig.requests("POST", "/reply")]
    # Each part has a key of its own, and the same one when the answer is sent again.
    uuid = bot_module._uuid  # pyright: ignore[reportPrivateUsage]
    assert keys == [uuid("om_1", "turn 1", str(index)) for index in range(len(keys))]
    assert len(set(keys)) == len(rig.replies())
    await rig.stop()


async def test_an_answer_that_cannot_be_sent_is_told(
    rig: Rig, caplog: pytest.LogCaptureFixture
) -> None:
    rig.harness.model.add(Reply(text="lost"))
    await paired(rig)
    rig.send = lambda request: refused(230002, "the bot is not in the chat")

    rig.say("hi", "om_1")
    await until(lambda: "an answer could not be sent to Feishu" in caplog.text)

    told = store.status(rig.home)
    assert told is not None
    assert told.error is not None
    assert "the bot is not in the chat" in told.error
    await rig.stop()


async def test_what_waited_when_the_program_ended_is_answered_to_the_owner(
    rig: Rig, harness: Harness
) -> None:
    rig.set_up(owner=OWNER, session_id="s-left")
    spec = harness.spec(cwd=rig.cwd)
    rig.cwd.mkdir(parents=True)
    left = await harness.agents.create(harness.ctx, spec, session_id="s-left")
    # What waited when the program ended: input in the inbox, which no turn took.
    waiting = InboxItem(message=UserMessage(content=(TextBlock(text="are you there"),)))
    enqueue(left.session, "next-turn", waiting)
    await left.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("owner", "I am")]
    sent = rig.requests("POST", "/open-apis/im/v1/messages")[0]
    assert sent.url.params["receive_id_type"] == "open_id"
    assert json.loads(sent.content)["receive_id"] == OWNER
    await rig.stop()


async def test_the_bot_stops_showing_that_it_writes_where_the_app_may_not(
    rig: Rig, caplog: pytest.LogCaptureFixture
) -> None:
    rig.harness.model.add(Reply(text="a"), Reply(text="b"))
    await paired(rig)
    rig.react = lambda request: refused(99991672, "no permission")

    rig.say("one", "om_1")
    await until(lambda: len(rig.replies()) == 1)
    rig.say("two", "om_2")
    await until(lambda: len(rig.replies()) == 2)

    assert len(rig.requests("POST", "/reactions")) == 1
    assert "the bot cannot show that it is writing" in caplog.text
    await rig.stop()


async def test_a_reaction_that_fails_for_now_is_tried_with_the_next_message(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="a"), Reply(text="b"))
    await paired(rig)
    rig.react = lambda request: httpx.Response(429)

    rig.say("one", "om_1")
    await until(lambda: len(rig.replies()) == 1)
    rig.say("two", "om_2")
    await until(lambda: len(rig.replies()) == 2)

    assert len(rig.requests("POST", "/reactions")) == 2
    await rig.stop()


async def test_a_reaction_that_cannot_be_taken_away_is_let_go(
    rig: Rig, caplog: pytest.LogCaptureFixture
) -> None:
    rig.harness.model.add(Reply(text="a"))
    await paired(rig)
    rig.feishu.on("DELETE", "/open-apis/im/v1/messages/om_1/reactions/r1", refused(1, "gone"))

    rig.say("one", "om_1")
    await until(lambda: "the bot cannot stop showing that it is writing" in caplog.text)
    await rig.stop()


async def test_a_reaction_without_an_id_is_not_taken_away(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="a"))
    await paired(rig)
    rig.react = lambda request: done({})

    rig.say("one", "om_1")
    await until(lambda: len(rig.replies()) == 1)
    await asyncio.sleep(0.05)

    assert rig.requests("DELETE", "/reactions/") == []
    await rig.stop()


# Commands


async def test_help_says_what_there_is(rig: Rig) -> None:
    await paired(rig)

    rig.say("/help", "om_1")
    await until(lambda: bool(rig.replies()))

    assert "/new" in rig.replies()[0][1]
    await rig.stop()


async def test_a_command_that_is_not_known_says_so(rig: Rig) -> None:
    await paired(rig)

    rig.say("/what", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("不认识这个命令")
    await rig.stop()


async def test_new_begins_a_new_session(rig: Rig) -> None:
    await paired(rig)
    before = store.settings(rig.home)

    rig.say("/new", "om_1")
    await until(lambda: bool(rig.replies()))

    after = store.settings(rig.home)
    assert before is not None
    assert after is not None
    assert after.session_id != before.session_id
    assert rig.replies() == [("om_1", "已开始新对话。")]
    await rig.stop()


async def test_new_drops_what_is_being_answered(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    rig.say("/new", "om_2")
    rig.harness.model.release()
    await until(lambda: bool(rig.replies()))
    await until(lambda: bool(rig.requests("DELETE", "/reactions/")))

    assert rig.replies() == [("om_2", "已开始新对话，没答完的 1 条消息不再回答。")]
    await rig.stop()


async def test_stop_with_nothing_to_stop_says_so(rig: Rig) -> None:
    await paired(rig)

    rig.say("/stop", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "现在没有在写的回答。")]
    await rig.stop()


async def test_stop_stops_the_answer_and_what_waits(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "om_2")
    await until(lambda: len(rig.requests("POST", "/reactions")) == 2)

    rig.say("/stop", "om_3")
    await until(lambda: bool(rig.replies()))
    await until(lambda: len(rig.requests("DELETE", "/reactions/")) == 2)

    assert rig.replies() == [("om_3", "已停下，排在后面的 1 条消息也不再回答。")]
    live = rig.harness.agents.live
    await until(lambda: all(one.at_rest for one in live.values()))
    # What is asked after the stop is answered as a reply to itself, and not to what waited.
    rig.harness.model.add(Reply(text="three it is"))
    rig.say("three", "om_4")
    await until(lambda: len(rig.replies()) == 2)
    assert rig.replies()[1] == ("om_4", "three it is")
    await rig.stop()


async def test_stop_stops_the_answer_alone_where_nothing_waits(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    rig.say("/stop", "om_2")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_2", "已停下。")]
    await rig.stop()


async def test_skills_without_the_plugin_of_skills_are_none(rig: Rig) -> None:
    await paired(rig)

    rig.say("/skills", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("还没有技能")
    await rig.stop()


SKILL = (
    b"---\nname: greet\ndescription: Greets the user warmly in their language.\n---\nSay hello.\n"
)


async def test_skills_are_listed(rig: Rig) -> None:
    skills = rig.cwd / "skills"
    (skills / "greet").mkdir(parents=True)
    (skills / "greet" / "SKILL.md").write_bytes(SKILL)
    await rig.harness.ctx.plugin(columbina.skills, {"home": skills})
    await paired(rig)

    rig.say("/skills", "om_1")
    await until(lambda: bool(rig.replies()))

    assert (
        rig.replies()[0][1]
        == "我有这些技能：\n\n- greet: Greets the user warmly in their language."
    )
    await rig.stop()


async def test_a_skill_of_the_assistant_is_deleted(rig: Rig) -> None:
    (rig.cwd / "skills" / "greet").mkdir(parents=True)
    (rig.cwd / "skills" / "greet" / "SKILL.md").write_bytes(SKILL)
    await paired(rig)

    rig.say("/skills rm greet", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "已删掉技能 greet。")]
    assert not (rig.cwd / "skills" / "greet").exists()
    await rig.stop()


@pytest.mark.parametrize("named", ["", "../escape", "missing"])
async def test_a_skill_that_is_not_there_is_not_deleted(rig: Rig, named: str) -> None:
    await paired(rig)

    rig.say(f"/skills rm {named}".strip(), "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1] == f"我自己的技能目录里没有叫 {named or '?'} 的技能。"
    await rig.stop()


async def test_a_name_that_leads_out_of_the_skills_deletes_nothing(rig: Rig) -> None:
    outside = rig.cwd / "escape"
    outside.mkdir(parents=True)
    (outside / "SKILL.md").write_bytes(SKILL)
    await paired(rig)

    rig.say("/skills rm ../escape", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1] == "我自己的技能目录里没有叫 ../escape 的技能。"
    assert (outside / "SKILL.md").read_bytes() == SKILL
    await rig.stop()


async def test_a_skill_with_other_files_is_left_for_the_user_to_delete(rig: Rig) -> None:
    skill = rig.cwd / "skills" / "greet"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_bytes(SKILL)
    (skill / "notes.txt").write_text("kept", encoding="utf-8")
    await paired(rig)

    rig.say("/skills rm greet", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("技能 greet 的目录里还有别的文件")
    assert (skill / "SKILL.md").exists()
    await rig.stop()


# A directory of its own


async def test_what_was_its_own_in_the_home_of_the_program_comes_along(
    rig: Rig, harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    # Before, the assistant worked where the program keeps its lock and how it is doing.
    rig.set_up(owner=OWNER, session_id="s-before")
    before = rig.folder
    (before / "skills" / "greet").mkdir(parents=True)
    (before / "skills" / "greet" / "SKILL.md").write_bytes(SKILL)
    (before / "AGENTS.md").write_text("Be kind.", encoding="utf-8")
    (before / "uploads" / "om_1").mkdir(parents=True)
    (before / "uploads" / "om_1" / "notes.md").write_text("notes", encoding="utf-8")
    (before / SEEN).write_text(json.dumps(["om_old"]), encoding="utf-8")
    (before / store.STATUS).write_text("{}", encoding="utf-8")
    left = await harness.agents.create(harness.ctx, harness.spec(cwd=before), session_id="s-before")
    waiting = InboxItem(message=UserMessage(content=(TextBlock(text="are you there"),)))
    enqueue(left.session, "next-turn", waiting)
    await left.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.replies()))

    copied = sorted(
        path.relative_to(rig.cwd).as_posix() for path in rig.cwd.rglob("*") if path.is_file()
    )
    assert copied == ["AGENTS.md", "skills/greet/SKILL.md", "uploads/om_1/notes.md"]
    # What was there stays where it was.
    assert (before / "skills" / "greet" / "SKILL.md").read_bytes() == SKILL
    # The conversation goes on in the directory of the assistant, and what was taken stays known.
    assert rig.replies() == [("owner", "I am")]
    rig.say("again", "om_old")
    await asyncio.sleep(0.05)
    assert rig.replies() == [("owner", "I am")]
    assert f"is copied to {rig.cwd}, where it works now" in capsys.readouterr().err
    # It is noted that it came along, and where to.
    assert (before / MOVED).read_text(encoding="utf-8") == f"{rig.cwd}\n"
    await rig.stop()


def waiting(text: str) -> InboxItem:
    """A message that was taken and not answered yet."""
    return InboxItem(message=UserMessage(content=(TextBlock(text=text),)))


async def test_a_new_assistant_brings_nothing_along(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)

    await rig.start()

    assert list(rig.cwd.iterdir()) == []
    assert "is copied to" not in capsys.readouterr().err
    # Nothing is looked for again.
    assert (rig.folder / MOVED).exists()
    await rig.stop()


async def test_a_session_alone_comes_along(
    rig: Rig, harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER, session_id="s-before")
    rig.folder.mkdir(parents=True)
    left = await harness.agents.create(
        harness.ctx, harness.spec(cwd=rig.folder), session_id="s-before"
    )
    enqueue(left.session, "next-turn", waiting("are you there"))
    await left.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("owner", "I am")]
    assert f"is copied to {rig.cwd}, where it works now" in capsys.readouterr().err
    await rig.stop()


async def test_a_session_that_is_stored_nowhere_begins_anew(rig: Rig) -> None:
    rig.set_up(owner=OWNER, session_id="s-gone")

    assistant = await rig.start()

    agent = assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None
    assert agent.id == "s-gone"
    await rig.stop()


async def test_what_was_its_own_comes_along_once(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    rig.folder.mkdir(parents=True)
    (rig.folder / "AGENTS.md").write_text("Be kind.", encoding="utf-8")
    # It came along before, and its owner has cleared its directory since.
    (rig.folder / MOVED).write_text(f"{rig.cwd}\n", encoding="utf-8")

    await rig.start()

    assert list(rig.cwd.iterdir()) == []
    assert "is copied to" not in capsys.readouterr().err
    await rig.stop()


async def test_a_session_that_came_along_before_goes_on_as_it_is(
    rig: Rig, harness: Harness
) -> None:
    # The start before was cut off after its session came along, and before that was noted.
    rig.set_up(owner=OWNER, session_id="s-before")
    rig.folder.mkdir(parents=True)
    left = await harness.agents.create(
        harness.ctx, harness.spec(cwd=rig.folder), session_id="s-before"
    )
    enqueue(left.session, "next-turn", waiting("are you there"))
    await left.close()
    rig.cwd.mkdir(parents=True)
    going = await harness.agents.create(
        harness.ctx, harness.spec(cwd=rig.cwd), session_id="s-before"
    )
    enqueue(going.session, "next-turn", waiting("hello again"))
    await going.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.replies()))

    log = (harness.sessions.directory(rig.cwd, "s-before") / SESSION_FILE).read_bytes()
    assert b"hello again" in log
    assert b"are you there" not in log
    await rig.stop()


async def test_a_session_whose_copy_was_cut_off_comes_along_whole(
    rig: Rig, harness: Harness
) -> None:
    rig.set_up(owner=OWNER, session_id="s-before")
    rig.folder.mkdir(parents=True)
    left = await harness.agents.create(
        harness.ctx, harness.spec(cwd=rig.folder), session_id="s-before"
    )
    enqueue(left.session, "next-turn", waiting("are you there"))
    await left.close()
    moved = harness.sessions.directory(rig.cwd, "s-before")
    half = moved.with_name("s-before.copying")
    half.mkdir(parents=True)
    (half / "half").write_text("half", encoding="utf-8")
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("owner", "I am")]
    assert not half.exists()
    assert not (moved / "half").exists()
    await rig.stop()


async def test_files_of_the_owner_named_as_what_the_program_keeps_come_along(rig: Rig) -> None:
    rig.set_up(owner=OWNER)
    sent = rig.folder / "uploads" / "om_1"
    sent.mkdir(parents=True)
    names = sorted([store.LOCK, store.STATUS, SEEN])
    for name in names:
        (sent / name).write_text(name, encoding="utf-8")

    await rig.start()

    assert sorted(path.name for path in (rig.cwd / "uploads" / "om_1").iterdir()) == names
    await rig.stop()


async def test_a_link_where_it_worked_before_stays_behind(rig: Rig, tmp_path: Path) -> None:
    rig.set_up(owner=OWNER)
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "private.md").write_text("private", encoding="utf-8")
    rig.folder.mkdir(parents=True)
    link_directory(rig.folder / "elsewhere", outside)
    (rig.folder / "AGENTS.md").write_text("Be kind.", encoding="utf-8")

    await rig.start()

    assert [path.name for path in rig.cwd.iterdir()] == ["AGENTS.md"]
    await rig.stop()


async def test_an_empty_directory_of_the_assistant_gets_what_was_its_own(rig: Rig) -> None:
    rig.set_up(owner=OWNER)
    rig.folder.mkdir(parents=True)
    (rig.folder / "AGENTS.md").write_text("Be kind.", encoding="utf-8")
    rig.cwd.mkdir(parents=True)

    await rig.start()

    assert (rig.cwd / "AGENTS.md").read_text(encoding="utf-8") == "Be kind."
    await rig.stop()


async def test_a_file_in_the_directory_of_the_assistant_is_left_as_it_is(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    rig.folder.mkdir(parents=True)
    (rig.folder / "AGENTS.md").write_text("Be kind.", encoding="utf-8")
    (rig.folder / "skills" / "greet").mkdir(parents=True)
    (rig.folder / "skills" / "greet" / "SKILL.md").write_bytes(SKILL)
    rig.cwd.mkdir(parents=True)
    (rig.cwd / "AGENTS.md").write_text("Be brief.", encoding="utf-8")
    # A start that was cut off left half the copy of a file.
    (rig.cwd / "skills" / "greet").mkdir(parents=True)
    (rig.cwd / "skills" / "greet" / "SKILL.md.copying").write_bytes(b"half")

    await rig.start()

    assert (rig.cwd / "AGENTS.md").read_text(encoding="utf-8") == "Be brief."
    assert [path.name for path in (rig.cwd / "skills" / "greet").iterdir()] == ["SKILL.md"]
    assert (rig.cwd / "skills" / "greet" / "SKILL.md").read_bytes() == SKILL
    assert "is copied to" in capsys.readouterr().err
    await rig.stop()


async def test_an_assistant_that_works_in_the_home_of_the_program_moves_nothing(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)

    await rig.start(cwd=rig.folder / ".")

    assert not rig.cwd.exists()
    assert "is copied to" not in capsys.readouterr().err
    await rig.stop()


async def test_a_directory_of_the_assistant_in_the_one_it_worked_in_is_not_copied_into(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    inside = rig.folder / "work"
    inside.mkdir(parents=True)
    (rig.folder / "AGENTS.md").write_text("Be kind.", encoding="utf-8")

    await rig.start(cwd=inside)

    assert rig.task is not None
    assert not rig.task.done()
    assert list(inside.iterdir()) == []
    assert "is copied to" not in capsys.readouterr().err
    await rig.stop()


# What its agent may do


async def test_an_assistant_started_to_change_files_that_its_owner_does_not_let_does_not_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)

    await rig.start(writable=True)

    assert rig.task is not None
    assert await rig.task == 2
    assert "which its owner does not let it do. Start it again" in capsys.readouterr().err
    assert not rig.cwd.exists()


async def test_an_assistant_whose_directory_lies_in_the_home_of_the_program_does_not_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER, writable=True)

    await rig.start(writable=True, cwd=rig.home / "assistant")

    assert rig.task is not None
    assert await rig.task == 2
    assert "lies in the home of the program, where it may change no file" in (
        capsys.readouterr().err
    )


async def kept_in_check(rig: Rig, preset: str) -> None:
    """Load what keeps a session in check, with the preset of the program as a whole."""
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await rig.harness.ctx.plugin(plugin)
    await rig.harness.ctx.plugin(columbina.permissions, {"preset": preset})


async def test_the_agent_of_the_assistant_changes_nothing_whatever_others_may_do(
    rig: Rig, harness: Harness
) -> None:
    await kept_in_check(rig, "full-access")

    assistant = await paired(rig)

    agent = assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None
    assert harness.ctx.use(PERMISSIONS).in_force(agent.session) == ("read-only", "ask")
    await rig.stop()


async def test_the_agent_that_its_owner_lets_change_files_changes_them_in_its_directory(
    rig: Rig, harness: Harness
) -> None:
    await kept_in_check(rig, "full-access")
    # Its conversation went on before, where it was let change nothing.
    rig.set_up(owner=OWNER, writable=True, session_id="s-before")
    rig.cwd.mkdir(parents=True)
    left = await harness.agents.create(
        harness.ctx, harness.spec(cwd=rig.cwd), session_id="s-before"
    )
    harness.ctx.use(PERMISSIONS).set_preset(left.session, "read-only")
    await left.close()

    assistant = await rig.start(writable=True)

    agent = assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None
    assert agent.id == "s-before"
    assert harness.ctx.use(PERMISSIONS).in_force(agent.session) == ("workspace-write", "ask")
    await rig.stop()


async def test_a_directory_of_the_assistant_that_cannot_be_made_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)
    rig.cwd.parent.mkdir(parents=True)
    rig.cwd.write_text("a file where the directory would be", encoding="utf-8")

    await rig.start()

    assert rig.task is not None
    assert await rig.task == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


# Files and skills


def file_event(name: str, message_id: str, key: str = "file_1") -> dict[str, Any]:
    return event(message_id=message_id, kind="file", content={"file_key": key, "file_name": name})


async def test_a_skill_that_is_sent_is_added(rig: Rig) -> None:
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content=SKILL)

    rig.link.heard(file_event("SKILL.md", "om_1"))
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "已装上技能 greet。")]
    assert (rig.cwd / "skills" / "greet" / "SKILL.md").read_bytes() == SKILL
    await rig.stop()


async def test_a_skill_that_is_sent_again_replaces_its_file_and_leaves_the_rest(
    rig: Rig,
) -> None:
    skill = rig.cwd / "skills" / "greet"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_bytes(b"old")
    (skill / "notes.txt").write_text("kept", encoding="utf-8")
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content=SKILL)

    rig.link.heard(file_event("greet.md", "om_1"))
    await until(lambda: bool(rig.replies()))

    said = rig.replies()[0][1]
    assert said.startswith("已更新技能 greet，换掉了原来的 SKILL.md。")
    assert "它们保持原样" in said
    assert (skill / "notes.txt").exists()
    await rig.stop()


async def test_a_skill_file_that_is_no_skill_says_why(rig: Rig) -> None:
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content=b"---\nname: Not A Name\n---\nx\n")

    rig.link.heard(file_event("SKILL.md", "om_1"))
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件不是能用的技能：")
    await rig.stop()


async def test_a_skill_that_cannot_be_written_says_so(rig: Rig) -> None:
    (rig.cwd / "skills").mkdir(parents=True)
    (rig.cwd / "skills" / "greet").write_text("a file", encoding="utf-8")
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content=SKILL)

    rig.link.heard(file_event("SKILL.md", "om_1"))
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件没能收下：")
    await rig.stop()


async def test_a_file_of_text_is_kept_and_the_agent_is_told_of_it(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="I read it"))
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content="会议纪要".encode())

    rig.link.heard(file_event("../纪要.md", "om_1"))
    await until(lambda: bool(rig.replies()))

    kept = rig.cwd / "uploads" / "om_1" / "纪要.md"
    assert kept.read_text(encoding="utf-8") == "会议纪要"
    assert rig.replies() == [("om_1", "I read it")]
    told = rig.harness.model.requests[0].messages[-1]
    assert isinstance(told, UserMessage)
    assert "uploads/om_1/纪要.md" in "".join(block.text for block in told.content)
    await rig.stop()


async def test_a_file_that_cannot_be_kept_says_so(rig: Rig) -> None:
    (rig.cwd / "uploads").mkdir(parents=True)
    (rig.cwd / "uploads" / "om_1").write_text("in the way", encoding="utf-8")
    await paired(rig)
    rig.files["file_1"] = httpx.Response(200, content=b"text")

    rig.link.heard(file_event("notes.txt", "om_1"))
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件没能收下：")
    await rig.stop()


@pytest.mark.parametrize(
    ("answer", "said"),
    [
        (httpx.Response(200, content=b"x" * (1024 * 1024 + 1)), "文件太大了"),
        (refused(234003, "no such file"), "这个文件没能收下："),
    ],
)
async def test_a_file_that_cannot_be_had_says_so(
    rig: Rig, answer: httpx.Response, said: str
) -> None:
    await paired(rig)
    rig.files["file_1"] = answer

    rig.link.heard(file_event("notes.txt", "om_1"))
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith(said)
    await rig.stop()


@pytest.mark.parametrize(
    "heard",
    [
        file_event("photo.png", "om_1"),
        event(message_id="om_1", kind="image", content={"image_key": "img_1"}),
    ],
)
async def test_what_is_no_text_is_said_to_be_so(rig: Rig, heard: dict[str, Any]) -> None:
    await paired(rig)

    rig.link.heard(heard)
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "我现在只看得懂文字和文本文件。")]
    await rig.stop()


# What is left


async def test_an_app_in_lark_speaks_english(rig: Rig) -> None:
    store.keep(
        rig.home,
        store.Settings(domain="lark", app_id="cli_a1b2c3d4", owner=OWNER),
        SECRET,
        Host("lark", "https://open.larksuite.com"),
    )
    await rig.start()

    rig.say("/stop", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "I am not writing an answer now.")]
    asked = rig.requests("POST", "/reply")[0]
    assert list(json.loads(json.loads(asked.content)["content"])) == ["en_us"]
    await rig.stop()


async def test_the_language_can_be_chosen(rig: Rig) -> None:
    await paired(rig, language="en")

    rig.say("/stop", "om_1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("om_1", "I am not writing an answer now.")]
    await rig.stop()


async def test_what_changed_of_the_app_that_cannot_be_written_down_is_logged(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    rig.set_up(owner=OWNER)

    def unwritable(home: Path, chosen: store.Settings) -> None:
        raise FeishuError("no room", code="STORE")

    monkeypatch.setattr(store, "save", unwritable)
    await rig.start()

    assert "what changed of the app cannot be written down: no room" in caplog.text
    await rig.stop()


@pytest.mark.parametrize("another", [True, False])
async def test_an_app_that_changed_while_the_assistant_ran_is_left_as_it_is(
    rig: Rig, caplog: pytest.LogCaptureFixture, another: bool
) -> None:
    await paired(rig)
    if another:
        store.keep(rig.home, store.Settings(domain="lark", app_id="cli_other1"), SECRET, FEISHU)
    else:
        store.forget(rig.home)

    rig.say("/new", "om_1")
    await until(lambda: bool(rig.replies()))

    after = store.settings(rig.home)
    assert (after.app_id if after else None) == ("cli_other1" if another else None)
    assert "the app changed while the assistant ran, so it is left as it is" in caplog.text
    await rig.stop()


async def test_a_message_that_cannot_be_taken_is_logged(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await paired(rig)

    async def broken(event: dict[str, Any]) -> None:
        raise RuntimeError("broken")

    assert rig.assistant is not None
    monkeypatch.setattr(rig.assistant, "_take", broken)
    rig.say("hi", "om_1")
    await until(lambda: "a message from Feishu could not be taken" in caplog.text)
    await rig.stop()


async def test_ending_takes_away_what_shows_that_the_bot_writes(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    await until(lambda: bool(rig.requests("POST", "/reactions")))

    await rig.stop()

    assert rig.requests("DELETE", "/reactions/r1")
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "stopped"


async def test_what_is_still_sent_when_the_assistant_stops_is_broken_off(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(bot_module, "_LAST_SECONDS", 0.05)
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.hanging.append("/reactions")
    rig.say("one", "om_1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    await until(lambda: rig.hung == 1)

    await asyncio.wait_for(rig.stop(), timeout=5)

    told = store.status(rig.home)
    assert told is not None
    assert told.state == "stopped"


async def test_an_agent_that_ends_badly_is_logged(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.ERROR)
    await paired(rig)
    assert rig.assistant is not None
    agent = rig.assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None

    async def broken() -> None:
        raise RuntimeError("will not close")

    monkeypatch.setattr(agent, "close", broken)
    await rig.stop()

    assert "the agent of the assistant stopped badly: will not close" in caplog.text


def test_a_name_of_a_file_is_made_safe() -> None:
    safe = bot_module._safe  # pyright: ignore[reportPrivateUsage]

    assert safe("../../etc/passwd") == "passwd"
    assert safe("C:\\Users\\a\\b<c>.txt") == "b_c_.txt"
    assert safe(" . ") == "file"
    assert len(safe("x" * 300 + ".md")) == 120


def test_a_key_of_a_message_is_the_same_each_time() -> None:
    uuid = bot_module._uuid  # pyright: ignore[reportPrivateUsage]

    assert uuid("om_1", "turn 1", "0") == uuid("om_1", "turn 1", "0")
    assert uuid("om_1", "turn 1", "0") != uuid("om_1", "turn 1", "1")
    assert len(uuid("om_1", "x", "0")) == 32


async def test_the_tool_that_reads_documents_has_the_client_while_connected(rig: Rig) -> None:
    assistant = await paired(rig)

    assert assistant.client() is not None
    await rig.stop()
    assert assistant.client() is None
