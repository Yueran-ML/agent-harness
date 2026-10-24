"""The command ``columbina wecom``: setting the assistant in WeCom up, and running it."""

import io
import json
import os
import socket
import subprocess
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import cast

import pytest

from columbina.apps import launcher
from columbina.apps.launcher import main
from columbina.kernel import Entry
from columbina.session.lock import acquire
from columbina.wecom import setup as wecom_setup
from columbina.wecom import store
from support.fake_wecom import BOT_ID, PLACE, SECRET
from support.vault import Memory

BOT = {"bot_id": BOT_ID, "secret": SECRET}


class Input(io.TextIOWrapper):
    """A standard input that another program writes to, or a terminal."""

    def __init__(self, data: bytes, *, terminal: bool = False) -> None:
        super().__init__(io.BytesIO(data), encoding="utf-8")
        self._terminal = terminal

    def isatty(self) -> bool:
        return self._terminal


@pytest.fixture(autouse=True)
def wecom_takes_every_bot(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """WeCom that takes every bot, without the network."""
    asked: list[str] = []

    async def checked(bot_id: str, secret: str, env: object, **given: object) -> None:
        asked.append(bot_id)

    monkeypatch.setattr(wecom_setup, "checked", checked)
    return asked


def answer_of(capsys: pytest.CaptureFixture[str]) -> object:
    return json.loads(capsys.readouterr().out)


def set_up(home: Path, **changes: object) -> None:
    store.keep(home, store.Settings(bot_id=BOT_ID).model_copy(update=changes), SECRET, PLACE)


# Keeping the bot


def test_another_program_gives_the_bot_on_the_standard_input(
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    wecom_takes_every_bot: list[str],
) -> None:
    monkeypatch.setattr("sys.stdin", Input(json.dumps(BOT).encode()))

    assert main(["wecom", "--from-stdin"]) == 0

    assert answer_of(capsys) == {"stored": {"bot_id": BOT_ID}, "secret_in": "file"}
    assert store.secret(home, PLACE) == SECRET
    assert wecom_takes_every_bot == [BOT_ID]


@pytest.mark.parametrize(
    ("data", "terminal", "said"),
    [
        pytest.param(b"", True, "this one is a terminal", id="a terminal"),
        pytest.param(b"x" * (64 * 1024 + 1), False, "more than a document of a bot", id="too long"),
        pytest.param(b"[1]", False, "no object of JSON", id="no object"),
        pytest.param(b'{"secret": "x"}', False, "the Bot ID is the one", id="no bot"),
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

    assert main(["wecom", "--from-stdin"]) == 2

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert said in answer["error"]
    assert answer["code"] == "INVALID"


def test_a_person_sets_the_bot_up_at_a_terminal(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def asked(question: str) -> str:
        return BOT_ID

    def asked_unseen(question: str) -> str:
        return SECRET

    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)
    monkeypatch.setattr("builtins.input", asked)
    monkeypatch.setattr("getpass.getpass", asked_unseen)

    assert main(["wecom", "--setup"]) == 0

    assert "WeCom takes the bot, and it is kept." in capsys.readouterr().out
    assert store.secret(home, PLACE) == SECRET


def test_setting_up_that_is_interrupted_stores_nothing(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def interrupted(question: str) -> str:
        raise KeyboardInterrupt

    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)
    monkeypatch.setattr("builtins.input", interrupted)

    assert main(["wecom", "--setup"]) == 130

    assert "nothing was stored" in capsys.readouterr().err
    assert store.settings(home) is None


def test_setting_up_asks_at_a_terminal_only(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("sys.stdin", Input(b""))

    assert main(["wecom", "--setup"]) == 2

    assert "asks its questions at a terminal" in capsys.readouterr().err


@pytest.mark.parametrize("doing", ["--setup", "--dump-config"])
def test_json_goes_with_status_forget_or_a_run(
    home: Path, capsys: pytest.CaptureFixture[str], doing: str
) -> None:
    assert main(["wecom", doing, "--json"]) == 2

    assert "--json goes with --status, --forget or a run of the assistant" in (
        capsys.readouterr().err
    )


# Saying how it is doing, and forgetting it


def test_the_status_is_answered_in_json(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["wecom", "--status", "--json"]) == 0

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert (answer["set_up"], answer["bot_id"], answer["running"]) == (True, BOT_ID, False)
    assert answer["folder"] == str(home.parent / "Columbina" / "wecom")


def test_nothing_set_up_is_said_to_a_person(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["wecom", "--status"]) == 0

    assert "No bot of WeCom is set up." in capsys.readouterr().out


def test_an_assistant_that_does_not_run_is_said_to_a_person(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    store.tell(home, 7, "failed", paired=False, error="WeCom refuses it")

    assert main(["wecom", "--status"]) == 0

    out = capsys.readouterr().out
    assert f"The bot {BOT_ID} is set up." in out
    where = home.parent / "Columbina" / "wecom"
    assert f"Its assistant works in {where}, and changes no file there." in out
    assert "Its assistant does not run. 'columbina wecom' runs it." in out
    assert "Nobody has paired with it yet." in out
    assert "What went wrong last: WeCom refuses it" in out


def test_a_running_assistant_that_waits_for_pairing_shows_its_code(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home, owner="zhangsan", writable=True)
    store.tell(home, 7, "connected", paired=False, code="123456")
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["wecom", "--status"]) == 0

    assert held is not None
    held.release()
    out = capsys.readouterr().out
    assert "and may change the files there." in out
    assert "Its assistant runs, and is connected." in out
    assert "Send the bot this code in WeCom to pair: 123456" in out


def test_a_running_assistant_that_said_nothing_yet_is_starting(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home, owner="zhangsan")
    (home / store.FOLDER).mkdir()
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["wecom", "--status"]) == 0

    assert held is not None
    held.release()
    out = capsys.readouterr().out
    assert "Its assistant runs, and is starting." in out
    assert "Nobody has paired" not in out


def test_forgetting_deletes_the_bot(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["wecom", "--forget"]) == 0
    assert main(["wecom", "--forget"]) == 0
    assert main(["wecom", "--forget", "--json"]) == 0

    out = capsys.readouterr().out.splitlines()
    assert out[:2] == ["The bot of WeCom is deleted.", "No bot was set up."]
    assert json.loads(out[2]) == {"forgotten": False}


@pytest.mark.parametrize("json_lines", [True, False])
def test_a_bot_that_cannot_be_forgotten_says_so(
    home: Path, capsys: pytest.CaptureFixture[str], json_lines: bool
) -> None:
    vault = Memory().install()
    set_up(home)
    vault.locked = True

    assert main(["wecom", "--forget", *(["--json"] if json_lines else [])]) == 2

    captured = capsys.readouterr()
    if json_lines:
        assert json.loads(captured.out)["code"] == "STORE"
    else:
        assert "the Secret of the bot cannot be used" in captured.err


# Running it


def test_the_profile_of_the_assistant_is_printed(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["wecom", "--dump-config"]) == 0

    out = capsys.readouterr().out
    assert "plugin: columbina.wecom.bot" in out
    assert "preset: read-only" in out


def started(monkeypatch: pytest.MonkeyPatch) -> list[tuple[dict[str, Entry], Mapping[str, str]]]:
    # The assistant runs no further than the entries and the environment that it would run with.
    runs: list[tuple[dict[str, Entry], Mapping[str, str]]] = []

    async def run(entries: Sequence[Entry], env: Mapping[str, str]) -> int:
        runs.append(({entry.id: entry for entry in entries}, env))
        return 0

    monkeypatch.setattr(launcher, "_run", run)
    return runs


def test_the_assistant_works_beside_the_home_of_the_program_or_where_it_is_told(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home)

    assert main(["wecom"]) == 0
    monkeypatch.setenv(store.ASSISTANT_HOME, str(home / "mine"))
    assert main(["wecom"]) == 0

    (entries, beside), (_, told) = runs
    assert beside[store.ASSISTANT_HOME] == str(home.parent / "Columbina" / "wecom")
    assert told[store.ASSISTANT_HOME] == str(home / "mine")
    frontend = cast("dict[str, object]", entries["frontend"].config)
    skills = cast("dict[str, object]", entries["skills"].config)
    assert (frontend["cwd"], skills["home"]) == (
        "${COLUMBINA_WECOM_HOME}",
        "${COLUMBINA_WECOM_HOME}/skills",
    )
    # And every command is told where it is, as it is told where that of Feishu is.
    assert "COLUMBINA_FEISHU_HOME" in beside


def test_an_assistant_whose_agent_may_change_files_has_the_tools_for_it(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home)
    assert main(["wecom"]) == 0
    wecom_setup.made_writable(home, True, {})
    assert main(["wecom"]) == 0

    (reads, _), (writes, _) = runs
    assert (reads["tool-write"].disabled, reads["tool-edit"].disabled) == (True, True)
    assert cast("dict[str, object]", reads["permissions"].config)["preset"] == "read-only"
    assert (writes["tool-write"].disabled, writes["tool-edit"].disabled) == (False, False)
    assert cast("dict[str, object]", writes["permissions"].config)["preset"] == "workspace-write"
    assert reads["system-prompt-conduct"].config == {"leave_out": ["changes"]}
    assert writes["system-prompt-conduct"].config is None
    assert writes["tool-shell"].disabled is True
    # The assistant is told, and runs only where its owner lets it.
    assert "writable" not in cast("dict[str, object]", reads["frontend"].config)
    assert cast("dict[str, object]", writes["frontend"].config)["writable"] is True
    # The rules of the user hold for it, as for every other agent.
    for loaded in (reads, writes):
        permissions = cast("dict[str, object]", loaded["permissions"].config)
        assert permissions["user_file"] == "${COLUMBINA_HOME}/permissions.yml"


def test_a_profile_that_is_named_is_the_one_loaded(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home, writable=True)

    assert main(["wecom", "--profile", "cli"]) == 0

    ((entries, _),) = runs
    assert entries["frontend"].plugin != "columbina.wecom.bot"


def test_a_bot_that_cannot_be_read_lets_the_agent_change_nothing(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    (home / store.SETTINGS).mkdir(parents=True)

    assert main(["wecom"]) == 0

    ((entries, _),) = runs
    assert entries["tool-write"].disabled is True


def test_another_program_says_whether_the_agent_may_change_files(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    monkeypatch.setattr("sys.stdin", Input(b'{"writable": true}'))

    assert main(["wecom", "--from-stdin"]) == 0

    assert answer_of(capsys) == {"writable": True}
    settings = store.settings(home)
    assert settings is not None
    assert settings.writable is True


def test_an_assistant_without_a_bot_does_not_run(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["wecom"]) == 2

    assert "no bot of WeCom is set up" in capsys.readouterr().err


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

    assert main(["wecom", *given]) == 0

    ((entries, _),) = runs
    config = cast("dict[str, object]", entries["frontend"].config)
    assert (config.get("language"), config.get("json_lines")) == (language, json_lines)


def free_port() -> int:
    """Return a port of this machine that nothing listens on."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def test_a_program_runs_the_assistant_and_stops_it_by_closing_the_input(home: Path) -> None:
    # WeCom where nothing answers: the assistant keeps trying to connect.
    nowhere = f"ws://127.0.0.1:{free_port()}"
    store.keep(home, store.Settings(bot_id=BOT_ID), SECRET, nowhere)
    env = {
        name: value
        for name, value in os.environ.items()
        if name.lower() not in {"http_proxy", "https_proxy", "all_proxy"}
    }
    env["COLUMBINA_WECOM_WS"] = nowhere
    child = subprocess.Popen(
        [sys.executable, "-m", "columbina", "wecom", "--json"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    assert child.stdout is not None
    try:
        # The code of pairing is said at once, while the assistant runs.
        first = child.stdout.readline()
        # This closes the standard input first, as the program that stops the assistant does.
        out, err = child.communicate(timeout=60)
    except BaseException:
        child.kill()
        child.communicate()
        raise

    said = err.decode(errors="replace")
    assert first, said
    lines = [json.loads(first), *(json.loads(line) for line in out.decode().splitlines())]
    assert lines[0]["state"] == "connecting"
    assert str(lines[0]["code"]).isdigit()
    assert lines[-1]["state"] == "stopped"
    assert child.returncode == 0, said
    assert SECRET not in said
    told = store.status(home)
    assert told is not None
    assert told.state == "stopped"


def test_the_model_and_the_level_kept_for_the_assistant_hold_and_the_command_line_counts_more(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = started(monkeypatch)
    set_up(home, model="deepseek-v4-pro", effort="high")

    assert main(["wecom"]) == 0
    assert main(["wecom", "--effort", "off"]) == 0

    (kept, _), (named, _) = runs
    first = cast("dict[str, object]", cast("dict[str, object]", kept["frontend"].config)["model"])
    second = cast("dict[str, object]", cast("dict[str, object]", named["frontend"].config)["model"])
    assert (first["model"], first["reasoning"]) == ("deepseek-v4-pro", "high")
    assert (second["model"], second["reasoning"]) == ("deepseek-v4-pro", "off")
