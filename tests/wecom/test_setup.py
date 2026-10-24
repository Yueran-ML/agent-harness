"""Setting up the assistant in WeCom: ``columbina.wecom.setup``."""

import json
from pathlib import Path
from typing import Any

import pytest

from columbina.session.lock import acquire
from columbina.wecom import store
from columbina.wecom.app import WecomError
from columbina.wecom.setup import (
    NEXT,
    at_a_terminal,
    checked,
    forgotten,
    kept_from,
    made_thinking,
    made_writable,
    read_given,
    said,
    status_of,
)
from support.fake_wecom import BOT_ID, PLACE, SECRET, FakeSocket, Opened, answered, serving
from support.vault import Memory

BOT = {"bot_id": BOT_ID, "secret": SECRET}


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def taking() -> Opened:
    return Opened(FakeSocket())


def refusing(errcode: int) -> Opened:
    return Opened(FakeSocket(refusing={"aibot_subscribe": errcode}))


# Keeping a bot


async def test_a_bot_that_wecom_takes_is_kept(home: Path) -> None:
    opened = taking()

    answer = await kept_from(home, BOT, {}, opener=opened)

    assert answer == {"stored": {"bot_id": BOT_ID}, "secret_in": "file"}
    assert store.settings(home) == store.Settings(bot_id=BOT_ID)
    assert store.secret(home, PLACE) == SECRET
    # WeCom was asked once, where the bot signs in, and the connection was closed.
    assert opened.asked == [(PLACE, None)]


async def test_the_answer_says_that_the_secret_went_into_the_keyring(home: Path) -> None:
    Memory().install()

    answer = await kept_from(home, BOT, {}, opener=taking())

    assert answer["secret_in"] == "keyring"


async def test_a_bot_is_kept_without_the_blanks_around_its_id_and_its_secret(home: Path) -> None:
    await kept_from(home, {"bot_id": f" {BOT_ID} ", "secret": f" {SECRET}\n"}, {}, opener=taking())

    assert store.settings(home) == store.Settings(bot_id=BOT_ID)
    assert store.secret(home, PLACE) == SECRET


async def test_a_bot_is_kept_for_the_place_that_the_environment_names(home: Path) -> None:
    place = "wss://wecom.example.com"
    opened = taking()

    await kept_from(home, BOT, {"COLUMBINA_WECOM_WS": place}, opener=opened)

    assert opened.asked == [(place, None)]
    assert store.secret(home, place) == SECRET


@pytest.mark.parametrize(
    ("given", "says"),
    [
        ({"secret": SECRET}, "the Bot ID is the one"),
        ({"bot_id": "", "secret": SECRET}, "the Bot ID is the one"),
        ({"bot_id": "two words", "secret": SECRET}, "the Bot ID is the one"),
        ({"bot_id": "x" * 129, "secret": SECRET}, "the Bot ID is the one"),
        ({"bot_id": 7, "secret": SECRET}, "the Bot ID is the one"),
        ({"bot_id": BOT_ID}, "the Secret is the one"),
        ({"bot_id": BOT_ID, "secret": "two halves"}, "the Secret is the one"),
        ({"bot_id": BOT_ID, "secret": 7}, "the Secret is the one"),
    ],
)
async def test_what_is_no_bot_is_not_kept(home: Path, given: dict[str, object], says: str) -> None:
    opened = taking()

    with pytest.raises(WecomError) as refused:
        await kept_from(home, given, {}, opener=opened)

    assert refused.value.code == "INVALID"
    assert str(refused.value).startswith(says)
    assert opened.asked == []
    assert store.settings(home) is None


async def test_a_bot_id_of_the_longest_length_is_one(home: Path) -> None:
    longest = "x" * 128

    answer = await kept_from(home, {"bot_id": longest, "secret": SECRET}, {}, opener=taking())

    assert answer["stored"] == {"bot_id": longest}


@pytest.mark.parametrize(("errcode", "code"), [(853000, "REFUSED"), (846601, "NOT_LONG")])
async def test_a_bot_that_wecom_does_not_take_is_not_kept(
    home: Path, errcode: int, code: str
) -> None:
    with pytest.raises(WecomError) as refused:
        await kept_from(home, BOT, {}, opener=refusing(errcode))

    assert refused.value.code == code
    assert store.settings(home) is None
    assert SECRET not in str(refused.value)


async def test_a_bot_is_checked_with_the_library_of_websockets_where_no_opener_is_given() -> None:
    asked: list[dict[str, Any]] = []

    async def wecom(connection: Any) -> None:
        async for text in connection:
            frame = json.loads(text)
            asked.append(frame)
            await connection.send(answered(frame))

    place, stop = serving(wecom)
    try:
        await checked(BOT_ID, SECRET, {"COLUMBINA_WECOM_WS": place})
    finally:
        stop()

    assert [frame["body"] for frame in asked] == [{"bot_id": BOT_ID, "secret": SECRET}]


async def test_the_bot_is_neither_kept_nor_deleted_while_an_assistant_runs(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        with pytest.raises(WecomError) as running:
            await kept_from(home, {"bot_id": "aibOther", "secret": SECRET}, {}, opener=taking())
        with pytest.raises(WecomError) as deleting:
            forgotten(home)
    finally:
        held.release()

    assert running.value.code == deleting.value.code == "RUNNING"
    assert store.settings(home) == store.Settings(bot_id=BOT_ID)


async def test_a_directory_of_the_assistant_that_cannot_be_used_keeps_nothing(home: Path) -> None:
    home.mkdir()
    (home / store.FOLDER).write_text("a file where the folder would be", encoding="utf-8")

    with pytest.raises(WecomError) as refused:
        await kept_from(home, BOT, {}, opener=taking())

    assert refused.value.code == "STORE"
    assert store.settings(home) is None


# Whether the agent may change files


async def test_whether_the_agent_may_change_files_is_kept_by_itself(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    kept = store.Settings(bot_id=BOT_ID, owner="zhangsan", session_id="s-1")
    store.save(home, kept)

    assert await kept_from(home, {"writable": True}, {}) == {"writable": True}

    assert store.settings(home) == kept.model_copy(update={"writable": True})
    assert status_of(home, {})["writable"] is True
    assert made_writable(home, False, {}) == {"writable": False}
    assert store.settings(home) == kept


@pytest.mark.parametrize("given", ["true", 1, None])
def test_whether_the_agent_may_change_files_is_true_or_false(home: Path, given: object) -> None:
    with pytest.raises(WecomError) as refused:
        made_writable(home, given, {})

    assert refused.value.code == "INVALID"


def test_without_a_bot_its_agent_is_let_change_nothing(home: Path) -> None:
    with pytest.raises(WecomError) as refused:
        made_writable(home, True, {})

    assert refused.value.code == "NONE"


async def test_whether_the_agent_may_change_files_is_not_changed_while_it_runs(
    home: Path,
) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        with pytest.raises(WecomError) as refused:
            made_writable(home, True, {})
    finally:
        held.release()

    assert refused.value.code == "RUNNING"
    assert store.settings(home) == store.Settings(bot_id=BOT_ID)


async def test_an_agent_whose_directory_lies_in_the_home_of_the_program_changes_nothing(
    home: Path,
) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    env = {store.ASSISTANT_HOME: str(home / "assistant")}

    with pytest.raises(WecomError) as refused:
        await kept_from(home, {"writable": True}, env)

    assert refused.value.code == "FOLDER"
    assert store.ASSISTANT_HOME in str(refused.value)
    assert store.settings(home) == store.Settings(bot_id=BOT_ID)
    # It may always be let change nothing.
    assert made_writable(home, False, env) == {"writable": False}


# How it is doing


def test_nothing_set_up_is_said(home: Path) -> None:
    assert status_of(home, {}) == {
        "set_up": False,
        "running": False,
        "folder": str(home.parent / "Columbina" / "wecom"),
    }


async def test_a_bot_that_is_set_up_and_not_running_is_said(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    store.tell(home, 1, "connected", paired=False, code="123456")

    assert status_of(home, {store.ASSISTANT_HOME: str(home.parent / "mine")}) == {
        "set_up": True,
        "bot_id": BOT_ID,
        "paired": False,
        "folder": str(home.parent / "mine"),
        "writable": False,
        "model": None,
        "effort": None,
        "running": False,
        "state": None,
        "code": None,
        "error": None,
    }


async def test_a_running_assistant_says_how_it_is_doing(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    store.save(home, store.Settings(bot_id=BOT_ID, owner="zhangsan"))
    store.tell(home, 1, "connected", paired=True, code=None, error="a hiccup")
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        said_now = status_of(home, {})
    finally:
        held.release()

    assert (said_now["running"], said_now["paired"], said_now["state"], said_now["error"]) == (
        True,
        True,
        "connected",
        "a hiccup",
    )


async def test_an_assistant_that_failed_says_why_after_it_ended(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    store.tell(home, 1, "failed", paired=False, code="123456", error="refused")

    said_now = status_of(home, {})

    assert (said_now["state"], said_now["code"], said_now["error"]) == ("failed", None, "refused")


async def test_an_assistant_that_stopped_says_nothing_of_its_last_run(home: Path) -> None:
    await kept_from(home, BOT, {}, opener=taking())
    store.tell(home, 1, "stopped", paired=False, error="old")

    said_now = status_of(home, {})

    assert (said_now["state"], said_now["error"]) == (None, None)


# Forgetting


async def test_a_bot_is_forgotten_once(home: Path) -> None:
    # Where nothing was ever kept, there is nothing to forget and nothing to hold.
    assert forgotten(home) == {"forgotten": False}
    await kept_from(home, BOT, {}, opener=taking())

    assert forgotten(home) == {"forgotten": True}
    assert forgotten(home) == {"forgotten": False}
    assert store.settings(home) is None


# What another program gives and is answered


def test_a_document_of_json_is_read_and_answered_in_one_line() -> None:
    assert read_given('{"bot_id": "x"}') == {"bot_id": "x"}
    assert said({"stored": {"bot_id": "笔"}}) == '{"stored": {"bot_id": "笔"}}'
    for text in ("", "[1]", "{"):
        with pytest.raises(WecomError) as refused:
            read_given(text)
        assert refused.value.code == "INVALID"


async def test_a_document_that_says_more_than_that_is_one_of_a_bot(home: Path) -> None:
    with pytest.raises(WecomError) as refused:
        await kept_from(home, {"writable": True, "bot_id": BOT_ID}, {})

    assert refused.value.code == "INVALID"


# At a terminal


async def test_a_person_sets_the_bot_up_at_a_terminal(home: Path) -> None:
    shown: list[str] = []

    code = await at_a_terminal(
        home, {}, lambda question: BOT_ID, lambda question: SECRET, shown.append, opener=taking()
    )

    assert code == 0
    assert store.secret(home, PLACE) == SECRET
    assert shown[0].startswith("The assistant answers you in WeCom")
    assert f"lies as it is in {home / store.CREDENTIALS}" in shown[1]
    assert shown[1].endswith(NEXT)


async def test_a_person_is_told_where_the_keyring_took_the_secret(home: Path) -> None:
    Memory().install()
    shown: list[str] = []

    await at_a_terminal(
        home, {}, lambda question: BOT_ID, lambda question: SECRET, shown.append, opener=taking()
    )

    assert "The Secret is in the keyring of the system." in shown[1]


async def test_a_bot_that_is_not_kept_is_said_to_be_so_at_a_terminal(home: Path) -> None:
    shown: list[str] = []

    code = await at_a_terminal(
        home,
        {},
        lambda question: BOT_ID,
        lambda question: SECRET,
        shown.append,
        opener=refusing(853000),
    )

    assert code == 2
    assert shown[-1].startswith("The bot is not kept: WeCom does not take this Bot ID")
    assert json.dumps(SECRET) not in "".join(shown)


# The model of the assistant, and how hard it thinks


async def test_the_model_and_how_hard_it_thinks_are_kept_by_themselves(home: Path) -> None:
    kept = store.Settings(bot_id=BOT_ID, owner="zhangsan", groups={"group-1": "s-1"})
    store.keep(home, kept, SECRET, PLACE)

    answer = await kept_from(home, {"model": "deepseek-flash", "effort": "low"}, {})

    assert answer == {"model": "deepseek-flash", "effort": "low"}
    assert store.settings(home) == kept.model_copy(
        update={"model": "deepseek-flash", "effort": "low"}
    )
    assert status_of(home, {})["effort"] == "low"
    assert made_thinking(home, {"effort": None}) == {"model": None, "effort": None}


def test_a_model_or_a_level_that_is_none_is_refused_and_nothing_without_a_bot(home: Path) -> None:
    with pytest.raises(WecomError) as none:
        made_thinking(home, {"effort": "off"})
    store.keep(home, store.Settings(bot_id=BOT_ID), SECRET, PLACE)
    with pytest.raises(WecomError, match="a model is named by its id") as wrong:
        made_thinking(home, {"model": "two words"})

    assert (none.value.code, wrong.value.code) == ("NONE", "INVALID")
