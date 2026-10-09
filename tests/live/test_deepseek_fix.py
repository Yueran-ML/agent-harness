"""The real DeepSeek service mends a small repository. Skipped unless DEEPSEEK_API_KEY is set.

The repository is the one of ``tests/apps/test_fix_a_repository.py``: a
calculator with a mistake and a check that fails because of it. The run costs
a few thousand tokens. Run it with::

    uv run pytest tests/live -m live
"""

import json
import os
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from support.scenario import ALLOWED, TASK, check, fresh

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(300),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A copy of the repository, and a data directory that is thrown away."""
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    return fresh(tmp_path / "repository", monkeypatch)


def run(*args: str, capsys: pytest.CaptureFixture[str]) -> tuple[int, list[dict[str, Any]], str]:
    code = main(["run", *args, "--json"])
    captured = capsys.readouterr()
    return code, [json.loads(line) for line in captured.out.splitlines()], captured.err


def test_a_repository_is_mended_and_checked(
    repository: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert check(repository)[0] == 1

    code, lines, errors = run(TASK, "--cwd", str(repository), *ALLOWED, capsys=capsys)

    assert errors == ""
    assert code == 0
    assert lines[-1]["reason"] == "completed"
    # Whatever way the model took, the repository passes its check afterwards,
    assert check(repository) == (0, "ok\n")
    # and the model has seen that for itself, with the one command that the rule allows.
    calls = {line["id"]: line for line in lines if line["type"] == "tool_call"}
    checked = [
        line
        for line in lines
        if line["type"] == "tool_result"
        and calls[line["id"]]["name"] == shell.NAME
        and line["error"] is None
    ]
    assert checked
    assert checked[-1]["text"] == "ok\n[exit code 0]"


def test_without_the_rule_the_model_is_told_that_the_check_did_not_run(
    repository: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines, _ = run(TASK, "--cwd", str(repository), capsys=capsys)

    calls = {line["id"]: line for line in lines if line["type"] == "tool_call"}
    commands = [
        line
        for line in lines
        if line["type"] == "tool_result" and calls[line["id"]]["name"] == shell.NAME
    ]
    assert code == 0
    assert lines[-1]["reason"] == "completed"
    # No command ran: nobody was there to say yes.
    assert all(line["error"] == "APPROVAL_UNAVAILABLE" for line in commands)
