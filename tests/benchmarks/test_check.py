"""Whether a task can be judged at all: ``bench.check``."""

import json
from pathlib import Path

import pytest

from bench.check import check, main
from bench.task import load

HIDDEN = """import unittest

from calc import add


class Hidden(unittest.TestCase):
    def test_adds(self) -> None:
        self.assertEqual(add(2, 3), 5)
"""

WRONG = "def add(a, b):\n    return a - b\n"
RIGHT = "def add(a, b):\n    return a + b\n"


def task(root: Path, name: str, category: str, written: dict[str, str]) -> Path:
    directory = root / name
    directory.mkdir(parents=True)
    (directory / "task.yml").write_text(
        f"id: {name}\ncategory: {category}\nprompt: Do it.\n", encoding="utf-8"
    )
    for path, content in written.items():
        (directory / path).parent.mkdir(parents=True, exist_ok=True)
        (directory / path).write_text(content, encoding="utf-8")
    return directory


def code_task(root: Path, name: str, start: str, fixed: str) -> Path:
    return task(
        root,
        name,
        "code",
        {"workspace/calc.py": start, "reference/calc.py": fixed, "checks/test_hidden.py": HIDDEN},
    )


def test_a_task_whose_start_fails_and_whose_reference_passes_is_good(tmp_path: Path) -> None:
    loaded = load(code_task(tmp_path / "tasks", "code-good", WRONG, RIGHT))

    assert check(loaded, tmp_path / "scratch") == []


def test_a_task_whose_start_passes_already_says_so(tmp_path: Path) -> None:
    loaded = load(code_task(tmp_path / "tasks", "code-done", RIGHT, RIGHT))

    assert check(loaded, tmp_path / "scratch") == ["its start passes already"]


def test_a_task_whose_reference_fails_says_so_and_why(tmp_path: Path) -> None:
    loaded = load(code_task(tmp_path / "tasks", "code-bad", WRONG, WRONG))

    (problem,) = check(loaded, tmp_path / "scratch")
    assert problem.startswith("its reference does not pass:")
    assert "AssertionError" in problem


def test_the_reference_of_a_code_task_is_laid_over_its_workspace(tmp_path: Path) -> None:
    # The reference holds only what a fix changes; the rest of the workspace stays.
    directory = code_task(tmp_path / "tasks", "code-two", WRONG, RIGHT)
    (directory / "workspace" / "other.py").write_text("VALUE = 5\n", encoding="utf-8")
    (directory / "checks" / "test_other.py").write_text(
        "import unittest\nfrom other import VALUE\n\n\n"
        "class Other(unittest.TestCase):\n"
        "    def test_value(self) -> None:\n        self.assertEqual(VALUE, 5)\n",
        encoding="utf-8",
    )

    assert check(load(directory), tmp_path / "scratch") == []


def test_the_workspace_of_a_task_is_not_changed_by_its_check(tmp_path: Path) -> None:
    loaded = load(code_task(tmp_path / "tasks", "code-good", WRONG, RIGHT))

    check(loaded, tmp_path / "scratch")

    assert (loaded.workspace / "calc.py").read_text(encoding="utf-8") == WRONG


def test_the_reference_of_a_files_task_is_the_tree_it_should_become(tmp_path: Path) -> None:
    rules = json.dumps({"exists": ["b.txt"], "absent": ["a.txt"]})
    good = task(
        tmp_path / "tasks",
        "files-good",
        "files",
        {"workspace/a.txt": "x", "reference/b.txt": "x", "checks/expected.json": rules},
    )
    bad = task(
        tmp_path / "tasks",
        "files-bad",
        "files",
        {"workspace/a.txt": "x", "reference/a.txt": "x", "checks/expected.json": rules},
    )

    assert check(load(good), tmp_path / "scratch") == []
    assert check(load(bad), tmp_path / "scratch") == [
        "its reference does not pass: b.txt is not there\na.txt is still there"
    ]


def test_every_task_of_a_directory_is_checked_and_the_bad_ones_are_named(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code_task(tmp_path / "tasks", "code-good", WRONG, RIGHT)
    code_task(tmp_path / "tasks", "code-done", RIGHT, RIGHT)

    assert main([str(tmp_path / "tasks")]) == 1
    printed = capsys.readouterr().out
    assert "code-done: its start passes already" in printed
    assert "code-good" not in printed
    assert printed.rstrip().endswith("2 tasks, 1 with something wrong")


def test_a_directory_of_good_tasks_is_said_to_be_good(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code_task(tmp_path / "tasks", "code-good", WRONG, RIGHT)

    assert main([str(tmp_path / "tasks")]) == 0
    assert capsys.readouterr().out.rstrip().endswith("1 task, all of them good")
