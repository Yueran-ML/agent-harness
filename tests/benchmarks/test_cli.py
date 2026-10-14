"""The commands of the benchmark: ``bench.cli`` and ``python -m bench``."""

import os
import subprocess
import sys
from pathlib import Path

import pytest

from bench.cli import USAGE, main

REPO = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("argv", [[], ["race"]], ids=["nothing", "no such command"])
def test_without_a_command_it_says_how_it_is_called(
    argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(argv) == 2
    assert capsys.readouterr().err == USAGE + "\n"


def test_a_command_is_run_with_the_rest_of_the_arguments(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["generate", str(tmp_path)]) == 0
    assert main(["check", str(tmp_path / "compute")]) == 0
    assert capsys.readouterr().out.rstrip().endswith("30 tasks, all of them good")


def test_the_benchmark_runs_as_a_module(tmp_path: Path) -> None:
    done = subprocess.run(
        [sys.executable, "-m", "bench", "generate", str(tmp_path)],
        cwd=REPO,
        env={**os.environ, "PYTHONPATH": str(REPO)},
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )

    assert done.returncode == 0, done.stderr
    assert "tasks of SQL" in done.stdout
