"""Whether a task can be judged: its start fails, and its reference passes.

A task whose start passes measures nothing, and one whose reference fails
fails every model. Both are found here, without a model: the start is judged
as it is, and the reference is judged as if an attempt had left it. The
hidden tests run with this Python, since the code they run is the reference.

``python -m bench check [DIRECTORY]`` checks every task below a directory,
``bench/tasks`` if none is named, and says which ones are wrong.
"""

import json
import shutil
import tempfile
from pathlib import Path

from bench import grade
from bench.grade import Call, Verdict
from bench.task import Loaded, tasks

__all__ = ["check", "judged", "main", "reference_calls"]

TASKS = Path(__file__).parent / "tasks"
"""Where the tasks of the benchmark are."""


def judged(
    loaded: Loaded, workspace: Path, calls: tuple[Call, ...], *, container: str | None
) -> Verdict:
    """Judge what an attempt left, by the category of its task.

    Args:
        loaded: The task.
        workspace: The working directory as the attempt left it.
        calls: The calls of tools that the attempt made.
        container: The image that the hidden tests of a code task run in, or
            ``None`` to run them with this Python.
    """
    category = loaded.task.category
    if category == "code":
        return grade.code(loaded, workspace, container=container)
    if category == "files":
        return grade.files(loaded, workspace)
    if category == "sql":
        return grade.sql(loaded, calls)
    return grade.compute(loaded, calls)


def reference_calls(loaded: Loaded) -> tuple[Call, ...]:
    """Return the calls of tools that a right attempt at a task makes, if its category has any."""
    reference = loaded.directory / "reference"
    if loaded.task.category == "sql":
        sql = (reference / "answer.sql").read_text(encoding="utf-8")
        return (Call(grade.SUBMIT_SQL, {"sql": sql}),)
    if loaded.task.category == "compute":
        answer = json.loads((reference / "answer.json").read_text(encoding="utf-8"))
        looked = tuple(Call(grade.LOOKUP, {"name": name}) for name in answer["lookups"])
        return (*looked, Call(grade.SUBMIT_ANSWER, {"value": answer["value"]}))
    return ()


def _reference_tree(loaded: Loaded, target: Path) -> None:
    """Make the working directory that a right attempt leaves."""
    reference = loaded.directory / "reference"
    if loaded.task.category == "files":
        # The reference is the whole tree that the start should become.
        shutil.copytree(reference, target)
        return
    shutil.copytree(loaded.workspace, target)
    if loaded.task.category == "code":
        # The reference holds what a fix changes, laid over the start.
        shutil.copytree(reference, target, dirs_exist_ok=True)


def check(loaded: Loaded, scratch: Path) -> list[str]:
    """Say what is wrong with a task, or nothing.

    Args:
        loaded: The task.
        scratch: A directory to make copies of its working directory in.
    """
    place = scratch / loaded.task.id
    shutil.rmtree(place, ignore_errors=True)
    problems: list[str] = []
    start = place / "start"
    shutil.copytree(loaded.workspace, start)
    if judged(loaded, start, (), container=None).passed:
        problems.append("its start passes already")
    right = place / "reference"
    _reference_tree(loaded, right)
    verdict = judged(loaded, right, reference_calls(loaded), container=None)
    if not verdict.passed:
        problems.append(f"its reference does not pass: {verdict.said}")
    return problems


def main(argv: list[str]) -> int:
    """Check every task below a directory. Return 1 if one of them is wrong, else 0."""
    root = Path(argv[0]) if argv else TASKS
    found = tasks(root)
    wrong = 0
    with tempfile.TemporaryDirectory(prefix="bench-check-") as scratch:
        for loaded in found:
            problems = check(loaded, Path(scratch))
            wrong += bool(problems)
            for problem in problems:
                print(f"{loaded.task.id}: {problem}")
    count = f"{len(found)} task" if len(found) == 1 else f"{len(found)} tasks"
    print(f"{count}, all of them good" if not wrong else f"{count}, {wrong} with something wrong")
    return 1 if wrong else 0
