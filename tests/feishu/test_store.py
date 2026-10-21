"""What is kept for the assistant in Feishu: ``columbina.feishu.store``."""

import os
import sys
from pathlib import Path

import pytest

from columbina.feishu import store as store_module
from columbina.feishu.app import FeishuError, Host
from columbina.feishu.store import (
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
    within,
)
from columbina.llm import credentials
from columbina.llm.credentials import key_for
from support.vault import Memory

POSIX = sys.platform != "win32"
FEISHU = Host("feishu", "https://open.feishu.cn")
LARK = Host("lark", "https://open.larksuite.com")
APP = Settings(domain="feishu", app_id="cli_a1b2c3")
SECRET = "s3cr3t-0123456789abcdef"


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def test_an_app_is_kept_and_read_again(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)

    assert settings(home) == APP
    assert secret(home, FEISHU) == SECRET


def test_the_secret_lies_in_the_keyring_under_a_file_of_its_own(home: Path) -> None:
    vault = Memory().install()

    kept = keep(home, APP, SECRET, FEISHU)

    assert kept.keyring is not None
    filed_under = f"columbina {(home / CREDENTIALS).resolve()}"
    assert list(vault.held) == [(filed_under, "app-secret")]
    assert SECRET.encode() not in (home / CREDENTIALS).read_bytes()
    assert SECRET.encode() not in (home / SETTINGS).read_bytes()
    assert not (home / "credentials.json").exists()


def test_where_no_keyring_takes_it_the_secret_lies_in_its_own_file(home: Path) -> None:
    kept = keep(home, APP, SECRET, FEISHU)

    assert kept.keyring is None
    assert SECRET.encode() in (home / CREDENTIALS).read_bytes()
    assert SECRET.encode() not in (home / SETTINGS).read_bytes()
    if POSIX:
        assert (home / CREDENTIALS).stat().st_mode & 0o777 == 0o600
        assert (home / SETTINGS).stat().st_mode & 0o777 == 0o600


def test_the_secret_is_handed_out_only_for_the_host_of_its_app(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)

    with pytest.raises(FeishuError) as refused:
        secret(home, LARK)

    assert refused.value.code == "STORE"
    assert SECRET not in str(refused.value)
    with pytest.raises(credentials.CredentialsError):
        key_for(home / CREDENTIALS, "app-secret", "https://open.larksuite.com")


def test_the_secret_of_an_app_is_apart_from_the_keys_of_models(home: Path) -> None:
    credentials.keep(home / "credentials.json", "app-secret", "sk-model-0123", "https://m.test")

    keep(home, APP, SECRET, FEISHU)

    assert secret(home, FEISHU) == SECRET
    assert key_for(home / "credentials.json", "app-secret", "https://m.test") == "sk-model-0123"


def test_the_same_app_kept_again_keeps_whom_it_answers_and_where(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)
    said = {"owner": "ou_owner", "session_id": "20261020T000000-ab", "writable": True}
    save(home, APP.model_copy(update=said))

    keep(home, APP, "another-secret-0123", FEISHU)

    assert settings(home) == APP.model_copy(update=said)
    assert secret(home, FEISHU) == "another-secret-0123"


@pytest.mark.parametrize(
    "other",
    [
        Settings(domain="feishu", app_id="cli_other"),
        Settings(domain="lark", app_id="cli_a1b2c3"),
    ],
)
def test_another_app_begins_with_nobody_to_answer(home: Path, other: Settings) -> None:
    keep(home, APP, SECRET, FEISHU)
    save(home, APP.model_copy(update={"owner": "ou_owner", "session_id": "s", "writable": True}))

    keep(home, other, SECRET, FEISHU if other.domain == "feishu" else LARK)

    assert settings(home) == other


def test_an_app_kept_before_its_agent_could_change_files_changes_none(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)
    written = (home / SETTINGS).read_text(encoding="utf-8").replace('  "writable": false\n', "")
    (home / SETTINGS).write_text(written.replace(",\n}", "\n}"), encoding="utf-8")

    assert "writable" not in (home / SETTINGS).read_text(encoding="utf-8")
    assert settings(home) == APP


def test_the_directory_of_the_assistant_lies_beside_the_home_of_the_program(
    tmp_path: Path,
) -> None:
    home = tmp_path / ".columbina"

    assert directory(home, {}) == tmp_path / "Columbina" / "feishu"
    assert directory(home, {ASSISTANT_HOME: str(tmp_path / "elsewhere")}) == tmp_path / "elsewhere"
    # A variable that names nothing is no directory.
    assert directory(home, {ASSISTANT_HOME: ""}) == tmp_path / "Columbina" / "feishu"


def test_a_path_lies_in_a_folder_as_the_two_are_written(tmp_path: Path) -> None:
    folder = tmp_path / "home"

    assert within(folder, folder)
    assert within(folder / "feishu" / "skills", folder)
    assert within(folder / "x" / ".." / "feishu", folder)
    assert not within(folder / ".." / "feishu", folder)
    assert not within(tmp_path / "home2", folder)
    assert not within(tmp_path, folder)
    assert within(folder, Path(folder.anchor))


def test_a_path_in_other_cases_lies_in_a_folder_where_the_system_does_not_tell_them_apart(
    tmp_path: Path,
) -> None:
    folder = tmp_path / "home"

    shouted = Path(str(folder / "feishu").upper())

    assert within(shouted, folder) == (os.path.normcase("A") == "a")


def test_nothing_is_set_up_where_nothing_is_kept(home: Path) -> None:
    assert settings(home) is None
    assert secret(home, FEISHU) is None


@pytest.mark.parametrize("text", ["{", '{"version": 2, "domain": "feishu", "app_id": "x"}', "[]"])
def test_a_file_that_this_program_did_not_write_is_no_app(home: Path, text: str) -> None:
    home.mkdir()
    (home / SETTINGS).write_text(text, encoding="utf-8")

    assert settings(home) is None


def test_a_file_that_cannot_be_read_is_said_to_be_so(home: Path) -> None:
    (home / SETTINGS).mkdir(parents=True)

    with pytest.raises(FeishuError) as refused:
        settings(home)

    assert refused.value.code == "STORE"
    assert str(home / SETTINGS) in str(refused.value)


def test_a_file_that_is_no_text_is_said_to_be_so(home: Path) -> None:
    home.mkdir()
    (home / SETTINGS).write_bytes(b"\xff\xfe\x00")

    with pytest.raises(FeishuError, match="it is not text"):
        settings(home)


def test_a_secret_that_cannot_be_read_is_said_to_be_unusable(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)
    (home / CREDENTIALS).write_text("not what is written there", encoding="utf-8")

    with pytest.raises(FeishuError) as refused:
        secret(home, FEISHU)

    assert refused.value.code == "STORE"
    assert str(refused.value).startswith("the App Secret cannot be used: ")


def test_a_secret_that_cannot_be_one_is_not_kept(home: Path) -> None:
    with pytest.raises(FeishuError) as refused:
        keep(home, APP, "two halves", FEISHU)

    assert refused.value.code == "STORE"
    assert "halves" not in str(refused.value)
    assert not (home / SETTINGS).exists()


def unwritable(path: Path, model: object, *, private: bool) -> None:
    raise FeishuError(f"{path} cannot be written: no room", code="STORE")


def test_a_new_app_that_cannot_be_written_down_leaves_the_secret_of_the_old_one(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(home, APP, SECRET, FEISHU)
    monkeypatch.setattr(store_module, "_written", unwritable)

    with pytest.raises(FeishuError, match="no room"):
        keep(home, Settings(domain="lark", app_id="cli_other"), "other-secret-0123", LARK)

    assert settings(home) == APP
    assert secret(home, FEISHU) == SECRET


def test_a_first_app_that_cannot_be_written_down_leaves_no_secret(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(store_module, "_written", unwritable)

    with pytest.raises(FeishuError, match="no room"):
        keep(home, APP, SECRET, FEISHU)

    assert credentials.stored(home / CREDENTIALS) == {}


def test_an_app_that_cannot_be_written_down_is_said_to_be_so(home: Path) -> None:
    (home / SETTINGS / "inside").mkdir(parents=True)

    with pytest.raises(FeishuError) as refused:
        save(home, APP)

    assert refused.value.code == "STORE"
    assert str(home / SETTINGS) in str(refused.value)


def test_forgetting_deletes_the_secret_and_the_app_and_leaves_the_folder(home: Path) -> None:
    vault = Memory().install()
    keep(home, APP, SECRET, FEISHU)
    (home / FOLDER / "skills").mkdir(parents=True)

    assert forget(home) is True

    assert vault.held == {}
    assert settings(home) is None
    assert (home / FOLDER / "skills").is_dir()
    assert forget(home) is False


def test_forgetting_a_secret_alone_is_forgetting_something(home: Path) -> None:
    keep(home, APP, SECRET, FEISHU)
    (home / SETTINGS).unlink()

    assert forget(home) is True
    assert secret(home, FEISHU) is None


def test_a_secret_that_its_keyring_does_not_give_up_is_said_to_be_so(home: Path) -> None:
    vault = Memory().install()
    keep(home, APP, SECRET, FEISHU)
    vault.locked = True

    with pytest.raises(FeishuError) as refused:
        forget(home)

    assert refused.value.code == "STORE"
    assert (home / SETTINGS).exists()


def test_how_the_assistant_is_doing_is_written_down_and_read_again(home: Path) -> None:
    told = tell(home, 1234, "connected", paired=False, code="123456", now=1.5)

    assert told == Status(pid=1234, state="connected", paired=False, code="123456", at=1.5)
    assert status(home) == told


def test_a_status_says_when_it_was_written(home: Path) -> None:
    told = tell(home, 1, "connecting", paired=True)

    assert told.at > 1_700_000_000


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
