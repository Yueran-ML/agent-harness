"""The examples run, and the documents show them as they are and point at what is there.

There are two documents, each in three languages. The README says how to get the program
and how to start it, and the guide under docs/ says the rest. The English one of each is
held to the repository, and the other two are held to the English one.
"""

import json
import os
import posixpath
import re
import runpy
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

import pytest
import yaml

ROOT = Path(__file__).parent.parent
READMES = ("README.md", "README.zh-CN.md", "README.ja.md")
GUIDES = ("docs/guide.md", "docs/guide.zh-CN.md", "docs/guide.ja.md")
DOCUMENTS = READMES + GUIDES
# The English document, and a translation that is held to it.
TRANSLATIONS = [(family[0], other) for family in (READMES, GUIDES) for other in family[1:]]
# The pictures of the README are left out of the source distribution.
PICTURES = "docs/assets/"
ADDRESS = ("http://", "https://")
FENCE = "`" * 3
FENCED = re.compile(rf"^{FENCE}.*?^{FENCE}$", flags=re.DOTALL | re.MULTILINE)
# A letter of Chinese or of Japanese. Signs such as a full stop are not among them.
LETTER = "[\u4e00-\u9fff\u3040-\u309f\u30a0-\u30ff]"


def _text(name: str) -> str:
    return (ROOT / name).read_text(encoding="utf-8")


def _prose(name: str) -> str:
    """Return a document without its fenced blocks, which are shown as they stand."""
    return FENCED.sub("", _text(name))


def _blocks(name: str) -> list[str]:
    """Return the fenced blocks of a document, in their order."""
    return FENCED.findall(_text(name))


def _bare(drawing: str) -> str:
    """Return a drawing without the words in it."""
    return re.sub(r'"[^"]*"', '""', drawing)


def _targets(prose: str) -> list[str]:
    """Return what a text points at, in Markdown and in HTML."""
    return re.findall(r"\]\(([^)\s]+)\)", prose) + re.findall(r'(?:href|src)="([^"]+)"', prose)


def _family(name: str) -> tuple[str, ...]:
    """Return a document in all its languages."""
    return READMES if name in READMES else GUIDES


def _reached(name: str, target: str) -> str:
    """Return the file of the repository that a link in a document leads to."""
    path = target.partition("#")[0]
    return posixpath.normpath(posixpath.join(posixpath.dirname(name), path))


def _files(name: str) -> list[str]:
    """Return the files of the repository that a document points at."""
    targets = [target for target in _targets(_prose(name)) if not target.startswith("#")]
    return [_reached(name, target) for target in targets if not target.startswith(ADDRESS)]


def _elsewhere(name: str) -> list[str]:
    """Return what a document points at, beside its own headings and its other languages.

    A document in another language is named as the English one, so that what the three
    languages of a document point at can be compared.
    """
    found: list[str] = []
    for target in _targets(_prose(name)):
        if target.startswith(ADDRESS):
            found.append(target)
        elif not target.startswith("#") and _reached(name, target) not in _family(name):
            found.append(re.sub(r"\.(zh-CN|ja)\.md$", ".md", _reached(name, target)))
    return found


def _headings(name: str) -> set[str]:
    """Return the names that a code host gives the headings of a document."""
    headings = re.findall(r"^#{1,6} (.+)$", _prose(name), flags=re.MULTILINE)
    # Small letters, a hyphen for a space, and no other sign.
    return {re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-") for heading in headings}


def _numbers(name: str) -> list[str]:
    """Return every number that a document gives in digits outside its blocks."""
    return sorted(re.findall(r"[0-9][0-9.,]*[0-9]|[0-9]", _prose(name)))


def _shape(name: str) -> dict[str, object]:
    """Return what a document is made of, without its words."""
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


@pytest.mark.parametrize("name", GUIDES)
@pytest.mark.parametrize("example", ["kernel_demo.py", "word_count.py"])
def test_the_guide_shows_an_example_unchanged(example: str, name: str) -> None:
    code = (ROOT / "examples" / example).read_text(encoding="utf-8")

    assert f"{FENCE}python\n{code}{FENCE}\n" in _text(name)


def _ran(
    work: Path, plugins: list[dict[str, Any]], calls: list[dict[str, Any]], *more: str
) -> list[dict[str, Any]]:
    """Run a task in which a model makes these calls, with these plugins of the examples.

    Return the results of the calls, in their order.
    """
    replies = [*({"calls": [call]} for call in calls), {"text": "Done."}]
    model = {"provider": "deepseek", "replies": replies}
    patch = work.parent / "patch.yml"
    patch.write_text(
        yaml.safe_dump(
            [
                {"insert": plugins},
                {"id": "model", "plugin": "columbina.llm.adapters.scripted", "config": model},
            ]
        ),
        encoding="utf-8",
    )
    env = {name: value for name, value in os.environ.items() if not name.startswith("COLUMBINA_")}
    env.pop("DEEPSEEK_API_KEY", None)
    env |= {
        "COLUMBINA_HOME": str(work.parent / "home"),
        "COLUMBINA_AGENTS_HOME": str(work.parent / "agents"),
        "PYTHONPATH": str(ROOT / "examples"),
        "PYTHON_KEYRING_BACKEND": "keyring.backends.null.Keyring",
    }

    done = subprocess.run(
        [sys.executable, "-m", "columbina", "run", "--json", "--profile", "headless",
         "--patch", str(patch), "--cwd", str(work), *more, "Do it."],
        env=env, capture_output=True, text=True, encoding="utf-8", timeout=120, check=False,
    )  # fmt: skip

    assert done.returncode == 0, done.stderr
    lines = [json.loads(line) for line in done.stdout.splitlines() if line.strip()]
    return [line for line in lines if line["type"] == "tool_result"]


def test_the_smallest_plugin_of_a_tool_tells_the_time_in_thirty_lines(tmp_path: Path) -> None:
    (tmp_path / "work").mkdir()

    (told,) = _ran(tmp_path / "work", [{"id": "clock", "plugin": "clock"}], [{"name": "now"}])

    assert told["error"] is None
    assert re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\+00:00", told["text"])
    assert len((ROOT / "examples" / "clock.py").read_text(encoding="utf-8").splitlines()) <= 30


def test_the_plugin_example_counts_words_inside_the_working_directory_alone(
    tmp_path: Path,
) -> None:
    work = tmp_path / "work"
    work.mkdir()
    (work / "notes.txt").write_text("one two three\nfour\n", encoding="utf-8")
    (tmp_path / "outside.txt").write_text("not to be read\n", encoding="utf-8")
    calls = [
        {"name": "count_words", "arguments": {"path": "notes.txt"}},
        {"name": "count_words", "arguments": {"path": "../outside.txt"}},
    ]

    counted, outside = _ran(work, [{"id": "word-count", "plugin": "word_count"}], calls)

    assert counted["text"] == "notes.txt has 4 words."
    assert counted["error"] is None
    assert outside["error"] is not None
    assert "not to be read" not in outside["text"]


def test_the_guard_of_the_example_refuses_a_push_and_nothing_else(tmp_path: Path) -> None:
    (tmp_path / "work").mkdir()
    shell = "powershell" if sys.platform == "win32" else "bash"
    calls = [
        {"name": shell, "arguments": {"command": "git push origin main"}},
        {"name": shell, "arguments": {"command": "echo not-pushed"}},
    ]

    pushed, echoed = _ran(
        tmp_path / "work",
        [{"id": "no-push", "plugin": "no_push"}],
        calls,
        "--permission",
        "full-access",
        "--sandbox",
        "none",
    )

    assert pushed["error"] == "NO_PUSH"
    assert "left to the user" in pushed["text"]
    assert echoed["error"] is None
    assert "not-pushed" in echoed["text"]


@pytest.mark.parametrize("name", GUIDES)
def test_the_guide_shows_the_screen_of_a_conversation_as_a_test_left_it(name: str) -> None:
    snapshot = ROOT / "tests" / "apps" / "cli" / "snapshots" / "the_screen_of_a_session.txt"
    screen = snapshot.read_text(encoding="utf-8")

    assert f"{FENCE}text\n{screen}{FENCE}\n" in _text(name)


def test_the_readme_clones_the_repository_that_the_project_names() -> None:
    project = tomllib.loads(_text("pyproject.toml"))["project"]

    assert f"git clone {project['urls']['Repository']}.git\n" in _text("README.md")


@pytest.mark.parametrize("name", DOCUMENTS)
def test_the_files_that_a_document_points_at_are_there(name: str) -> None:
    files = _files(name)
    if not (ROOT / PICTURES).is_dir():
        files = [file for file in files if not file.startswith(PICTURES)]

    assert files
    assert [file for file in files if not _spelled(file)] == []


def test_a_file_is_only_there_under_the_name_it_has_on_disk() -> None:
    assert _spelled("tests/test_examples.py")
    assert not _spelled("tests/Test_examples.py")
    assert not _spelled("tests/test_examples.py/more")
    assert not _spelled("tests/no_such_file.py")
    assert not _spelled("../" + ROOT.name)


def test_a_link_is_followed_from_the_place_of_its_document() -> None:
    assert _reached("README.md", "docs/guide.md#development") == "docs/guide.md"
    assert _reached("docs/guide.ja.md", "../README.ja.md") == "README.ja.md"
    assert _reached("docs/guide.md", "plans/x.md") == "docs/plans/x.md"
    assert _reached("docs/guide.md", "../../outside") == "../outside"


@pytest.mark.parametrize("name", DOCUMENTS)
def test_a_link_to_a_heading_leads_to_one(name: str) -> None:
    astray: list[str] = []
    followed = 0
    for target in _targets(_prose(name)):
        path, sign, heading = target.partition("#")
        if sign and not target.startswith(ADDRESS):
            followed += 1
            if heading not in _headings(_reached(name, target) if path else name):
                astray.append(target)

    assert followed
    assert astray == []


@pytest.mark.parametrize("name", DOCUMENTS)
def test_a_document_leads_to_its_other_two_languages(name: str) -> None:
    family = _family(name)

    assert sorted(file for file in _files(name) if file in family) == sorted(set(family) - {name})


@pytest.mark.parametrize(("readme", "guide"), list(zip(READMES, GUIDES, strict=True)))
def test_a_readme_and_the_guide_in_its_language_lead_to_each_other(readme: str, guide: str) -> None:
    # More than once is fine. Another language is not.
    assert {file for file in _files(readme) if file in GUIDES} == {guide}
    assert {file for file in _files(guide) if file in READMES} == {readme}


@pytest.mark.parametrize(("english", "name"), TRANSLATIONS)
def test_a_translation_shows_the_blocks_of_the_english_text(english: str, name: str) -> None:
    ours, theirs = _blocks(english), _blocks(name)

    assert ours
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


@pytest.mark.parametrize(("english", "name"), TRANSLATIONS)
def test_a_translation_has_the_shape_of_the_english_text(english: str, name: str) -> None:
    assert _shape(name) == _shape(english)


@pytest.mark.parametrize(("english", "name"), TRANSLATIONS)
def test_a_translation_points_at_what_the_english_text_points_at(english: str, name: str) -> None:
    assert _elsewhere(english)
    assert _elsewhere(name) == _elsewhere(english)


@pytest.mark.parametrize(("english", "name"), TRANSLATIONS)
def test_a_translation_gives_the_numbers_of_the_english_text(english: str, name: str) -> None:
    assert _numbers(english)
    assert _numbers(name) == _numbers(english)


@pytest.mark.parametrize("name", [name for _english, name in TRANSLATIONS])
def test_a_paragraph_of_a_translation_is_one_line(name: str) -> None:
    # The end of a line is shown as a space, which these languages do not put between letters.
    assert re.findall(f"{LETTER}\n{LETTER}", _prose(name)) == []
