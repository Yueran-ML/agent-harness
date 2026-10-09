"""The grep tool: the lines of files that a pattern matches."""

import asyncio
import os
import sys
import threading
import time
from collections import Counter
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.builtin_tools import grep
from columbina.fs import Access, Refuse, bound, locate
from columbina.fs.walk import Found
from columbina.kernel import Context
from columbina.tools import Deny, Invocation
from support.desk import Desk
from support.links import link_directory, unlink_directory
from support.processes import gone

MIB = 1024 * 1024
FFFD = chr(0xFFFD)

CUT = (
    "[The search stopped after 250 lines that match, which is as many as it shows. "
    "There may be more: narrow the pattern or the path.]"
)


def put(desk: Desk, path: str, content: str | bytes) -> None:
    """Make a file on the desk."""
    file = desk.work / path
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_bytes(content if isinstance(content, bytes) else content.encode())


async def lines(desk: Desk, pattern: str, **more: Any) -> list[str]:
    outcome = await desk.call("grep", pattern=pattern, **more)
    assert outcome.error is None, outcome.text
    return outcome.text.splitlines()


async def failed(desk: Desk, pattern: str, **more: Any) -> tuple[str, str]:
    """Search, and return the code and the text of the error that comes back."""
    outcome = await desk.call("grep", pattern=pattern, **more)
    assert outcome.error is not None, outcome.text
    return outcome.error.code, outcome.text


def numbered(count: int, text: str = "needle") -> str:
    """As many lines as asked for, each with the text and its own number."""
    return "".join(f"{text} {number}\n" for number in range(1, count + 1))


# What a search finds


async def test_the_lines_that_match_come_back_with_their_path_and_number(desk: Desk) -> None:
    put(desk, "a.py", "import os\n\ndef main():\n    pass\n")
    put(desk, "src/b.py", "def other():\n    return 1\n")
    put(desk, "src/pkg/c.py", "x = 1\n")

    assert await lines(desk, r"def \w+") == ["a.py:3:def main():", "src/b.py:1:def other():"]


async def test_a_search_can_be_kept_to_one_directory_or_one_file(desk: Desk) -> None:
    put(desk, "a.py", "needle\n")
    put(desk, "src/b.py", "hay\nneedle\n")
    put(desk, "src/c.py", "needle\n")

    assert await lines(desk, "needle", path="src") == ["src/b.py:2:needle", "src/c.py:1:needle"]
    assert await lines(desk, "needle", path="./src/b.py") == ["src/b.py:2:needle"]


async def test_nothing_that_matches_is_said_in_words(desk: Desk) -> None:
    put(desk, "a.py", "hay\n")

    assert await lines(desk, "needle") == ["No line matches needle in .."]
    assert await lines(desk, "needle", path="a.py") == ["No line matches needle in a.py."]


async def test_only_files_whose_paths_fit_are_searched_if_the_call_says_so(desk: Desk) -> None:
    put(desk, "a.py", "needle\n")
    put(desk, "a.md", "needle\n")
    put(desk, "src/b.py", "needle\n")
    put(desk, "src/b.md", "needle\n")

    assert await lines(desk, "needle", glob="*.py") == ["a.py:1:needle", "src/b.py:1:needle"]
    assert await lines(desk, "needle", glob="src/*.md") == ["src/b.md:1:needle"]
    assert await lines(desk, "needle", glob="*.md", path="src") == ["src/b.md:1:needle"]


async def test_a_file_that_the_call_names_is_searched_whatever_the_paths_are_to_fit(
    desk: Desk,
) -> None:
    put(desk, "notes.txt", "needle\n")

    assert await lines(desk, "needle", path="notes.txt", glob="*.py") == ["notes.txt:1:needle"]


async def test_case_counts_unless_the_call_says_otherwise(desk: Desk) -> None:
    put(desk, "a.txt", "Needle\nneedle\n")

    assert await lines(desk, "needle") == ["a.txt:2:needle"]
    assert await lines(desk, "needle", ignore_case=True) == ["a.txt:1:Needle", "a.txt:2:needle"]


async def test_a_line_is_shown_without_its_line_break_whichever_kind_it_is(desk: Desk) -> None:
    put(desk, "unix.txt", "one\ntwo needle\n")
    put(desk, "windows.txt", "one\r\ntwo needle\r\nthree\r\n")

    assert await lines(desk, "needle$") == ["unix.txt:2:two needle", "windows.txt:2:two needle"]


async def test_the_number_of_a_line_is_the_one_that_the_read_tool_gives_it(desk: Desk) -> None:
    put(desk, "a.txt", "one\r\n\r\nthree\nfour needle\nfive")

    found = await lines(desk, "needle")
    shown = await desk.call("read", path="a.txt", offset=3, limit=1)

    assert found == ["a.txt:4:four needle"]
    assert shown.text.splitlines()[0] == "     4\tfour needle"


async def test_text_of_any_script_is_found_and_shown_whole(desk: Desk) -> None:
    moon = chr(0x6708) + chr(0x795E)
    name = chr(0x6CE8) + chr(0x91CA) + ".py"
    put(desk, name, f"# {moon} {chr(0x1F600)}\nplain\n")

    assert await lines(desk, moon) == [f"{name}:1:# {moon} {chr(0x1F600)}"]


async def test_bytes_that_are_no_text_are_shown_with_a_sign_in_their_place(desk: Desk) -> None:
    put(desk, "a.txt", b"caf\xe9 needle\n")

    assert await lines(desk, "needle") == [f"a.txt:1:caf{FFFD} needle"]


async def test_of_a_long_line_the_beginning_is_shown_and_the_rest_is_counted(desk: Desk) -> None:
    put(desk, "a.txt", "needle " + "x" * 300 + "\nneedle\n")

    assert await lines(desk, "needle") == [
        "a.txt:1:needle " + "x" * 193 + " [107 more characters]",
        "a.txt:2:needle",
    ]


async def test_the_history_of_a_repository_and_links_are_not_searched(
    desk: Desk, tmp_path: Path
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, ".git/config", "needle\n")
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "c.txt").write_text("needle\n")
    link_directory(desk.work / "out", outside)

    assert await lines(desk, "needle") == ["a.txt:1:needle"]


# What a search shows of much


async def test_a_search_shows_as_many_lines_as_it_may_and_says_that_there_may_be_more(
    desk: Desk,
) -> None:
    put(desk, "a.txt", numbered(251))

    found = await lines(desk, "needle")

    assert found[:250] == [f"a.txt:{number}:needle {number}" for number in range(1, 251)]
    assert found[250:] == [CUT]


async def test_a_search_that_shows_all_there_is_does_not_say_that_there_may_be_more(
    desk: Desk,
) -> None:
    put(desk, "a.txt", numbered(250))

    found = await lines(desk, "needle")

    assert len(found) == 250
    assert found[-1] == "a.txt:250:needle 250"


async def test_what_was_found_before_counts_against_what_is_shown_of_the_next_files(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", numbered(200))
    put(desk, "b.txt", numbered(100))
    monkeypatch.setattr(grep, "BATCH_BYTES", 1)

    found = await lines(desk, "needle")

    assert found[199:201] == ["a.txt:200:needle 200", "b.txt:1:needle 1"]
    assert found[249:] == ["b.txt:50:needle 50", CUT]


async def test_a_search_that_has_enough_does_not_look_at_the_files_that_are_left(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", numbered(250))
    put(desk, "b.txt", "hay\n")
    monkeypatch.setattr(grep, "BATCH_BYTES", 1)
    opened = watch_what_is_opened(monkeypatch)

    found = await lines(desk, "needle")

    assert found[249:] == ["a.txt:250:needle 250", CUT]
    assert opened == ["a.txt"]


async def test_files_that_are_read_together_are_told_apart_in_what_comes_back(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("a", "b", "c", "d", "e"):
        put(desk, f"{name}.txt", f"hay\nneedle of {name}\n")
    monkeypatch.setattr(grep, "BATCH_BYTES", 40)
    started = count_the_processes(monkeypatch)

    found = await lines(desk, "needle")

    assert found == [f"{name}.txt:2:needle of {name}" for name in ("a", "b", "c", "d", "e")]
    # Each file has 16 bytes: three make a batch, and the two that are left make another.
    assert started == [3, 2]


async def test_a_search_that_has_enough_does_not_wait_until_all_it_read_was_taken(
    desk: Desk,
) -> None:
    put(desk, "a.txt", numbered(300))
    for number in range(4):
        put(desk, f"more{number}.txt", ("hay " * 20 + "\n") * 30_000)

    found = await lines(desk, "needle")

    assert len(found) == 251
    assert found[-1] == CUT


async def test_a_search_stops_when_it_has_read_as_much_as_it_may(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    half = "needle\n" + "x" * (MIB // 2)
    put(desk, "a.txt", half)
    put(desk, "b.txt", half)
    put(desk, "c.txt", "needle\n")
    monkeypatch.setattr(grep, "BATCH_BYTES", 1)
    monkeypatch.setattr(grep, "MAX_BYTES", MIB)

    assert await lines(desk, "needle$") == [
        "a.txt:1:needle",
        "b.txt:1:needle",
        "[The search stopped after 1 MiB of text, so there may be more. Search a narrower path.]",
    ]


async def test_a_search_that_read_as_much_as_it_may_and_all_there_is_says_nothing_of_more(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    half = "needle\n" + "x" * (MIB // 2)
    put(desk, "a.txt", half)
    put(desk, "b.txt", half)
    monkeypatch.setattr(grep, "BATCH_BYTES", 1)
    monkeypatch.setattr(grep, "MAX_BYTES", MIB)

    assert await lines(desk, "needle$") == ["a.txt:1:needle", "b.txt:1:needle"]


async def test_a_walk_that_was_cut_short_is_said(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("a", "b", "c"):
        put(desk, f"{name}.txt", "needle\n")
    monkeypatch.setattr(grep, "MAX_ENTRIES", 2)

    assert await lines(desk, "needle") == [
        "a.txt:1:needle",
        "b.txt:1:needle",
        "[The search stopped after 2 files and directories, so there may be more. "
        "Search a narrower directory.]",
    ]


# What a search passes over


async def test_files_that_do_not_look_like_text_are_not_searched_and_are_counted(
    desk: Desk,
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "image.bin", b"\x89PNG\x00needle\n")

    assert await lines(desk, "needle") == [
        "a.txt:1:needle",
        "[1 file was not searched, because it does not look like text.]",
    ]

    put(desk, "other.bin", b"needle\n\x00")

    assert (await lines(desk, "needle"))[1:] == [
        "[2 files were not searched, because they do not look like text.]"
    ]


async def test_a_file_that_does_not_look_like_text_is_not_read_to_its_end(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "big.bin", b"\x00" + b"x" * 100_000)
    reads: list[int] = []
    really_open: Any = bound.open_for_reading

    class Counting:
        def __init__(self, file: Any) -> None:
            self.file = file

        def read(self, size: int) -> bytes:
            reads.append(size)
            return self.file.read(size)

        def close(self) -> None:
            self.file.close()

    def opening(path: Path) -> Any:
        file, status = really_open(path)
        return Counting(file), status

    monkeypatch.setattr(bound, "open_for_reading", opening)

    await lines(desk, "needle")

    assert reads == [8192]


async def test_a_zero_byte_far_into_a_file_does_not_make_it_something_else_than_text(
    desk: Desk,
) -> None:
    put(desk, "a.txt", "needle\n" + "x" * 8192 + "\x00\n")

    assert await lines(desk, "needle") == ["a.txt:1:needle"]


async def test_files_that_are_too_large_are_not_searched_and_are_counted(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "exact.txt", "needle\n" + "x" * (MIB - 7))
    put(desk, "large.txt", "needle\n" + "x" * (MIB - 6))
    monkeypatch.setattr(grep, "MAX_FILE_BYTES", MIB)
    opened = watch_what_is_opened(monkeypatch)

    assert await lines(desk, "needle$") == [
        "a.txt:1:needle",
        "exact.txt:1:needle",
        "[1 file was not searched, because it is larger than 1 MiB.]",
    ]
    assert opened == ["a.txt", "exact.txt"]

    put(desk, "larger.txt", "x" * (MIB + 100))

    assert (await lines(desk, "needle$"))[2:] == [
        "[2 files were not searched, because they are larger than 1 MiB.]"
    ]


async def test_a_file_that_has_grown_since_it_was_listed_is_not_taken_in_beyond_the_limit(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "grown.txt", "needle\n" + "x" * MIB)
    monkeypatch.setattr(grep, "MAX_FILE_BYTES", MIB)
    really_list: Any = bound.listing

    def listing(path: Path) -> Any:
        return [
            bound.Listed(entry.name, entry.kind, 0, entry.modified_ns)
            for entry in really_list(path)
        ]

    monkeypatch.setattr(bound, "listing", listing)

    assert await lines(desk, "needle$") == [
        "a.txt:1:needle",
        "[1 file was not searched, because it is larger than 1 MiB.]",
    ]


def watch_what_is_opened(monkeypatch: pytest.MonkeyPatch, *locked: str) -> list[str]:
    """Write down the names of the files that are opened. Those in ``locked`` cannot be."""
    opened: list[str] = []
    really_open: Any = bound.open_for_reading

    def opening(path: Path) -> Any:
        if path.name in locked:
            raise PermissionError(13, "Permission denied")
        opened.append(path.name)
        return really_open(path)

    monkeypatch.setattr(bound, "open_for_reading", opening)
    return opened


def count_the_processes(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """Write down, for every matching process that is started, how many files it is given."""
    started: list[int] = []
    really_run: Any = process.run

    async def running(argv: Any, **options: Any) -> Any:
        started.append(options["stdin"].count(b'{"id": '))
        return await really_run(argv, **options)

    monkeypatch.setattr(process, "run", running)
    return started


async def test_files_that_cannot_be_read_are_counted(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "locked.txt", "needle\n")
    put(desk, "sealed.txt", "needle\n")
    watch_what_is_opened(monkeypatch, "locked.txt")

    assert await lines(desk, "needle") == [
        "a.txt:1:needle",
        "sealed.txt:1:needle",
        "[1 file could not be read.]",
    ]

    watch_what_is_opened(monkeypatch, "locked.txt", "sealed.txt")

    assert (await lines(desk, "needle"))[1:] == ["[2 files could not be read.]"]


async def test_a_file_that_is_somewhere_else_by_the_time_it_is_opened_counts_as_unreadable(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "moved.txt", "needle\n")
    really_open: Any = bound.open_for_reading

    def opening(path: Path) -> Any:
        if path.name == "moved.txt":
            raise bound.MovedError(str(path))
        return really_open(path)

    monkeypatch.setattr(bound, "open_for_reading", opening)

    assert await lines(desk, "needle") == ["a.txt:1:needle", "[1 file could not be read.]"]


async def test_a_search_without_a_file_that_it_could_look_into_starts_no_process(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "image.bin", b"\x00needle")
    started = count_the_processes(monkeypatch)

    assert await lines(desk, "needle") == [
        "No line matches needle in ..",
        "[1 file was not searched, because it does not look like text.]",
    ]
    assert started == []


# What a search may not see


async def test_what_a_rule_keeps_from_the_tool_is_not_opened_and_is_counted(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    def private(access: Access) -> Refuse | None:
        if access.tool == "grep" and access.where.relative.startswith("private"):
            return Refuse("That is private.", "DENIED")
        return None

    put(desk, "a.txt", "needle\n")
    put(desk, "private.txt", "needle\n")
    put(desk, "private/keys.txt", "needle\n")
    desk.fs.restrictions.add(desk.ctx, "private", private)
    opened = watch_what_is_opened(monkeypatch)

    assert await lines(desk, "needle") == [
        "a.txt:1:needle",
        "[2 files and directories were not searched, because this tool may not see them.]",
    ]
    assert opened == ["a.txt"]


async def test_a_file_that_a_rule_keeps_from_the_tool_cannot_be_searched_by_naming_it(
    desk: Desk,
) -> None:
    def private(access: Access) -> Refuse | None:
        return Refuse("That is private.", "DENIED") if access.where.relative == "p.txt" else None

    put(desk, "p.txt", "needle\n")
    desk.fs.restrictions.add(desk.ctx, "private", private)

    assert await failed(desk, "needle", path="p.txt") == ("DENIED", "That is private.")


async def test_the_seam_is_told_which_file_the_call_named_and_which_ones_were_come_upon(
    desk: Desk,
) -> None:
    asked: list[tuple[str, str, str, bool]] = []

    def watching(access: Access) -> Refuse | None:
        asked.append((access.tool, access.kind, access.where.relative, access.named))
        return None

    put(desk, "src/a.txt", "needle\n")
    desk.fs.restrictions.add(desk.ctx, "watching", watching)

    await lines(desk, "needle", path="src")
    await lines(desk, "needle", path="src/a.txt")

    assert asked == [
        ("grep", "read", "src", True),
        ("grep", "read", "src/a.txt", False),
        ("grep", "read", "src/a.txt", True),
    ]


@pytest.mark.parametrize(
    ("path", "code"), [("nowhere", "NOT_FOUND"), ("..", "OUTSIDE_WORKING_DIRECTORY")]
)
async def test_a_place_that_cannot_be_searched_is_an_error(
    desk: Desk, path: str, code: str
) -> None:
    put(desk, "a.txt", "needle\n")

    assert (await failed(desk, "needle", path=path))[0] == code


async def test_a_place_that_the_system_does_not_let_the_tool_look_at_is_an_error(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "locked/a.txt", "needle\n")
    really_stat: Any = os.stat

    def status(path: Any, *arguments: Any, **options: Any) -> Any:
        if Path(path).name == "locked":
            raise PermissionError(13, "Permission denied")
        return really_stat(path, *arguments, **options)

    with monkeypatch.context() as patch:
        patch.setattr(os, "stat", status)
        code, text = await failed(desk, "needle", path="locked")

    assert code == "TOOL_ERROR"
    assert text.startswith("locked cannot be searched: ")
    assert "Permission denied" in text


async def test_a_session_that_is_read_only_can_search(desk: Desk) -> None:
    put(desk, "a.txt", "needle\n")
    desk.sandbox.set_mode(desk.session, "read-only")

    assert await lines(desk, "needle") == ["a.txt:1:needle"]


async def test_a_path_that_leads_elsewhere_than_when_the_call_was_checked_is_not_searched(
    desk: Desk,
) -> None:
    put(desk, "drafts/a.txt", "needle\n")
    put(desk, "final/b.txt", "needle\n")
    link_directory(desk.work / "door", desk.work / "drafts")

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(desk.work / "door")
        link_directory(desk.work / "door", desk.work / "final")
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    assert (await failed(desk, "needle", path="door"))[0] == "MOVED"


# Patterns that cannot be used, and searches that do not end


async def test_a_pattern_that_is_no_regular_expression_is_an_error(desk: Desk) -> None:
    put(desk, "a.txt", "needle\n")

    code, text = await failed(desk, "(needle")

    assert code == "INVALID_PATTERN"
    assert text.startswith("The pattern is no regular expression that can be used: ")
    assert "position 0" in text


@pytest.mark.parametrize(
    ("arguments", "code"),
    [
        ({"pattern": ""}, "INVALID_ARGS"),
        ({"pattern": "needle", "glob": "../*.py"}, "INVALID_PATTERN"),
        ({"pattern": "needle", "glob": "../*.py", "path": "a.txt"}, "INVALID_PATTERN"),
    ],
)
async def test_arguments_that_cannot_be_used_are_an_error(
    desk: Desk, arguments: dict[str, str], code: str
) -> None:
    put(desk, "a.txt", "needle\n")

    outcome = await desk.call("grep", **arguments)

    assert outcome.error is not None
    assert outcome.error.code == code


async def test_a_pattern_that_takes_too_long_is_stopped_and_the_agent_stays_awake(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "a" * 40 + "b\n")
    monkeypatch.setattr(grep, "SECONDS", 1.5)
    gaps: list[float] = []

    async def ticking() -> None:
        last = time.monotonic()
        while True:
            await asyncio.sleep(0.02)
            now = time.monotonic()
            gaps.append(now - last)
            last = now

    ticker = asyncio.create_task(ticking())
    began = time.monotonic()
    try:
        code, text = await failed(desk, "(a+)+$")
    finally:
        ticker.cancel()
    took = time.monotonic() - began

    assert code == "SEARCH_TIMEOUT"
    assert text.startswith("The search took longer than 1.5 seconds and was stopped.")
    assert 1.4 < took < 10
    assert len(gaps) > 20
    assert max(gaps) < 0.5


async def test_a_search_whose_time_is_up_before_it_matches_starts_no_process(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    monkeypatch.setattr(grep, "SECONDS", 0.0)
    started = count_the_processes(monkeypatch)

    assert (await failed(desk, "needle"))[0] == "SEARCH_TIMEOUT"
    assert started == []


async def test_the_time_of_a_search_is_one_for_all_it_reads(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "b.txt", "needle\n")
    monkeypatch.setattr(grep, "BATCH_BYTES", 1)
    monkeypatch.setattr(grep, "SECONDS", 30.0)
    given: list[float] = []
    really_run: Any = process.run

    async def running(argv: Any, **options: Any) -> Any:
        given.append(options["timeout_seconds"])
        await asyncio.sleep(0.3)
        return await really_run(argv, **options)

    monkeypatch.setattr(process, "run", running)

    await lines(desk, "needle")

    assert len(given) == 2
    assert 29 < given[0] <= 30
    assert given[1] < given[0] - 0.3


def stand_in(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, script: str) -> None:
    """Put a program of the test in the place of the process that matches."""
    program = tmp_path / "stand_in.py"
    program.write_text(script, encoding="utf-8")
    monkeypatch.setattr(grep, "_WORKER", program)


DONE = 'print(\'{"done": true, "files": 1, "cut": false}\')'
HIT = 'print(\'{"id": %d, "line": 1, "text": "needle", "more": 0}\')'


@pytest.mark.parametrize(
    ("script", "said"),
    [
        ("print('this is no answer')", "exit code 0 and said: this is no answer"),
        ("print('[1, 2]')", "exit code 0 and said: [1, 2]"),
        ("", "exit code 0 and said: nothing"),
        ("raise SystemExit(3)", "exit code 3 and said: nothing"),
        (DONE + "\nraise SystemExit(1)", "exit code 1 and said: {"),
        (HIT % 0, "exit code 0 and said: {"),
        ("print('{\"files\": 1}')", "exit code 0 and said: {"),
        ('print(\'{"done": true, "files": 0, "cut": false}\')', "exit code 0 and said: {"),
        (HIT % 1 + "\n" + DONE, "exit code 0 and said: {"),
        (HIT % -1 + "\n" + DONE, "exit code 0 and said: {"),
    ],
    ids=[
        "no json",
        "no object",
        "nothing",
        "exit code alone",
        "exit code after a whole answer",
        "no last line",
        "a last line that does not say that it is the last",
        "fewer files than were sent",
        "a file that was not sent",
        "a file before the first",
    ],
)
async def test_a_matching_process_that_does_not_answer_as_it_should_is_an_error(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, script: str, said: str
) -> None:
    put(desk, "a.txt", "needle\n")
    stand_in(tmp_path, monkeypatch, script)

    code, text = await failed(desk, "needle")

    assert code == "SEARCH_FAILED"
    assert text.startswith("The search failed. The process that matches the lines ended with the ")
    assert said in text


async def test_a_matching_process_that_answers_as_it_should_is_believed(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "hay\n")
    stand_in(tmp_path, monkeypatch, HIT % 0 + "\n" + DONE)

    assert await lines(desk, "needle") == ["a.txt:1:needle"]


async def test_the_matching_process_is_started_with_nothing_but_python(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "hay\n")
    # A module beside the program, named like one of Python. It must not be taken for that.
    (tmp_path / "json.py").write_text("def dumps(value):\n    return 'not the json of Python'\n")
    stand_in(
        tmp_path,
        monkeypatch,
        "import json, sys\n"
        "print(json.dumps({'done': True, 'files': 1, 'cut': False}))\n"
        "print(json.dumps({'isolated': sys.flags.isolated, 'no_site': sys.flags.no_site}))\n",
    )
    said: list[str] = []
    really_run: Any = process.run

    async def running(argv: Any, **options: Any) -> Any:
        done = await really_run(argv, **options)
        said.extend(done.output.splitlines())
        return done

    monkeypatch.setattr(process, "run", running)

    await desk.call("grep", pattern="needle")

    assert said == [
        '{"done": true, "files": 1, "cut": false}',
        '{"isolated": 1, "no_site": 1}',
    ]


async def test_a_matching_process_that_says_more_than_it_may_is_an_error(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "hay\n")
    stand_in(tmp_path, monkeypatch, f"for _ in range(100):\n    {HIT % 0}\n{DONE}")
    monkeypatch.setattr(grep, "_OUTPUT_BYTES", 2000)

    assert (await failed(desk, "needle"))[0] == "SEARCH_FAILED"


async def test_only_the_end_of_what_a_failed_process_said_is_passed_on(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "hay\n")
    stand_in(tmp_path, monkeypatch, "print('x' * 5000 + 'the end')")

    _, text = await failed(desk, "needle")

    assert text.endswith("said: " + "x" * 293 + "the end")


async def test_a_matching_process_that_cannot_be_started_is_an_error(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    monkeypatch.setattr(sys, "executable", str(desk.work / "no-such-python"))

    code, text = await failed(desk, "needle")

    assert code == "SEARCH_FAILED"
    assert text.startswith("The search failed: the program ")


async def test_a_search_that_is_cancelled_leaves_no_process_behind(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    note = tmp_path / "pid"
    stand_in(
        tmp_path,
        monkeypatch,
        "import os, sys, time\n"
        "with open(sys.argv[0] + '.pid', 'w') as note:\n"
        "    note.write(str(os.getpid()))\n"
        f"os.replace(sys.argv[0] + '.pid', {str(note)!r})\n"
        "time.sleep(600)\n",
    )
    calling = asyncio.create_task(desk.call("grep", pattern="needle"))
    deadline = time.monotonic() + 20
    while not note.exists():
        assert time.monotonic() < deadline
        await asyncio.sleep(0.02)
    pid = int(note.read_text())

    calling.cancel()
    with pytest.raises(asyncio.CancelledError):
        await calling

    assert await gone(pid)


async def test_a_search_that_is_cancelled_while_it_reads_opens_no_further_file(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "b.txt", "needle\n")
    reached = threading.Event()
    released = threading.Event()
    finished = threading.Event()
    opened: list[str] = []
    really_open: Any = bound.open_for_reading
    really_load: Any = vars(grep)["_load"]

    def opening(path: Path) -> Any:
        opened.append(path.name)
        reached.set()
        released.wait(20)
        return really_open(path)

    def loading(*arguments: Any) -> Any:
        try:
            return really_load(*arguments)
        finally:
            finished.set()

    monkeypatch.setattr(bound, "open_for_reading", opening)
    monkeypatch.setattr(grep, "_load", loading)
    calling = asyncio.create_task(desk.call("grep", pattern="needle"))
    assert await asyncio.to_thread(reached.wait, 20)

    calling.cancel()
    with pytest.raises(asyncio.CancelledError):
        await calling
    released.set()

    assert await asyncio.to_thread(finished.wait, 20)
    assert opened == ["a.txt"]


async def test_reading_stops_at_the_next_file_when_the_search_was_given_up(desk: Desk) -> None:
    put(desk, "a.txt", "needle\n")
    put(desk, "b.txt", "needle\n")
    load: Any = vars(grep)["_load"]

    def found(name: str) -> Found:
        where = locate(desk.work, name)
        permit = desk.fs.permit(Access(desk.session, desk.work, where, "read", "grep"))
        return Found(where, (name,), 7, 0, permit)

    files = [found("a.txt"), found("b.txt")]
    stopped = threading.Event()

    assert load(files, stopped) == (
        [("a.txt", b"needle\n"), ("b.txt", b"needle\n")],
        Counter(),
    )

    stopped.set()

    assert load(files, stopped) == ([], Counter())


# What the checks and the model are told


async def test_a_search_says_what_it_acts_on(desk: Desk) -> None:
    seen: list[tuple[str, str | None]] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append((invocation.tool.effect, invocation.target))
        return None

    put(desk, "src/a.py", "needle\n")
    desk.tools.guards.add(desk.ctx, "watching", watching)

    await desk.call("grep", pattern="needle")
    await desk.call("grep", pattern="needle", path="./src/a.py")

    assert seen == [("read", "."), ("read", "src/a.py")]


def test_searches_may_run_beside_each_other(desk: Desk) -> None:
    assert desk.begin("grep", pattern="needle").concurrency_safe
