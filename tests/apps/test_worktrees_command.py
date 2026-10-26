"""``columbina worktrees``: the worktrees of the tasks of a project, from the command line."""

import json
from pathlib import Path

import pytest

from columbina import programs
from columbina.apps.launcher import main
from support.git_repos import apart, git, needs_git

pytestmark = needs_git


@pytest.fixture
def project(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    apart(monkeypatch, tmp_path)
    top = tmp_path / "project"
    top.mkdir()
    git(top, "init", "--quiet")
    (top / "README.md").write_text("Hello.\n", encoding="utf-8")
    git(top, "add", "--all")
    git(top, "commit", "--quiet", "-m", "first")
    return top


def test_a_task_is_given_a_worktree_and_ended_from_the_command_line(
    home: Path, project: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    del home
    where = ["--cwd", str(project)]
    assert main(["worktrees", "list", *where]) == 0
    assert "There are no worktrees." in capsys.readouterr().out

    assert main(["worktrees", "add", "Write notes", *where, "--json"]) == 0
    made = json.loads(capsys.readouterr().out)
    assert (made["name"], made["branch"], made["done"]) == (
        "write-notes",
        "columbina/write-notes",
        "add",
    )
    (Path(made["path"]) / "notes.txt").write_text("notes\n", encoding="utf-8")
    assert main(["worktrees", "list", *where]) == 0
    assert capsys.readouterr().out.startswith("write-notes  columbina/write-notes  ")
    assert main(["worktrees", "list", *where, "--json"]) == 0
    assert [one["name"] for one in json.loads(capsys.readouterr().out)["worktrees"]] == [
        "write-notes"
    ]

    assert main(["worktrees", "merge", "write-notes", *where]) == 0
    assert capsys.readouterr().out == (
        "Merged columbina/write-notes into the project, and took write-notes away.\n"
    )
    assert (project / "notes.txt").is_file()


def test_a_task_is_dropped_from_the_command_line(
    home: Path, project: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    del home
    (project / ".gitignore").write_text(".env\n", encoding="utf-8")
    (project / ".env").write_text("KEY=1\n", encoding="utf-8")
    (project / ".worktreeinclude").write_text(".env\n", encoding="utf-8")
    monkeypatch.chdir(project)

    assert main(["worktrees", "add", "try"]) == 0
    said = capsys.readouterr().out
    assert said.startswith("Made try at ")
    assert said.endswith(", on the branch columbina/try. Copied into it: .env.\n")
    assert main(["worktrees", "remove", "try", "--force"]) == 0
    assert capsys.readouterr().out == "Took try away, with its branch columbina/try.\n"


def test_what_the_command_line_cannot_do_is_said(
    home: Path, project: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    del home
    where = ["--cwd", str(project)]
    assert main(["worktrees", "remove", "none", *where]) == 2
    assert capsys.readouterr().out == "columbina: there is no worktree none\n"
    assert main(["worktrees", "merge", "none", *where, "--json"]) == 2
    assert json.loads(capsys.readouterr().out) == {"error": "there is no worktree none"}

    def nowhere(*args: object, **more: object) -> None:
        return None

    monkeypatch.setattr(programs, "found", nowhere)
    assert main(["worktrees", "list", *where]) == 2
    assert capsys.readouterr().out == "columbina: git is not on the search path\n"
