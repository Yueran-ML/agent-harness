"""The real DeepSeek service mends a small repository with its commands in a container.

Skipped unless DEEPSEEK_API_KEY is set, a Docker engine answers and the image
``python:3.12-slim`` is on this machine. The repository is the one of
``tests/live/test_deepseek_fix.py``. Here no rule allows the check: the
commands of the model run in a Linux container without a network, so nobody
has to be asked about them. The model is on Windows, macOS or Linux and is
told that its commands are not: it has to use the bash of the container.

The run costs a few thousand tokens. Run it with::

    uv run pytest tests/live -m live
"""

import json
import os
from pathlib import Path
from typing import Any

import pytest
import yaml

from columbina.apps.launcher import main
from support.engines import containers, real
from support.scenario import TASK, check, fresh

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(600),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

IMAGE = "python:3.12-slim"
"""An image with bash and with the Python that the check of the repository needs."""


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A copy of the repository, and a data directory that is thrown away."""
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(tmp_path / "agents"))
    return fresh(tmp_path / "repository", monkeypatch)


def test_a_repository_is_mended_with_commands_that_run_in_a_container(
    repository: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    program = real((IMAGE,))
    before = containers(program)
    config = {"provider": "docker", "docker": {"program": program, "image": IMAGE}}
    patch = tmp_path / "in-a-container.yml"
    patch.write_text(yaml.safe_dump([{"id": "sandbox", "config": config}]), encoding="utf-8")
    assert check(repository)[0] == 1

    code = main(["run", TASK, "--cwd", str(repository), "--patch", str(patch), "--json"])

    captured = capsys.readouterr()
    lines: list[dict[str, Any]] = [json.loads(line) for line in captured.out.splitlines()]
    assert captured.err == ""
    assert code == 0
    assert lines[-1]["reason"] == "completed"
    # Whatever way the model took, the repository passes its check afterwards, on this
    # machine as in the container,
    assert check(repository) == (0, "ok\n")
    # and the model has seen that for itself, with a command that nobody was asked about.
    calls = {line["id"]: line for line in lines if line["type"] == "tool_call"}
    ran = [
        line
        for line in lines
        if line["type"] == "tool_result" and calls[line["id"]]["name"] == "bash"
    ]
    assert ran
    assert all(line["error"] is None for line in ran)
    # It may have run the check in a line of its own making, so only the answer is looked at.
    assert any(line["text"].startswith("ok\n") for line in ran)
    # The tool of this machine was not offered, and nothing is left of the container.
    assert {call["name"] for call in calls.values()}.isdisjoint({"powershell"})
    assert containers(program) <= before
