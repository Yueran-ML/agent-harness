"""How an attempt is judged: ``bench.grade``."""

import json
import os
import shutil
import sqlite3
import subprocess
from pathlib import Path

import pytest

from bench import grade
from bench.check import check
from bench.grade import Call, Verdict, code, code_argv, compute, files, last, sql
from bench.task import load

IMAGE = "python:3.12-slim"


def task(root: Path, category: str, written: dict[str, str] | None = None) -> Path:
    """Write a task of a category, with files by their paths in its directory."""
    directory = root / f"{category}-one"
    (directory / "workspace").mkdir(parents=True)
    (directory / "task.yml").write_text(
        f"id: {category}-one\ncategory: {category}\nprompt: Do it.\n", encoding="utf-8"
    )
    for name, content in (written or {}).items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return directory


def tree(root: Path, content: dict[str, str]) -> Path:
    """Write files by their paths below a directory, as they are, line ends and all."""
    root.mkdir(parents=True, exist_ok=True)
    for name, text in content.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode("utf-8"))
    return root


HIDDEN = """import unittest

from calc import add


class Hidden(unittest.TestCase):
    def test_adds(self) -> None:
        self.assertEqual(add(2, 3), 5)
"""


# The calls of an attempt


def test_the_last_call_of_a_tool_that_went_through_counts() -> None:
    calls = (
        Call("submit", {"x": 1}),
        Call("submit", {"x": 2}),
        Call("submit", {"x": 3}, failed=True),
        Call("lookup", {"name": "a"}),
    )

    assert last(calls, "submit") == Call("submit", {"x": 2})
    assert last(calls, "other") is None


# Code


def test_hidden_tests_that_pass_pass_the_attempt(tmp_path: Path) -> None:
    loaded = load(task(tmp_path, "code", {"checks/test_hidden.py": HIDDEN}))
    workspace = tree(tmp_path / "work", {"calc.py": "def add(a, b):\n    return a + b\n"})

    verdict = code(loaded, workspace, container=None)

    assert verdict.passed
    assert "OK" in verdict.said


@pytest.mark.parametrize(
    "source",
    ["def add(a, b):\n    return a - b\n", "def plus(a, b):\n    return a + b\n", ""],
    ids=["wrong", "missing", "empty"],
)
def test_hidden_tests_that_fail_or_cannot_run_fail_the_attempt(tmp_path: Path, source: str) -> None:
    loaded = load(task(tmp_path, "code", {"checks/test_hidden.py": HIDDEN}))
    workspace = tree(tmp_path / "work", {"calc.py": source})

    verdict = code(loaded, workspace, container=None)

    assert not verdict.passed
    assert verdict.said


def test_hidden_tests_that_run_too_long_fail_the_attempt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    slow = "import time\nimport unittest\n\n\nclass Slow(unittest.TestCase):\n"
    slow += "    def test_waits(self) -> None:\n        time.sleep(30)\n"
    loaded = load(task(tmp_path, "code", {"checks/test_slow.py": slow}))
    monkeypatch.setattr(grade, "_TESTS_SECONDS", 1)

    verdict = code(loaded, loaded.workspace, container=None)

    assert verdict == Verdict(False, "the hidden tests ran for more than 1 seconds")


def test_a_task_without_hidden_tests_passes_nothing(tmp_path: Path) -> None:
    loaded = load(task(tmp_path, "code"))
    (loaded.directory / "checks").mkdir()

    verdict = code(loaded, loaded.workspace, container=None)

    assert verdict == Verdict(False, "the task has no hidden tests")


def test_in_a_container_the_hidden_tests_are_read_only_and_there_is_no_network(
    tmp_path: Path,
) -> None:
    checks, workspace = tmp_path / "checks", tmp_path / "work"

    argv = code_argv(checks, workspace, IMAGE)

    assert argv[:3] == ["docker", "run", "--rm"]
    assert argv[argv.index("--network") + 1] == "none"
    assert argv[argv.index("-v") + 1] == f"{workspace}:/work"
    assert f"{checks}:/checks:ro" in argv
    assert argv[-9:] == [
        IMAGE,
        "python",
        "-m",
        "unittest",
        "discover",
        "-s",
        "/checks",
        "-t",
        "/checks",
    ]


def _image_is_here() -> bool:
    docker = shutil.which("docker")
    if docker is None:
        return False
    done = subprocess.run(
        [docker, "image", "inspect", IMAGE], capture_output=True, timeout=60, check=False
    )
    return done.returncode == 0


@pytest.mark.skipif(
    os.environ.get("COLUMBINA_BENCH_CONTAINER") != "1" and not _image_is_here(),
    reason=f"no Docker engine with {IMAGE} answers",
)
def test_hidden_tests_run_in_a_container_where_one_is_named(tmp_path: Path) -> None:
    loaded = load(task(tmp_path, "code", {"checks/test_hidden.py": HIDDEN}))
    workspace = tree(tmp_path / "work", {"calc.py": "def add(a, b):\n    return a + b\n"})

    assert code(loaded, workspace, container=IMAGE).passed
    (workspace / "calc.py").write_text("def add(a, b):\n    return 0\n", encoding="utf-8")
    assert not code(loaded, workspace, container=IMAGE).passed


# Files


def expected(**rules: object) -> str:
    return json.dumps(rules)


def test_a_tree_that_holds_what_it_should_passes(tmp_path: Path) -> None:
    rules = expected(
        exists=["docs/a.md"],
        absent=["old.txt"],
        equal={"notes.txt": "one\ntwo\n"},
        json={"config.json": {"server.port": 8080, "debug": False}},
        lines={"names.txt": ["b", "a"]},
    )
    loaded = load(task(tmp_path, "files", {"checks/expected.json": rules}))
    workspace = tree(
        tmp_path / "work",
        {
            "docs/a.md": "#",
            "notes.txt": "one\r\ntwo\r\n",
            "config.json": '{"server": {"port": 8080}, "debug": false}',
            "names.txt": "a\nb\n",
        },
    )

    assert files(loaded, workspace) == Verdict(True, "")


@pytest.mark.parametrize(
    ("rules", "content", "said"),
    [
        ({"exists": ["a.txt"]}, {}, "a.txt is not there"),
        ({"absent": ["a.txt"]}, {"a.txt": ""}, "a.txt is still there"),
        ({"equal": {"a.txt": "x\n"}}, {"a.txt": "y\n"}, "a.txt does not hold what it should"),
        ({"equal": {"a.txt": "x\n"}}, {}, "a.txt does not hold what it should"),
        ({"json": {"c.json": {"a.b": 1}}}, {"c.json": '{"a": {"b": 2}}'}, "c.json: a.b is not 1"),
        ({"json": {"c.json": {"a.b": 1}}}, {"c.json": '{"a": 1}'}, "c.json: a.b is not 1"),
        ({"json": {"c.json": {"a.b": None}}}, {"c.json": '{"a": 1}'}, "c.json: a.b is not None"),
        ({"json": {"c.json": {"a": 1}}}, {"c.json": "{"}, "c.json is no JSON"),
        ({"json": {"c.json": {"a": 1}}}, {}, "c.json is no JSON"),
        ({"lines": {"n.txt": ["a", "b"]}}, {"n.txt": "a\na\n"}, "n.txt does not hold the lines"),
        ({"lines": {"n.txt": ["a", "b"]}}, {"n.txt": "b\na\nb\n"}, "n.txt does not hold the lines"),
    ],
)
def test_a_tree_that_does_not_hold_what_it_should_fails_and_says_why(
    tmp_path: Path, rules: dict[str, object], content: dict[str, str], said: str
) -> None:
    loaded = load(task(tmp_path, "files", {"checks/expected.json": expected(**rules)}))
    workspace = tree(tmp_path / "work", content)

    verdict = files(loaded, workspace)

    assert not verdict.passed
    assert said in verdict.said


# SQL


def databases(directory: Path, rows: list[tuple[str, int]], changed: list[tuple[str, int]]) -> None:
    for name, held in (("database.sqlite", rows), ("changed.sqlite", changed)):
        (directory / "private").mkdir(exist_ok=True)
        with sqlite3.connect(directory / "private" / name) as made:
            made.execute("CREATE TABLE sales (city TEXT, amount INTEGER)")
            made.executemany("INSERT INTO sales VALUES (?, ?)", held)
        made.close()


@pytest.fixture
def sql_task(tmp_path: Path) -> Path:
    directory = task(
        tmp_path,
        "sql",
        {
            "reference/answer.sql": "SELECT city, SUM(amount) FROM sales GROUP BY city "
            "ORDER BY SUM(amount) DESC"
        },
    )
    databases(
        directory,
        [("Brisbane", 5), ("Perth", 3), ("Brisbane", 4)],
        [("Brisbane", 1), ("Perth", 7), ("Hobart", 2)],
    )
    return directory


def submitted(query: str) -> tuple[Call, ...]:
    return (Call("submit_sql", {"sql": query}),)


def test_a_query_that_is_right_on_both_databases_passes(sql_task: Path) -> None:
    query = "select city, sum(amount) as total from sales group by city order by total desc"

    assert sql(load(sql_task), submitted(query)) == Verdict(True, "")


def test_a_query_that_holds_the_answer_and_not_the_way_to_it_fails(sql_task: Path) -> None:
    query = "SELECT 'Brisbane', 9 UNION ALL SELECT 'Perth', 3"

    verdict = sql(load(sql_task), submitted(query))

    assert verdict == Verdict(
        False,
        "on changed.sqlite the query returns 2 rows, and not the 3 that it should in that order",
    )


def test_rows_in_another_order_fail_where_the_order_was_asked_for(sql_task: Path) -> None:
    query = "SELECT city, SUM(amount) FROM sales GROUP BY city ORDER BY city"

    assert not sql(load(sql_task), submitted(query)).passed


def test_rows_in_any_order_pass_where_no_order_was_asked_for(tmp_path: Path) -> None:
    directory = task(tmp_path, "sql", {"reference/answer.sql": "SELECT city FROM sales"})
    databases(directory, [("A", 1), ("B", 2)], [("C", 1), ("D", 2)])

    query = "SELECT city FROM sales ORDER BY city DESC"
    assert sql(load(directory), submitted(query)).passed


def test_a_query_that_fails_says_where_and_why(sql_task: Path) -> None:
    verdict = sql(load(sql_task), submitted("SELECT * FROM orders"))

    assert verdict == Verdict(False, "the query fails on database.sqlite: no such table: orders")


@pytest.mark.parametrize(
    "calls",
    [(), (Call("submit_sql", {"sql": "SELECT 1"}, failed=True),), (Call("submit_sql", {}),)],
    ids=["none", "failed", "without a query"],
)
def test_without_a_query_that_went_through_nothing_passes(
    sql_task: Path, calls: tuple[Call, ...]
) -> None:
    assert sql(load(sql_task), calls) == Verdict(False, "no query was submitted")


def test_the_reference_of_a_sql_task_is_its_query_submitted(sql_task: Path, tmp_path: Path) -> None:
    assert check(load(sql_task), tmp_path / "scratch") == []


# Compute


@pytest.fixture
def compute_task(tmp_path: Path) -> Path:
    answer = json.dumps({"value": 1260, "lookups": ["rate", "hours", "days"]})
    return task(tmp_path, "compute", {"reference/answer.json": answer})


def looked(*names: str) -> tuple[Call, ...]:
    return tuple(Call("lookup", {"name": name}) for name in names)


def answered(value: object) -> Call:
    return Call("submit_answer", {"value": value})


def test_the_right_answer_after_every_number_was_looked_up_passes(compute_task: Path) -> None:
    calls = (*looked("rate", "hours", "days", "rate"), answered(1260))

    assert compute(load(compute_task), calls) == Verdict(True, "")


def test_a_wrong_answer_fails_and_says_both(compute_task: Path) -> None:
    calls = (*looked("rate", "hours", "days"), answered(1200))

    assert compute(load(compute_task), calls) == Verdict(
        False, "the answer was 1200, and it is 1260"
    )


def test_a_right_answer_without_every_number_looked_up_fails(compute_task: Path) -> None:
    calls = (
        *looked("rate", "hours"),
        Call("lookup", {"name": "days"}, failed=True),
        answered(1260),
    )

    assert compute(load(compute_task), calls) == Verdict(
        False, "the answer is right, and days was not looked up"
    )


def test_the_last_answer_counts(compute_task: Path) -> None:
    calls = (*looked("rate", "hours", "days"), answered(1260), answered(7))

    assert not compute(load(compute_task), calls).passed
    assert compute(load(compute_task), looked("rate")) == Verdict(False, "no answer was submitted")


def test_the_reference_of_a_compute_task_looks_up_every_number_and_answers(
    compute_task: Path, tmp_path: Path
) -> None:
    assert check(load(compute_task), tmp_path / "scratch") == []
