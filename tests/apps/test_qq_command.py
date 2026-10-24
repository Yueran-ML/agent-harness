"""The command ``columbina qq``: setting the assistant in QQ up, and running it."""

import io
import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import cast

import pytest

from columbina.apps import launcher
from columbina.apps import setup as chosen_service
from columbina.apps.launcher import main
from columbina.kernel import Entry
from columbina.qq import setup as qq_setup
from columbina.qq import store
from columbina.qq.app import TOKEN_URL
from columbina.session.lock import acquire
from support.fake_qq import APP_ID, SECRET
from support.vault import Memory

BOT = {"app_id": APP_ID, "secret": SECRET}


class Input(io.TextIOWrapper):
    """A standard input that another program writes to, or a terminal."""

    def __init__(self, data: bytes, *, terminal: bool = False) -> None:
        super().__init__(io.BytesIO(data), encoding="utf-8")
        self._terminal = terminal

    def isatty(self) -> bool:
        return self._terminal


@pytest.fixture(autouse=True)
def qq_takes_every_bot(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """QQ that takes every bot, without the network."""
    asked: list[str] = []

    async def checked(app_id: str, secret: str, env: object, **given: object) -> None:
        asked.append(app_id)

    monkeypatch.setattr(qq_setup, "checked", checked)
    return asked


def answer_of(capsys: pytest.CaptureFixture[str]) -> object:
    return json.loads(capsys.readouterr().out)


def set_up(home: Path, **changes: object) -> None:
    store.keep(home, store.Settings(app_id=APP_ID).model_copy(update=changes), SECRET, TOKEN_URL)


# Keeping the bot


def test_another_program_gives_the_bot_on_the_standard_input(
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    qq_takes_every_bot: list[str],
) -> None:
    monkeypatch.setattr("sys.stdin", Input(json.dumps(BOT).encode()))

    assert main(["qq", "--from-stdin"]) == 0

    assert answer_of(capsys) == {"stored": {"app_id": APP_ID}, "secret_in": "file"}
    assert store.secret(home, TOKEN_URL) == SECRET
    assert qq_takes_every_bot == [APP_ID]


@pytest.mark.parametrize(
    ("data", "terminal", "said"),
    [
        pytest.param(b"", True, "this one is a terminal", id="a terminal"),
        pytest.param(b"x" * (64 * 1024 + 1), False, "more than a document of a bot", id="too long"),
        pytest.param(b"[1]", False, "no object of JSON", id="no object"),
        pytest.param(b'{"secret": "x"}', False, "the AppID is the one", id="no bot"),
    ],
)
def test_what_another_program_gives_that_is_no_bot_is_answered_as_an_error(
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    data: bytes,
    terminal: bool,
    said: str,
) -> None:
    monkeypatch.setattr("sys.stdin", Input(data, terminal=terminal))

    assert main(["qq", "--from-stdin"]) == 2

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert said in answer["error"]
    assert answer["code"] == "INVALID"


def test_a_person_sets_the_bot_up_at_a_terminal(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)

    def asked(question: str) -> str:
        return APP_ID

    def asked_unseen(question: str) -> str:
        return SECRET

    monkeypatch.setattr("builtins.input", asked)
    monkeypatch.setattr("getpass.getpass", asked_unseen)

    assert main(["qq", "--setup"]) == 0

    assert "QQ takes the bot, and it is kept." in capsys.readouterr().out
    assert store.secret(home, TOKEN_URL) == SECRET


def test_setting_up_that_is_interrupted_stores_nothing(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def interrupted(question: str) -> str:
        raise KeyboardInterrupt

    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)
    monkeypatch.setattr("builtins.input", interrupted)

    assert main(["qq", "--setup"]) == 130

    assert "nothing was stored" in capsys.readouterr().err
    assert store.settings(home) is None


def test_setting_up_asks_at_a_terminal_only(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("sys.stdin", Input(b""))

    assert main(["qq", "--setup"]) == 2

    assert "asks its questions at a terminal" in capsys.readouterr().err


@pytest.mark.parametrize("doing", ["--setup", "--dump-config"])
def test_json_goes_with_status_forget_or_a_run(
    home: Path, capsys: pytest.CaptureFixture[str], doing: str
) -> None:
    assert main(["qq", doing, "--json"]) == 2

    assert "--json goes with --status, --forget or a run of the assistant" in (
        capsys.readouterr().err
    )


# Saying how it is doing, and forgetting it


def test_the_status_is_answered_in_json(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["qq", "--status", "--json"]) == 0

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert (answer["set_up"], answer["app_id"], answer["running"]) == (True, APP_ID, False)
    assert answer["folder"] == str(home.parent / "Columbina" / "qq")


def test_nothing_set_up_is_said_to_a_person(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["qq", "--status"]) == 0

    assert "No bot of QQ is set up." in capsys.readouterr().out


def test_an_assistant_that_does_not_run_is_said_to_a_person(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    store.tell(home, 7, "failed", paired=False, error="QQ refuses it")

    assert main(["qq", "--status"]) == 0

    out = capsys.readouterr().out
    where = home.parent / "Columbina" / "qq"
    assert f"The bot {APP_ID} is set up, and its assistant works in {where}." in out
    assert "Its assistant does not run. 'columbina qq' runs it." in out
    assert "Nobody has paired with it yet." in out
    assert "What went wrong last: QQ refuses it" in out


def test_a_running_assistant_that_waits_for_pairing_shows_its_code(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    store.tell(home, 7, "connected", paired=False, code="123456")
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["qq", "--status"]) == 0

    assert held is not None
    held.release()
    out = capsys.readouterr().out
    assert "Its assistant runs, and is connected." in out
    assert "Send the bot this code in QQ to pair: 123456" in out


def test_a_running_assistant_that_said_nothing_yet_is_starting(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home, owner="OWNER01")
    (home / store.FOLDER).mkdir()
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["qq", "--status"]) == 0

    assert held is not None
    held.release()
    out = capsys.readouterr().out
    assert "Its assistant runs, and is starting." in out
    assert "Nobody has paired" not in out


def test_forgetting_deletes_the_bot(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["qq", "--forget"]) == 0
    assert main(["qq", "--forget"]) == 0
    assert main(["qq", "--forget", "--json"]) == 0

    out = capsys.readouterr().out.splitlines()
    assert out[:2] == ["The bot of QQ is deleted.", "No bot was set up."]
    assert json.loads(out[2]) == {"forgotten": False}


@pytest.mark.parametrize("json_lines", [True, False])
def test_a_bot_that_cannot_be_forgotten_says_so(
    home: Path, capsys: pytest.CaptureFixture[str], json_lines: bool
) -> None:
    vault = Memory().install()
    set_up(home)
    vault.locked = True

    assert main(["qq", "--forget", *(["--json"] if json_lines else [])]) == 2

    captured = capsys.readouterr()
    if json_lines:
        assert json.loads(captured.out)["code"] == "STORE"
    else:
        assert "the AppSecret of the bot cannot be used" in captured.err


# Running it


def test_the_profile_of_the_assistant_is_printed(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["qq", "--dump-config"]) == 0

    out = capsys.readouterr().out
    assert "plugin: columbina.qq.bot" in out
    assert "preset: read-only" in out
    assert "reasoning: 'off'" in out


def started(monkeypatch: pytest.MonkeyPatch) -> list[tuple[dict[str, Entry], Mapping[str, str]]]:
    runs: list[tuple[dict[str, Entry], Mapping[str, str]]] = []

    async def run(entries: Sequence[Entry], env: Mapping[str, str]) -> int:
        runs.append(({entry.id: entry for entry in entries}, env))
        return 0

    monkeypatch.setattr(launcher, "_run", run)
    return runs


def chosen(home: Path, text: str) -> None:
    """What 'columbina setup' would have written: the service in use, and how hard it thinks."""
    home.mkdir(parents=True, exist_ok=True)
    (home / chosen_service.CHOICE).write_text(text, encoding="utf-8")


def model_of(entries: dict[str, Entry]) -> dict[str, object]:
    config = cast("dict[str, object]", entries["frontend"].config)
    return cast("dict[str, object]", config["model"])


def test_the_assistant_works_beside_the_home_of_the_program_or_where_it_is_told(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home)

    assert main(["qq"]) == 0
    monkeypatch.setenv(store.ASSISTANT_HOME, str(home / "mine"))
    assert main(["qq"]) == 0

    (entries, beside), (_, told) = runs
    assert beside[store.ASSISTANT_HOME] == str(home.parent / "Columbina" / "qq")
    assert told[store.ASSISTANT_HOME] == str(home / "mine")
    assert cast("dict[str, object]", entries["frontend"].config)["cwd"] == "${COLUMBINA_QQ_HOME}"
    # Nothing that changes files, runs commands or loads skills is loaded.
    for left_out in ("tool-write", "tool-edit", "tool-shell", "skills", "subagents"):
        assert entries[left_out].disabled


def test_the_assistant_thinks_not_at_all_whatever_was_chosen_for_the_service(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    chosen(home, "use: deepseek\nservices:\n  deepseek:\n    effort: high\n")

    assert main(["qq"]) == 0
    assert main(["qq", "--provider", "deepseek"]) == 0

    for entries, _ in runs:
        assert model_of(entries)["reasoning"] == "off"


def test_with_no_service_chosen_the_assistant_thinks_not_at_all_either(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    chosen(home, "services:\n  deepseek:\n    effort: max\n")

    assert main(["qq"]) == 0

    ((entries, _),) = runs
    assert model_of(entries)["reasoning"] == "off"


def test_an_assistant_without_a_bot_does_not_run(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["qq"]) == 2

    assert "no bot of QQ is set up" in capsys.readouterr().err


@pytest.mark.parametrize(
    ("given", "language", "json_lines"),
    [
        (["--json", "--language", "en"], "en", True),
        (["--language", "zh"], "zh", None),
        ([], None, None),
    ],
)
def test_the_language_and_the_lines_of_json_are_given_to_the_assistant(
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    given: list[str],
    language: str | None,
    json_lines: bool | None,
) -> None:
    runs = started(monkeypatch)

    assert main(["qq", *given]) == 0

    ((entries, _),) = runs
    config = cast("dict[str, object]", entries["frontend"].config)
    assert (config.get("language"), config.get("json_lines")) == (language, json_lines)
    assert "writable" not in config


def test_the_model_and_the_level_kept_for_the_assistant_hold_and_the_command_line_counts_more(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home, model="deepseek-v4-pro", effort="low")

    assert main(["qq"]) == 0
    assert main(["qq", "--effort", "max", "--model", "deepseek-flash"]) == 0

    (kept, _), (named, _) = runs
    assert (model_of(kept)["model"], model_of(kept)["reasoning"]) == ("deepseek-v4-pro", "low")
    assert (model_of(named)["model"], model_of(named)["reasoning"]) == ("deepseek-flash", "max")


def test_a_bot_that_cannot_be_read_keeps_no_model_of_its_own(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    (home / store.SETTINGS).mkdir(parents=True)

    assert main(["qq"]) == 0

    ((entries, _),) = runs
    assert model_of(entries)["reasoning"] == "off"
