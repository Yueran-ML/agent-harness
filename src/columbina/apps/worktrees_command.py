"""What ``columbina worktrees`` does: the worktrees of the tasks of a project, made and ended.

``list`` says what worktrees the project of the directory has. ``add NAME`` makes one for a task,
and says where it is: a session that is begun there changes no file of the project. ``merge
NAME`` takes the work of a task into the branch of the project, and ``remove NAME`` drops it;
either takes the worktree away. With ``--json`` what is said is a document of JSON, for another
program. See :mod:`columbina.worktrees`.
"""

import os
from collections.abc import Callable, Mapping
from pathlib import Path

from columbina import programs, worktrees
from columbina.worktrees import Worktree, WorktreeError

__all__ = ["ADD", "LIST", "MERGE", "REMOVE", "WORKTREES", "run"]

WORKTREES = "worktrees"
LIST = "list"
ADD = "add"
MERGE = "merge"
REMOVE = "remove"


def _shown(worktree: Worktree) -> dict[str, object]:
    return {
        "name": worktree.name,
        "path": str(worktree.path),
        "branch": worktree.branch,
        "copied": list(worktree.copied),
    }


def run(
    what: str,
    name: str | None,
    *,
    cwd: Path,
    force: bool,
    as_json: bool,
    say: Callable[[str], None],
    answer: Callable[[Mapping[str, object]], None],
) -> int:
    """Carry out the command, and return its exit code.

    Args:
        what: ``list``, ``add``, ``merge`` or ``remove``.
        name: The task, for all but ``list``.
        cwd: A directory of the project.
        force: Whether ``remove`` takes away what was not committed.
        as_json: Whether what is said is a document of JSON.
        say: Says a line to a person.
        answer: Writes a document of JSON.
    """
    git = programs.found("git", os.environ.get("PATH", ""), outside=[cwd])
    try:
        if git is None:
            raise WorktreeError("git is not on the search path")
        if what == LIST:
            found = worktrees.listed(git, cwd)
            if as_json:
                answer({"worktrees": [_shown(one) for one in found]})
            elif not found:
                say("There are no worktrees. 'columbina worktrees add NAME' makes one for a task.")
            else:
                say("\n".join(f"{one.name}  {one.branch}  {one.path}" for one in found))
            return 0
        task = str(name)
        if what == ADD:
            done = worktrees.made(git, cwd, task)
            told = f"Made {done.name} at {done.path}, on the branch {done.branch}."
            if done.copied:
                told += f" Copied into it: {', '.join(done.copied)}."
        elif what == MERGE:
            done = worktrees.merged(git, cwd, worktrees.named(task))
            told = f"Merged {done.branch} into the project, and took {done.name} away."
        else:
            done = worktrees.removed(git, cwd, worktrees.named(task), force=force)
            told = f"Took {done.name} away, with its branch {done.branch}."
    except WorktreeError as error:
        if as_json:
            answer({"error": str(error)})
        else:
            say(f"columbina: {error}")
        return 2
    if as_json:
        answer({**_shown(done), "done": what})
    else:
        say(told)
    return 0
