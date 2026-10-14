"""How the work of an attempt is judged, for each category of task.

A judgement looks at the working directory as the attempt left it, and at the
calls of tools that the attempt made. It says whether the attempt passed, and
in a few lines why.

code
    ``checks/`` holds tests of ``unittest``. They run on the working
    directory and must pass. In a run they run in a container, since what
    they run is code that a model wrote; in the check of a task, where the
    code is the reference, they run with this Python.
files
    ``checks/expected.json`` says what the working directory must hold:
    ``exists`` and ``absent`` name paths, ``equal`` maps a path to its
    whole text, ``json`` maps a path to values that must be in it under
    dotted keys, and ``lines`` maps a path to the lines it holds, in any
    order. Line ends count for nothing.
sql
    The last query that was submitted runs on the two databases that the
    statements in ``private/`` make, and must return on each what
    ``reference/answer.sql`` returns there.
compute
    ``reference/answer.json`` holds the answer and the names of the numbers
    that have to be looked up. The last answer that was submitted must be
    it, and each of those names must have been looked up.
"""

import json
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from bench import database
from bench.task import Loaded

__all__ = [
    "LOOKUP",
    "SUBMIT_ANSWER",
    "SUBMIT_SQL",
    "Call",
    "Verdict",
    "code",
    "code_argv",
    "compute",
    "files",
    "last",
    "named",
    "sql",
]

SUBMIT_SQL = "submit_sql"
"""The tool that an attempt at a task of the category ``sql`` submits its query with."""

LOOKUP = "lookup"
"""The tool that an attempt at a task of the category ``compute`` looks a number up with."""

SUBMIT_ANSWER = "submit_answer"
"""The tool that an attempt at a task of the category ``compute`` submits its answer with."""

_TESTS_SECONDS = 300
"""How long the hidden tests of a task may run."""

_SAID_LINES = 12
"""How many lines of what the hidden tests printed a verdict keeps, from the end."""


@dataclass(frozen=True)
class Call:
    """A call of a tool that an attempt made.

    Attributes:
        name: The name of the tool.
        arguments: What it was called with.
        failed: Whether the tool said that it failed.
    """

    name: str
    arguments: dict[str, Any]
    failed: bool = False


@dataclass(frozen=True)
class Verdict:
    """How an attempt was judged.

    Attributes:
        passed: Whether it did what the task asked.
        said: Why, in a few lines.
    """

    passed: bool
    said: str = ""


def named(name: object) -> object:
    """Return the name of a number as a call may write it, without spaces and brackets around it."""
    return name.strip().strip("[]").strip() if isinstance(name, str) else name


def last(calls: tuple[Call, ...], name: str) -> Call | None:
    """Return the last call of a tool by this name that went through."""
    return next((call for call in reversed(calls) if call.name == name and not call.failed), None)


def code_argv(checks: Path, workspace: Path, image: str) -> list[str]:
    """Return how the hidden tests are run in a container of an image.

    The container has no network. The working directory is mounted where the
    tests run, and the tests are mounted read-only beside it.
    """
    return [
        "docker", "run", "--rm", "--network", "none",
        "-v", f"{workspace}:/work",
        "-v", f"{checks}:/checks:ro",
        "-w", "/work",
        image, "python", "-m", "unittest", "discover", "-s", "/checks", "-t", "/checks",
    ]  # fmt: skip


def code(loaded: Loaded, workspace: Path, *, container: str | None) -> Verdict:
    """Run the hidden tests of a task on a working directory.

    Args:
        loaded: The task.
        workspace: The working directory, as the attempt left it.
        container: The image to run the tests in, or ``None`` to run them
            with this Python.
    """
    checks = loaded.directory / "checks"
    if not any(checks.glob("test*.py")):
        return Verdict(False, "the task has no hidden tests")
    if container is None:
        argv = [sys.executable, "-m", "unittest", "discover", "-s", str(checks), "-t", str(checks)]
    else:
        argv = code_argv(checks, workspace, container)
    try:
        done = subprocess.run(
            argv,
            cwd=workspace,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=_TESTS_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return Verdict(False, f"the hidden tests ran for more than {_TESTS_SECONDS} seconds")
    said = "\n".join((done.stdout + done.stderr).strip().splitlines()[-_SAID_LINES:])
    return Verdict(done.returncode == 0, said)


def files(loaded: Loaded, workspace: Path) -> Verdict:
    """Compare a working directory with what ``checks/expected.json`` says that it should hold."""
    expected = json.loads(
        (loaded.directory / "checks" / "expected.json").read_text(encoding="utf-8")
    )
    wrong = [
        f"{name} is not there" for name in expected.get("exists", ()) if not _there(workspace, name)
    ]
    wrong.extend(
        f"{name} is still there" for name in expected.get("absent", ()) if _there(workspace, name)
    )
    for name, text in expected.get("equal", {}).items():
        if _read(workspace, name) != text.replace("\r\n", "\n"):
            wrong.append(f"{name} does not hold what it should")
    for name, values in expected.get("json", {}).items():
        try:
            held = json.loads(_read(workspace, name) or "")
        except json.JSONDecodeError:
            wrong.append(f"{name} is no JSON")
            continue
        wrong.extend(
            f"{name}: {dotted} is not {value!r}"
            for dotted, value in values.items()
            if _at(held, dotted) != value
        )
    for name, lines in expected.get("lines", {}).items():
        if Counter((_read(workspace, name) or "").splitlines()) != Counter(lines):
            wrong.append(f"{name} does not hold the lines it should")
    return Verdict(not wrong, "\n".join(wrong))


def _there(root: Path, name: str) -> bool:
    """Tell whether a path is there as it is written, each of its names spelled the same way.

    A disk that takes no notice of case finds ``inbox/csv`` in ``inbox/CSV``,
    and a tree is judged the same on every system. So every name on the way
    is looked for among the names of its directory.
    """
    place = root
    for part in Path(name).parts:
        try:
            if part not in {entry.name for entry in place.iterdir()}:
                return False
        except OSError:
            return False
        place = place / part
    return True


def sql(loaded: Loaded, calls: tuple[Call, ...], databases: Path) -> Verdict:
    """Judge the query that an attempt submitted, on the database and on a changed copy.

    It must return what the query of the reference returns, on both. A query
    that is right on the first alone holds the answer, and not the way to it.

    Args:
        loaded: The task.
        calls: The calls of tools that the attempt made.
        databases: The directory that the two databases of the task were
            made in, see :func:`bench.database.built`.
    """
    submitted = last(calls, SUBMIT_SQL)
    if submitted is None or not isinstance(submitted.arguments.get("sql"), str):
        return Verdict(False, "no query was submitted")
    query = cast(str, submitted.arguments["sql"])
    reference = (loaded.directory / "reference" / "answer.sql").read_text(encoding="utf-8")
    in_order = database.ordered(reference)
    for name in database.DATABASES:
        file = databases / name
        want = database.read(file, reference)
        try:
            got = database.read(file, query)
        except database.QueryError as error:
            return Verdict(False, f"the query fails on {name}: {error}")
        if not database.same(want, got, in_order=in_order):
            return Verdict(
                False,
                f"on {name} the query returns {len(got.rows)} rows, and not the "
                f"{len(want.rows)} that it should{' in that order' if in_order else ''}",
            )
    return Verdict(True, "")


def compute(loaded: Loaded, calls: tuple[Call, ...]) -> Verdict:
    """Judge the answer that an attempt submitted, and whether it looked up every number."""
    answer = json.loads(
        (loaded.directory / "reference" / "answer.json").read_text(encoding="utf-8")
    )
    submitted = last(calls, SUBMIT_ANSWER)
    if submitted is None:
        return Verdict(False, "no answer was submitted")
    looked = {
        named(call.arguments.get("name"))
        for call in calls
        if call.name == LOOKUP and not call.failed
    }
    missing = [name for name in answer["lookups"] if name not in looked]
    value = submitted.arguments.get("value")
    if value != answer["value"]:
        return Verdict(False, f"the answer was {value!r}, and it is {answer['value']}")
    if missing:
        return Verdict(False, f"the answer is right, and {', '.join(missing)} was not looked up")
    return Verdict(True, "")


def _read(root: Path, name: str) -> str | None:
    """Return the text of a file as it is named, with its line ends made plain, or ``None``."""
    if not _there(root, name):
        return None
    try:
        return (root / name).read_text(encoding="utf-8").replace("\r\n", "\n")
    except (OSError, UnicodeDecodeError):
        return None


_MISSING = object()


def _at(held: object, dotted: str) -> object:
    """Return what lies under dotted keys in a value of JSON, or a mark that nothing does."""
    for key in dotted.split("."):
        if not isinstance(held, dict) or key not in held:
            return _MISSING
        held = cast(dict[str, object], held)[key]
    return held
