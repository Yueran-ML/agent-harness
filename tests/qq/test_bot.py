"""The assistant in QQ: ``columbina.qq.bot``."""

import asyncio
import importlib.util
import json
import logging
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina.apps import FRONTEND
from columbina.llm import LlmFailure, TextBlock, UserMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.permissions import PERMISSIONS
from columbina.qq import bot as bot_module
from columbina.qq import store
from columbina.qq.app import TOKEN_URL, Api, QqError
from columbina.qq.bot import GROUP_PROMPT, OWNER_TOOLS, PROMPT, SEEN, Assistant
from columbina.qq.link import Link
from columbina.session import SessionError
from columbina.session.lock import acquire
from columbina.tools import ToolRun
from support.fake_qq import (
    APP_ID,
    OWNER,
    SECRET,
    STRANGER,
    FakeApi,
    FakeSocket,
    Opened,
    in_group,
    private,
)
from support.loop import MODEL, Harness


async def until(condition: Callable[[], bool], seconds: float = 5.0) -> None:
    async with asyncio.timeout(seconds):
        while not condition():  # noqa: ASYNC110
            await asyncio.sleep(0.002)


async def forever(seconds: float) -> None:
    await asyncio.Event().wait()


async def at_once(seconds: float) -> None:
    await asyncio.sleep(0)


class Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


class Rig:
    """An assistant at work in a test, with QQ on the other end."""

    def __init__(self, harness: Harness, home: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        self.harness = harness
        self.home = home
        self.cwd = home.parent / "Columbina" / "qq"
        self.qq = FakeApi()
        self.socket = FakeSocket()
        self.opened = Opened(self.socket)
        self.clock = Clock()
        self.assistant: Assistant | None = None
        self.task: asyncio.Task[int] | None = None

        def api(app_id: str, secret: str, env: Any) -> Api:
            return Api(app_id, secret, env, transport=self.qq.transport)

        def linked(*args: Any, **kwargs: Any) -> Link:
            return Link(*args, **kwargs, opener=self.opened, sleep=at_once, ticking=forever)

        monkeypatch.setattr(bot_module, "Api", api)
        monkeypatch.setattr(bot_module, "Link", linked)

    def set_up(self, **changes: object) -> None:
        settings = store.Settings(app_id=APP_ID).model_copy(update=changes)
        store.keep(self.home, settings, SECRET, TOKEN_URL)

    async def start(self, **config: object) -> Assistant:
        await self.harness.ctx.plugin(
            bot_module,
            {"model": MODEL.model_dump(), "cwd": self.cwd, "home": self.home, **config},
        )
        assistant = self.harness.ctx.use(FRONTEND)
        assert isinstance(assistant, Assistant)
        assistant._clock = self.clock  # pyright: ignore[reportPrivateUsage]
        self.assistant = assistant
        self.task = asyncio.ensure_future(assistant.run())
        await until(self._ready)
        return assistant

    def _ready(self) -> bool:
        told = store.status(self.home)
        connected = told is not None and told.state == "connected"
        return connected or (self.task is not None and self.task.done())

    def tell(self, event: tuple[str, dict[str, Any]]) -> None:
        kind, data = event
        self.socket.dispatch(kind, data)

    def said(self) -> list[tuple[str, str, int]]:
        return self.qq.said()

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


def asked_of(request: Any) -> str:
    last = request.messages[-1]
    assert isinstance(last, UserMessage)
    return "".join(block.text for block in last.content)


# Starting


async def test_nothing_set_up_is_said_and_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    await rig.start()

    assert ended(rig) == 2
    assert "no bot of QQ is set up" in capsys.readouterr().err


async def test_only_one_assistant_runs_on_a_machine(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    (rig.home / store.FOLDER).mkdir(parents=True, exist_ok=True)
    held = acquire(rig.home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        await rig.start()
    finally:
        held.release()

    assert ended(rig) == 2
    assert "an assistant in QQ runs on this machine already" in capsys.readouterr().err


async def test_the_package_that_it_needs_is_named_where_it_is_missing(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    found = importlib.util.find_spec

    def find_spec(name: str, *args: Any) -> Any:
        return None if name == "websockets" else found(name, *args)

    monkeypatch.setattr(importlib.util, "find_spec", find_spec)

    await rig.start()

    assert ended(rig) == 2
    assert "pip install columbina[qq]" in capsys.readouterr().err


async def test_a_directory_of_what_the_program_keeps_that_cannot_be_made_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.home.mkdir(parents=True)
    (rig.home / store.FOLDER).write_text("a file", encoding="utf-8")

    await rig.start()

    assert ended(rig) == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


async def test_a_bot_that_cannot_be_read_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    (rig.home / store.SETTINGS).mkdir(parents=True)

    await rig.start()

    assert ended(rig) == 2
    assert "cannot be read" in capsys.readouterr().err


async def test_a_directory_of_the_assistant_that_cannot_be_made_ends_the_run(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()
    rig.cwd.parent.mkdir(parents=True)
    rig.cwd.write_text("a file", encoding="utf-8")

    await rig.start()

    assert ended(rig) == 2
    assert "the directory of the assistant cannot be used" in capsys.readouterr().err


async def test_a_session_that_cannot_be_opened_ends_the_run(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()

    async def broken(self: Assistant, talk: Any, session_id: str | None) -> Any:
        raise SessionError("the log is broken")

    monkeypatch.setattr(Assistant, "_opened", broken)
    await rig.start()

    assert ended(rig) == 1
    told = store.status(rig.home)
    assert told is not None
    assert told.state == "failed"
    assert "the session of the assistant cannot be opened: the log is broken" in (told.error or "")


async def test_a_bot_that_qq_refuses_ends_the_run_and_says_why(rig: Rig) -> None:
    rig.set_up()
    rig.qq.refusing = None
    rig.socket.ready = False
    rig.socket.incoming.put_nowait(None)
    rig.opened.outcomes = [_Bye(4915)]

    await rig.start()
    await until(lambda: rig.task is not None and rig.task.done())

    assert ended(rig) == 1
    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.error) == (
        "failed",
        "QQ closed the connection: the bot is banned (code 4915)",
    )


class _Bye(Exception):  # noqa: N818 - what the library raises, played here
    def __init__(self, code: int) -> None:
        super().__init__(f"received {code}")
        self.rcvd = type("Close", (), {"code": code})()


async def test_the_bot_connects_and_shows_the_code_of_pairing(
    rig: Rig, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.set_up()

    await rig.start()

    told = store.status(rig.home)
    assert told is not None
    assert (told.state, told.paired) == ("connected", False)
    assert told.code is not None
    assert f"send its bot this code in QQ: {told.code}" in capsys.readouterr().err
    await rig.stop()
    after = store.status(rig.home)
    assert after is not None
    assert after.state == "stopped"


# Pairing


def the_code(rig: Rig) -> str:
    told = store.status(rig.home)
    assert told is not None
    assert told.code is not None
    return told.code


async def test_the_one_who_sends_the_code_becomes_the_owner(rig: Rig) -> None:
    rig.set_up()
    await rig.start()

    rig.tell(private("hello", "m1", sender=STRANGER))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(private("again", "m2", sender=STRANGER))
    rig.tell(private(the_code(rig), "m3"))
    await until(lambda: len(rig.said()) == 2)

    assert rig.said()[0] == (
        "m1",
        "这是一个私人助手。请发送配对码，配对码在 Columbina 里能看到。",
        1,
    )
    assert rig.said()[1][0] == "m3"
    assert rig.said()[1][1].startswith("配对成功")
    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.owner == OWNER
    told = store.status(rig.home)
    assert told is not None
    assert (told.paired, told.code) == (True, None)
    await rig.stop()


async def test_the_code_is_given_up_after_too_many_wrong_tries(rig: Rig) -> None:
    rig.set_up()
    await rig.start()

    for number in range(5):
        rig.tell(private("000000", f"m{number}", sender=STRANGER))
    await until(
        lambda: (
            (
                store.status(rig.home)
                or store.Status(pid=0, state="connecting", paired=False, at=0)
            ).code
            is None
        )
    )

    told = store.status(rig.home)
    assert told is not None
    assert "given up on after too many wrong tries" in (told.error or "")
    # A code that was given up on is no code: nothing more is said.
    rig.tell(private("000000", "m9", sender=STRANGER))
    await asyncio.sleep(0.05)
    assert len(rig.said()) == 1
    await rig.stop()


async def test_tries_from_all_senders_together_are_counted_too(rig: Rig) -> None:
    rig.set_up()
    await rig.start()

    for number in range(20):
        rig.tell(private("000000", f"m{number}", sender=f"SENDER{number}"))
    await until(
        lambda: (
            (
                store.status(rig.home)
                or store.Status(pid=0, state="connecting", paired=False, at=0)
            ).code
            is None
        )
    )

    await rig.stop()


async def test_a_code_that_ran_out_is_made_anew(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    rig.set_up()
    assistant = await rig.start()
    old = the_code(rig)
    assistant._now = lambda: 10**12  # pyright: ignore[reportPrivateUsage]

    rig.tell(private(old, "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert the_code(rig) != old or rig.said()[0][1].startswith("这是一个私人助手")
    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.owner is None
    await rig.stop()


async def test_a_pairing_that_cannot_be_written_down_is_said_to_be_so(rig: Rig) -> None:
    rig.set_up()
    await rig.start()
    # The bot is set up anew while the assistant runs.
    store.keep(rig.home, store.Settings(app_id="999"), SECRET, TOKEN_URL)

    rig.tell(private(the_code(rig), "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said()[0][1].startswith(
        "配对没能记下来：the bot was changed while the assistant ran"
    )
    await rig.stop()


# The conversation with the owner


async def test_a_message_of_the_owner_is_answered_as_an_answer_to_it(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="你好呀"))
    assistant = await paired(rig)

    rig.tell(private("你好", "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("m1", "你好呀", 1)]
    path, _ = rig.qq.answers[0]
    assert path == f"/v2/users/{OWNER}/messages"
    request = rig.harness.model.requests[0]
    assert request.messages[0].content[0].text == PROMPT  # type: ignore[union-attr]
    assert asked_of(request) == "你好"
    agent = assistant._owner.agent  # pyright: ignore[reportPrivateUsage]
    assert agent is not None
    assert agent.spec.tools == OWNER_TOOLS
    assert agent.spec.strangers
    await rig.stop()


async def test_what_somebody_else_writes_alone_is_passed_over(rig: Rig) -> None:
    await paired(rig)

    rig.tell(private("hi", "m1", sender=STRANGER))
    await asyncio.sleep(0.05)

    assert rig.said() == []
    await rig.stop()


async def test_a_message_that_holds_no_words_is_said_to_be_unread(rig: Rig) -> None:
    await paired(rig)

    rig.tell(private("  ", "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("m1", "我现在只看得懂文字。", 1)]
    await rig.stop()


async def test_an_answer_too_long_for_one_message_goes_on_in_more_and_is_cut_where_qq_takes_no_more(
    rig: Rig,
) -> None:
    line = "段落" * 400
    rig.harness.model.add(Reply(text="\n".join([line] * 10)))
    await paired(rig)

    rig.tell(private("说吧", "m1"))
    await until(lambda: len(rig.said()) == 4)
    await asyncio.sleep(0.05)

    numbers = [number for _, _, number in rig.said()]
    assert numbers == [1, 2, 3, 4]
    assert all(len(text.encode()) <= 3100 for _, text, _ in rig.said())
    assert rig.said()[-1][1].endswith("（回答太长，被截断了。）")
    await rig.stop()


@pytest.mark.parametrize(
    ("reply", "said"),
    [
        (
            Reply(failure=LlmFailure(message="the key is wrong", code="AUTH", status=401)),
            "（这次没能回答：",
        ),
        (Reply(text=" "), "（这次没有写出回答。）"),
    ],
    ids=["failed", "empty"],
)
async def test_a_turn_that_did_not_end_well_says_so(rig: Rig, reply: Reply, said: str) -> None:
    rig.harness.model.add(reply)
    await paired(rig)

    rig.tell(private("hi", "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said()[0][1].startswith(said)
    await rig.stop()


async def test_an_answer_that_comes_too_late_is_not_sent_and_is_said_to_be_so(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="late", hold=True))
    await paired(rig)
    rig.tell(private("hi", "m1"))
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    rig.clock.now += 3600
    rig.harness.model.release()
    await until(
        lambda: "as old as this one" in ((store.status(rig.home) or _nothing()).error or "")
    )

    assert rig.said() == []
    await rig.stop()


def _nothing() -> store.Status:
    return store.Status(pid=0, state="connecting", paired=False, at=0)


async def test_an_answer_that_qq_refuses_is_said_to_be_so_until_one_goes_out(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="one"), Reply(text="two"))
    await paired(rig)
    rig.qq.refusing = (400, {"code": 22009, "message": "too many"})

    rig.tell(private("hi", "m1"))
    await until(lambda: "too many" in ((store.status(rig.home) or _nothing()).error or ""))
    rig.qq.refusing = None
    rig.tell(private("again", "m2"))
    await until(lambda: len(rig.said()) == 1)
    await until(lambda: (store.status(rig.home) or _nothing()).error is None)

    assert rig.said() == [("m2", "two", 1)]
    await rig.stop()


# The commands


async def test_help_and_a_command_that_is_not_known(rig: Rig) -> None:
    await paired(rig)

    rig.tell(private("/help", "m1"))
    rig.tell(private("/what", "m2"))
    await until(lambda: len(rig.said()) == 2)

    assert rig.said()[0][1].startswith("直接给我发消息就能聊天。")
    assert rig.said()[1][1] == "不认识这个命令。发 /help 看看有哪些。"
    await rig.stop()


async def test_new_begins_a_new_conversation_and_drops_what_waited(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.tell(private("one", "m1"))
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.tell(private("two", "m2"))
    before = store.settings(rig.home)
    assert before is not None

    rig.tell(private("/new", "m3"))
    await until(lambda: len(rig.said()) == 3)

    assert sorted(rig.said()) == [
        ("m1", "（这条不再回答。）", 1),
        ("m2", "（这条不再回答。）", 1),
        ("m3", "已开始新对话，没答完的 2 条消息不再回答。", 1),
    ]
    after = store.settings(rig.home)
    assert after is not None
    assert after.session_id != before.session_id
    rig.harness.model.release()
    await rig.stop()


async def test_new_with_nothing_waiting_says_so_plainly(rig: Rig) -> None:
    await paired(rig)

    rig.tell(private("/new", "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("m1", "已开始新对话。", 1)]
    await rig.stop()


async def test_a_new_conversation_that_cannot_begin_says_so(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    assistant = await paired(rig)

    async def broken(talk: Any, session_id: str | None) -> Any:
        raise ValueError("the sessions are gone")

    monkeypatch.setattr(assistant, "_opened", broken)
    rig.tell(private("/new", "m1"))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(private("hello?", "m2"))
    rig.tell(private("/stop", "m3"))
    rig.tell(private("/new", "m4"))
    await until(lambda: len(rig.said()) == 4)

    assert rig.said() == [
        ("m1", "没能开始新对话：the sessions are gone。可以再发一次 /new 试试。", 1),
        ("m2", "现在没有可用的对话。发 /new 开始一个新对话。", 1),
        ("m3", "现在没有在写的回答。", 1),
        ("m4", "没能开始新对话：the sessions are gone。可以再发一次 /new 试试。", 1),
    ]
    await rig.stop()


async def test_stop_stops_the_answer_and_drops_what_waited(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.tell(private("one", "m1"))
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)
    rig.tell(private("two", "m2"))

    rig.tell(private("/stop", "m3"))
    await until(lambda: len(rig.said()) == 3)

    assert sorted(rig.said()) == [
        ("m1", "（停下了，没有答完。）", 1),
        ("m2", "（这条不再回答。）", 1),
        ("m3", "已停下，排在后面的 1 条消息也不再回答。", 1),
    ]
    await rig.stop()


async def test_stop_with_one_answer_under_way_says_so_plainly(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig)
    rig.tell(private("one", "m1"))
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    rig.tell(private("/stop", "m2"))
    await until(lambda: len(rig.said()) == 2)

    assert ("m2", "已停下。", 1) in rig.said()
    await rig.stop()


# Groups


async def test_whoever_mentions_the_bot_in_a_group_is_answered_in_the_conversation_of_the_group(
    rig: Rig,
) -> None:
    rig.harness.model.add(Reply(text="大家好"))
    assistant = await paired(rig)

    rig.tell(in_group("你好", "g1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("g1", "大家好", 1)]
    assert rig.qq.answers[0][0] == "/v2/groups/GROUP01/messages"
    request = rig.harness.model.requests[0]
    assert request.messages[0].content[0].text == GROUP_PROMPT  # type: ignore[union-attr]
    assert asked_of(request) == f"[{STRANGER} wrote:]\n你好"
    assert [tool.name for tool in request.tools] == ["remember"]
    group = assistant._groups["GROUP01"].agent  # pyright: ignore[reportPrivateUsage]
    assert group is not None
    assert group.spec.tools == frozenset({"remember"})
    assert group.spec.strangers
    assert group.cwd == rig.cwd / "groups" / "GROUP01"
    kept = store.settings(rig.home)
    assert kept is not None
    assert kept.groups == {"GROUP01": group.id}
    await rig.stop()


async def test_a_group_is_answered_also_before_anybody_is_paired(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="在"))
    rig.set_up()
    await rig.start()

    rig.tell(in_group("在吗", "g1", sender=OWNER))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("g1", "在", 1)]
    await rig.stop()


async def test_each_group_has_a_conversation_of_its_own_apart_from_the_owner(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="one"), Reply(text="two"), Reply(text="three"))
    await paired(rig)

    rig.tell(in_group("甲", "g1", group="GROUP01"))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(in_group("乙", "g2", group="GROUP02"))
    await until(lambda: len(rig.said()) == 2)
    rig.tell(private("丙", "m3"))
    await until(lambda: len(rig.said()) == 3)

    _, second, third = rig.harness.model.requests
    assert "甲" not in str(second.messages)
    assert "甲" not in str(third.messages)
    assert "乙" not in str(third.messages)
    await rig.stop()


async def test_the_conversation_of_a_group_goes_on_after_a_new_start(
    rig: Rig, harness: Harness
) -> None:
    rig.set_up(owner=OWNER, groups={"GROUP01": "s-group"})
    cwd = rig.cwd / "groups" / "GROUP01"
    cwd.mkdir(parents=True)
    before = await harness.agents.create(harness.ctx, harness.spec(cwd=cwd), session_id="s-group")
    harness.model.add(Reply(text="记下了"), Reply(text="42"))
    before.followup("记住 42")
    await before.when_idle()
    await before.close()

    await rig.start()
    rig.tell(in_group("是几？", "g1"))
    await until(lambda: len(rig.said()) == 1)

    assert "记住 42" in str(harness.model.requests[-1].messages)
    await rig.stop()


async def test_a_message_in_a_group_that_holds_no_words_is_said_to_be_unread(rig: Rig) -> None:
    await paired(rig)

    rig.tell(in_group("", "g1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said() == [("g1", "我现在只看得懂文字。", 1)]
    await rig.stop()


async def test_a_long_answer_in_a_group_goes_on_in_up_to_five_messages(rig: Rig) -> None:
    line = "段落" * 400
    rig.harness.model.add(Reply(text="\n".join([line] * 10)))
    await paired(rig)

    rig.tell(in_group("说吧", "g1"))
    await until(lambda: len(rig.said()) == 5)
    await asyncio.sleep(0.05)

    assert [number for _, _, number in rig.said()] == [1, 2, 3, 4, 5]
    await rig.stop()


async def test_a_group_whose_conversation_cannot_be_opened_says_so_and_tries_again_later(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.harness.model.add(Reply(text="好了"))
    assistant = await paired(rig)
    real = assistant._opened  # pyright: ignore[reportPrivateUsage]
    broken = [True]

    async def opened(talk: Any, session_id: str | None) -> Any:
        if broken[0]:
            raise OSError("the disk is full")
        return await real(talk, session_id)

    monkeypatch.setattr(assistant, "_opened", opened)
    rig.tell(in_group("你好", "g1"))
    await until(lambda: len(rig.said()) == 1)
    broken[0] = False
    rig.tell(in_group("你好", "g2"))
    await until(lambda: len(rig.said()) == 2)

    assert rig.said() == [
        ("g1", "这个群的对话没能打开：the disk is full。", 1),
        ("g2", "好了", 1),
    ]
    await rig.stop()


async def test_the_agents_of_the_assistant_change_nothing(rig: Rig, harness: Harness) -> None:
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await rig.harness.ctx.plugin(plugin)
    await rig.harness.ctx.plugin(columbina.permissions, {"preset": "full-access"})
    rig.harness.model.add(Reply(text="ok"))
    assistant = await paired(rig)

    rig.tell(in_group("改一下", "g1"))
    await until(lambda: len(rig.said()) == 1)

    owner = assistant._owner.agent  # pyright: ignore[reportPrivateUsage]
    group = assistant._groups["GROUP01"].agent  # pyright: ignore[reportPrivateUsage]
    assert owner is not None
    assert group is not None
    permissions = harness.ctx.use(PERMISSIONS)
    assert permissions.in_force(owner.session) == ("read-only", "ask")
    assert permissions.in_force(group.session) == ("read-only", "ask")
    await rig.stop()


# What comes in more than once, or is no message


async def test_a_message_that_comes_again_is_passed_over_also_after_a_new_start(
    rig: Rig, harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.harness.model.add(Reply(text="once"))
    await paired(rig)

    rig.tell(private("hi", "m1"))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(private("hi", "m1"))
    await asyncio.sleep(0.05)

    assert len(rig.said()) == 1
    assert json.loads((rig.home / store.FOLDER / SEEN).read_text(encoding="utf-8")) == ["m1"]
    await rig.stop()


async def test_messages_that_were_taken_cannot_be_written_down_is_said(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="still answered"))
    (rig.home / store.FOLDER / SEEN).mkdir(parents=True)
    await paired(rig)

    rig.tell(private("hi", "m1"))
    await until(lambda: len(rig.said()) == 1)

    told = store.status(rig.home)
    assert told is not None
    assert (told.error or "").startswith("which messages were taken cannot be written down")
    await rig.stop()


@pytest.mark.parametrize(
    "event",
    [
        ("GUILD_MESSAGE_CREATE", {"id": "x1", "content": "hi"}),
        ("C2C_MESSAGE_CREATE", {"content": "hi"}),
        ("C2C_MESSAGE_CREATE", {"id": "x2", "content": "hi"}),
        (
            "GROUP_AT_MESSAGE_CREATE",
            {"id": "x3", "content": "hi", "author": {"member_openid": "M"}},
        ),
        ("GROUP_AT_MESSAGE_CREATE", {"id": "x4", "content": "hi", "group_openid": "G"}),
    ],
    ids=["other kind", "no id", "nobody", "no group", "no member"],
)
async def test_what_is_no_message_to_be_answered_is_passed_over(
    rig: Rig, event: tuple[str, dict[str, Any]]
) -> None:
    await paired(rig)

    rig.tell(event)
    await asyncio.sleep(0.05)

    assert rig.said() == []
    await rig.stop()


async def test_a_message_from_before_a_new_start_is_answered_where_it_waited(
    rig: Rig, harness: Harness
) -> None:
    from columbina.agent import InboxItem
    from columbina.agent.inbox import enqueue

    rig.set_up(owner=OWNER, session_id="s-left")
    rig.cwd.mkdir(parents=True)
    left = await harness.agents.create(harness.ctx, harness.spec(cwd=rig.cwd), session_id="s-left")
    enqueue(
        left.session, "next-turn", InboxItem(message=UserMessage(content=(TextBlock(text="hi"),)))
    )
    await left.close()
    harness.model.add(Reply(text="I am"))

    await rig.start()
    await until(lambda: len(harness.model.requests) == 1)
    await asyncio.sleep(0.05)

    # Nobody knows any more which message it answers, so it is not sent.
    assert rig.said() == []
    await rig.stop()


# Another program that runs the assistant


@pytest.fixture
def input_closes(monkeypatch: pytest.MonkeyPatch) -> asyncio.Event:
    closes = asyncio.Event()
    monkeypatch.setattr(bot_module, "when_the_input_closes", lambda: closes)
    return closes


def lines_of(out: str) -> list[dict[str, Any]]:
    return [json.loads(line) for line in out.splitlines() if line.startswith("{")]


async def test_a_program_that_runs_the_assistant_hears_how_it_is_doing_and_stops_it(
    rig: Rig, input_closes: asyncio.Event, capsys: pytest.CaptureFixture[str]
) -> None:
    rig.harness.model.add(Reply(text="never", hold=True))
    await paired(rig, json_lines=True)
    rig.tell(private("one", "m1"))
    await asyncio.wait_for(rig.harness.model.held(), timeout=5)

    input_closes.set()
    assert rig.task is not None
    assert await asyncio.wait_for(rig.task, timeout=5) == 0

    assert rig.said() == [("m1", "（Columbina 中途退出了，这次的回答没写完。）", 1)]
    states = [line["state"] for line in lines_of(capsys.readouterr().out)]
    assert states[0] == "connecting"
    assert "connected" in states
    assert states[-1] == "stopped"


async def test_what_is_said_of_how_it_is_doing_is_let_go_where_nobody_reads_it(
    rig: Rig,
    input_closes: asyncio.Event,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    caplog.set_level(logging.INFO)

    class Gone:
        def write(self, text: str) -> int:
            raise OSError("the pipe is gone")

        def flush(self) -> None:
            raise OSError("the pipe is gone")

    rig.set_up(owner=OWNER)
    monkeypatch.setattr("sys.stdout", Gone())
    await rig.start(json_lines=True)
    input_closes.set()
    assert rig.task is not None
    await asyncio.wait_for(rig.task, timeout=5)

    assert "how the assistant is doing cannot be said: the pipe is gone" in caplog.text


async def test_a_program_that_stops_an_assistant_that_failed_hears_why(
    rig: Rig, input_closes: asyncio.Event
) -> None:
    rig.set_up(owner=OWNER)
    rig.opened.outcomes = [_Bye(4914)]

    await rig.start(json_lines=True)
    await until(lambda: rig.task is not None and rig.task.done())

    assert ended(rig) == 1


# Ending


async def test_an_agent_that_ends_badly_is_logged(
    rig: Rig, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.ERROR)
    assistant = await paired(rig)
    agent = assistant._owner.agent  # pyright: ignore[reportPrivateUsage]
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

    async def take(kind: str, data: dict[str, Any]) -> None:
        if not taken:
            taken.append("broken")
            raise RuntimeError("broken")
        await real(kind, data)

    monkeypatch.setattr(assistant, "_take", take)
    rig.tell(private("one", "m1"))
    rig.tell(private("two", "m2"))
    await until(lambda: len(rig.said()) == 1)

    assert "a message from QQ could not be taken" in caplog.text
    await rig.stop()


async def test_a_second_message_in_a_group_goes_on_in_its_conversation(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="one"), Reply(text="two"))
    await paired(rig)

    rig.tell(in_group("甲", "g1"))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(in_group("乙", "g2"))
    await until(lambda: len(rig.said()) == 2)

    assert "甲" in str(rig.harness.model.requests[1].messages)
    await rig.stop()


async def test_a_pairing_that_cannot_be_written_is_said_to_be_so(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    rig.set_up()
    await rig.start()

    def unwritable(home: Path, chosen: store.Settings) -> None:
        raise QqError("qq.json cannot be written: no room", code="STORE")

    monkeypatch.setattr(store, "save", unwritable)
    rig.tell(private(the_code(rig), "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert rig.said()[0][1].startswith("配对没能记下来：qq.json cannot be written: no room")
    await rig.stop()


async def test_what_waits_on_the_side_when_the_assistant_stops_is_broken_off(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    assistant = await paired(rig)
    hanging = asyncio.Event()

    async def hangs(asked: Any, text: str) -> None:
        hanging.set()
        await asyncio.Event().wait()

    monkeypatch.setattr(assistant, "_closed", hangs)
    rig.tell(private("/help", "m1"))
    await asyncio.wait_for(hanging.wait(), timeout=5)

    await rig.stop()

    assert rig.task is not None
    assert rig.task.done()


async def test_nothing_goes_out_once_the_assistant_has_stopped_and_an_answer_goes_out_once(
    rig: Rig,
) -> None:
    assistant = await paired(rig)
    await rig.stop()
    asked = bot_module._Asked("m1", None, OWNER, rig.clock())  # pyright: ignore[reportPrivateUsage]

    await assistant._closed(asked, "late")  # pyright: ignore[reportPrivateUsage]
    await assistant._closed(asked, "later")  # pyright: ignore[reportPrivateUsage]

    assert rig.said() == []


async def test_an_event_of_another_kind_is_not_even_noted(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="ok"))
    await paired(rig)

    rig.tell(("GUILD_MESSAGE_CREATE", {"id": "x1", "content": "hi"}))
    rig.tell(private("hi", "m1"))
    await until(lambda: len(rig.said()) == 1)

    assert json.loads((rig.home / store.FOLDER / SEEN).read_text(encoding="utf-8")) == ["m1"]
    await rig.stop()


async def test_a_message_is_answered_once(rig: Rig) -> None:
    assistant = await paired(rig)
    asked = bot_module._Asked("m1", None, OWNER, rig.clock())  # pyright: ignore[reportPrivateUsage]

    await assistant._closed(asked, "first")  # pyright: ignore[reportPrivateUsage]
    await assistant._closed(asked, "second")  # pyright: ignore[reportPrivateUsage]

    assert rig.said() == [("m1", "first", 1)]
    await rig.stop()


# The memory of a group


async def test_a_group_remembers_what_it_is_asked_to_and_is_told_it_after(rig: Rig) -> None:
    (rig.cwd).mkdir(parents=True, exist_ok=True)
    (rig.cwd / "AGENTS.md").write_text("Call yourself 小月.", encoding="utf-8")
    rig.harness.model.add(
        Reply(calls=(Call(name="remember", arguments={"note": "周五聚餐在七点"}),)),
        Reply(text="记住了"),
        Reply(text="七点"),
    )
    await paired(rig)

    rig.tell(in_group("记一下周五聚餐七点", "g1"))
    await until(lambda: len(rig.said()) == 1)
    rig.tell(in_group("聚餐几点？", "g2"))
    await until(lambda: len(rig.said()) == 2)

    memory_file = rig.cwd / "groups" / "GROUP01" / "memory.md"
    assert memory_file.read_text(encoding="utf-8") == "周五聚餐在七点\n"
    first, _, last = rig.harness.model.requests
    assert any(
        "This group has no memory yet." in "".join(block.text for block in message.content)
        for message in first.messages
        if isinstance(message, UserMessage)
    )
    context = [
        "".join(block.text for block in message.content)
        for message in last.messages
        if isinstance(message, UserMessage)
    ]
    assert any("Call yourself 小月." in said and "周五聚餐在七点" in said for said in context)


async def test_the_owner_is_told_nothing_of_the_groups(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="hi"))
    assistant = await paired(rig)

    rig.tell(private("你好", "m1"))
    await until(lambda: len(rig.said()) == 1)

    owner = assistant._owner.agent  # pyright: ignore[reportPrivateUsage]
    assert owner is not None
    assert await assistant.told_in_a_group(owner) is None
    run = ToolRun("c", rig.cwd, owner.session)
    assert assistant.memory_file(run) is None


async def test_a_named_group_keeps_its_memory_under_its_name(rig: Rig) -> None:
    rig.harness.model.add(
        Reply(calls=(Call(name="remember", arguments={"note": "猫叫团子"}),)),
        Reply(text="好"),
    )
    rig.set_up(owner=OWNER, names={"GROUP01": "家人群"})
    await rig.start()

    rig.tell(in_group("记住猫叫团子", "g1"))
    await until(lambda: len(rig.said()) == 1)

    assert (rig.cwd / "groups" / "家人群" / "memory.md").read_text(encoding="utf-8") == "猫叫团子\n"


async def test_names_that_cannot_be_read_now_are_those_that_were_read(rig: Rig) -> None:
    rig.harness.model.add(Reply(text="好"))
    rig.set_up(owner=OWNER, names={"GROUP01": "家"})
    assistant = await rig.start()
    rig.tell(in_group("你好", "g1"))
    await until(lambda: len(rig.said()) == 1)
    (rig.home / store.SETTINGS).unlink()
    (rig.home / store.SETTINGS).mkdir()

    group = assistant._groups["GROUP01"].agent  # pyright: ignore[reportPrivateUsage]
    assert group is not None
    where = assistant.memory_file(ToolRun("c", rig.cwd, group.session))

    assert where == rig.cwd / "groups" / "家" / "memory.md"
