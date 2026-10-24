"""`columbina reasoning-language`: the language that the model reasons in, from the next turn."""

import json
from pathlib import Path

import pytest

from columbina.apps.launcher import main


def test_a_language_is_chosen_and_said(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["reasoning-language"]) == 0
    assert capsys.readouterr().out == "none\n"

    assert main(["reasoning-language", "zh"]) == 0
    assert (home / "reasoning-language").read_text(encoding="utf-8") == "zh\n"
    assert capsys.readouterr().out == ""

    assert main(["reasoning-language"]) == 0
    assert capsys.readouterr().out == "zh\n"


def test_none_asks_for_no_language(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["reasoning-language", "en"]) == 0

    assert main(["reasoning-language", "none"]) == 0

    assert not (home / "reasoning-language").exists()
    assert main(["reasoning-language"]) == 0
    assert capsys.readouterr().out == "none\n"


def test_a_language_that_cannot_be_chosen_is_refused(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["reasoning-language", "fr"])

    assert stopped.value.code == 2
    said = capsys.readouterr().err
    # How the choices are quoted depends on the version of Python.
    assert "argument language: invalid choice: 'fr'" in said
    assert "[{zh,en,ja,none}]" in said
    assert not (home / "reasoning-language").exists()


def test_another_program_is_answered_in_json(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["reasoning-language", "--json"]) == 0
    assert capsys.readouterr().out == '{"language": null}\n'

    assert main(["reasoning-language", "ja", "--json"]) == 0
    assert capsys.readouterr().out == '{"language": "ja"}\n'

    assert main(["reasoning-language", "none", "--json"]) == 0
    assert capsys.readouterr().out == '{"language": null}\n'

    (home / "reasoning-language").mkdir()
    assert main(["reasoning-language", "zh", "--json"]) == 2
    said = json.loads(capsys.readouterr().out)
    assert said["error"].startswith("the language of reasoning cannot be kept in ")


def test_a_choice_that_cannot_be_kept_is_said(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # What stands where the file would be is a directory, which cannot be written as one.
    (home / "reasoning-language").mkdir(parents=True)

    assert main(["reasoning-language", "ja"]) == 2

    said = capsys.readouterr().err
    assert said.startswith("columbina: the language of reasoning cannot be kept in ")
    assert str(home / "reasoning-language") in said
