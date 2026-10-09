"""The scenario that stage 3 ends with: a small repository that does not pass its own check.

The repository holds a calculator with a mistake, a check that fails because
of it, and a note for agents that says how the check is run. The same
repository and the same task are given to a scripted model, to the real
service, and to the replay of a run that was recorded.
"""

import subprocess
import sys
from pathlib import Path

import pytest

from support.neutral import copy_of

__all__ = ["ALLOWED", "CHECK", "REPOSITORY", "TASK", "check", "fresh"]

REPOSITORY = Path(__file__).parent.parent / "fixtures" / "recorded" / "deepseek-fix" / "work"
"""The repository as it is before a run."""

TASK = (
    "check.py fails. Find the mistake in calc.py and fix it, "
    "and make sure that `python check.py` prints ok."
)
CHECK = "python check.py"
ALLOWED = ("--allow", f"run({CHECK})")
"""What the command line says to let the agent run the check, and nothing else."""


def fresh(to: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Copy the repository for a run in which running Python leaves nothing behind."""
    monkeypatch.setenv("PYTHONDONTWRITEBYTECODE", "1")
    return copy_of(REPOSITORY, to)


def check(repository: Path) -> tuple[int, str]:
    """Run the check of a repository the way a user would, and return its code and output."""
    done = subprocess.run(
        [sys.executable, "check.py"],
        cwd=repository,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    return done.returncode, done.stdout
