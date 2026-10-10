"""The glob tool: files by the pattern of their paths."""

import os
import sys
import time
from pathlib import Path
from typing import Any

import pytest

from columbina.builtin_tools import glob
from columbina.builtin_tools.glob import matcher
from columbina.fs import Access, Refuse, bound
from columbina.kernel import Context
from columbina.tools import Deny, Invocation, ToolFailure
from support.desk import Desk
from support.links import link_directory, unlink_directory

NOW = 1_700_000_000


def put(desk: Desk, *paths: str) -> None:
    """Make files on the desk. Each is a second older than the one before it."""
    for age, path in enumerate(paths):
        file = desk.work / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(path)
        os.utime(file, (NOW - age, NOW - age))


async def listed(desk: Desk, pattern: str, **more: Any) -> list[str]:
    outcome = await desk.call("glob", pattern=pattern, **more)
    assert outcome.error is None, outcome.text
    return outcome.text.splitlines()


# Patterns


@pytest.mark.parametrize(
    ("pattern", "path", "fits"),
    [
        ("*.py", "a.py", True),
        ("*.py", "src/pkg/a.py", True),
        ("*.py", "a.pyc", False),
        ("**/*.py", "a.py", True),
        ("**/*.py", "src/pkg/a.py", True),
        ("src/*.py", "src/a.py", True),
        ("src/*.py", "src/pkg/a.py", False),
        ("src/*.py", "a.py", False),
        ("src/**/test_*.py", "src/test_a.py", True),
        ("src/**/test_*.py", "src/x/y/test_a.py", True),
        ("src/**/test_*.py", "src/x/y/a_test.py", False),
        ("**/docs/**/*.md", "docs/a.md", True),
        ("**/docs/**/*.md", "x/docs/y/z/a.md", True),
        ("**/docs/**/*.md", "x/doc/a.md", False),
        ("file?.txt", "file1.txt", True),
        ("file?.txt", "file12.txt", False),
        ("file[12].txt", "file2.txt", True),
        ("file[12].txt", "file3.txt", False),
        ("file[!1].txt", "file1.txt", False),
        ("file[!1].txt", "file2.txt", True),
        ("**", "a/b/c.txt", True),
        ("src/**", "src/a/b.txt", True),
        ("src/**", "lib/a.txt", False),
        ("./src//a.py", "src/a.py", True),
        ("src\\*.py", "src/a.py", True),
        ("a/**/b", "a/b", True),
        ("a/**/**/b", "a/x/b", True),
    ],
)
def test_a_pattern_fits_the_paths_it_means(pattern: str, path: str, fits: bool) -> None:
    assert matcher(pattern)(tuple(path.split("/"))) is fits


@pytest.mark.parametrize("pattern", ["/etc/*.conf", "C:/Windows/*", "../*.py", "src/../../x", "./"])
def test_a_pattern_that_does_not_stay_below_the_directory_is_refused(pattern: str) -> None:
    with pytest.raises(ToolFailure) as refused:
        matcher(pattern)

    assert refused.value.code == "INVALID_PATTERN"


def test_upper_and_lower_case_count_where_the_disk_tells_them_apart() -> None:
    assert matcher("*.PY")(("a.py",)) is glob.IGNORES_CASE
    assert matcher("SRC/*.py")(("src", "a.py")) is glob.IGNORES_CASE


def test_a_pattern_with_many_stars_does_not_take_long_on_a_long_name() -> None:
    matches = matcher("*a*a*a*a*a*a*a*a*a*a*a*a*b")
    began = time.monotonic()

    assert not matches(("a" * 250,))
    assert not matches(tuple(["a" * 50] * 40))
    assert time.monotonic() - began < 2


def test_a_pattern_of_many_parts_and_a_path_of_many_names_are_matched_all_the_same() -> None:
    depth = sys.getrecursionlimit() * 2

    assert matcher("**/" * depth + "*.py")(("a.py",))
    assert matcher("**/*.py")((*["d"] * depth, "a.py"))
    assert not matcher("**/*.py")((*["d"] * depth, "a.txt"))


# What the tool lists


async def test_the_files_that_fit_are_listed_with_the_newest_first(desk: Desk) -> None:
    put(desk, "src/new.py", "old.py", "src/pkg/older.py", "README.md")

    assert await listed(desk, "**/*.py") == ["src/new.py", "old.py", "src/pkg/older.py"]


async def test_files_of_the_same_age_are_listed_by_their_paths(desk: Desk) -> None:
    put(desk, "b.py", "a.py", "c.py")
    for name in ("a.py", "b.py", "c.py"):
        os.utime(desk.work / name, (NOW, NOW))

    assert await listed(desk, "*.py") == ["a.py", "b.py", "c.py"]


async def test_a_pattern_is_read_from_the_directory_that_is_searched(desk: Desk) -> None:
    put(desk, "src/a.py", "src/pkg/b.py", "lib/c.py")

    assert await listed(desk, "*.py", path="src") == ["src/a.py", "src/pkg/b.py"]
    assert await listed(desk, "pkg/*.py", path="src") == ["src/pkg/b.py"]
    assert await listed(desk, "src/*.py", path="src") == ["No file fits src/*.py in src."]


async def test_nothing_that_fits_is_said_in_words(desk: Desk) -> None:
    put(desk, "a.py")

    assert await listed(desk, "*.rs") == ["No file fits *.rs in .."]


async def test_only_the_hundred_newest_are_listed_and_the_rest_are_counted(desk: Desk) -> None:
    put(desk, *(f"f{number:03}.txt" for number in range(105)))

    lines = await listed(desk, "*.txt")

    assert lines[:100] == [f"f{number:03}.txt" for number in range(100)]
    assert lines[100:] == [
        "[These are the 100 files that were changed last, of 105 that fit. "
        "Narrow the pattern to see the others.]"
    ]


async def test_the_history_of_a_repository_and_links_are_not_listed(
    desk: Desk, tmp_path: Path
) -> None:
    put(desk, "a.py", ".git/hooks/b.py")
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "c.py").write_text("c")
    link_directory(desk.work / "out", outside)

    assert await listed(desk, "**/*.py") == ["a.py"]


# What the tool does not see


async def test_what_a_rule_keeps_from_the_tool_is_not_listed_and_is_counted(desk: Desk) -> None:
    def private(access: Access) -> Refuse | None:
        if access.tool == "glob" and access.where.relative.startswith("private"):
            return Refuse("That is private.", "DENIED")
        return None

    put(desk, "a.py", "private/b.py", "private/c.py")
    desk.fs.restrictions.add(desk.ctx, "private", private)

    assert await listed(desk, "**/*.py") == [
        "a.py",
        "[1 file or directory was not searched, because this tool may not see them.]",
    ]


async def test_several_things_that_were_kept_from_the_tool_are_counted_together(
    desk: Desk,
) -> None:
    def private(access: Access) -> Refuse | None:
        return Refuse("Private.", "DENIED") if access.where.relative.endswith(".key") else None

    put(desk, "a.py", "one.key", "sub/two.key")
    desk.fs.restrictions.add(desk.ctx, "private", private)

    assert await listed(desk, "*") == [
        "a.py",
        "[2 files and directories were not searched, because this tool may not see them.]",
    ]


async def test_a_search_that_looked_at_as_much_as_it_may_says_that_more_may_fit(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.py", "b.py", "c.py", "d.py")
    monkeypatch.setattr(glob, "MAX_ENTRIES", 3)

    lines = await listed(desk, "*.py")

    assert lines == [
        "a.py",
        "b.py",
        "c.py",
        "[The search stopped after 3 files and directories, so there may be more. "
        "Search a narrower directory.]",
    ]


def locked(monkeypatch: pytest.MonkeyPatch, *names: str) -> None:
    """Make the directories with the given names ones that cannot be listed."""
    really_list: Any = bound.listing

    def listing(path: Path, *limits: int) -> Any:
        if path.name in names:
            raise PermissionError(13, "Permission denied")
        return really_list(path, *limits)

    monkeypatch.setattr(bound, "listing", listing)


async def test_directories_that_could_not_be_listed_are_counted(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.py", "locked/b.py", "sealed/c.py")
    locked(monkeypatch, "locked", "sealed")

    assert await listed(desk, "**/*.py") == ["a.py", "[2 directories could not be listed.]"]


async def test_one_directory_that_could_not_be_listed_is_counted_as_one(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.py", "locked/b.py")
    locked(monkeypatch, "locked")

    assert await listed(desk, "**/*.py") == ["a.py", "[1 directory could not be listed.]"]


# Where the tool does not search


@pytest.mark.parametrize(
    ("path", "code"),
    [
        ("a.py", "NOT_A_DIRECTORY"),
        ("nowhere", "NOT_FOUND"),
        ("..", "OUTSIDE_WORKING_DIRECTORY"),
    ],
)
async def test_a_place_that_cannot_be_searched_is_an_error(
    desk: Desk, path: str, code: str
) -> None:
    put(desk, "a.py")

    outcome = await desk.call("glob", pattern="*.py", path=path)

    assert outcome.error is not None
    assert outcome.error.code == code


@pytest.mark.parametrize(
    ("pattern", "code"), [("../*.py", "INVALID_PATTERN"), ("", "INVALID_ARGS")]
)
async def test_a_pattern_that_cannot_be_used_is_an_error(
    desk: Desk, pattern: str, code: str
) -> None:
    outcome = await desk.call("glob", pattern=pattern)

    assert outcome.error is not None
    assert outcome.error.code == code


async def test_a_session_that_is_read_only_can_search(desk: Desk) -> None:
    put(desk, "a.py")
    desk.sandbox.set_mode(desk.session, "read-only")

    assert await listed(desk, "*.py") == ["a.py"]


async def test_a_directory_that_leads_elsewhere_than_when_the_call_was_checked_is_not_searched(
    desk: Desk,
) -> None:
    put(desk, "drafts/a.py", "final/b.py")
    link_directory(desk.work / "door", desk.work / "drafts")

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(desk.work / "door")
        link_directory(desk.work / "door", desk.work / "final")
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    outcome = await desk.call("glob", pattern="*.py", path="door")

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"


# What the checks and the model are told


async def test_a_search_says_which_directory_it_acts_on(desk: Desk) -> None:
    seen: list[tuple[str, str | None]] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append((invocation.tool.effect, invocation.target))
        return None

    put(desk, "src/a.py")
    desk.tools.guards.add(desk.ctx, "watching", watching)

    await desk.call("glob", pattern="*.py")
    await desk.call("glob", pattern="*.py", path="./src/")

    assert seen == [("read", "."), ("read", "src")]


def test_searches_may_run_beside_each_other(desk: Desk) -> None:
    assert desk.begin("glob", pattern="*.py").concurrency_safe


async def test_a_file_whose_name_holds_half_a_character_is_listed_with_a_sign_in_its_place(
    desk: Desk,
) -> None:
    # Windows takes any unit of UTF-16 in a name, and the others take any byte.
    half = chr(0xD800) if sys.platform == "win32" else chr(0xDC80)
    try:
        (desk.work / f"notes{half}.txt").write_text("x")
    except OSError:
        pytest.skip("this disk takes no such name")
    put(desk, "plain.txt")

    names = await listed(desk, "*.txt")

    # A text with half a character in it could not be written into the log of the session.
    assert sorted(names) == sorted(["plain.txt", f"notes{chr(0xFFFD)}.txt"])
    for name in names:
        name.encode("utf-8")
