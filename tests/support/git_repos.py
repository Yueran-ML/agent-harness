"""Repositories of git for the tests: made with the git of this machine, and none of its settings.

The settings of whoever runs the tests, such as the name of the first branch or a helper for
passwords, would change what git does. A test that uses :func:`apart` runs git, its own and
the one that a tool runs, with a file of settings of its own, and without those of the system.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

__all__ = ["GIT", "apart", "git", "needs_git"]

GIT = shutil.which("git")
needs_git = pytest.mark.skipif(GIT is None, reason="git is not installed here")


def apart(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Have every git that this test runs go by settings of the test alone."""
    settings = tmp_path / "gitconfig"
    settings.write_text(
        "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", str(settings))
    monkeypatch.setenv("GIT_CONFIG_NOSYSTEM", "1")
    monkeypatch.setenv("GIT_TERMINAL_PROMPT", "0")
    for variable in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_CONFIG_PARAMETERS"):
        monkeypatch.delenv(variable, raising=False)


def git(where: Path, *arguments: str) -> str:
    """Run git in a directory, and return what it wrote."""
    assert GIT is not None
    done = subprocess.run(
        [GIT, *arguments],
        cwd=where,
        env=dict(os.environ),
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
        stdin=subprocess.DEVNULL,
    )
    assert done.returncode == 0, done.stderr
    return done.stdout.strip()
