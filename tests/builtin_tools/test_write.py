"""The write tool: a file is made, or all that it holds is replaced."""

import asyncio
import os
from pathlib import Path
from typing import Any

import pytest

import columbina.fs
from columbina.fs import bound
from columbina.kernel import Context
from columbina.session import SESSIONS, ToolError
from columbina.tools import Deny, Invocation
from support.desk import Desk
from support.held import HeldWrite
from support.links import link_directory, unlink_directory

# A file that is made


async def test_a_file_that_is_not_there_is_made(desk: Desk) -> None:
    outcome = await desk.call("write", path="notes.txt", content="first\nsecond\n")

    assert outcome.error is None
    assert outcome.text == "Created notes.txt: 2 lines."
    assert (desk.work / "notes.txt").read_bytes() == b"first\nsecond\n"


async def test_directories_that_are_missing_are_made(desk: Desk) -> None:
    outcome = await desk.call("write", path="docs/guide/intro.md", content="# Intro")

    assert outcome.error is None
    assert outcome.text == "Created docs/guide/intro.md: 1 line."
    assert (desk.work / "docs" / "guide" / "intro.md").read_bytes() == b"# Intro"


async def test_the_content_is_stored_as_it_was_given(desk: Desk) -> None:
    content = "one\r\ntwo\nthree\rfour\n\tfive 好\n"

    await desk.call("write", path="exact.txt", content=content)

    assert (desk.work / "exact.txt").read_bytes() == content.encode()


async def test_a_file_can_be_made_empty(desk: Desk) -> None:
    outcome = await desk.call("write", path="empty.txt", content="")

    assert outcome.text == "Created empty.txt: 0 lines."
    assert (desk.work / "empty.txt").read_bytes() == b""


async def test_content_that_cannot_be_stored_as_text_is_refused(desk: Desk) -> None:
    # Half of a pair of code units, which JSON can carry and no file in UTF-8 can hold.
    outcome = await desk.call("write", path="notes.txt", content="half: " + chr(0xD83D))

    # The pipeline does not take such arguments, so the tool never sees them.
    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"
    assert desk.names() == []


# A file that is replaced


async def test_a_file_that_was_read_is_replaced(desk: Desk) -> None:
    (desk.work / "notes.txt").write_bytes(b"old\n")
    await desk.have_read("notes.txt")

    outcome = await desk.call("write", path="notes.txt", content="new\ncontent\n")

    assert outcome.error is None
    assert outcome.text == "Replaced notes.txt: 2 lines."
    assert (desk.work / "notes.txt").read_bytes() == b"new\ncontent\n"
    assert desk.names() == ["notes.txt"]


async def test_a_file_that_was_not_read_is_not_replaced(desk: Desk) -> None:
    (desk.work / "notes.txt").write_bytes(b"old\n")

    outcome = await desk.call("write", path="notes.txt", content="new\n")

    assert outcome.error == ToolError(
        code="NOT_READ",
        reason="notes.txt exists and has not been read. Read it before writing it.",
    )
    assert (desk.work / "notes.txt").read_bytes() == b"old\n"


async def test_a_file_that_changed_since_it_was_read_is_not_replaced(desk: Desk) -> None:
    (desk.work / "notes.txt").write_bytes(b"old\n")
    await desk.have_read("notes.txt")
    (desk.work / "notes.txt").write_bytes(b"changed by someone else\n")

    outcome = await desk.call("write", path="notes.txt", content="new\n")

    assert outcome.error == ToolError(
        code="CHANGED_SINCE_READ",
        reason="notes.txt has changed since it was read. Read it again before writing it.",
    )
    assert (desk.work / "notes.txt").read_bytes() == b"changed by someone else\n"


async def test_what_was_written_can_be_written_again_without_reading_it(desk: Desk) -> None:
    await desk.call("write", path="notes.txt", content="first\n")

    outcome = await desk.call("write", path="notes.txt", content="second\n")

    assert outcome.error is None
    assert outcome.text == "Replaced notes.txt: 1 line."
    assert (desk.work / "notes.txt").read_bytes() == b"second\n"


# Where nothing is written


async def test_a_session_that_is_read_only_writes_nothing(desk: Desk) -> None:
    desk.sandbox.set_mode(desk.session, "read-only")

    outcome = await desk.call("write", path="notes.txt", content="new\n")

    assert outcome.error is not None
    assert outcome.error.code == "READ_ONLY"
    assert desk.names() == []


@pytest.mark.parametrize(
    ("path", "code"),
    [
        ("../outside.txt", "OUTSIDE_WORKING_DIRECTORY"),
        (".git/hooks/pre-commit", "PROTECTED"),
        (".columbina/permissions.yml", "PROTECTED"),
    ],
)
async def test_nothing_is_written_where_the_mode_does_not_allow_it(
    desk: Desk, path: str, code: str
) -> None:
    outcome = await desk.call("write", path=path, content="x")

    assert outcome.error is not None
    assert outcome.error.code == code
    assert desk.names() == []
    assert not (desk.work.parent / "outside.txt").exists()


async def test_a_directory_is_not_replaced_by_a_file(desk: Desk) -> None:
    (desk.work / "docs").mkdir()

    outcome = await desk.call("write", path="docs", content="x")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_A_FILE"
    assert (desk.work / "docs").is_dir()


async def test_a_file_that_cannot_be_written_is_an_error_that_says_why(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(*arguments: Any, **options: Any) -> None:
        raise OSError(28, "No space left on device")

    (desk.work / "notes.txt").write_bytes(b"old\n")
    await desk.have_read("notes.txt")
    monkeypatch.setattr(os, "replace", full)

    outcome = await desk.call("write", path="notes.txt", content="new\n")

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_ERROR"
    assert outcome.text == "notes.txt cannot be written: [Errno 28] No space left on device"
    assert (desk.work / "notes.txt").read_bytes() == b"old\n"
    assert desk.names() == ["notes.txt"]


async def test_a_path_that_leads_elsewhere_than_when_the_call_was_checked_is_not_written(
    desk: Desk,
) -> None:
    (desk.work / "drafts").mkdir()
    (desk.work / "final").mkdir()
    link_directory(desk.work / "door", desk.work / "drafts")

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(desk.work / "door")
        link_directory(desk.work / "door", desk.work / "final")
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    outcome = await desk.call("write", path="door/notes.txt", content="x")

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert desk.names("drafts") == []
    assert desk.names("final") == []


# What the checks and the model are told


async def test_a_write_says_which_file_it_acts_on_by_the_place_of_the_file(desk: Desk) -> None:
    seen: list[tuple[str, str | None]] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append((invocation.tool.effect, invocation.target))
        return None

    (desk.work / "docs").mkdir()
    desk.tools.guards.add(desk.ctx, "watching", watching)

    await desk.call("write", path="docs/../notes.txt", content="x")
    await desk.call("write", path=str(desk.work / "docs" / "new.txt"), content="x")

    assert seen == [("write", "notes.txt"), ("write", "docs/new.txt")]


def test_a_write_does_not_run_beside_other_calls(desk: Desk) -> None:
    assert not desk.begin("write", path="notes.txt", content="").concurrency_safe


def test_the_model_is_told_that_a_file_has_to_be_read_first(desk: Desk) -> None:
    schema = next(schema for schema in desk.tools.schemas() if schema.name == "write")

    assert "has to be read first" in schema.description
    assert schema.parameters["required"] == ["path", "content"]


# A write that is cut short


async def test_a_write_that_is_cancelled_does_not_end_before_its_thread_has(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    (desk.work / "notes.txt").write_bytes(b"old\n")
    await desk.have_read("notes.txt")
    held = HeldWrite(monkeypatch)
    writing = asyncio.ensure_future(desk.call("write", path="notes.txt", content="new\n"))
    assert await asyncio.to_thread(held.reached.wait, 20)

    writing.cancel()
    await asyncio.sleep(0.1)
    stayed = not writing.done()
    held.go_on.set()
    with pytest.raises(asyncio.CancelledError):
        await writing

    assert stayed
    assert held.ended.is_set()
    # The thread was told in time that nobody wants the file any more.
    assert (desk.work / "notes.txt").read_bytes() == b"old\n"
    assert desk.names() == ["notes.txt"]


async def test_a_write_that_is_cancelled_twice_still_stays_with_its_thread(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    held = HeldWrite(monkeypatch)
    writing = asyncio.ensure_future(desk.call("write", path="notes.txt", content="new\n"))
    assert await asyncio.to_thread(held.reached.wait, 20)

    writing.cancel()
    await asyncio.sleep(0.05)
    writing.cancel()
    await asyncio.sleep(0.05)
    stayed = not writing.done()
    held.go_on.set()
    with pytest.raises(asyncio.CancelledError):
        await writing

    assert stayed
    assert held.ended.is_set()
    assert desk.names() == []


async def test_a_write_whose_thread_does_not_end_is_let_go_of_and_writes_nothing_later(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    (desk.work / "notes.txt").write_bytes(b"old\n")
    await desk.have_read("notes.txt")
    held = HeldWrite(monkeypatch)
    monkeypatch.setattr(columbina.fs, "_LET_GO_SECONDS", 0.05)
    writing = asyncio.ensure_future(desk.call("write", path="notes.txt", content="late\n"))
    assert await asyncio.to_thread(held.reached.wait, 20)

    writing.cancel()
    with pytest.raises(asyncio.CancelledError):
        await writing
    # The thread is still there. Another call writes the file meanwhile.
    assert not held.ended.is_set()
    outcome = await desk.call("write", path="notes.txt", content="in time\n")
    assert outcome.error is None
    held.go_on.set()
    assert await asyncio.to_thread(held.ended.wait, 20)

    assert (desk.work / "notes.txt").read_bytes() == b"in time\n"
    assert desk.names() == ["notes.txt"]


async def test_a_write_that_fails_in_its_thread_fails_the_call(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    def failing(*arguments: Any) -> Any:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(bound, "replace", failing)

    outcome = await desk.call("write", path="notes.txt", content="new\n")

    assert outcome.error is not None
    assert outcome.text == "notes.txt cannot be written: [Errno 13] Permission denied"


# A working directory that does not stay where it was


def through_a_door(desk: Desk, door: Path, target: Path) -> Desk:
    """A desk whose working directory is a link, which leads to ``target`` for now."""
    target.mkdir()
    link_directory(door, target)
    session = desk.ctx.use(SESSIONS).create(desk.ctx, door)
    return Desk(desk.ctx, desk.tools, desk.fs, desk.sandbox, session, door)


async def test_a_working_directory_that_leads_elsewhere_than_at_first_is_not_written(
    desk: Desk, tmp_path: Path
) -> None:
    door, first, second = tmp_path / "door", tmp_path / "first", tmp_path / "second"
    second.mkdir()
    through = through_a_door(desk, door, first)
    assert (await through.call("write", path="a.txt", content="a")).error is None

    unlink_directory(door)
    link_directory(door, second)
    outcome = await through.call("write", path="b.txt", content="b")

    assert outcome.error is not None
    assert outcome.error.code == "WORKING_DIRECTORY_MOVED"
    assert list(second.iterdir()) == []
    assert (first / "a.txt").read_text() == "a"


async def test_a_working_directory_that_is_turned_while_a_first_call_is_checked_is_not_written(
    desk: Desk, tmp_path: Path
) -> None:
    door, first, second = tmp_path / "door", tmp_path / "first", tmp_path / "second"
    second.mkdir()
    through = through_a_door(desk, door, first)

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(door)
        link_directory(door, second)
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    outcome = await through.call("write", path="a.txt", content="a")

    assert outcome.error is not None
    assert outcome.error.code == "WORKING_DIRECTORY_MOVED"
    assert list(first.iterdir()) == []
    assert list(second.iterdir()) == []
