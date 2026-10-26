"""A working directory of its own for a task: a worktree of git, beside the project."""

import shutil
from pathlib import Path

import pytest

from columbina import worktrees
from columbina.worktrees import PLACE, WorktreeError, listed, made, merged, named, removed
from support.git_repos import GIT, apart, git, needs_git

pytestmark = needs_git


@pytest.fixture
def project(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """A project with one commit, a file of secrets and installed packages that git passes over."""
    apart(monkeypatch, tmp_path)
    top = tmp_path / "project"
    top.mkdir()
    git(top, "init", "--quiet")
    (top / ".gitignore").write_text(".env\nnode_modules/\n", encoding="utf-8")
    (top / "README.md").write_text("Hello.\n", encoding="utf-8")
    git(top, "add", "--all")
    git(top, "commit", "--quiet", "-m", "first")
    (top / ".env").write_text("KEY=1\n", encoding="utf-8")
    (top / "node_modules" / "left").mkdir(parents=True)
    (top / "node_modules" / "left" / "pad.js").write_text("pad\n", encoding="utf-8")
    return top


def git_of() -> str:
    assert GIT is not None
    return GIT


# Making one


def test_a_worktree_is_made_for_a_task_with_what_the_project_lists(project: Path) -> None:
    (project / ".worktreeinclude").write_text(
        "# what the tasks need\n\n.env\nnode_modules\nREADME.md\nmissing*\n.git\n.e*\n",
        encoding="utf-8",
    )

    worktree = made(git_of(), project / "node_modules", "Fix the bug!")

    assert (worktree.name, worktree.branch) == ("fix-the-bug", "columbina/fix-the-bug")
    assert worktree.path == project / PLACE / "fix-the-bug"
    # What git keeps is there anyway; of the rest, what the project lists is copied.
    assert worktree.copied == (".env", "node_modules")
    assert (worktree.path / ".env").read_text(encoding="utf-8") == "KEY=1\n"
    assert (worktree.path / "node_modules" / "left" / "pad.js").is_file()
    assert git(worktree.path, "branch", "--show-current") == "columbina/fix-the-bug"
    # The project shows nothing new.
    assert git(project, "status", "--porcelain") == "?? .worktreeinclude"


def test_the_worktrees_are_not_copied_into_a_worktree(project: Path) -> None:
    made(git_of(), project, "one")
    (project / ".worktreeinclude").write_text(".columbina/*\n.env\n", encoding="utf-8")

    two = made(git_of(), project, "two")

    assert two.copied == (".env",)
    assert not (two.path / ".columbina").exists()


def test_git_is_told_once_to_pass_the_worktrees_over(project: Path) -> None:
    exclude = project / ".git" / "info" / "exclude"
    exclude.write_text("*.log", encoding="utf-8")

    made(git_of(), project, "one")
    made(git_of(), project, "two")

    assert exclude.read_text(encoding="utf-8") == "*.log\n/.columbina/worktrees/\n"


def test_a_worktree_is_made_in_the_project_from_another_worktree(project: Path) -> None:
    first = made(git_of(), project, "first")

    second = made(git_of(), first.path, "second")

    assert second.path == project / PLACE / "second"
    assert [one.name for one in listed(git_of(), first.path)] == ["first", "second"]
    removed(git_of(), first.path, "second")
    assert [one.name for one in listed(git_of(), project)] == ["first"]


@pytest.mark.parametrize(
    ("task", "name"),
    [("Fix the bug!", "fix-the-bug"), ("  ..a/b..  ", "a-b"), ("x" * 50, "x" * 40)],
)
def test_a_worktree_is_named_for_its_task(task: str, name: str) -> None:
    assert named(task) == name


@pytest.mark.parametrize("task", ["", "!!!", "..."])
def test_a_task_without_letters_or_digits_names_no_worktree(task: str) -> None:
    with pytest.raises(WorktreeError, match="gives no name for a worktree"):
        named(task)


def test_a_worktree_that_is_there_is_not_made_again(project: Path) -> None:
    made(git_of(), project, "same")

    with pytest.raises(WorktreeError, match="there is a worktree same already"):
        made(git_of(), project, "Same")


def test_a_worktree_is_made_only_in_a_project_of_git(tmp_path: Path, project: Path) -> None:
    alone = tmp_path / "alone"
    alone.mkdir()
    bare = tmp_path / "bare.git"
    git(tmp_path, "init", "--quiet", "--bare", str(bare))

    with pytest.raises(WorktreeError, match="is in no repository of git"):
        made(git_of(), alone, "task")
    with pytest.raises(WorktreeError, match="is not a directory"):
        made(git_of(), tmp_path / "nowhere", "task")
    with pytest.raises(WorktreeError, match="has no working directory of its own"):
        made(git_of(), bare, "task")
    with pytest.raises(WorktreeError, match="git could not be run"):
        made(str(tmp_path / "no-git"), project, "task")


def test_a_project_without_a_commit_has_no_worktrees_made(tmp_path: Path, project: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()
    git(empty, "init", "--quiet")

    with pytest.raises(WorktreeError, match="git failed"):
        made(git_of(), empty, "task")


@pytest.mark.parametrize(
    ("listed_in", "said"), [(b"**a\n", "is no pattern"), (b"\xff\n", "cannot be read")]
)
def test_a_list_that_cannot_be_read_makes_no_worktree(
    project: Path, listed_in: bytes, said: str
) -> None:
    (project / ".worktreeinclude").write_bytes(listed_in)

    with pytest.raises(WorktreeError, match=said):
        made(git_of(), project, "task")

    assert listed(git_of(), project) == []


def test_a_worktree_whose_files_cannot_be_copied_is_taken_away(
    project: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (project / ".worktreeinclude").write_text(".env\n", encoding="utf-8")

    def refused(*args: object, **more: object) -> None:
        raise OSError("the disk is full")

    monkeypatch.setattr(shutil, "copy2", refused)

    with pytest.raises(WorktreeError, match=r".env could not be copied into the worktree"):
        made(git_of(), project, "task")

    assert listed(git_of(), project) == []
    assert "columbina/task" not in git(project, "branch", "--list")


def test_git_that_cannot_be_told_to_pass_the_worktrees_over_makes_none(project: Path) -> None:
    exclude = project / ".git" / "info" / "exclude"
    exclude.unlink(missing_ok=True)
    exclude.mkdir()

    with pytest.raises(WorktreeError, match="cannot be told to pass the worktrees over"):
        made(git_of(), project, "task")


# Listing them


def test_only_the_worktrees_of_tasks_are_listed(tmp_path: Path, project: Path) -> None:
    git(project, "worktree", "add", "--quiet", "-b", "other", str(tmp_path / "other"))
    made(git_of(), project, "task")

    (one,) = listed(git_of(), project)

    assert (one.name, one.branch, one.path) == ("task", "columbina/task", project / PLACE / "task")


# Ending them


def test_the_work_of_a_task_is_merged_into_the_project(project: Path) -> None:
    worktree = made(git_of(), project, "task")
    (worktree.path / "made.txt").write_text("made\n", encoding="utf-8")

    merged(git_of(), project, "task")

    assert (project / "made.txt").read_text(encoding="utf-8") == "made\n"
    assert git(project, "log", "-1", "--format=%s") == "The work of the task task"
    assert not worktree.path.exists()
    assert git(project, "branch", "--list", "columbina/task") == ""


def test_work_that_conflicts_is_not_merged(project: Path) -> None:
    worktree = made(git_of(), project, "task")
    (worktree.path / "README.md").write_text("There.\n", encoding="utf-8")
    (project / "README.md").write_text("Here.\n", encoding="utf-8")
    git(project, "commit", "--quiet", "-am", "here")

    with pytest.raises(WorktreeError, match="it conflicts with the project"):
        merged(git_of(), project, "task")

    assert (project / "README.md").read_text(encoding="utf-8") == "Here.\n"
    assert git(project, "status", "--porcelain") == ""
    assert worktree.path.exists()


def test_work_that_would_overwrite_what_the_user_changed_is_not_merged(project: Path) -> None:
    worktree = made(git_of(), project, "task")
    (worktree.path / "README.md").write_text("There.\n", encoding="utf-8")
    (project / "README.md").write_text("Mine.\n", encoding="utf-8")

    with pytest.raises(WorktreeError, match=r"was not merged: .*The worktree is still at"):
        merged(git_of(), project, "task")

    assert (project / "README.md").read_text(encoding="utf-8") == "Mine.\n"


def test_work_has_nowhere_to_go_from_a_project_on_no_branch(project: Path) -> None:
    made(git_of(), project, "task")
    git(project, "checkout", "--quiet", "--detach")

    with pytest.raises(WorktreeError, match="on no branch"):
        merged(git_of(), project, "task")


def test_a_worktree_is_taken_away_with_its_work_only_when_asked(project: Path) -> None:
    worktree = made(git_of(), project, "task")
    (worktree.path / "README.md").write_text("Changed.\n", encoding="utf-8")

    with pytest.raises(WorktreeError, match="git failed"):
        removed(git_of(), project, "task")
    removed(git_of(), project, "task", force=True)

    assert not worktree.path.exists()
    assert listed(git_of(), project) == []


def test_a_worktree_that_is_not_there_cannot_be_ended(project: Path) -> None:
    with pytest.raises(WorktreeError, match="there is no worktree none"):
        merged(git_of(), project, "none")


def test_where_git_would_be_sent_elsewhere_it_is_not(
    project: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("GIT_DIR", str(tmp_path / "elsewhere"))

    assert made(git_of(), project, "task").path.exists()
    assert worktrees.listed(git_of(), project) != []
