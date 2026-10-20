"""The assistant in Feishu at the command line: ``columbina feishu``."""

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
from columbina.feishu import setup as feishu_setup
from columbina.feishu import store
from columbina.feishu.app import Host
from columbina.kernel import Entry
from columbina.session.lock import acquire
from support.fake_feishu import SECRET
from support.vault import Memory

FEISHU = Host("feishu", "https://open.feishu.cn")
APP = {"domain": "feishu", "app_id": "cli_a1b2c3d4", "app_secret": SECRET}


class Input(io.TextIOWrapper):
    """A standard input that another program writes to, or a terminal."""

    def __init__(self, data: bytes, *, terminal: bool = False) -> None:
        super().__init__(io.BytesIO(data), encoding="utf-8")
        self._terminal = terminal

    def isatty(self) -> bool:
        return self._terminal


@pytest.fixture(autouse=True)
def feishu_takes_every_app(monkeypatch: pytest.MonkeyPatch) -> list[tuple[str, str]]:
    """Feishu that gives a token for every app, without the network."""
    asked: list[tuple[str, str]] = []

    async def checked(domain: str, app_id: str, secret: str, env: object, **given: object) -> None:
        asked.append((domain, app_id))

    monkeypatch.setattr(feishu_setup, "checked", checked)
    return asked


def answer_of(capsys: pytest.CaptureFixture[str]) -> object:
    return json.loads(capsys.readouterr().out)


def set_up(home: Path, **changes: object) -> None:
    settings = store.Settings(domain="feishu", app_id="cli_a1b2c3d4").model_copy(update=changes)
    store.keep(home, settings, SECRET, FEISHU)


# Keeping the app


def test_another_program_gives_the_app_on_the_standard_input(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("sys.stdin", Input(json.dumps(APP).encode()))

    assert main(["feishu", "--from-stdin"]) == 0

    assert answer_of(capsys) == {
        "stored": {"domain": "feishu", "app_id": "cli_a1b2c3d4"},
        "secret_in": "file",
    }
    assert store.secret(home, FEISHU) == SECRET


@pytest.mark.parametrize(
    ("data", "terminal", "said"),
    [
        pytest.param(b"", True, "this one is a terminal", id="a terminal"),
        pytest.param(
            b"x" * (64 * 1024 + 1), False, "more than a document of an app", id="too long"
        ),
        pytest.param(b"[1]", False, "no object of JSON", id="no object"),
        pytest.param(
            json.dumps({**APP, "app_id": "x"}).encode(), False, "begins with cli_", id="no app"
        ),
    ],
)
def test_what_another_program_gives_that_is_no_app_is_answered_as_an_error(
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    data: bytes,
    terminal: bool,
    said: str,
) -> None:
    monkeypatch.setattr("sys.stdin", Input(data, terminal=terminal))

    assert main(["feishu", "--from-stdin"]) == 2

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert said in answer["error"]
    assert answer["code"] == "INVALID"


def test_a_person_sets_the_app_up_at_a_terminal(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    answers = iter(["feishu", "cli_a1b2c3d4"])
    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)

    def asked(question: str) -> str:
        return next(answers)

    def asked_unseen(question: str) -> str:
        return SECRET

    monkeypatch.setattr("builtins.input", asked)
    monkeypatch.setattr("getpass.getpass", asked_unseen)

    assert main(["feishu", "--setup"]) == 0

    out = capsys.readouterr().out
    assert "Feishu takes the app, and it is kept." in out
    assert store.secret(home, FEISHU) == SECRET


def test_setting_up_that_is_interrupted_stores_nothing(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def interrupted(question: str) -> str:
        raise KeyboardInterrupt

    monkeypatch.setattr("sys.stdin", Input(b"", terminal=True))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True)
    monkeypatch.setattr("builtins.input", interrupted)

    assert main(["feishu", "--setup"]) == 130

    assert "nothing was stored" in capsys.readouterr().err
    assert store.settings(home) is None


def test_setting_up_asks_at_a_terminal_only(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("sys.stdin", Input(b""))

    assert main(["feishu", "--setup"]) == 2

    assert "asks its questions at a terminal" in capsys.readouterr().err


@pytest.mark.parametrize("doing", ["--setup", "--dump-config"])
def test_json_goes_with_status_forget_or_a_run(
    home: Path, capsys: pytest.CaptureFixture[str], doing: str
) -> None:
    assert main(["feishu", doing, "--json"]) == 2

    assert "--json goes with --status, --forget or a run of the assistant" in (
        capsys.readouterr().err
    )


# Saying how it is doing, and forgetting it


def test_the_status_is_answered_in_json(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["feishu", "--status", "--json"]) == 0

    answer = answer_of(capsys)
    assert isinstance(answer, dict)
    assert (answer["set_up"], answer["app_id"], answer["running"]) == (True, "cli_a1b2c3d4", False)


def test_nothing_set_up_is_said_to_a_person(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["feishu", "--status"]) == 0

    assert "No app of Feishu is set up." in capsys.readouterr().out


def test_an_assistant_that_does_not_run_is_said_to_a_person(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    store.tell(home, 7, "failed", paired=False, error="Feishu refuses it")

    assert main(["feishu", "--status"]) == 0

    out = capsys.readouterr().out
    assert "The app cli_a1b2c3d4 in feishu is set up." in out
    assert "Its assistant does not run." in out
    assert "Nobody has paired with it yet." in out
    assert "What went wrong last: Feishu refuses it" in out


def test_a_running_assistant_that_waits_for_pairing_shows_its_code(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home, owner="ou_owner")
    store.tell(home, 7, "connected", paired=False, code="123456")
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["feishu", "--status"]) == 0

    assert held is not None
    held.release()
    out = capsys.readouterr().out
    assert "Its assistant runs, and is connected." in out
    assert "Send the bot this code in Feishu to pair: 123456" in out


def test_a_running_assistant_that_said_nothing_yet_is_starting(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home, owner="ou_owner")
    (home / store.FOLDER).mkdir()
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["feishu", "--status"]) == 0

    assert held is not None
    held.release()
    assert "Its assistant runs, and is starting." in capsys.readouterr().out


def test_forgetting_deletes_the_app(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    set_up(home)

    assert main(["feishu", "--forget"]) == 0
    assert main(["feishu", "--forget", "--json"]) == 0

    out = capsys.readouterr().out.splitlines()
    assert out[0] == "The app of Feishu is deleted."
    assert json.loads(out[1]) == {"forgotten": False}


@pytest.mark.parametrize("json_lines", [True, False])
def test_an_app_that_cannot_be_forgotten_says_so(
    home: Path, capsys: pytest.CaptureFixture[str], json_lines: bool
) -> None:
    vault = Memory().install()
    set_up(home)
    vault.locked = True

    assert main(["feishu", "--forget", *(["--json"] if json_lines else [])]) == 2

    captured = capsys.readouterr()
    if json_lines:
        assert json.loads(captured.out)["code"] == "STORE"
    else:
        assert "the App Secret cannot be used" in captured.err


def test_an_app_is_not_deleted_while_its_assistant_runs(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    set_up(home)
    (home / store.FOLDER).mkdir()
    held = acquire(home / store.FOLDER / store.LOCK)

    assert main(["feishu", "--forget", "--json"]) == 2

    assert held is not None
    held.release()
    assert json.loads(capsys.readouterr().out)["code"] == "RUNNING"
    assert store.settings(home) is not None


# Running it


def test_the_profile_of_the_assistant_is_printed(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["feishu", "--dump-config"]) == 0

    out = capsys.readouterr().out
    assert "plugin: columbina.feishu.bot" in out
    assert "preset: read-only" in out


def test_an_assistant_without_an_app_does_not_run(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["feishu"]) == 2

    assert "no app of Feishu is set up" in capsys.readouterr().err


@pytest.mark.parametrize(("given", "json_lines"), [(["--json"], True), ([], None)])
def test_an_assistant_that_another_program_runs_says_how_it_is_doing_in_lines_of_json(
    home: Path, monkeypatch: pytest.MonkeyPatch, given: list[str], json_lines: bool | None
) -> None:
    configs: list[dict[str, object]] = []

    async def run(entries: Sequence[Entry], env: Mapping[str, str]) -> int:
        configs.extend(
            cast("dict[str, object]", entry.config) for entry in entries if entry.id == "frontend"
        )
        return 0

    monkeypatch.setattr(launcher, "_run", run)

    assert main(["feishu", *given]) == 0

    (config,) = configs
    assert config.get("json_lines") == json_lines


def free_port() -> int:
    """Return a port of this machine that nothing listens on."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def test_a_program_runs_the_assistant_and_stops_it_by_closing_the_input(home: Path) -> None:
    # Feishu where nothing answers: the assistant keeps trying to connect.
    nowhere = f"http://127.0.0.1:{free_port()}"
    store.keep(
        home,
        store.Settings(domain="feishu", app_id="cli_a1b2c3d4"),
        SECRET,
        Host("feishu", nowhere),
    )
    env = {
        name: value
        for name, value in os.environ.items()
        if name.lower() not in {"http_proxy", "https_proxy", "all_proxy"}
    }
    env["COLUMBINA_FEISHU_API"] = nowhere
    child = subprocess.Popen(
        [sys.executable, "-m", "columbina", "feishu", "--json"],
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
