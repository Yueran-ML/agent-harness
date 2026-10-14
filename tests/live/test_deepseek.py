"""The real DeepSeek service. These tests are skipped unless DEEPSEEK_API_KEY is set.

They cost a few hundred tokens each. Run them with::

    uv run pytest tests/live -m live
"""

import json
import os
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(180),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

QUESTION = (
    "What is the codename of release 0.1? It is written in notes.txt. "
    "Answer with the codename only."
)


@pytest.fixture
def work(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A working directory with one file, and a data directory that is thrown away."""
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(tmp_path / "agents"))
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "notes.txt").write_text(
        'Release notes\n\nThe codename of release 0.1 is "Halyard".\n', encoding="utf-8"
    )
    return directory


def run(*args: str, capsys: pytest.CaptureFixture[str]) -> tuple[int, list[dict[str, Any]], str]:
    code = main(["run", *args, "--json"])
    captured = capsys.readouterr()
    return code, [json.loads(line) for line in captured.out.splitlines()], captured.err


def test_a_task_that_needs_a_tool_is_completed(
    work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines, errors = run(QUESTION, "--cwd", str(work), capsys=capsys)

    assert errors == ""
    assert code == 0
    calls = [line for line in lines if line["type"] == "tool_call"]
    results = [line for line in lines if line["type"] == "tool_result"]
    final = lines[-1]
    assert calls
    assert calls[0]["name"] == "read"
    assert json.loads(calls[0]["arguments"])["path"].endswith("notes.txt")
    assert results[0]["error"] is None
    assert "Halyard" in results[0]["text"]
    assert final["type"] == "final"
    assert final["reason"] == "completed"
    assert "halyard" in final["text"].lower()
    assert final["calls"] >= 2
    assert final["usage"]["output_tokens"] > 0


def test_a_question_without_tools_is_answered(
    work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines, errors = run(
        "Reply with the single word: ready", "--cwd", str(work), capsys=capsys
    )

    assert errors == ""
    assert code == 0
    assert "ready" in lines[-1]["text"].lower()
    assert lines[-1]["calls"] == 1


def test_a_key_that_is_not_valid_is_an_authentication_failure(
    work: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", "sk-this-key-is-not-valid")

    code, lines, _ = run("Hello.", "--cwd", str(work), capsys=capsys)

    assert code == 1
    assert lines[-1]["reason"] == "error"
    assert lines[-1]["detail"].startswith("AUTH: ")
    assert "sk-this-key-is-not-valid" not in json.dumps(lines)
