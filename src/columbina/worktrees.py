"""A working directory of its own for a task: a worktree of git, beside the project.

A task that runs in a worktree of its own changes no file that the user works on, and two tasks
that run at once change none of each other's. :func:`made` makes one for a task in the project
of a directory, in ``.columbina/worktrees/<name>``, on a branch ``columbina/<name>`` that begins
where the project is now. git is told to pass the directory of the worktrees over, in the file
of the repository that nobody shares, so the project shows nothing new.

A worktree holds the files that git keeps. What the project holds besides, such as a file of
settings with secrets or the packages that were installed, it does not, unless the project
lists it in ``.worktreeinclude``: a pattern on each line, as ``Path.glob`` reads it, from the top
of the project. Of what the patterns find, what git passes over is copied into the worktree.

When the task is done, :func:`merged` takes its work into the branch that the project is on, and
the worktree and its branch go away. What was not committed yet is committed first. Where the
work cannot be merged, nothing changes. :func:`removed` takes a worktree away with its branch,
and its work with them.
"""

import os
import re
import shutil
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from columbina import process

__all__ = [
    "BRANCHES",
    "INCLUDE",
    "PLACE",
    "Worktree",
    "WorktreeError",
    "listed",
    "made",
    "merged",
    "named",
    "removed",
]

PLACE = Path(".columbina") / "worktrees"
"""Where the worktrees of a project are, from its top."""

BRANCHES = "columbina/"
"""What the name of the branch of a worktree begins with."""

INCLUDE = ".worktreeinclude"
"""The file of a project that lists what a worktree is given of what git passes over."""

_SECONDS = 60.0
"""How long one run of git may take."""

_LONGEST = 40
"""How many characters the name of a worktree has at most."""

_ELSEWHERE = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR")
"""What would send git to another repository than that of the directory."""


class WorktreeError(Exception):
    """What was asked of a worktree cannot be done. The message says why, for a person."""


@dataclass(frozen=True, slots=True)
class Worktree:
    """The worktree of a task.

    Attributes:
        name: The name of the worktree.
        path: Where it is.
        branch: The branch that it is on.
        copied: What it was given of what git passes over, from the top of the project, when it
            was made. Empty for a worktree that was listed.
    """

    name: str
    path: Path
    branch: str
    copied: tuple[str, ...] = ()


def _tried(git: str, where: Path, *arguments: str) -> tuple[bool, str, str]:
    """Run git in a directory, and return whether it did it, what it wrote, and what it said.

    Raises:
        WorktreeError: git could not be run.
    """
    env = {
        name: value
        for name, value in process.cleaned(os.environ).items()
        if name.upper() not in _ELSEWHERE
    }
    try:
        done = subprocess.run(
            [git, *arguments],
            cwd=where,
            env={**env, "GIT_TERMINAL_PROMPT": "0"},
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
            stdin=subprocess.DEVNULL,
            timeout=_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise WorktreeError(f"git could not be run: {error}") from None
    said = (done.stderr or done.stdout).strip().splitlines()
    return done.returncode == 0, done.stdout.strip(), said[-1] if said else str(done.returncode)


def _git(git: str, where: Path, *arguments: str) -> str:
    """Run git in a directory, and return what it wrote.

    Raises:
        WorktreeError: git could not be run, or it failed.
    """
    done, wrote, said = _tried(git, where, *arguments)
    if not done:
        raise WorktreeError(f"git failed: {said}")
    return wrote


def _common(git: str, cwd: Path) -> Path:
    """Return the directory of git that the worktrees of the project of ``cwd`` share.

    Raises:
        WorktreeError: ``cwd`` is in no repository of git.
    """
    if not cwd.is_dir():
        raise WorktreeError(f"{cwd} is not a directory")
    done, wrote, _ = _tried(git, cwd, "rev-parse", "--git-common-dir")
    if not done:
        raise WorktreeError(f"{cwd} is in no repository of git")
    return (cwd / wrote).resolve()


def _top(git: str, cwd: Path) -> Path:
    """Return the top of the project of ``cwd``: of its first worktree, if it is in another."""
    common = _common(git, cwd)
    if common.name != ".git":
        raise WorktreeError(f"the repository of {cwd} has no working directory of its own")
    return common.parent


def named(name: str) -> str:
    """Return the name of a worktree for what the user calls a task.

    Raises:
        WorktreeError: Nothing of the name is left for a worktree.
    """
    slug = re.sub(r"[^a-z0-9._-]+", "-", name.lower()).strip("-.")[:_LONGEST].strip("-.")
    if not slug:
        raise WorktreeError(f"{name!r} gives no name for a worktree: say it with letters or digits")
    return slug


def _passed_over(git: str, common: Path) -> None:
    """Have git pass the worktrees over, in the file of the repository that nobody shares."""
    exclude = common / "info" / "exclude"
    line = f"/{PLACE.as_posix()}/"
    try:
        there = exclude.read_text(encoding="utf-8") if exclude.exists() else ""
        if line in there.splitlines():
            return
        exclude.parent.mkdir(parents=True, exist_ok=True)
        joined = there if not there or there.endswith("\n") else there + "\n"
        exclude.write_text(f"{joined}{line}\n", encoding="utf-8")
    except OSError as error:
        raise WorktreeError(f"git cannot be told to pass the worktrees over: {error}") from None


def _included(git: str, top: Path) -> list[str]:
    """Return what ``.worktreeinclude`` finds and git passes over, from the top of the project."""
    listed = top / INCLUDE
    try:
        lines = listed.read_text(encoding="utf-8").splitlines() if listed.is_file() else []
    except (OSError, UnicodeDecodeError) as error:
        raise WorktreeError(f"{INCLUDE} cannot be read: {error}") from None
    found: list[str] = []
    for line in lines:
        pattern = line.strip()
        if not pattern or pattern.startswith("#"):
            continue
        try:
            matches = sorted(top.glob(pattern))
        except (ValueError, NotImplementedError) as error:
            raise WorktreeError(f"{pattern!r} in {INCLUDE} is no pattern: {error}") from None
        for match in matches:
            relative = match.relative_to(top).as_posix()
            first = relative.split("/", 1)[0]
            if first in (".git", PLACE.parts[0]) or relative in found:
                continue
            found.append(relative)
    if not found:
        return []
    # git says nothing, and fails, where it passes none of them over.
    _, ignored, _ = _tried(git, top, "check-ignore", "--", *found)
    passed = set(ignored.splitlines())
    return [one for one in found if one in passed]


def _copied(top: Path, into: Path, paths: Sequence[str]) -> None:
    for relative in paths:
        source, target = top / relative, into / relative
        try:
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, target, dirs_exist_ok=True, symlinks=True)
            else:
                shutil.copy2(source, target, follow_symlinks=False)
        except OSError as error:
            raise WorktreeError(
                f"{relative} could not be copied into the worktree: {error}"
            ) from None


def made(git: str, cwd: Path, task: str) -> Worktree:
    """Make a worktree for a task in the project of a directory.

    Args:
        git: The program git.
        cwd: A directory of the project.
        task: What the user calls the task. The worktree is named for it.

    Raises:
        WorktreeError: It cannot be made. Nothing is left of it then.
    """
    name = named(task)
    common = _common(git, cwd)
    top = _top(git, cwd)
    path = top / PLACE / name
    if path.exists():
        raise WorktreeError(f"there is a worktree {name} already")
    branch = BRANCHES + name
    _passed_over(git, common)
    copying = _included(git, top)
    _git(git, top, "worktree", "add", "--quiet", "-b", branch, str(path), "HEAD")
    try:
        _copied(top, path, copying)
    except WorktreeError:
        removed(git, cwd, name, force=True)
        raise
    return Worktree(name, path, branch, tuple(copying))


def listed(git: str, cwd: Path) -> list[Worktree]:
    """Return the worktrees of the tasks of the project of a directory.

    Raises:
        WorktreeError: ``cwd`` is in no project of git.
    """
    top = _top(git, cwd)
    place = (top / PLACE).resolve()
    found: list[Worktree] = []
    for block in _git(git, top, "worktree", "list", "--porcelain").split("\n\n"):
        fields: dict[str, str] = {}
        for line in block.splitlines():
            key, _, value = line.partition(" ")
            fields[key] = value
        path = Path(fields.get("worktree", "")).resolve()
        branch = fields.get("branch", "").removeprefix("refs/heads/")
        if path.parent == place and branch.startswith(BRANCHES):
            found.append(Worktree(path.name, path, branch))
    return found


def _one(git: str, cwd: Path, name: str) -> tuple[Path, Worktree]:
    top = _top(git, cwd)
    for worktree in listed(git, cwd):
        if worktree.name == name:
            return top, worktree
    raise WorktreeError(f"there is no worktree {name}")


def merged(git: str, cwd: Path, name: str) -> Worktree:
    """Take the work of a worktree into the branch of the project, and take the worktree away.

    Raises:
        WorktreeError: It cannot be merged, and nothing was changed; or it was merged, and the
            worktree could not be taken away.
    """
    top, worktree = _one(git, cwd, name)
    if _git(git, worktree.path, "status", "--porcelain"):
        _git(git, worktree.path, "add", "--all")
        _git(git, worktree.path, "commit", "--quiet", "-m", f"The work of the task {name}")
    if not _git(git, top, "branch", "--show-current"):
        raise WorktreeError("the project is on no branch, so the work has nowhere to go")
    done, _, said = _tried(git, top, "merge", "--no-edit", worktree.branch)
    if not done:
        if _tried(git, top, "rev-parse", "-q", "--verify", "MERGE_HEAD")[0]:
            _tried(git, top, "merge", "--abort")
            said = "it conflicts with the project"
        raise WorktreeError(
            f"the work of {name} was not merged: {said}. The worktree is still at {worktree.path}"
        )
    removed(git, cwd, name)
    return worktree


def removed(git: str, cwd: Path, name: str, *, force: bool = False) -> Worktree:
    """Take a worktree away, with its branch and its work.

    Args:
        git: The program git.
        cwd: A directory of the project.
        name: The name of the worktree.
        force: Whether it goes although it holds what was not committed.

    Raises:
        WorktreeError: It cannot be taken away.
    """
    top, worktree = _one(git, cwd, name)
    _git(git, top, "worktree", "remove", *(["--force"] if force else []), str(worktree.path))
    _git(git, top, "branch", "-D", worktree.branch)
    return worktree
