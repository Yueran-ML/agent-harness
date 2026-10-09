"""What the tests of the command line share: a home for the program and a working directory."""

from pathlib import Path

import pytest

from columbina.apps.cli import prompt


@pytest.fixture(autouse=True)
def questions_without_the_wait(monkeypatch: pytest.MonkeyPatch) -> None:
    """Have the keys of a question count as good as at once.

    The wait is there for a person who is typing when a question appears. A
    test presses a key when it sees the keys offered, and that is where the
    wait is over, however long it was. The tests of the wait set it themselves.
    """
    monkeypatch.setattr(prompt, "_SETTLE_SECONDS", 0.05)


@pytest.fixture
def home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A data directory for Columbina that is thrown away after the test, and no API key."""
    directory = tmp_path / "home"
    monkeypatch.setenv("COLUMBINA_HOME", str(directory))
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.delenv("COLUMBINA_MODEL", raising=False)
    return directory


@pytest.fixture
def work(tmp_path: Path) -> Path:
    """A working directory with one file in it."""
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "notes.txt").write_text("The answer is 42.\n", encoding="utf-8")
    return directory
