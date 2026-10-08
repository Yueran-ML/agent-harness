"""The read tool: text files of the working directory, with line numbers and limits."""

from pathlib import Path

import pytest

import tack.tools
from tack.builtin_tools import read
from tack.kernel import Context
from tack.llm import ToolCallBlock
from tack.session import CORE_EVENTS, Session
from tack.tools import TOOLS, ToolOutcome, ToolRun, Tools


@pytest.fixture
async def tools(ctx: Context) -> Tools:
    await ctx.plugin(tack.tools)
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
    def locked(path: Path, offset: int, limit: int) -> None:
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
