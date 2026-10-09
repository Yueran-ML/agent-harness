"""Where stage 3 ends: a run without a user interface reads, changes and checks a small repository.

The repository holds a calculator with a mistake, a check that fails because
of it, and a note for agents that says how the check is run. The model here
is scripted. It does what a model would do: runs the check, reads the file,
changes it, runs the check again, and answers. The same repository is given
to the real service in ``tests/live``, and a run of that is kept under
``tests/fixtures/recorded/deepseek-fix``.
"""

from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from columbina.llm import render_requests
from columbina.llm.adapters import scripted
from support.launch import lines, profile
from support.neutral import neutral_requests
from support.scenario import ALLOWED, CHECK, TASK, check, fresh
from support.snapshot import stored

SNAPSHOT = Path(__file__).parent / "snapshots" / "fix_a_repository.txt"
ANSWER = "mean() divided by one too few. I fixed calc.py, and the check prints ok now."
WRONG = "mean([1.0, 2.0, 3.0]) is 3.0, expected 2.0"
NOBODY_TO_ASK = (
    "The call was not allowed: No rule allows this command, and nothing holds it here. "
    "Nobody is here who could be asked, so it was not done."
)


def call(name: str, **arguments: object) -> dict[str, Any]:
    return {"calls": [{"name": name, "arguments": arguments}]}


WHAT_A_MODEL_DOES: tuple[dict[str, Any], ...] = (
    {"text": "I will run the check first.", **call(shell.NAME, command=CHECK)},
    call("read", path="calc.py"),
    call("edit", path="calc.py", old="(len(values) - 1)", new="len(values)"),
    call(shell.NAME, command=CHECK),
    {"text": ANSWER},
)


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A copy of the repository as it is before a run."""
    return fresh(tmp_path / "repository", monkeypatch)


def test_the_repository_does_not_pass_its_check_as_it_is(repository: Path) -> None:
    assert check(repository) == (1, f"{WRONG}\n")


def test_a_run_reads_changes_and_checks_the_repository(
    home: Path, repository: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen = profile(tmp_path, *WHAT_A_MODEL_DOES)

    code = main(["run", TASK, "--profile", chosen, "--cwd", str(repository), *ALLOWED])

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == f"{ANSWER}\n"
    assert captured.err == ""
    assert check(repository) == (0, "ok\n")
    assert sorted(path.name for path in repository.iterdir()) == [
        "AGENTS.md",
        "calc.py",
        "check.py",
    ]
    assert "return sum(values) / len(values)\n" in (repository / "calc.py").read_text("utf-8")


def test_every_step_of_the_run_can_be_followed(
    home: Path, repository: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen = profile(tmp_path, *WHAT_A_MODEL_DOES)

    main(["run", TASK, "--profile", chosen, "--cwd", str(repository), "--json", *ALLOWED])

    reported = lines(capsys.readouterr().out)
    results = [(line["text"], line["error"]) for line in reported if line["type"] == "tool_result"]
    assert results == [
        (f"{WRONG}\n[exit code 1]", None),
        (
            '     1\t"""A small calculator."""\n'
            "     2\t\n"
            "     3\t\n"
            "     4\tdef mean(values: list[float]) -> float:\n"
            '     5\t    """Return the mean of the values."""\n'
            "     6\t    return sum(values) / (len(values) - 1)",
            None,
        ),
        ("Replaced 1 occurrence in calc.py.", None),
        ("ok\n[exit code 0]", None),
    ]
    assert reported[-1]["reason"] == "completed"
    assert reported[-1]["text"] == ANSWER


def test_without_the_rule_the_check_is_not_run_by_the_agent(
    home: Path, repository: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen = profile(tmp_path, *WHAT_A_MODEL_DOES)

    code = main(["run", TASK, "--profile", chosen, "--cwd", str(repository), "--json"])

    reported = lines(capsys.readouterr().out)
    results = [(line["text"], line["error"]) for line in reported if line["type"] == "tool_result"]
    assert code == 0
    assert [error for _, error in results] == [
        "APPROVAL_UNAVAILABLE",
        None,
        None,
        "APPROVAL_UNAVAILABLE",
    ]
    assert results[0][0] == NOBODY_TO_ASK
    # The files could be changed all the same: what a rule holds back is the command.
    assert check(repository) == (0, "ok\n")


def test_a_rule_for_one_command_allows_no_other(
    home: Path, repository: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    others = [
        call(shell.NAME, command="python check.py --verbose"),
        call(shell.NAME, command="python check.py; python calc.py"),
        call(shell.NAME, command="python calc.py"),
        {"text": "Done."},
    ]

    chosen = profile(tmp_path, *others)

    main(["run", TASK, "--profile", chosen, "--cwd", str(repository), "--json", *ALLOWED])

    reported = lines(capsys.readouterr().out)
    errors = [line["error"] for line in reported if line["type"] == "tool_result"]
    assert errors == ["APPROVAL_UNAVAILABLE", "APPROVAL_UNAVAILABLE", "APPROVAL_UNAVAILABLE"]


def test_what_the_model_is_sent_in_such_a_run(
    home: Path,
    repository: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    models: list[scripted.Scripted] = []

    class Watched(scripted.Scripted):
        def __init__(self, *args: Any, **options: Any) -> None:
            super().__init__(*args, **options)
            models.append(self)

    monkeypatch.setattr(scripted, "Scripted", Watched)
    chosen = profile(tmp_path, *WHAT_A_MODEL_DOES)

    main(["run", TASK, "--profile", chosen, "--cwd", str(repository), *ALLOWED])

    # The instructions of the repository, the facts about the run, the task, and after
    # that every call with its result, in a form that is the same on every platform.
    sent = neutral_requests(models[0].requests, repository, shell.NAME)
    seen = render_requests(sent) + "\n"
    assert seen == stored(SNAPSHOT, seen)
