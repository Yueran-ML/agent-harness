"""The examples run, and the README shows them as they are."""

import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).parent.parent


def test_the_kernel_demo_prints_one_tick(capsys: pytest.CaptureFixture[str]) -> None:
    runpy.run_path(str(ROOT / "examples" / "kernel_demo.py"), run_name="__main__")

    assert capsys.readouterr().out == "tick 101\n"


def test_the_readme_shows_the_kernel_demo_unchanged() -> None:
    demo = (ROOT / "examples" / "kernel_demo.py").read_text(encoding="utf-8")
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    assert demo in readme
