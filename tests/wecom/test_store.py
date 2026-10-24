"""What is kept for the assistant in WeCom: ``columbina.wecom.store``."""

import sys
from pathlib import Path

import pytest

from columbina.llm import credentials
from columbina.llm.credentials import key_for
from columbina.wecom import store as store_module
from columbina.wecom.app import WecomError
from columbina.wecom.store import (
    ASSISTANT_HOME,
    CREDENTIALS,
    FOLDER,
    SETTINGS,
    STATUS,
    Settings,
    Status,
    directory,
    forget,
    keep,
    save,
    secret,
    settings,
    status,
    tell,
)
from support.fake_wecom import BOT_ID, PLACE, SECRET
from support.vault import Memory

POSIX = sys.platform != "win32"
BOT = Settings(bot_id=BOT_ID)
ELSEWHERE = "wss://wecom.example.com"


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def test_a_bot_is_kept_and_read_again(home: Path) -> None:
    keep(home, BOT, SECRET, PLACE)

    assert settings(home) == BOT
    assert secret(home, PLACE) == SECRET


def test_the_secret_lies_in_the_keyring_under_a_file_of_its_own(home: Path) -> None:
    vault = Memory().install()

    kept = keep(home, BOT, SECRET, PLACE)

    assert kept.keyring is not None
    filed_under = f"columbina {(home / CREDENTIALS).resolve()}"
    assert list(vault.held) == [(filed_under, "bot-secret")]
    assert SECRET.encode() not in (home / CREDENTIALS).read_bytes()
    assert SECRET.encode() not in (home / SETTINGS).read_bytes()


def test_where_no_keyring_takes_it_the_secret_lies_in_its_own_file(home: Path) -> None:
    kept = keep(home, BOT, SECRET, PLACE)

    assert kept.keyring is None
    assert SECRET.encode() in (home / CREDENTIALS).read_bytes()
    assert SECRET.encode() not in (home / SETTINGS).read_bytes()
    if POSIX:
        assert (home / CREDENTIALS).stat().st_mode & 0o777 == 0o600
        assert (home / SETTINGS).stat().st_mode & 0o777 == 0o600


def test_the_secret_is_handed_out_only_for_the_place_of_its_connection(home: Path) -> None:
    keep(home, BOT, SECRET, PLACE)

    with pytest.raises(WecomError) as refused:
        secret(home, ELSEWHERE)

    assert refused.value.code == "STORE"
    assert str(refused.value).startswith("the Secret of the bot cannot be used: ")
    assert SECRET not in str(refused.value)
    with pytest.raises(credentials.CredentialsError):
        key_for(home / CREDENTIALS, "bot-secret", ELSEWHERE)


def test_the_same_bot_kept_again_keeps_whom_it_answers_and_where(home: Path) -> None:
    keep(home, BOT, SECRET, PLACE)
    said = {
        "owner": "zhangsan",
        "session_id": "20261022T000000-ab",
        "writable": True,
        "groups": {"group-1": "20261024T000000-cd"},
        "model": "deepseek-flash",
        "effort": "off",
    }
    save(home, BOT.model_copy(update=said))

    keep(home, BOT, "another-secret-0123", PLACE)

    assert settings(home) == BOT.model_copy(update=said)
    assert secret(home, PLACE) == "another-secret-0123"


def test_another_bot_begins_with_nobody_to_answer(home: Path) -> None:
    keep(home, BOT, SECRET, PLACE)
    save(home, BOT.model_copy(update={"owner": "zhangsan", "session_id": "s", "writable": True}))

    keep(home, Settings(bot_id="aibOther"), SECRET, PLACE)

    assert settings(home) == Settings(bot_id="aibOther")


def test_nothing_is_set_up_where_nothing_is_kept(home: Path) -> None:
    assert settings(home) is None
    assert secret(home, PLACE) is None


@pytest.mark.parametrize("text", ["{", '{"version": 2, "bot_id": "x"}', "[]"])
def test_a_file_that_this_program_did_not_write_is_no_bot(home: Path, text: str) -> None:
    home.mkdir()
    (home / SETTINGS).write_text(text, encoding="utf-8")

    assert settings(home) is None


def test_a_file_that_cannot_be_read_is_said_to_be_so(home: Path) -> None:
    (home / SETTINGS).mkdir(parents=True)

    with pytest.raises(WecomError) as refused:
        settings(home)

    assert refused.value.code == "STORE"
    assert str(home / SETTINGS) in str(refused.value)


def test_a_file_that_is_no_text_is_said_to_be_so(home: Path) -> None:
    home.mkdir()
    (home / SETTINGS).write_bytes(b"\xff\xfe\x00")

    with pytest.raises(WecomError, match="it is not text"):
        settings(home)


def test_a_secret_that_cannot_be_one_is_not_kept(home: Path) -> None:
    with pytest.raises(WecomError) as refused:
        keep(home, BOT, "two halves", PLACE)

    assert refused.value.code == "STORE"
    assert "halves" not in str(refused.value)
    assert not (home / SETTINGS).exists()


def unwritable(path: Path, model: object, *, private: bool) -> None:
    raise WecomError(f"{path} cannot be written: no room", code="STORE")


def test_a_new_bot_that_cannot_be_written_down_leaves_the_secret_of_the_old_one(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(home, BOT, SECRET, PLACE)
    monkeypatch.setattr(store_module, "_written", unwritable)

    with pytest.raises(WecomError, match="no room"):
        keep(home, Settings(bot_id="aibOther"), "other-secret-0123", PLACE)

    assert settings(home) == BOT
    assert secret(home, PLACE) == SECRET


def test_a_first_bot_that_cannot_be_written_down_leaves_no_secret(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(store_module, "_written", unwritable)

    with pytest.raises(WecomError, match="no room"):
        keep(home, BOT, SECRET, PLACE)

    assert credentials.stored(home / CREDENTIALS) == {}


def test_a_bot_that_cannot_be_written_down_is_said_to_be_so(home: Path) -> None:
    (home / SETTINGS / "inside").mkdir(parents=True)

    with pytest.raises(WecomError) as refused:
        save(home, BOT)

    assert refused.value.code == "STORE"
    assert str(home / SETTINGS) in str(refused.value)


def test_forgetting_deletes_the_secret_and_the_bot_and_leaves_the_folder(home: Path) -> None:
    vault = Memory().install()
    keep(home, BOT, SECRET, PLACE)
    (home / FOLDER).mkdir(parents=True)

    assert forget(home) is True

    assert vault.held == {}
    assert settings(home) is None
    assert (home / FOLDER).is_dir()
    assert forget(home) is False


def test_forgetting_a_secret_alone_is_forgetting_something(home: Path) -> None:
    keep(home, BOT, SECRET, PLACE)
    (home / SETTINGS).unlink()

    assert forget(home) is True
    assert secret(home, PLACE) is None


def test_a_secret_that_its_keyring_does_not_give_up_is_said_to_be_so(home: Path) -> None:
    vault = Memory().install()
    keep(home, BOT, SECRET, PLACE)
    vault.locked = True

    with pytest.raises(WecomError) as refused:
        forget(home)

    assert refused.value.code == "STORE"
    assert (home / SETTINGS).exists()


def test_the_directory_of_the_assistant_lies_beside_the_home_of_the_program(
    tmp_path: Path,
) -> None:
    home = tmp_path / ".columbina"

    assert directory(home, {}) == tmp_path / "Columbina" / "wecom"
    assert directory(home, {ASSISTANT_HOME: str(tmp_path / "elsewhere")}) == tmp_path / "elsewhere"
    # A variable that names nothing is no directory.
    assert directory(home, {ASSISTANT_HOME: ""}) == tmp_path / "Columbina" / "wecom"


def test_how_the_assistant_is_doing_is_written_down_and_read_again(home: Path) -> None:
    told = tell(home, 1234, "connected", paired=False, code="123456", now=1.5)

    assert told == Status(pid=1234, state="connected", paired=False, code="123456", at=1.5)
    assert status(home) == told
    assert tell(home, 1, "connecting", paired=True).at > 1_700_000_000


def test_a_status_that_cannot_be_written_is_let_go(home: Path) -> None:
    home.mkdir()
    (home / FOLDER).write_text("a file where the folder would be", encoding="utf-8")

    told = tell(home, 1, "failed", paired=True, error="it went wrong", now=2.0)

    assert told.error == "it went wrong"
    assert status(home) is None


@pytest.mark.parametrize("text", [None, "{", b"\xff"])
def test_a_status_that_cannot_be_read_says_nothing(home: Path, text: str | bytes | None) -> None:
    (home / FOLDER).mkdir(parents=True)
    if isinstance(text, str):
        (home / FOLDER / STATUS).write_text(text, encoding="utf-8")
    elif isinstance(text, bytes):
        (home / FOLDER / STATUS).write_bytes(text)

    assert status(home) is None
