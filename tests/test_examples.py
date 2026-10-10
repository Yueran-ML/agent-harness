"""The examples run, and the README shows them as they are and points at what is there."""

import re
import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).parent.parent
# The pictures of the README are left out of the source distribution.
PICTURES = "docs/assets/"


def _prose() -> str:
    """Return the README without its fenced blocks, which are shown as they stand."""
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    return re.sub(r"```.*?```", "", readme, flags=re.DOTALL)


def _targets(prose: str) -> list[str]:
    """Return what a text points at, in Markdown and in HTML."""
    return re.findall(r"\]\(([^)\s]+)\)", prose) + re.findall(r'(?:href|src)="([^"]+)"', prose)


def _spelled(path: str) -> bool:
    """Say whether a file of the repository is there under exactly this name.

    A system that ignores the case of letters finds a file under a name that a code host
    does not find it under.
    """
    at = ROOT
    for part in path.split("/"):
        if not at.is_dir() or part not in {entry.name for entry in at.iterdir()}:
            return False
        at = at / part
    return True


def test_the_kernel_demo_prints_one_tick(capsys: pytest.CaptureFixture[str]) -> None:
    runpy.run_path(str(ROOT / "examples" / "kernel_demo.py"), run_name="__main__")

    assert capsys.readouterr().out == "tick 101\n"


def test_the_readme_shows_the_kernel_demo_unchanged() -> None:
    demo = (ROOT / "examples" / "kernel_demo.py").read_text(encoding="utf-8")
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    assert demo in readme


def test_the_readme_shows_the_screen_of_a_conversation_as_a_test_left_it() -> None:
    snapshot = ROOT / "tests" / "apps" / "cli" / "snapshots" / "the_screen_of_a_session.txt"
    screen = snapshot.read_text(encoding="utf-8")
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    fence = "`" * 3
    assert f"{fence}text\n{screen}{fence}\n" in readme


def test_the_files_that_the_readme_points_at_are_there() -> None:
    elsewhere = ("http://", "https://", "#")
    files = [target for target in _targets(_prose()) if not target.startswith(elsewhere)]
    if not (ROOT / PICTURES).is_dir():
        files = [file for file in files if not file.startswith(PICTURES)]

    assert files
    assert [file for file in files if not _spelled(file.partition("#")[0])] == []


def test_a_file_is_only_there_under_the_name_it_has_on_disk() -> None:
    assert _spelled("tests/test_examples.py")
    assert not _spelled("tests/Test_examples.py")
    assert not _spelled("tests/test_examples.py/more")
    assert not _spelled("tests/no_such_file.py")


def test_the_links_within_the_readme_lead_to_its_headings() -> None:
    prose = _prose()
    headings = re.findall(r"^#{1,6} (.+)$", prose, flags=re.MULTILINE)
    # The name a code host gives a heading: small letters, a hyphen for a space, no other sign.
    names = {re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-") for heading in headings}
    anchors = [target[1:] for target in _targets(prose) if target.startswith("#")]

    assert anchors
    assert [anchor for anchor in anchors if anchor not in names] == []
