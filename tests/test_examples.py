"""The examples run, and the README shows them as they are and points at what is there.

The README is there in three languages. The English one is held to the repository, and the
other two are held to the English one.
"""

import re
import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).parent.parent
READMES = ("README.md", "README.zh-CN.md", "README.ja.md")
TRANSLATIONS = READMES[1:]
# The pictures of the README are left out of the source distribution.
PICTURES = "docs/assets/"
FENCE = "`" * 3
FENCED = re.compile(rf"^{FENCE}.*?^{FENCE}$", flags=re.DOTALL | re.MULTILINE)
# A letter of Chinese or of Japanese. Signs such as a full stop are not among them.
LETTER = "[\u4e00-\u9fff\u3040-\u309f\u30a0-\u30ff]"


def _text(name: str) -> str:
    return (ROOT / name).read_text(encoding="utf-8")


def _prose(name: str) -> str:
    """Return a README without its fenced blocks, which are shown as they stand."""
    return FENCED.sub("", _text(name))


def _blocks(name: str) -> list[str]:
    """Return the fenced blocks of a README, in their order."""
    return FENCED.findall(_text(name))


def _bare(drawing: str) -> str:
    """Return a drawing without the words in it."""
    return re.sub(r'"[^"]*"', '""', drawing)


def _targets(prose: str) -> list[str]:
    """Return what a text points at, in Markdown and in HTML."""
    return re.findall(r"\]\(([^)\s]+)\)", prose) + re.findall(r'(?:href|src)="([^"]+)"', prose)


def _elsewhere(name: str) -> list[str]:
    """Return what a README points at, beside its own headings and the other languages."""
    targets = _targets(_prose(name))
    return [target for target in targets if not target.startswith("#") and target not in READMES]


def _numbers(name: str) -> list[str]:
    """Return every number that a README gives in digits outside its blocks."""
    return sorted(re.findall(r"[0-9][0-9.,]*[0-9]|[0-9]", _prose(name)))


def _shape(name: str) -> dict[str, object]:
    """Return what a README is made of, without its words."""
    prose = _prose(name)
    lines = prose.splitlines()
    return {
        "headings": [len(line) - len(line.lstrip("#")) for line in lines if line.startswith("#")],
        "items of lists": sum(line.startswith("- ") for line in lines),
        "rows of tables": sum(line.startswith("|") for line in lines),
        "paragraphs": len([part for part in re.split(r"\n\s*\n", prose) if part.strip()]),
        "tags": sorted(re.findall(r"</?[a-z][a-z0-9]*", prose)),
    }


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


@pytest.mark.parametrize("name", READMES)
def test_the_readme_shows_the_kernel_demo_unchanged(name: str) -> None:
    demo = (ROOT / "examples" / "kernel_demo.py").read_text(encoding="utf-8")

    assert demo in _text(name)


@pytest.mark.parametrize("name", READMES)
def test_the_readme_shows_the_screen_of_a_conversation_as_a_test_left_it(name: str) -> None:
    snapshot = ROOT / "tests" / "apps" / "cli" / "snapshots" / "the_screen_of_a_session.txt"
    screen = snapshot.read_text(encoding="utf-8")

    assert f"{FENCE}text\n{screen}{FENCE}\n" in _text(name)


@pytest.mark.parametrize("name", READMES)
def test_the_files_that_the_readme_points_at_are_there(name: str) -> None:
    elsewhere = ("http://", "https://", "#")
    files = [target for target in _targets(_prose(name)) if not target.startswith(elsewhere)]
    if not (ROOT / PICTURES).is_dir():
        files = [file for file in files if not file.startswith(PICTURES)]

    assert files
    assert [file for file in files if not _spelled(file.partition("#")[0])] == []


def test_a_file_is_only_there_under_the_name_it_has_on_disk() -> None:
    assert _spelled("tests/test_examples.py")
    assert not _spelled("tests/Test_examples.py")
    assert not _spelled("tests/test_examples.py/more")
    assert not _spelled("tests/no_such_file.py")


@pytest.mark.parametrize("name", READMES)
def test_the_links_within_the_readme_lead_to_its_headings(name: str) -> None:
    prose = _prose(name)
    headings = re.findall(r"^#{1,6} (.+)$", prose, flags=re.MULTILINE)
    # The name a code host gives a heading: small letters, a hyphen for a space, no other sign.
    names = {re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-") for heading in headings}
    anchors = [target[1:] for target in _targets(prose) if target.startswith("#")]

    assert anchors
    assert [anchor for anchor in anchors if anchor not in names] == []


@pytest.mark.parametrize("name", READMES)
def test_the_readme_leads_to_the_other_two_languages(name: str) -> None:
    others = [target for target in _targets(_prose(name)) if target in READMES]

    assert sorted(others) == sorted(set(READMES) - {name})


@pytest.mark.parametrize("name", TRANSLATIONS)
def test_a_translation_shows_the_blocks_of_the_readme(name: str) -> None:
    ours, theirs = _blocks("README.md"), _blocks(name)

    assert len(theirs) == len(ours)
    for one, other in zip(ours, theirs, strict=True):
        if one.startswith(FENCE + "mermaid"):
            # A drawing has words in it, and they are translated. What it draws stays.
            assert _bare(other) == _bare(one)
            assert other != one
        else:
            assert other == one


def test_a_drawing_without_its_words_is_what_it_draws() -> None:
    drawing = 'agent["Agent loop"] -- "calls" --> tools[("Tools")]'

    assert _bare(drawing) == 'agent[""] -- "" --> tools[("")]'


@pytest.mark.parametrize("name", TRANSLATIONS)
def test_a_translation_has_the_shape_of_the_readme(name: str) -> None:
    assert _shape(name) == _shape("README.md")


@pytest.mark.parametrize("name", TRANSLATIONS)
def test_a_translation_points_at_what_the_readme_points_at(name: str) -> None:
    assert _elsewhere(name) == _elsewhere("README.md")


@pytest.mark.parametrize("name", TRANSLATIONS)
def test_a_translation_gives_the_numbers_of_the_readme(name: str) -> None:
    assert _numbers(name) == _numbers("README.md")


@pytest.mark.parametrize("name", TRANSLATIONS)
def test_a_paragraph_of_a_translation_is_one_line(name: str) -> None:
    # The end of a line is shown as a space, which these languages do not put between letters.
    assert re.findall(f"{LETTER}\n{LETTER}", _prose(name)) == []
