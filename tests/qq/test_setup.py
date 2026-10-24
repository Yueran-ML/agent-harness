"""Setting up the assistant in QQ: ``columbina.qq.setup``."""

from pathlib import Path

import pytest

from columbina.qq import store
from columbina.qq.app import TOKEN_URL, QqError
from columbina.qq.setup import (
    NEXT,
    at_a_terminal,
    forgotten,
    kept_from,
    read_given,
    status_of,
)
from columbina.session.lock import acquire
from support.fake_qq import APP_ID, SECRET, FakeApi
from support.vault import Memory

BOT = {"app_id": APP_ID, "secret": SECRET}


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


@pytest.fixture
def qq() -> FakeApi:
    return FakeApi()


# Keeping a bot


async def test_a_bot_that_qq_takes_is_kept(home: Path, qq: FakeApi) -> None:
    answer = await kept_from(home, BOT, {}, transport=qq.transport)

    assert answer == {"stored": {"app_id": APP_ID}, "secret_in": "file"}
    assert store.settings(home) == store.Settings(app_id=APP_ID)
    assert store.secret(home, TOKEN_URL) == SECRET
    assert qq.tokens == 1


async def test_the_answer_says_that_the_secret_went_into_the_keyring(
    home: Path, qq: FakeApi
) -> None:
    Memory().install()

    answer = await kept_from(home, BOT, {}, transport=qq.transport)

    assert answer["secret_in"] == "keyring"


async def test_a_bot_is_kept_without_the_blanks_around_its_id_and_its_secret(
    home: Path, qq: FakeApi
) -> None:
    given = {"app_id": f" {APP_ID} ", "secret": f" {SECRET}\n"}

    await kept_from(home, given, {}, transport=qq.transport)

    assert store.secret(home, TOKEN_URL) == SECRET


@pytest.mark.parametrize(
    ("given", "said"),
    [
        ({"secret": SECRET}, "the AppID is the one"),
        ({"app_id": "9" * 65, "secret": SECRET}, "the AppID is the one"),
        ({"app_id": "12 34\n", "secret": SECRET}, "the AppID is the one"),
        ({"app_id": APP_ID}, "the AppSecret is the one"),
        ({"app_id": APP_ID, "secret": 7}, "the AppSecret is the one"),
        ({"app_id": APP_ID, "secret": "two halves"}, "the AppSecret is the one"),
    ],
    ids=["no id", "too long", "not sendable", "no secret", "no text", "not sendable secret"],
)
async def test_what_is_no_bot_is_not_kept(
    home: Path, qq: FakeApi, given: dict[str, object], said: str
) -> None:
    with pytest.raises(QqError, match=said) as refused:
        await kept_from(home, given, {}, transport=qq.transport)

    assert refused.value.code == "INVALID"
    assert qq.tokens == 0
    assert store.settings(home) is None


async def test_a_bot_that_qq_does_not_take_is_not_kept(home: Path, qq: FakeApi) -> None:
    with pytest.raises(QqError) as refused:
        await kept_from(
            home, {"app_id": APP_ID, "secret": "another-secret"}, {}, transport=qq.transport
        )

    assert refused.value.code == "REFUSED"
    assert store.settings(home) is None


async def test_a_bot_is_not_changed_while_its_assistant_runs(home: Path, qq: FakeApi) -> None:
    (home / store.FOLDER).mkdir(parents=True)
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        with pytest.raises(QqError, match="an assistant in QQ runs") as refused:
            await kept_from(home, BOT, {}, transport=qq.transport)
        with pytest.raises(QqError, match="an assistant in QQ runs"):
            forgotten(home)
    finally:
        held.release()

    assert refused.value.code == "RUNNING"


async def test_a_directory_that_cannot_be_made_is_said_to_be_so(home: Path, qq: FakeApi) -> None:
    home.mkdir()
    (home / store.FOLDER).write_text("a file", encoding="utf-8")

    with pytest.raises(QqError) as refused:
        await kept_from(home, BOT, {}, transport=qq.transport)

    assert refused.value.code == "STORE"


# How it is doing


def test_nothing_set_up_is_said_so(home: Path, tmp_path: Path) -> None:
    assert status_of(home, {}) == {
        "set_up": False,
        "running": False,
        "folder": str(tmp_path / "Columbina" / "qq"),
    }


async def test_a_bot_whose_assistant_does_not_run_is_said_so(home: Path, qq: FakeApi) -> None:
    await kept_from(home, BOT, {}, transport=qq.transport)

    said = status_of(home, {store.ASSISTANT_HOME: "/somewhere"})

    assert said == {
        "set_up": True,
        "app_id": APP_ID,
        "paired": False,
        "folder": str(Path("/somewhere")),
        "running": False,
        "state": None,
        "code": None,
        "error": None,
    }


async def test_a_running_assistant_says_how_it_is_doing(home: Path, qq: FakeApi) -> None:
    await kept_from(home, BOT, {}, transport=qq.transport)
    store.tell(home, 1, "connected", paired=False, code="123456", error="slow")
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        said = status_of(home, {})
    finally:
        held.release()

    assert (said["running"], said["state"], said["code"], said["error"]) == (
        True,
        "connected",
        "123456",
        "slow",
    )


async def test_an_assistant_that_failed_says_why_after_it_ended(home: Path, qq: FakeApi) -> None:
    await kept_from(home, BOT, {}, transport=qq.transport)
    store.save(home, store.Settings(app_id=APP_ID, owner="OWNER01"))
    store.tell(home, 1, "failed", paired=True, code="123456", error="QQ refused it")

    said = status_of(home, {})

    assert (said["paired"], said["state"], said["code"], said["error"]) == (
        True,
        "failed",
        None,
        "QQ refused it",
    )


async def test_an_assistant_that_stopped_says_nothing_of_how_it_was(
    home: Path, qq: FakeApi
) -> None:
    await kept_from(home, BOT, {}, transport=qq.transport)
    store.tell(home, 1, "stopped", paired=False)

    assert status_of(home, {})["state"] is None


# Forgetting it


async def test_a_bot_is_forgotten(home: Path, qq: FakeApi) -> None:
    await kept_from(home, BOT, {}, transport=qq.transport)

    assert forgotten(home) == {"forgotten": True}
    assert forgotten(home) == {"forgotten": False}
    assert store.settings(home) is None


def test_nothing_is_forgotten_where_nothing_was_ever_kept(home: Path) -> None:
    assert forgotten(home) == {"forgotten": False}


# What another program gives


def test_a_bot_is_read_from_a_document_of_json() -> None:
    assert read_given('{"app_id": "1", "secret": "s"}') == {"app_id": "1", "secret": "s"}


@pytest.mark.parametrize("text", ["", "[1]", "{"])
def test_what_is_no_object_of_json_is_refused(text: str) -> None:
    with pytest.raises(QqError, match="no object of JSON") as refused:
        read_given(text)

    assert refused.value.code == "INVALID"


# At a terminal


async def test_a_person_sets_the_bot_up_at_a_terminal(home: Path, qq: FakeApi) -> None:
    shown: list[str] = []
    asked: list[str] = []

    def ask(question: str) -> str:
        asked.append(question)
        return APP_ID

    def ask_unseen(question: str) -> str:
        asked.append(question)
        return SECRET

    code = await at_a_terminal(home, {}, ask, ask_unseen, shown.append, transport=qq.transport)

    assert code == 0
    assert asked == ["AppID: ", "AppSecret: "]
    assert "q.qq.com" in shown[0]
    assert shown[1].startswith("QQ takes the bot, and it is kept. No keyring of the system took")
    assert shown[1].endswith(NEXT)


async def test_a_person_is_told_that_the_secret_went_into_the_keyring(
    home: Path, qq: FakeApi
) -> None:
    Memory().install()
    shown: list[str] = []

    await at_a_terminal(
        home, {}, lambda q: APP_ID, lambda q: SECRET, shown.append, transport=qq.transport
    )

    assert "The AppSecret is in the keyring of the system." in shown[1]


async def test_a_person_is_told_why_the_bot_is_not_kept(home: Path, qq: FakeApi) -> None:
    shown: list[str] = []

    code = await at_a_terminal(
        home, {}, lambda q: APP_ID, lambda q: "wrong", shown.append, transport=qq.transport
    )

    assert code == 2
    assert shown[1].startswith("The bot is not kept: QQ does not take this AppID")
