"""How an attempt is judged: ``bench.grade``."""

import json
import os
from pathlib import Path

import pytest

from bench import database, grade
from bench.check import check
from bench.grade import Call, Verdict, code, code_argv, compute, files, last, sql
from bench.task import load
from support.engines import real

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


def test_hidden_tests_run_in_a_container_where_one_is_named(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The judge calls docker by its name, so the one that was found goes first on the path.
    program = real((IMAGE,))
    monkeypatch.setenv("PATH", f"{Path(program).parent}{os.pathsep}{os.environ.get('PATH', '')}")
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
        ({"exists": ["a.txt/b.txt"]}, {"a.txt": "x"}, "a.txt/b.txt is not there"),
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


def test_a_file_that_is_no_text_of_utf8_holds_nothing_that_is_compared(tmp_path: Path) -> None:
    rules = expected(equal={"a.txt": "x"}, lines={"a.txt": ["x"]})
    loaded = load(task(tmp_path, "files", {"checks/expected.json": rules}))
    workspace = tmp_path / "work"
    workspace.mkdir()
    (workspace / "a.txt").write_bytes(b"\xff\xfe x")

    assert files(loaded, workspace) == Verdict(
        False, "a.txt does not hold what it should\na.txt does not hold the lines it should"
    )


def test_a_name_counts_only_as_it_is_written_on_every_system(tmp_path: Path) -> None:
    # A disk that does not tell upper case from lower would find inbox/csv in inbox/CSV.
    rules = expected(exists=["inbox/csv/a.csv"], absent=["inbox/CSV/a.csv", "old.txt"])
    loaded = load(task(tmp_path, "files", {"checks/expected.json": rules}))

    wrong = tree(tmp_path / "wrong", {"inbox/CSV/a.csv": "x", "OLD.txt": "y"})
    right = tree(tmp_path / "right", {"inbox/csv/a.csv": "x", "OLD.txt": "y"})

    assert files(loaded, wrong) == Verdict(
        False, "inbox/csv/a.csv is not there\ninbox/CSV/a.csv is still there"
    )
    assert files(loaded, right) == Verdict(True, "")


def test_a_file_is_read_only_by_the_name_as_it_is_written(tmp_path: Path) -> None:
    rules = expected(equal={"Notes.txt": "x\n"}, json={"Config.json": {"a": 1}})
    loaded = load(task(tmp_path, "files", {"checks/expected.json": rules}))
    workspace = tree(tmp_path / "work", {"notes.txt": "x\n", "config.json": '{"a": 1}'})

    assert files(loaded, workspace) == Verdict(
        False, "Notes.txt does not hold what it should\nConfig.json is no JSON"
    )


# SQL


def statements(rows: list[tuple[str, int]]) -> str:
    """The statements that make a database of sales with these rows."""
    made = ["CREATE TABLE sales (city TEXT, amount INTEGER);"]
    made.extend(f"INSERT INTO sales VALUES ('{city}', {amount});" for city, amount in rows)
    return "\n".join(made) + "\n"


def databases(directory: Path, rows: list[tuple[str, int]], changed: list[tuple[str, int]]) -> None:
    """Write the statements of the two databases of a task of SQL."""
    (directory / "private").mkdir(exist_ok=True)
    (directory / "private" / "database.sql").write_text(statements(rows), encoding="utf-8")
    (directory / "private" / "changed.sql").write_text(statements(changed), encoding="utf-8")


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


def judging(directory: Path, query: str | None = None, *, calls: tuple[Call, ...] = ()) -> Verdict:
    """Build the databases of a task of SQL beside it, and judge a query on them."""
    loaded = load(directory)
    built = database.built(loaded, directory.parent / "built")
    asked = calls if query is None else (Call("submit_sql", {"sql": query}),)
    return sql(loaded, asked, built)


def test_the_databases_of_a_task_are_made_from_their_statements(sql_task: Path) -> None:
    built = database.built(load(sql_task), sql_task.parent / "built")

    assert sorted(path.name for path in built.iterdir()) == ["changed.sqlite", "database.sqlite"]
    assert database.read(built / "changed.sqlite", "SELECT COUNT(*) FROM sales").rows == ((3,),)


def test_making_the_databases_again_starts_them_afresh(sql_task: Path) -> None:
    database.built(load(sql_task), sql_task.parent / "built")
    built = database.built(load(sql_task), sql_task.parent / "built")

    assert database.read(built / "database.sqlite", "SELECT COUNT(*) FROM sales").rows == ((3,),)


def test_a_query_that_is_right_on_both_databases_passes(sql_task: Path) -> None:
    query = "select city, sum(amount) as total from sales group by city order by total desc"

    assert judging(sql_task, query) == Verdict(True, "")


def test_a_query_that_holds_the_answer_and_not_the_way_to_it_fails(sql_task: Path) -> None:
    query = "SELECT 'Brisbane', 9 UNION ALL SELECT 'Perth', 3"

    assert judging(sql_task, query) == Verdict(
        False,
        "on changed.sqlite the query returns 2 rows, and not the 3 that it should in that order",
    )


def test_rows_in_another_order_fail_where_the_order_was_asked_for(sql_task: Path) -> None:
    query = "SELECT city, SUM(amount) FROM sales GROUP BY city ORDER BY city"

    assert not judging(sql_task, query).passed


def test_rows_in_any_order_pass_where_no_order_was_asked_for(tmp_path: Path) -> None:
    directory = task(tmp_path, "sql", {"reference/answer.sql": "SELECT city FROM sales"})
    databases(directory, [("A", 1), ("B", 2)], [("C", 1), ("D", 2)])

    assert judging(directory, "SELECT city FROM sales ORDER BY city DESC").passed


def test_a_query_that_fails_says_where_and_why(sql_task: Path) -> None:
    assert judging(sql_task, "SELECT * FROM orders") == Verdict(
        False, "the query fails on database.sqlite: no such table: orders"
    )


@pytest.mark.parametrize(
    "calls",
    [(), (Call("submit_sql", {"sql": "SELECT 1"}, failed=True),), (Call("submit_sql", {}),)],
    ids=["none", "failed", "without a query"],
)
def test_without_a_query_that_went_through_nothing_passes(
    sql_task: Path, calls: tuple[Call, ...]
) -> None:
    assert judging(sql_task, calls=calls) == Verdict(False, "no query was submitted")


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


def test_a_name_looked_up_with_its_brackets_counts(compute_task: Path) -> None:
    calls = (*looked("[rate]", " hours ", "[ days ]"), answered(1260))

    assert compute(load(compute_task), calls) == Verdict(True, "")


def test_the_last_answer_counts(compute_task: Path) -> None:
    calls = (*looked("rate", "hours", "days"), answered(1260), answered(7))

    assert not compute(load(compute_task), calls).passed
    assert compute(load(compute_task), looked("rate")) == Verdict(False, "no answer was submitted")


def test_the_reference_of_a_compute_task_looks_up_every_number_and_answers(
    compute_task: Path, tmp_path: Path
) -> None:
    assert check(load(compute_task), tmp_path / "scratch") == []
