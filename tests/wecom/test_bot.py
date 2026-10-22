"""The assistant in WeCom: ``columbina.wecom.bot``."""

import asyncio
import importlib.util
import json
import logging
import sys
from collections.abc import Callable, Mapping
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
from columbina.apps import FRONTEND
from columbina.llm import LlmFailure, TextBlock, UserMessage
from columbina.llm.adapters.scripted import Reply
from columbina.permissions import PERMISSIONS
from columbina.session.lock import acquire
from columbina.wecom import bot as bot_module
from columbina.wecom import store
from columbina.wecom.app import WecomError
from columbina.wecom.bot import SEEN, Assistant
from columbina.wecom.link import RESPOND, SEND, WELCOME, Link
from support.fake_wecom import (
    BOT_ID,
    KEY,
    OWNER,
    PLACE,
    SECRET,
    STRANGER,
    FakeSocket,
    Opened,
    encrypted,
    event,
    message,
)
from support.loop import MODEL, Harness

SKILL = b"---\nname: greet\ndescription: Greets the user warmly.\n---\nSay hello.\n"
THINKING = "正在想……"


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    """Wait for something that the assistant does in its own time."""
    async with asyncio.timeout(seconds):
        # What is waited for is done by the assistant, which sets no event a test can see.
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.002)


class Pauses:
    """The pauses of the assistant: none, but the one after which an answer is written no more."""

    def __init__(self) -> None:
        self.taken: list[float] = []
        self.ends = asyncio.Event()

    async def __call__(self, seconds: float) -> None:
        self.taken.append(seconds)
        if seconds == 540.0:
            await self.ends.wait()
        else:
            await asyncio.sleep(0)


class Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


async def forever(seconds: float) -> None:
    await asyncio.Event().wait()


async def at_once(seconds: float) -> None:
    await asyncio.sleep(0)


class Rig:
    """An assistant at work in a test, with WeCom on the other end of its connection."""

    def __init__(self, harness: Harness, home: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        self.harness = harness
        self.home = home
        self.folder = home / store.FOLDER
        # The directory of the assistant, apart from the home of the program.
        self.cwd = home.parent / "Columbina" / "wecom"
        self.socket = FakeSocket()
        self.opened = Opened(self.socket)
        self.files: dict[str, httpx.Response] = {}
        self.pauses = Pauses()
        self.clock = Clock()
        self.assistant: Assistant | None = None
        self.task: asyncio.Task[int] | None = None

        def linked(*args: Any, **kwargs: Any) -> Link:
            made = Link(*args, **kwargs)
            made.opener = self.opened
            made.sleep = at_once
            # No ping goes out in a test.
            made.ticking = forever
            return made

        def downloaded(request: httpx.Request) -> httpx.Response:
            return self.files.get(str(request.url), httpx.Response(404))

        transport = httpx.MockTransport(downloaded)

        def downloads(env: Mapping[str, str]) -> httpx.AsyncClient:
            return httpx.AsyncClient(transport=transport)

        monkeypatch.setattr(bot_module, "Link", linked)
        monkeypatch.setattr(bot_module, "_downloads", downloads)

    def set_up(self, **changes: object) -> None:
        settings = store.Settings(bot_id=BOT_ID).model_copy(update=changes)
        store.keep(self.home, settings, SECRET, PLACE)

    async def start(self, **config: object) -> Assistant:
        await self.harness.ctx.plugin(
            bot_module,
            {"model": MODEL.model_dump(), "cwd": self.cwd, "home": self.home, **config},
        )
        assistant = self.harness.ctx.use(FRONTEND)
        assert isinstance(assistant, Assistant)
        assistant._pause = self.pauses  # pyright: ignore[reportPrivateUsage]
        assistant._clock = self.clock  # pyright: ignore[reportPrivateUsage]
        self.assistant = assistant
        self.task = asyncio.ensure_future(assistant.run())
        await until(self._ready)
        return assistant

    def _ready(self) -> bool:
        told = store.status(self.home)
        connected = told is not None and told.state == "connected"
        return connected or (self.task is not None and self.task.done())

    def say(self, text: str, message_id: str, sender: str = OWNER, **more: Any) -> None:
        self.socket.tell(message(text, message_id=message_id, sender=sender, **more))

    def answers(self) -> list[tuple[str, str, str, bool | None]]:
        """What the bot answered, as the message, the kind, the content and whether it ended."""
        said: list[tuple[str, str, str, bool | None]] = []
        for frame in self.socket.of(RESPOND):
            body = frame["body"]
            kind = body["msgtype"]
            if kind == "stream":
                stream = body["stream"]
                said.append((frame["headers"]["req_id"], kind, stream["content"], stream["finish"]))
            else:
                said.append((frame["headers"]["req_id"], kind, body[kind]["content"], None))
        return said

    def replies(self) -> list[tuple[str, str]]:
        """What the bot said in its last word to each message, and in messages of its own."""
        last = {
            req_id: content
            for req_id, _kind, content, finish in self.answers()
            if finish is not False
        }
        return [*last.items(), *(("owner", text) for text in self.pushed())]

    def pushed(self) -> list[str]:
        """What the bot sent its owner in messages of its own."""
        return [frame["body"]["markdown"]["content"] for frame in self.socket.of(SEND)]

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


def ended(rig: Rig) -> int:
    assert rig.task is not None
    assert rig.task.done()
    return rig.task.result()


# Starting


async def test_nothing_set_up_is_said_and_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    await rig.start()

    assert ended(rig) == 2
    assert "no bot of WeCom is set up" in capsys.readouterr().err


async def test_only_one_assistant_runs_on_a_machine(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    rig.folder.mkdir(parents=True)
    held = acquire(rig.folder / store.LOCK)
    assert held is not None
    try:
        await rig.start()
    finally:
        held.release()

    assert ended(rig) == 2
    assert "an assistant in WeCom runs on this machine already" in capsys.readouterr().err


async def test_the_packages_that_it_needs_are_named_where_one_is_missing(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    real = importlib.util.find_spec

    def find_spec(name: str, package: str | None = None) -> Any:
        return None if name == "cryptography" else real(name, package)

    monkeypatch.setattr(importlib.util, "find_spec", find_spec)
    rig.set_up()

    await rig.start()

    assert ended(rig) == 2
    assert "'pip install columbina[wecom]'" in capsys.readouterr().err


async def test_a_directory_of_what_the_program_keeps_that_cannot_be_made_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    rig.folder.write_text("a file where the folder would be", encoding="utf-8")

    await rig.start()

    assert ended(rig) == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


async def test_a_secret_that_cannot_be_read_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    (rig.home / store.CREDENTIALS).write_text("not what is written there", encoding="utf-8")

    await rig.start()

    assert ended(rig) == 2
    assert "the Secret of the bot cannot be used" in capsys.readouterr().err


async def test_a_bot_without_its_secret_is_not_set_up(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    (rig.home / store.CREDENTIALS).unlink()

    await rig.start()

    assert ended(rig) == 2
    assert "no bot of WeCom is set up" in capsys.readouterr().err


async def test_a_directory_of_the_assistant_that_cannot_be_made_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    rig.cwd.parent.mkdir(parents=True)
    rig.cwd.write_text("a file where the directory would be", encoding="utf-8")

    await rig.start()

    assert ended(rig) == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


async def test_an_assistant_started_to_change_files_that_its_owner_does_not_let_does_not_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER)

    await rig.start(writable=True)

    assert ended(rig) == 2
    assert "which its owner does not let it do. Start it again" in capsys.readouterr().err
    assert not rig.cwd.exists()


async def test_an_assistant_whose_directory_lies_in_the_home_of_the_program_does_not_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER, writable=True)

    await rig.start(writable=True, cwd=rig.home / "assistant")

    assert ended(rig) == 2
    assert "lies in the home of the program, where it may change no file" in (
        capsys.readouterr().err
    )


async def test_a_session_that_cannot_be_opened_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up(owner=OWNER, session_id="../outside")

    await rig.start()

    assert ended(rig) == 1
    assert "the session of the assistant cannot be opened" in capsys.readouterr().err
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "failed"


async def test_the_bot_signs_in_and_shows_the_code_of_pairing(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()

    await rig.start()

    (signed,) = rig.socket.of("aibot_subscribe")
    assert signed["body"] == {"bot_id": BOT_ID, "secret": SECRET}
    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.paired, told.error) == ("connected", False, None)
    assert told.code is not None
    err = capsys.readouterr().err
    assert f"send its bot this code in WeCom: {told.code}" in err
    assert "connected to WeCom" in err
    assert f"the assistant of the bot {BOT_ID} is starting" in err
    assert rig.cwd.is_dir()
    await rig.stop()


async def test_a_bot_that_wecom_refuses_ends_the_run_and_says_why(rig: Rig) -> None:
    rig.socket.refusing["aibot_subscribe"] = 853000
    rig.set_up(owner=OWNER)

    await rig.start()

    assert ended(rig) == 1
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "failed"
    assert told.error is not None
    assert "does not take this Bot ID with this Secret" in told.error


@pytest.fixture
def input_closes(monkeypatch: pytest.MonkeyPatch) -> asyncio.Event:
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

    input_closes.set()

    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0
    said = lines_of(capsys.readouterr().out)
    assert [line["state"] for line in said] == ["connecting", "connecting", "connected", "stopped"]
    assert {line["bot_id"] for line in said} == {BOT_ID}
    assert {(line["paired"], line["error"]) for line in said} == {(False, None)}
    assert isinstance(said[0]["code"], str)


async def test_the_answers_being_written_say_so_when_the_assistant_is_stopped(
    rig: Rig, input_closes: asyncio.Event
) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig, json_lines=True)
    rig.say("one", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "m2")
    await until(lambda: len(rig.answers()) == 2)

    input_closes.set()
    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0

    assert rig.answers()[2:] == [
        ("req-m1", "stream", "（Columbina 中途退出了，这次的回答没写完。）", True),
        ("req-m2", "stream", "（Columbina 中途退出了，这次的回答没写完。）", True),
    ]
    assert rig.pushed() == []


async def test_a_program_that_stops_an_assistant_that_failed_hears_why(
    rig: Rig, input_closes: asyncio.Event
) -> None:
    rig.socket.refusing["aibot_subscribe"] = 846601
    rig.set_up(owner=OWNER)

    await rig.start(json_lines=True)

    assert ended(rig) == 1


async def test_what_is_said_of_how_it_is_doing_is_let_go_where_nobody_reads_it(
    rig: Rig, input_closes: asyncio.Event, monkeypatch: pytest.MonkeyPatch
) -> None:
    class Gone:
        def write(self, text: str) -> int:
            raise OSError("the pipe is gone")

        def flush(self) -> None:
            raise OSError("the pipe is gone")

    monkeypatch.setattr(sys, "stdout", Gone())
    rig.set_up()
    await rig.start(json_lines=True)

    input_closes.set()

    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0


# Pairing


async def test_the_one_who_sends_the_code_becomes_the_owner(rig: Rig) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    code = told.code
    assert code is not None

    rig.say("hello", "m1", sender=STRANGER)
    rig.say("hello again", "m2", sender=STRANGER)
    rig.say(code, "m3", sender=OWNER)
    await until(lambda: len(rig.replies()) == 2)

    words = rig.assistant._words  # pyright: ignore[reportPrivateUsage,reportOptionalMemberAccess]
    assert rig.replies() == [
        ("req-m1", words.pair_please),
        ("req-m3", f"{words.paired}\n\n{words.help}"),
    ]
    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.owner == OWNER
    after = store.status(rig.home)
    assert after is not None
    assert (after.paired, after.code) == (True, None)
    await rig.stop()


async def test_the_code_is_given_up_after_too_many_wrong_tries(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    await rig.start()

    for index in range(5):
        rig.say("000000", f"m{index}", sender=STRANGER)
    await until(lambda: "given up on" in (getattr(store.status(rig.home), "error", "") or ""))

    told = store.status(rig.home)
    assert told is not None
    assert told.code is None
    rig.say("123456", "m9", sender=OWNER)
    await asyncio.sleep(0.05)
    assert len(rig.replies()) == 1
    assert "given up on after too many wrong tries" in capsys.readouterr().err
    await rig.stop()


async def test_tries_from_all_senders_together_are_counted_too(rig: Rig) -> None:
    rig.set_up()
    await rig.start()

    for index in range(20):
        rig.say("000000", f"m{index}", sender=f"user-{index % 10}")
    await until(lambda: "given up on" in (getattr(store.status(rig.home), "error", "") or ""))
    await rig.stop()


async def test_a_code_that_ran_out_is_made_anew(rig: Rig) -> None:
    rig.set_up()
    assistant = await rig.start()
    first = store.status(rig.home)
    assert first is not None
    assistant._now = lambda: 10**12  # pyright: ignore[reportPrivateUsage]

    assert first.code is not None
    rig.say(first.code, "m1", sender=OWNER)
    await until(lambda: getattr(store.status(rig.home), "code", None) != first.code)

    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.owner is None
    await rig.stop()


async def test_a_pairing_that_cannot_be_written_down_is_said_to_be_so(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    assert told.code is not None

    def unwritable(home: Path, chosen: store.Settings) -> None:
        raise WecomError("no room", code="STORE")

    monkeypatch.setattr(store, "save", unwritable)
    rig.say(told.code, "m1", sender=OWNER)
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "配对没能记下来：no room。请过一会儿再发一次配对码。")]
    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.owner is None
    await rig.stop()


async def test_a_bot_that_was_changed_meanwhile_is_not_written_over(rig: Rig) -> None:
    rig.set_up()
    await rig.start()
    told = store.status(rig.home)
    assert told is not None
    assert told.code is not None
    store.keep(rig.home, store.Settings(bot_id="aibOther"), SECRET, PLACE)

    rig.say(told.code, "m1", sender=OWNER)
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("配对没能记下来：the bot was changed")
    assert store.settings(rig.home) == store.Settings(bot_id="aibOther")
    await rig.stop()


async def test_whoever_opens_the_chat_before_anybody_is_paired_is_asked_for_the_code(
    rig: Rig,
) -> None:
    rig.set_up()
    await rig.start()

    rig.socket.tell(event("enter_chat", req_id="req-enter"))
    await until(lambda: bool(rig.socket.of(WELCOME)))

    (welcome,) = rig.socket.of(WELCOME)
    assert welcome["headers"] == {"req_id": "req-enter"}
    assert welcome["body"] == {
        "msgtype": "text",
        "text": {"content": "这是一个私人助手。请发送配对码，配对码在 Columbina 里能看到。"},
    }
    await rig.stop()


async def test_once_paired_nobody_who_opens_the_chat_is_greeted(rig: Rig) -> None:
    await paired(rig)

    rig.socket.tell(event("enter_chat"))
    rig.socket.tell(event("feedback_event"))
    rig.socket.tell({"cmd": "aibot_event_callback", "body": {"event": {"eventtype": "enter_chat"}}})
    await asyncio.sleep(0.05)

    assert rig.socket.of(WELCOME) == []
    await rig.stop()


# The conversation


async def test_a_message_is_answered_where_it_said_that_the_agent_was_thinking(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="## 你好\n\n- 一"))
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len(rig.answers()) == 2)

    assert rig.answers() == [
        ("req-m1", "stream", THINKING, False),
        ("req-m1", "stream", "## 你好\n\n- 一", True),
    ]
    first, last = rig.socket.of(RESPOND)
    assert first["body"]["stream"]["id"] == last["body"]["stream"]["id"]
    assert first["body"]["stream"]["id"].startswith("stream_")
    await rig.stop()


async def test_each_message_is_answered_in_a_turn_of_its_own(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="first", hold=True), Reply(text="second"))
    await paired(rig)

    rig.say("one", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "m2")
    rig.harness.model.release()
    await until(lambda: len(rig.replies()) == 2)

    assert rig.replies() == [("req-m1", "first"), ("req-m2", "second")]
    await rig.stop()


async def test_an_answer_too_long_for_one_message_goes_on_in_messages_of_its_own(
    rig: Rig,
) -> None:
    line = "段落" * 900
    long = "\n".join([line] * 8)
    rig.harness.model.add(Reply(text=long))
    await paired(rig)

    rig.say("tell me", "m1")
    await until(lambda: len(rig.pushed()) >= 1)
    await asyncio.sleep(0.05)

    ended_with = rig.answers()[-1]
    assert ended_with[3] is True
    parts = [ended_with[2], *rig.pushed()]
    assert "\n".join(parts) == long
    assert all(len(part.encode()) <= 20_480 for part in parts)
    (first_push, *_) = rig.socket.of(SEND)
    assert first_push["body"]["chatid"] == OWNER
    assert first_push["body"]["chat_type"] == 1
    assert first_push["body"]["msgtype"] == "markdown"
    await rig.stop()


async def test_a_turn_that_failed_says_so(rig: Rig) -> None:
    rig.harness.model.add(
        Reply(failure=LlmFailure(message="the key is wrong", code="AUTH", status=401))
    )
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len(rig.answers()) == 2)

    assert rig.answers()[1][2].startswith("（这次没能回答：")
    assert "the key is wrong" in rig.answers()[1][2]
    await rig.stop()


@pytest.mark.parametrize(
    "heard",
    [
        message("hi", message_id="m-g", chattype="group", chatid="group-1"),
        message("hi", message_id="m-s", sender=STRANGER),
        message("hi", message_id="m-c", compat_info={"source": "channel"}),
        message("   ", message_id="m-blank"),
        message("hi", message_id="m-nobody", sender=""),
        {"cmd": "aibot_msg_callback", "headers": {}, "body": {"msgid": "m-noreq"}},
        {"cmd": "aibot_msg_callback", "headers": {"req_id": "r"}, "body": {"msgid": ""}},
        {"cmd": "aibot_msg_callback", "headers": {"req_id": "r"}, "body": {}},
    ],
    ids=["group", "stranger", "passed on", "blank", "no sender", "no req_id", "no id", "nothing"],
)
async def test_what_is_no_message_of_the_owner_to_the_bot_is_passed_over(
    rig: Rig, heard: dict[str, Any]
) -> None:
    await paired(rig)

    rig.socket.tell(heard)
    await asyncio.sleep(0.05)

    assert rig.answers() == []
    await rig.stop()


async def test_a_message_that_comes_again_is_passed_over_also_after_a_new_start(
    rig: Rig, harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.harness.model.add(Reply(text="once"))
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len(rig.answers()) == 2)
    rig.say("hi", "m1")
    await asyncio.sleep(0.05)

    assert len(rig.answers()) == 2
    assert json.loads((rig.folder / SEEN).read_text(encoding="utf-8")) == ["m1"]
    await rig.stop()


async def test_messages_that_were_taken_cannot_be_written_down_is_said(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.harness.model.add(Reply(text="still answered"))
    (rig.folder / SEEN).mkdir(parents=True)
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len(rig.answers()) == 2)

    told = store.status(rig.home)
    assert told is not None
    assert told.error is not None
    assert told.error.startswith("which messages were taken cannot be written down")
    await rig.stop()


async def test_what_waited_when_the_program_ended_is_answered_in_a_message_of_its_own(
    rig: Rig, harness: Harness
) -> None:
    rig.set_up(owner=OWNER, session_id="s-left")
    rig.cwd.mkdir(parents=True)
    left = await harness.agents.create(harness.ctx, harness.spec(cwd=rig.cwd), session_id="s-left")
    waiting = InboxItem(message=UserMessage(content=(TextBlock(text="are you there"),)))
    enqueue(left.session, "next-turn", waiting)
    await left.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: bool(rig.pushed()))

    assert rig.pushed() == ["I am"]
    assert rig.answers() == []
    await rig.stop()


async def test_an_answer_that_takes_longer_than_it_may_be_written_comes_in_a_message_of_its_own(
    rig: Rig,
) -> None:
    rig.harness.model.add(Reply(text="at last", hold=True))
    await paired(rig)
    rig.say("think hard", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    # Nine minutes pass while the agent works.
    rig.pauses.ends.set()
    await until(lambda: len(rig.answers()) == 2)
    rig.harness.model.release()
    await until(lambda: bool(rig.pushed()))

    assert rig.answers() == [
        ("req-m1", "stream", THINKING, False),
        ("req-m1", "stream", "（还在想，想好了另发一条。）", True),
    ]
    assert rig.pushed() == ["at last"]
    await rig.stop()


async def test_an_answer_that_may_be_written_no_longer_is_sent_as_a_message_of_its_own(
    rig: Rig,
) -> None:
    rig.harness.model.add(Reply(text="at last", hold=True))
    await paired(rig)
    rig.say("think hard", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    await until(lambda: len(rig.answers()) == 1)

    rig.clock.now += 541.0
    rig.harness.model.release()
    await until(lambda: bool(rig.pushed()))

    assert rig.answers() == [("req-m1", "stream", THINKING, False)]
    assert rig.pushed() == ["at last"]
    await rig.stop()


async def test_an_answer_that_wecom_does_not_take_in_its_place_is_sent_as_a_message(
    rig: Rig,
) -> None:
    rig.socket.refusing[RESPOND] = 846605
    rig.harness.model.add(Reply(text="here"))
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: bool(rig.pushed()))

    assert rig.pushed() == ["here"]
    # The answer went out after all, so nothing is wrong now.
    await until(lambda: getattr(store.status(rig.home), "error", "") is None)
    await rig.stop()


async def test_an_answer_that_wecom_cannot_take_for_now_is_sent_again_after_a_pause(
    rig: Rig,
) -> None:
    rig.socket.queued[RESPOND] = [846607, 846607]
    rig.harness.model.add(Reply(text="here"))
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len([a for a in rig.answers() if a[3]]) == 1)

    assert rig.pauses.taken[:2] == [2.0, 5.0]
    assert rig.replies() == [("req-m1", "here")]
    await rig.stop()


async def test_an_answer_that_cannot_be_sent_at_all_is_said_in_the_status(
    rig: Rig, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.ERROR)
    rig.socket.refusing[RESPOND] = 846605
    rig.socket.refusing[SEND] = 846607
    rig.harness.model.add(Reply(text="lost"))
    await paired(rig)

    rig.say("hi", "m1")
    await until(lambda: len(rig.socket.of(SEND)) == 4)
    await asyncio.sleep(0.05)

    told = store.status(rig.home)
    assert told is not None
    assert told.error is not None
    assert told.error.startswith("an answer could not be sent to WeCom: WeCom refused it")
    assert "an answer could not be sent to WeCom (message)" in caplog.text
    assert rig.pauses.taken.count(15.0) >= 1
    await rig.stop()


async def test_what_waits_on_the_side_when_the_assistant_stops_is_broken_off(
    rig: Rig,
) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("hi", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    # The answer was begun, and what ends it after nine minutes waits.
    await until(lambda: 540.0 in rig.pauses.taken)

    stopped = await asyncio.wait_for(rig.stop(), timeout=5)

    assert isinstance(stopped, asyncio.CancelledError)
    assert rig.answers() == [("req-m1", "stream", THINKING, False)]


async def test_files_are_downloaded_by_a_client_of_their_own() -> None:
    client = bot_module._downloads({})  # pyright: ignore[reportPrivateUsage]
    try:
        assert client.headers["User-Agent"].startswith("Columbina/")
        assert client.timeout.read == 60.0
    finally:
        await client.aclose()


async def test_nothing_goes_out_once_the_assistant_has_stopped(rig: Rig) -> None:
    assistant = await paired(rig)
    await rig.stop()

    sent = await assistant._sent(SEND, {}, None, "late")  # pyright: ignore[reportPrivateUsage]

    assert sent is False


async def test_a_voice_message_is_given_to_the_agent_as_its_words(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="heard"))
    await paired(rig)

    rig.say("明天几点开会", "m1", kind="voice")
    await until(lambda: len(rig.answers()) == 2)

    asked = rig.harness.model.requests[-1].messages[-1]
    assert isinstance(asked, UserMessage)
    assert "明天几点开会" in str(asked.content)
    await rig.stop()


async def test_a_message_that_quotes_another_is_given_with_what_it_quotes(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="ok"))
    await paired(rig)

    quote = {"msgtype": "text", "text": {"content": "原来的话"}}
    rig.say("这是什么意思", "m1", quote=quote)
    await until(lambda: len(rig.answers()) == 2)

    asked = rig.harness.model.requests[-1].messages[-1]
    assert isinstance(asked, UserMessage)
    assert asked.content == (TextBlock(text="> 原来的话\n\n这是什么意思"),)
    await rig.stop()


@pytest.mark.parametrize("kind", ["image", "video"])
async def test_a_message_that_holds_no_words_is_said_to_be_unread(rig: Rig, kind: str) -> None:
    await paired(rig)

    rig.say("", "m1", kind=kind, content={"url": "https://x/1", "aeskey": KEY})
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "我现在只看得懂文字、语音和文本文件。")]
    await rig.stop()


# The commands


async def test_help_says_what_there_is(rig: Rig) -> None:
    await paired(rig, language="en")

    rig.say("/HELP", "m1")
    rig.say("/what", "m2")
    await until(lambda: len(rig.replies()) == 2)

    help_said, unknown = rig.replies()
    assert help_said[1].startswith("Write to me, or speak, and I answer.")
    assert unknown == ("req-m2", "I do not know that command. Send /help to see the commands.")
    await rig.stop()


async def test_new_begins_a_new_conversation_and_drops_what_waited(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    assistant = await paired(rig)
    rig.say("one", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "m2")
    await until(lambda: len(rig.answers()) == 2)
    before = store.settings(rig.home)
    assert before is not None

    rig.say("/new", "m3")
    await until(lambda: len(rig.answers()) == 5)

    assert sorted(rig.answers()[2:]) == [
        ("req-m1", "stream", "（这条不再回答。）", True),
        ("req-m2", "stream", "（这条不再回答。）", True),
        ("req-m3", "markdown", "已开始新对话，没答完的 2 条消息不再回答。", None),
    ]
    after = store.settings(rig.home)
    assert after is not None
    assert after.session_id != before.session_id
    assert assistant._agent is not None  # pyright: ignore[reportPrivateUsage]
    rig.harness.model.release()
    await rig.stop()


async def test_new_with_nothing_waiting_says_so_plainly(rig: Rig) -> None:
    await paired(rig)

    rig.say("/new", "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "已开始新对话。")]
    await rig.stop()


async def test_a_new_conversation_that_cannot_begin_says_so_and_nothing_is_answered_then(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    assistant = await paired(rig)

    async def broken(session_id: str | None) -> Any:
        raise ValueError("the sessions are gone")

    monkeypatch.setattr(assistant, "_opened", broken)
    rig.say("/new", "m1")
    await until(lambda: bool(rig.replies()))
    rig.say("hello?", "m2")
    rig.say("/stop", "m3")
    # Without a conversation, there is none to end before the next is begun.
    rig.say("/new", "m4")
    await until(lambda: len(rig.replies()) == 4)

    assert rig.replies() == [
        ("req-m1", "没能开始新对话：the sessions are gone。可以再发一次 /new 试试。"),
        ("req-m2", "现在没有可用的对话。发 /new 开始一个新对话。"),
        ("req-m3", "现在没有在写的回答。"),
        ("req-m4", "没能开始新对话：the sessions are gone。可以再发一次 /new 试试。"),
    ]
    await rig.stop()


async def test_stop_stops_the_answer_and_drops_what_waited(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.say("two", "m2")
    await until(lambda: len(rig.answers()) == 2)

    rig.say("/stop", "m3")
    await until(lambda: len(rig.answers()) == 5)

    assert sorted(rig.answers()[2:]) == [
        ("req-m1", "stream", "（停下了，没有答完。）", True),
        ("req-m2", "stream", "（这条不再回答。）", True),
        ("req-m3", "markdown", "已停下，排在后面的 1 条消息也不再回答。", None),
    ]
    await rig.stop()


async def test_stop_with_one_answer_under_way_says_so_plainly(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.say("one", "m1")
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    rig.say("/stop", "m2")
    await until(lambda: len(rig.answers()) == 3)

    assert sorted(rig.replies()) == [("req-m1", "（停下了，没有答完。）"), ("req-m2", "已停下。")]
    await rig.stop()


async def test_a_turn_from_before_a_new_start_that_is_stopped_says_nothing_more(
    rig: Rig, harness: Harness
) -> None:
    rig.set_up(owner=OWNER, session_id="s-left")
    rig.cwd.mkdir(parents=True)
    left = await harness.agents.create(harness.ctx, harness.spec(cwd=rig.cwd), session_id="s-left")
    waiting = InboxItem(message=UserMessage(content=(TextBlock(text="are you there"),)))
    enqueue(left.session, "next-turn", waiting)
    await left.close()
    harness.model.add(Reply(text="never", hold=True))
    await rig.start()
    await asyncio.wait_for(harness.model.held(), timeout=5)

    rig.say("/stop", "m1")
    await until(lambda: bool(rig.replies()))
    await asyncio.sleep(0.05)

    assert rig.replies() == [("req-m1", "已停下。")]
    assert rig.pushed() == []
    await rig.stop()


async def test_stop_with_nothing_under_way_says_so(rig: Rig) -> None:
    await paired(rig)

    rig.say("/stop", "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "现在没有在写的回答。")]
    await rig.stop()


async def skilled(rig: Rig) -> Path:
    skills = rig.cwd / "skills"
    await rig.harness.ctx.plugin(columbina.skills, {"home": skills})
    return skills


async def test_skills_are_listed_and_one_is_deleted(rig: Rig) -> None:
    skills = await skilled(rig)
    (skills / "greet").mkdir(parents=True)
    (skills / "greet" / "SKILL.md").write_bytes(SKILL)
    await paired(rig)

    rig.say("/skills", "m1")
    await until(lambda: bool(rig.replies()))
    rig.say("/skills rm greet", "m2")
    rig.say("/skills rm greet", "m3")
    rig.say("/skills rm", "m4")
    rig.say("/skills", "m5")
    await until(lambda: len(rig.replies()) == 5)

    assert rig.replies() == [
        ("req-m1", "我有这些技能：\n\n- greet: Greets the user warmly."),
        ("req-m2", "已删掉技能 greet。"),
        ("req-m3", "我自己的技能目录里没有叫 greet 的技能。"),
        ("req-m4", "我自己的技能目录里没有叫 ? 的技能。"),
        ("req-m5", "还没有技能。把 SKILL.md 当文件发给我就能装上。"),
    ]
    assert not (skills / "greet").exists()
    await rig.stop()


async def test_a_list_of_skills_too_long_for_one_message_goes_on_in_more(rig: Rig) -> None:
    skills = await skilled(rig)
    for index in range(160):
        named = f"skill-{index:03d}"
        (skills / named).mkdir(parents=True)
        (skills / named / "SKILL.md").write_text(
            f"---\nname: {named}\ndescription: {'所做的事 ' * 60}\n---\nDo it.\n",
            encoding="utf-8",
        )
    await paired(rig)

    rig.say("/skills", "m1")
    await until(lambda: any("skill-159" in text for text in rig.pushed()))

    ((req_id, listed),) = rig.replies()[:1]
    assert req_id == "req-m1"
    assert listed.startswith("我有这些技能：")
    assert "skill-000" in listed
    assert all(len(text.encode()) <= 20_480 for text in [listed, *rig.pushed()])
    await rig.stop()


async def test_a_skill_whose_directory_holds_other_files_is_left_to_the_owner(rig: Rig) -> None:
    skills = await skilled(rig)
    (skills / "greet").mkdir(parents=True)
    (skills / "greet" / "SKILL.md").write_bytes(SKILL)
    (skills / "greet" / "notes.md").write_text("mine", encoding="utf-8")
    await paired(rig)

    rig.say("/skills rm greet", "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("技能 greet 的目录里还有别的文件")
    assert (skills / "greet" / "SKILL.md").exists()
    await rig.stop()


async def test_without_the_plugin_of_skills_there_are_none(rig: Rig) -> None:
    await paired(rig)

    rig.say("/skills", "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "还没有技能。把 SKILL.md 当文件发给我就能装上。")]
    await rig.stop()


# Files and skills

URL = "https://wework.qpic.cn/file/1"


def sent_file(rig: Rig, content: bytes, name: str | None) -> None:
    headers = {} if name is None else {"Content-Disposition": f'attachment; filename="{name}"'}
    rig.files[URL] = httpx.Response(200, content=encrypted(content), headers=headers)


def file_message(rig: Rig, message_id: str) -> None:
    rig.say("", message_id, kind="file", content={"url": URL, "aeskey": KEY})


async def test_a_file_of_text_is_kept_and_given_to_the_agent(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="read it"))
    await paired(rig)
    sent_file(rig, "笔记".encode(), "notes.md")

    file_message(rig, "m1")
    await until(lambda: len(rig.answers()) == 2)

    kept = rig.cwd / "uploads" / "m1" / "notes.md"
    assert kept.read_text(encoding="utf-8") == "笔记"
    asked = rig.harness.model.requests[-1].messages[-1]
    assert "[I sent you the file uploads/m1/notes.md, in your working directory." in str(
        asked.content
    )
    assert rig.replies() == [("req-m1", "read it")]
    await rig.stop()


async def test_a_file_of_text_without_a_name_is_kept_as_one(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="read it"))
    await paired(rig)
    sent_file(rig, b"plain words", None)

    file_message(rig, "m1")
    await until(lambda: len(rig.answers()) == 2)

    assert (rig.cwd / "uploads" / "m1" / "file.txt").read_bytes() == b"plain words"
    await rig.stop()


@pytest.mark.parametrize(
    ("content", "name"),
    [(b"\x00\x01binary", None), (b"\xff\xfe", None), (b"words", "picture.png")],
    ids=["nul", "no utf-8", "no text"],
)
async def test_a_file_that_is_no_text_is_not_read(
    rig: Rig, content: bytes, name: str | None
) -> None:
    await paired(rig)
    sent_file(rig, content, name)

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "我现在只看得懂文字、语音和文本文件。")]
    assert not (rig.cwd / "uploads").exists()
    await rig.stop()


@pytest.mark.parametrize("name", ["SKILL.md", "greeting.md", None])
async def test_a_skill_that_is_sent_is_added(rig: Rig, name: str | None) -> None:
    await paired(rig)
    sent_file(rig, SKILL, name)

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "已装上技能 greet。")]
    assert (rig.cwd / "skills" / "greet" / "SKILL.md").read_bytes() == SKILL
    await rig.stop()


async def test_a_skill_that_is_there_is_replaced_and_its_other_files_are_left(rig: Rig) -> None:
    await paired(rig)
    directory = rig.cwd / "skills" / "greet"
    directory.mkdir(parents=True)
    (directory / "SKILL.md").write_text("old", encoding="utf-8")
    (directory / "notes.md").write_text("mine", encoding="utf-8")
    sent_file(rig, SKILL, "SKILL.md")

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    said = rig.replies()[0][1]
    assert said.startswith("已更新技能 greet，换掉了原来的 SKILL.md。")
    assert "它们保持原样" in said
    assert (directory / "notes.md").read_text(encoding="utf-8") == "mine"
    await rig.stop()


async def test_a_skill_md_that_is_no_skill_is_said_to_be_none(rig: Rig) -> None:
    await paired(rig)
    sent_file(rig, b"# just a title\n", "SKILL.md")

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件不是能用的技能：")
    await rig.stop()


async def test_a_file_that_cannot_be_had_says_why(rig: Rig) -> None:
    await paired(rig)

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [
        ("req-m1", "这个文件没能收下：the file could not be downloaded: HTTP 404")
    ]
    await rig.stop()


async def test_a_file_that_is_too_large_is_not_taken(rig: Rig) -> None:
    await paired(rig)
    rig.files[URL] = httpx.Response(200, content=b"x" * (1024 * 1024 + 64))

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "文件太大了，我只收 1 MB 以内的文本文件。")]
    await rig.stop()


async def test_a_file_that_is_too_large_once_it_is_decrypted_is_not_taken(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    await paired(rig)
    monkeypatch.setattr(bot_module, "MAX_BYTES", 10)
    sent_file(rig, b"x" * 20, "notes.md")

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies() == [("req-m1", "文件太大了，我只收 1 MB 以内的文本文件。")]
    await rig.stop()


async def test_a_file_that_cannot_be_kept_says_why(rig: Rig) -> None:
    await paired(rig)
    (rig.cwd / "uploads" / "m1").mkdir(parents=True)
    (rig.cwd / "uploads" / "m1" / "notes.md").mkdir()
    sent_file(rig, b"words", "notes.md")

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件没能收下：")
    await rig.stop()


async def test_a_skill_that_cannot_be_kept_says_why(rig: Rig) -> None:
    await paired(rig)
    (rig.cwd / "skills" / "greet" / "SKILL.md").mkdir(parents=True)
    sent_file(rig, SKILL, "SKILL.md")

    file_message(rig, "m1")
    await until(lambda: bool(rig.replies()))

    assert rig.replies()[0][1].startswith("这个文件没能收下：")
    await rig.stop()


async def test_the_name_of_a_file_is_made_safe(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="ok"))
    await paired(rig)
    sent_file(rig, b"words", "..\\..\\evil:name?.txt")

    file_message(rig, "m1")
    await until(lambda: len(rig.answers()) == 2)

    assert (rig.cwd / "uploads" / "m1" / "evil_name_.txt").read_bytes() == b"words"
    await rig.stop()


# What its agent may do


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
    rig.set_up(owner=OWNER, writable=True)

    assistant = await rig.start(writable=True)

    agent = assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None
    assert harness.ctx.use(PERMISSIONS).in_force(agent.session) == ("workspace-write", "ask")
    await rig.stop()


async def test_an_agent_that_ends_badly_is_logged(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.ERROR)
    assistant = await paired(rig)
    agent = assistant._agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None

    async def broken() -> None:
        raise RuntimeError("will not close")

    monkeypatch.setattr(agent, "close", broken)
    await rig.stop()

    assert "the agent of the assistant stopped badly: will not close" in caplog.text


async def test_a_message_that_cannot_be_taken_is_logged_and_the_next_one_is_taken(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.ERROR)
    rig.harness.model.add(Reply(text="second"))
    assistant = await paired(rig)
    real = assistant._take  # pyright: ignore[reportPrivateUsage]
    taken: list[str] = []

    async def take(frame: dict[str, Any]) -> None:
        if not taken:
            taken.append("broken")
            raise RuntimeError("broken")
        await real(frame)

    monkeypatch.setattr(assistant, "_take", take)
    rig.say("one", "m1")
    rig.say("two", "m2")
    await until(lambda: len(rig.answers()) == 2)

    assert "a message from WeCom could not be taken" in caplog.text
    await rig.stop()
