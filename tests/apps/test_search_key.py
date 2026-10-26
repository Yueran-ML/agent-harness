"""``columbina search-key``: the key of a service that searches the web."""

import io
import json
from pathlib import Path

import pytest

from columbina.apps.launcher import main
from columbina.web import keys


class Input(io.StringIO):
    def __init__(self, text: str, *, terminal: bool = False) -> None:
        super().__init__(text)
        self._terminal = terminal

    def isatty(self) -> bool:
        return self._terminal


def given(monkeypatch: pytest.MonkeyPatch, text: str, *, terminal: bool = False) -> None:
    monkeypatch.setattr("sys.stdin", Input(text, terminal=terminal))


def test_a_key_from_another_program_is_kept(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    given(monkeypatch, "tvly-123\n")

    assert main(["search-key", "tavily", "--json"]) == 0

    assert json.loads(capsys.readouterr().out) == {"kept": "tavily", "keyring": None}
    assert keys.key_of(home, {}, "tavily") == "tvly-123"


def test_a_key_typed_at_a_terminal_is_kept_unseen(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    given(monkeypatch, "", terminal=True)
    asked: list[str] = []

    def typed(question: str) -> str:
        asked.append(question)
        return "b-key"

    monkeypatch.setattr("getpass.getpass", typed)

    assert main(["search-key", "brave"]) == 0

    assert asked == ["The key of brave: "]
    assert capsys.readouterr().out == "The key of brave is kept in the file of keys.\n"
    assert keys.key_of(home, {}, "brave") == "b-key"


def test_what_is_kept_is_listed(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    keys.keep(home, "tavily", "t")
    monkeypatch.setenv("BRAVE_API_KEY", "b")

    assert main(["search-key"]) == 0
    assert capsys.readouterr().out == "tavily: kept\nbrave: in BRAVE_API_KEY\n"
    assert main(["search-key", "--json"]) == 0
    assert json.loads(capsys.readouterr().out)["services"][0] == {
        "name": "tavily",
        "key": "kept",
        "variable": "TAVILY_API_KEY",
    }
    monkeypatch.delenv("BRAVE_API_KEY")
    assert main(["search-key"]) == 0
    assert capsys.readouterr().out == "tavily: kept\nbrave: none\n"


def test_a_key_is_forgotten(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    keys.keep(home, "tavily", "t")

    assert main(["search-key", "tavily", "--forget"]) == 0
    assert capsys.readouterr().out == "The key of tavily was deleted.\n"
    assert main(["search-key", "tavily", "--forget"]) == 0
    assert capsys.readouterr().out == "No key of tavily was kept.\n"
    assert main(["search-key", "tavily", "--forget", "--json"]) == 0
    assert json.loads(capsys.readouterr().out) == {"forgot": False}


def test_forgetting_names_a_service(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["search-key", "--forget"]) == 2

    assert "--forget names the service whose key is deleted" in capsys.readouterr().err


def test_what_cannot_be_kept_is_said_without_the_key(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    given(monkeypatch, "two words")

    assert main(["search-key", "brave"]) == 2
    err = capsys.readouterr().err
    assert "this cannot be a key" in err
    assert "two words" not in err
    given(monkeypatch, "")
    assert main(["search-key", "brave", "--json"]) == 2
    assert "there is no key to keep" in json.loads(capsys.readouterr().out)["error"]
