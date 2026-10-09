"""The read tool: text files of the working directory, with line numbers and limits."""

import asyncio
import codecs
import shutil
import tempfile
import threading
import tracemalloc
from pathlib import Path
from typing import Any, cast
from unittest import mock

import pytest
from hypothesis import example, given, settings
from hypothesis import strategies as st

import columbina.fs
import columbina.tools
from columbina.builtin_tools import read
from columbina.fs import FS, Access, Fingerprint, Fs, Permit, Refuse, locate
from columbina.kernel import Context, Next
from columbina.llm import ToolCallBlock
from columbina.session import CORE_EVENTS, Session
from columbina.tools import (
    POST_EXECUTE,
    TOOLS,
    Block,
    Deny,
    Executed,
    Invocation,
    ToolOutcome,
    ToolRun,
    Tools,
    Verdict,
)
from support.links import link_directory, unlink_directory


@pytest.fixture
async def tools(ctx: Context) -> Tools:
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(read)
    return ctx.use(TOOLS)


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    return directory


async def call(ctx: Context, tools: Tools, work: Path, arguments: str) -> ToolOutcome:
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})
    asked = ToolCallBlock(call_id="call-1", name="read", arguments=arguments)
    return await tools.run(ctx, asked, ToolRun(call_id="call-1", cwd=work, session=session))


def allowed(path: Path) -> Permit:
    """A permit to read a file, for the tests that read without the tool around them."""
    where = locate(path.parent, path.name)
    return Permit(Access(cast("Session", None), path.parent, where, "read", "read"), Fs())


def arguments(path: str | Path, **more: int) -> str:
    import json

    return json.dumps({"path": str(path), **more})


async def test_a_file_is_read_with_its_line_numbers(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "notes.txt").write_text("first\nsecond\n\nfourth\n", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("notes.txt"))

    assert outcome.error is None
    assert outcome.text == "     1\tfirst\n     2\tsecond\n     3\t\n     4\tfourth"


async def test_a_path_may_be_given_in_full_if_it_is_inside_the_working_directory(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "sub").mkdir()
    (work / "sub" / "a.txt").write_text("inside", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments(work / "sub" / "a.txt"))

    assert outcome.text == "     1\tinside"


@pytest.mark.parametrize("path", ["../secret.txt", "sub/../../secret.txt"])
async def test_a_file_outside_the_working_directory_is_not_read(
    ctx: Context, tools: Tools, work: Path, path: str
) -> None:
    (work / "sub").mkdir()
    (work.parent / "secret.txt").write_text("secret", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments(path))

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert "secret" not in outcome.text.replace("secret.txt", "")
    assert outcome.text == (
        f"The path {path} is outside the working directory. Only files inside it can be read."
    )


async def test_a_full_path_outside_the_working_directory_is_not_read(
    ctx: Context, tools: Tools, work: Path
) -> None:
    outside = work.parent / "secret.txt"
    outside.write_text("secret", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments(outside))

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"


async def test_a_file_that_is_not_there_is_an_error_that_says_so(
    ctx: Context, tools: Tools, work: Path
) -> None:
    outcome = await call(ctx, tools, work, arguments("nowhere.txt"))

    assert outcome.error is not None
    assert outcome.error.code == "NOT_FOUND"
    assert outcome.text == "There is no file nowhere.txt."


async def test_a_file_that_cannot_be_opened_is_an_error_that_says_why(
    ctx: Context, tools: Tools, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def locked(permit: Permit, offset: int, limit: int, stopped: threading.Event) -> None:
        raise PermissionError(13, "Permission denied")

    (work / "locked.txt").write_text("x", encoding="utf-8")
    monkeypatch.setattr(read, "_load", locked)

    outcome = await call(ctx, tools, work, arguments("locked.txt"))

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_ERROR"
    assert outcome.text == "locked.txt cannot be read: [Errno 13] Permission denied"


async def test_a_directory_is_not_a_file(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "sub").mkdir()

    outcome = await call(ctx, tools, work, arguments("sub"))

    assert outcome.error is not None
    assert outcome.error.code == "NOT_A_FILE"
    assert outcome.text == "sub is a directory, not a file."


async def test_an_empty_file_says_that_it_is_empty(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "empty.txt").write_bytes(b"")

    outcome = await call(ctx, tools, work, arguments("empty.txt"))

    assert outcome.error is None
    assert outcome.text == "(the file is empty)"


async def test_a_part_of_a_file_can_be_asked_for(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "ten.txt").write_text("".join(f"line {n}\n" for n in range(1, 11)), encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("ten.txt", offset=3, limit=2))

    assert outcome.text == (
        "     4\tline 4\n     5\tline 5\n[lines 4 to 5 of 10. Pass offset=5 to read on.]"
    )


async def test_the_end_of_a_file_needs_no_note(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "ten.txt").write_text("".join(f"line {n}\n" for n in range(1, 11)), encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("ten.txt", offset=8))

    assert outcome.text == "     9\tline 9\n    10\tline 10"


async def test_an_offset_past_the_end_says_how_long_the_file_is(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "ten.txt").write_text("".join(f"line {n}\n" for n in range(1, 11)), encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("ten.txt", offset=10))

    assert outcome.error is None
    assert outcome.text == "[the file has 10 lines, and the offset skips them all]"


async def test_no_more_than_two_thousand_lines_are_returned_at_once(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "long.txt").write_text("x\n" * 2500, encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("long.txt"))

    lines = outcome.text.split("\n")
    assert len(lines) == 2001
    assert lines[1999] == "  2000\tx"
    assert lines[2000] == "[lines 1 to 2000 of 2500. Pass offset=2000 to read on.]"


async def test_a_very_long_line_is_cut(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "wide.txt").write_text("a" * 2500 + "\nshort\n", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("wide.txt"))

    first, second = outcome.text.split("\n")
    assert first == "     1\t" + "a" * 2000 + " [500 more characters]"
    assert second == "     2\tshort"


async def test_a_long_line_of_wide_characters_is_cut_at_a_character(
    ctx: Context, tools: Tools, work: Path
) -> None:
    # Each character takes three bytes, so the part of the line that is kept ends inside one.
    (work / "wide.txt").write_text("好" * 3000 + "\n", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("wide.txt"))

    assert outcome.text == "     1\t" + "好" * 2000 + " [1000 more characters]"


async def test_bytes_that_are_no_text_count_as_the_characters_they_are_shown_as(
    ctx: Context, tools: Tools, work: Path
) -> None:
    # Each of the hundred bytes is shown as one replacement character.
    raw = b"a" * 8000 + b"\x80" * 100
    (work / "odd.txt").write_bytes(raw + b"\n")

    outcome = await call(ctx, tools, work, arguments("odd.txt"))

    left_out = len(raw.decode("utf-8", errors="replace")) - 2000
    assert left_out == 6100
    assert outcome.text == "     1\t" + "a" * 2000 + f" [{left_out} more characters]"


async def test_the_line_break_of_windows_is_not_counted_as_a_character(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "wide.txt").write_bytes(b"a" * 9000 + b"\r\n" + b"b" * 8000 + b"\r\nshort\r\n")

    outcome = await call(ctx, tools, work, arguments("wide.txt"))

    assert outcome.text.split("\n") == [
        "     1\t" + "a" * 2000 + " [7000 more characters]",
        "     2\t" + "b" * 2000 + " [6000 more characters]",
        "     3\tshort",
    ]


def test_a_read_holds_little_in_memory_however_long_a_line_is(tmp_path: Path) -> None:
    megabyte = b"a" * (1024 * 1024)
    wide = tmp_path / "wide.txt"
    with wide.open("wb") as file:
        for _ in range(8):
            file.write(megabyte)
        file.write(b"\nsecond\n")
    del megabyte

    tracemalloc.start()
    try:
        part = read._load(allowed(wide), 0, 10, threading.Event())  # pyright: ignore[reportPrivateUsage]
        _, peak = tracemalloc.get_traced_memory()
    finally:
        tracemalloc.stop()

    assert part is not None
    assert part.shown == [
        "     1\t" + "a" * 2000 + f" [{8 * 1024 * 1024 - 2000} more characters]",
        "     2\tsecond",
    ]
    assert peak < 1024 * 1024


async def test_the_output_stops_at_a_size_limit(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "big.txt").write_text(("b" * 999 + "\n") * 200, encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("big.txt"))

    lines = outcome.text.split("\n")
    assert len(outcome.text.encode()) <= 51_200 + 200
    assert (
        lines[-1]
        == f"[lines 1 to {len(lines) - 1} of 200. Pass offset={len(lines) - 1} to read on.]"
    )
    assert 40 < len(lines) - 1 < 60


async def test_output_of_exactly_the_size_limit_is_returned_in_full(
    ctx: Context, tools: Tools, work: Path
) -> None:
    # A line takes 1024 bytes with its number, so fifty of them are exactly the limit.
    (work / "big.txt").write_text(("b" * 1016 + "\n") * 51, encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("big.txt"))

    lines = outcome.text.split("\n")
    assert len("\n".join(lines[:50]).encode()) + 1 == 50 * 1024
    assert lines[50:] == ["[lines 1 to 50 of 51. Pass offset=50 to read on.]"]


async def test_a_file_that_is_not_text_is_not_read(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "image.png").write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR")

    outcome = await call(ctx, tools, work, arguments("image.png"))

    assert outcome.error is not None
    assert outcome.error.code == "NOT_TEXT"
    assert outcome.text == "image.png does not look like a text file."


async def test_bytes_that_are_not_utf8_are_shown_as_replacement_characters(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "latin.txt").write_bytes("caf\xe9\r\nna\xefve".encode("latin-1"))

    outcome = await call(ctx, tools, work, arguments("latin.txt"))

    assert outcome.text == "     1\tcaf�\n     2\tna�ve"


async def test_text_in_any_script_is_read(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "中文.txt").write_text("你好\n世界\n", encoding="utf-8")

    outcome = await call(ctx, tools, work, arguments("中文.txt"))

    assert outcome.text == "     1\t你好\n     2\t世界"


async def test_the_limits_of_the_arguments_are_part_of_the_schema(tools: Tools) -> None:
    schema = tools.registry["read"].schema

    assert schema.name == "read"
    assert schema.parameters["required"] == ["path"]
    properties = schema.parameters["properties"]
    assert isinstance(properties, dict)
    assert properties["offset"] == {
        "type": "integer",
        "minimum": 0,
        "default": 0,
        "description": "How many lines to skip at the beginning.",
    }
    assert properties["limit"] == {
        "type": "integer",
        "minimum": 1,
        "maximum": 2000,
        "default": 2000,
        "description": "How many lines to return at most.",
    }


async def test_reads_may_run_beside_each_other(ctx: Context, tools: Tools, work: Path) -> None:
    (work / "a.txt").write_text("a", encoding="utf-8")
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})
    asked = ToolCallBlock(call_id="call-1", name="read", arguments=arguments("a.txt"))

    pending = tools.begin(ctx, asked, ToolRun(call_id="call-1", cwd=work, session=session))

    assert pending.concurrency_safe


# A file that is larger than a read looks at


@pytest.fixture
def hundred(work: Path) -> Path:
    """A file of a hundred lines of ten bytes each."""
    path = work / "hundred.txt"
    path.write_bytes(b"".join(f"line {number:>4}\n".encode() for number in range(1, 101)))
    return path


async def test_a_read_looks_at_the_beginning_of_a_large_file_only(
    ctx: Context, tools: Tools, work: Path, hundred: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(read, "_SCAN_BYTES", 255)

    outcome = await call(ctx, tools, work, arguments("hundred.txt", limit=2))

    # The first 255 bytes hold 25 lines and half of the next one.
    assert outcome.error is None
    assert outcome.text.split("\n") == [
        "     1\tline    1",
        "     2\tline    2",
        "[lines 1 to 2 of at least 26. Pass offset=2 to read on. "
        "Only the first 255 bytes of a file are looked at.]",
    ]


async def test_the_last_line_that_a_read_looks_at_may_be_cut_short(
    ctx: Context, tools: Tools, work: Path, hundred: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(read, "_SCAN_BYTES", 255)

    outcome = await call(ctx, tools, work, arguments("hundred.txt", offset=24))

    assert outcome.text.split("\n") == [
        "    25\tline   25",
        "    26\tline ",
        "[lines 25 to 26 of at least 26. Only the first 255 bytes of a file are looked at.]",
    ]


async def test_an_offset_beyond_what_a_read_looks_at_says_so(
    ctx: Context, tools: Tools, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "large.txt").write_bytes(b"x" * 1023 + b"\n" + b"y" * (1024 * 1024))
    monkeypatch.setattr(read, "_SCAN_BYTES", 1024 * 1024)

    outcome = await call(ctx, tools, work, arguments("large.txt", offset=5))

    assert outcome.error is None
    assert outcome.text == (
        "[Only the first 1 MiB of a file are looked at. "
        "They hold 2 lines here, and the offset skips them all.]"
    )


async def test_a_line_that_goes_on_beyond_what_a_read_looks_at_says_so(
    ctx: Context, tools: Tools, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "one.txt").write_bytes(b"z" * 40_000)
    monkeypatch.setattr(read, "_SCAN_BYTES", 30_000)

    outcome = await call(ctx, tools, work, arguments("one.txt"))

    assert outcome.text.split("\n") == [
        "     1\t" + "z" * 2000 + " [at least 28000 more characters]",
        "[lines 1 to 1 of at least 1. Only the first 30000 bytes of a file are looked at.]",
    ]


async def test_a_file_that_ends_where_a_read_stops_looking_was_read_in_full(
    ctx: Context, tools: Tools, work: Path, hundred: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(read, "_SCAN_BYTES", 1000)

    outcome = await call(ctx, tools, work, arguments("hundred.txt", offset=98))

    assert outcome.text == "    99\tline   99\n   100\tline  100"


# Stopping


async def test_a_read_that_is_cancelled_tells_its_thread_to_stop(
    ctx: Context, tools: Tools, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    began = threading.Event()
    told: list[bool] = []
    done = threading.Event()

    def slow(permit: Permit, offset: int, limit: int, stopped: threading.Event) -> None:
        began.set()
        told.append(stopped.wait(timeout=5))
        done.set()

    (work / "a.txt").write_text("a", encoding="utf-8")
    monkeypatch.setattr(read, "_load", slow)
    reading = asyncio.ensure_future(call(ctx, tools, work, arguments("a.txt")))
    assert await asyncio.to_thread(began.wait, 5)

    reading.cancel()
    await asyncio.gather(reading, return_exceptions=True)

    assert await asyncio.to_thread(done.wait, 5)
    assert told == [True]


def test_a_thread_that_is_told_to_stop_reads_no_further(
    hundred: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class AfterThreePieces(threading.Event):
        asked = 0

        def is_set(self) -> bool:
            self.asked += 1
            return self.asked > 3

    monkeypatch.setattr(read, "_CHUNK_BYTES", 100)

    part = read._load(allowed(hundred), 0, 2000, AfterThreePieces())  # pyright: ignore[reportPrivateUsage]

    assert part is not None
    assert (len(part.shown), part.lines, part.complete) == (30, 30, False)


# The reader against the simple way of doing the same


PIECES = [
    b"\n",
    b"\n",
    b"\r\n",
    b"\r",
    b"a",
    b"word",
    b"  ",
    "é".encode(),
    "好".encode(),
    "🙂".encode(),
    b"\xff",
    b"\x80",
    b"\xe5\xa5",
]
CHARACTERS_OF_A_LINE = 5


def simply(content: bytes, offset: int, limit: int, looked_at: int, size_limit: int) -> object:
    """What a read must find, worked out with the whole file in memory."""
    seen, complete = content[:looked_at], len(content) <= looked_at
    lines = seen.split(b"\n")
    ends_inside_a_line = lines[-1] != b""
    if not ends_inside_a_line:
        lines.pop()
    shown: list[str] = []
    size = 0
    for index, raw in enumerate(lines):
        if not offset <= index < offset + limit:
            continue
        cut_short = not complete and ends_inside_a_line and index == len(lines) - 1
        # A line that was cut short may end inside a character, which is then not shown yet.
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        text = decoder.decode(raw, final=not cut_short).removesuffix("\r")
        more = len(text) - CHARACTERS_OF_A_LINE
        if more > 0:
            counted = f"at least {more}" if cut_short else str(more)
            text = f"{text[:CHARACTERS_OF_A_LINE]} [{counted} more characters]"
        numbered = f"{index + 1:>6}\t{text}"
        size += len(numbered.encode()) + 1
        if shown and size > size_limit:
            break
        shown.append(numbered)
    return (shown, len(lines), complete)


@settings(max_examples=500, deadline=None)
@given(
    content=st.lists(st.sampled_from(PIECES), max_size=80).map(b"".join),
    offset=st.integers(min_value=0, max_value=8),
    limit=st.integers(min_value=1, max_value=6),
    piece=st.one_of(st.integers(min_value=1, max_value=4), st.integers(min_value=1, max_value=40)),
    looked_at=st.one_of(st.just(10**6), st.integers(min_value=1, max_value=120)),
    size_limit=st.sampled_from([20, 60, 200]),
)
# Cases that a random run meets only now and then, pinned so that every run meets them.
# A carriage return in the middle of a line, with the line in pieces of one byte.
@example(content=b"a\rb\nnext\n", offset=0, limit=2, piece=1, looked_at=10**6, size_limit=200)
# The line break of Windows, parted between two pieces, after more than is shown.
@example(
    content=b"abcdefgh\r\nnext\r\n", offset=0, limit=2, piece=9, looked_at=10**6, size_limit=200
)
# A character of three bytes in pieces of two, and one that the line ends inside of.
@example(
    content="好好".encode() + b"\xe5\xa5\nnext\n",
    offset=0,
    limit=2,
    piece=2,
    looked_at=10**6,
    size_limit=200,
)
# A line that the read stops looking at inside a character.
@example(content="abc好def".encode(), offset=0, limit=1, piece=3, looked_at=4, size_limit=200)
# A long line and then a short one: nothing of the first may be left over for the second.
@example(
    content=b"abcdefghij\rk\nxy\n", offset=0, limit=2, piece=4, looked_at=10**6, size_limit=200
)
def test_reading_in_pieces_finds_what_reading_at_once_would(
    content: bytes, offset: int, limit: int, piece: int, looked_at: int, size_limit: int
) -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "file.txt"
        path.write_bytes(content)
        with mock.patch.multiple(
            read,
            _CHUNK_BYTES=piece,
            _SCAN_BYTES=looked_at,
            MAX_LINE_CHARACTERS=CHARACTERS_OF_A_LINE,
            MAX_BYTES=size_limit,
        ):
            part = read._load(allowed(path), offset, limit, threading.Event())  # pyright: ignore[reportPrivateUsage]

    assert part is not None
    expected = simply(content, offset, limit, looked_at, size_limit)
    assert (part.shown, part.lines, part.complete) == expected


# Through the seam of the file system


async def test_a_file_that_was_read_is_one_that_the_model_was_shown(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "notes.txt").write_text("first\n", encoding="utf-8")
    session = Session(ctx, "s-9", {kind.event_type: kind for kind in CORE_EVENTS})
    asked = ToolCallBlock(call_id="call-1", name="read", arguments=arguments("notes.txt"))

    outcome = await tools.run(ctx, asked, ToolRun(call_id="call-1", cwd=work, session=session))

    status = (work / "notes.txt").stat()
    assert outcome.error is None
    assert ctx.use(FS).seen(session, locate(work, "notes.txt").path) == Fingerprint(
        status.st_size, status.st_mtime_ns
    )


async def test_a_file_that_could_not_be_shown_is_not_remembered(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "blob.bin").write_bytes(bytes([0, 1, 2]))
    session = Session(ctx, "s-9", {kind.event_type: kind for kind in CORE_EVENTS})
    asked = ToolCallBlock(call_id="call-1", name="read", arguments=arguments("blob.bin"))

    outcome = await tools.run(ctx, asked, ToolRun(call_id="call-1", cwd=work, session=session))

    assert outcome.error is not None
    assert outcome.error.code == "NOT_TEXT"
    assert ctx.use(FS).seen(session, locate(work, "blob.bin").path) is None


async def test_a_restriction_of_the_seam_keeps_a_file_from_being_read(
    ctx: Context, tools: Tools, work: Path
) -> None:
    def closed(access: Access) -> Refuse | None:
        return Refuse("The notes are closed.", "CLOSED") if access.tool == "read" else None

    (work / "notes.txt").write_text("a secret", encoding="utf-8")
    ctx.use(FS).restrictions.add(ctx, "closed", closed)

    outcome = await call(ctx, tools, work, arguments("notes.txt"))

    assert outcome.error is not None
    assert outcome.error.code == "CLOSED"
    assert outcome.text == "The notes are closed."


async def test_a_read_says_which_file_it_acts_on_by_the_place_of_the_file(
    ctx: Context, tools: Tools, work: Path
) -> None:
    seen: list[str | None] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append(invocation.target)
        return None

    (work / "sub").mkdir()
    (work / "notes.txt").write_text("n", encoding="utf-8")
    tools.guards.add(ctx, "watching", watching)

    await call(ctx, tools, work, arguments("sub/../notes.txt"))
    await call(ctx, tools, work, arguments(work / "notes.txt"))

    assert seen == ["notes.txt", "notes.txt"]


async def test_a_path_that_is_not_used_here_is_refused_under_its_own_code(
    ctx: Context, tools: Tools, work: Path
) -> None:
    outcome = await call(ctx, tools, work, arguments("notes" + chr(0) + ".txt"))

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_PATH"
    assert "no path can contain" in outcome.text


async def test_a_file_that_is_elsewhere_by_the_time_it_is_opened_is_not_read(
    ctx: Context, tools: Tools, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "d").mkdir()
    (work / "d" / "notes.txt").write_text("inside", encoding="utf-8")
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    (outside / "notes.txt").write_text("a secret", encoding="utf-8")
    load: Any = vars(read)["_load"]

    def swapping(permit: Permit, offset: int, limit: int, stopped: threading.Event) -> Any:
        # The file was allowed. Now its directory becomes a link that leads out.
        shutil.rmtree(work / "d")
        link_directory(work / "d", outside)
        return load(permit, offset, limit, stopped)

    monkeypatch.setattr(read, "_load", swapping)

    outcome = await call(ctx, tools, work, arguments("d/notes.txt"))

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert "a secret" not in outcome.text


async def test_a_path_that_leads_elsewhere_than_when_the_call_was_checked_is_not_read(
    ctx: Context, tools: Tools, work: Path
) -> None:
    (work / "public").mkdir()
    (work / "public" / "notes.txt").write_text("for everyone", encoding="utf-8")
    (work / "private").mkdir()
    (work / "private" / "notes.txt").write_text("a secret", encoding="utf-8")
    link_directory(work / "door", work / "public")
    checked: list[str | None] = []

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        # The check sees where the door leads. Then the door is turned.
        checked.append(invocation.target)
        unlink_directory(work / "door")
        link_directory(work / "door", work / "private")
        return None

    tools.guards.add(ctx, "turning", turning)

    outcome = await call(ctx, tools, work, arguments("door/notes.txt"))

    assert checked == ["public/notes.txt"]
    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert "a secret" not in outcome.text


async def test_a_file_whose_content_was_withheld_was_not_shown_to_the_model(
    ctx: Context, tools: Tools, work: Path
) -> None:
    async def withholding(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return Block(reason="this is not for the model")

    (work / "notes.txt").write_text("first\n", encoding="utf-8")
    session = Session(ctx, "s-9", {kind.event_type: kind for kind in CORE_EVENTS})
    asked = ToolCallBlock(call_id="call-1", name="read", arguments=arguments("notes.txt"))
    ctx.on(POST_EXECUTE, withholding)

    outcome = await tools.run(ctx, asked, ToolRun(call_id="call-1", cwd=work, session=session))

    assert outcome.error is not None
    assert outcome.error.code == "RESULT_BLOCKED"
    assert ctx.use(FS).seen(session, locate(work, "notes.txt").path) is None
