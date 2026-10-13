"""The edit tool: new text in the place of a piece of a file."""

import asyncio
import os
from pathlib import Path
from typing import Any

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.builtin_tools import edit
from columbina.builtin_tools.edit import EditArgs
from columbina.fs import Access, Fingerprint, Permit, locate
from columbina.kernel import Context
from columbina.session import ToolError
from columbina.tools import Deny, Invocation, ToolFailure
from support.desk import Desk
from support.held import HeldWrite
from support.links import link_directory, unlink_directory


async def shown(desk: Desk, path: str, content: bytes) -> Path:
    """Put a file on the desk and have the model read it."""
    file = desk.work / path
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_bytes(content)
    await desk.have_read(path)
    return file


# The piece that is replaced


async def test_the_one_place_that_matches_is_replaced(desk: Desk) -> None:
    file = await shown(desk, "calc.py", b"def add(a, b):\n    return a - b\n")

    outcome = await desk.call("edit", path="calc.py", old="a - b", new="a + b")

    assert outcome.error is None
    assert outcome.text == "Replaced 1 occurrence in calc.py."
    assert file.read_bytes() == b"def add(a, b):\n    return a + b\n"
    assert desk.names() == ["calc.py"]


async def test_a_piece_of_several_lines_is_replaced(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\ntwo\nthree\nfour\n")

    outcome = await desk.call("edit", path="a.txt", old="two\nthree\n", new="2\n")

    assert outcome.error is None
    assert file.read_bytes() == b"one\n2\nfour\n"


async def test_a_piece_can_be_taken_out(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"keep this, drop this, keep that\n")

    await desk.call("edit", path="a.txt", old=" drop this,", new="")

    assert file.read_bytes() == b"keep this, keep that\n"


async def test_text_that_is_not_in_the_file_changes_nothing(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\ntwo\n")

    outcome = await desk.call("edit", path="a.txt", old="three", new="3")

    assert outcome.error is not None
    assert outcome.error.code == "NO_MATCH"
    assert "does not occur in a.txt" in outcome.text
    assert file.read_bytes() == b"one\ntwo\n"


async def test_text_that_is_there_more_than_once_is_not_replaced_by_guessing(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"x = 1\ny = 1\nz = 1\n")

    outcome = await desk.call("edit", path="a.txt", old="= 1", new="= 2")

    assert outcome.error is not None
    assert outcome.error.code == "AMBIGUOUS"
    assert "occurs 3 times in a.txt" in outcome.text
    assert file.read_bytes() == b"x = 1\ny = 1\nz = 1\n"


async def test_every_place_is_replaced_if_that_is_asked_for(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"x = 1\ny = 1\nz = 1\n")

    outcome = await desk.call("edit", path="a.txt", old="= 1", new="= 2", replace_all=True)

    assert outcome.error is None
    assert outcome.text == "Replaced 3 occurrences in a.txt."
    assert file.read_bytes() == b"x = 2\ny = 2\nz = 2\n"


async def test_the_same_text_for_old_and_new_is_no_edit(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\n")

    outcome = await desk.call("edit", path="a.txt", old="one", new="one")

    assert outcome.error == ToolError(
        code="NO_CHANGE",
        reason="The old text and the new text are the same, so there is nothing to change.",
    )
    assert file.read_bytes() == b"one\n"


async def test_no_old_text_is_no_edit(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\n")

    outcome = await desk.call("edit", path="a.txt", old="", new="zero\n")

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"
    assert file.read_bytes() == b"one\n"


async def test_characters_of_several_bytes_are_matched_and_written_whole(desk: Desk) -> None:
    file = await shown(desk, "a.txt", "名字是旧\n".encode())

    await desk.call("edit", path="a.txt", old="旧", new="新的")

    assert file.read_bytes() == "名字是新的\n".encode()


@settings(max_examples=80, deadline=None)
@given(
    before=st.text(alphabet="ab \n", max_size=30),
    old=st.text(alphabet="ab \n", min_size=1, max_size=4),
    after=st.text(alphabet="ab \n", max_size=30),
    new=st.text(alphabet="abc \n", max_size=6),
    replace_all=st.booleans(),
)
def test_an_edit_does_what_replacing_in_the_text_would(
    before: str, old: str, after: str, new: str, replace_all: bool
) -> None:
    text = before + old + after
    args = EditArgs(path="a.txt", old=old, new=new, replace_all=replace_all)
    change: Any = vars(edit)["_change"]
    count = text.count(old)

    if count > 1 and not replace_all:
        with pytest.raises(ToolFailure) as refused:
            change(text, args)
        assert refused.value.code == "AMBIGUOUS"
    else:
        assert change(text, args) == (text.replace(old, new), count)


async def test_new_text_that_cannot_be_stored_as_text_is_refused(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\n")

    outcome = await desk.call("edit", path="a.txt", old="one", new="1" + chr(0xD83D))

    # The pipeline does not take such arguments, so the tool never sees them.
    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"
    assert file.read_bytes() == b"one\n"


# The line breaks of the file


async def test_a_file_with_the_line_breaks_of_windows_keeps_them(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\r\ntwo\r\nthree\r\n")

    outcome = await desk.call("edit", path="a.txt", old="one\ntwo\n", new="1\n2\n2.5\n")

    assert outcome.error is None
    assert file.read_bytes() == b"1\r\n2\r\n2.5\r\nthree\r\n"


async def test_new_lines_in_such_a_file_end_like_its_other_lines(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\r\ntwo\r\n")

    await desk.call("edit", path="a.txt", old="two", new="two\nthree")

    assert file.read_bytes() == b"one\r\ntwo\r\nthree\r\n"


async def test_old_text_with_the_line_breaks_of_windows_matches_such_a_file_too(
    desk: Desk,
) -> None:
    file = await shown(desk, "a.txt", b"one\r\ntwo\r\n")

    await desk.call("edit", path="a.txt", old="one\r\ntwo", new="1\r\n2")

    assert file.read_bytes() == b"1\r\n2\r\n"


async def test_a_file_with_plain_line_feeds_keeps_them(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\ntwo\n")

    await desk.call("edit", path="a.txt", old="two", new="two\nthree")

    assert file.read_bytes() == b"one\ntwo\nthree\n"


async def test_a_file_with_both_kinds_of_line_breaks_is_matched_as_it_is(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\r\ntwo\nthree\n")

    missed = await desk.call("edit", path="a.txt", old="one\ntwo", new="x")
    hit = await desk.call("edit", path="a.txt", old="two\nthree", new="2\n3")

    assert missed.error is not None
    assert missed.error.code == "NO_MATCH"
    assert hit.error is None
    assert file.read_bytes() == b"one\r\n2\n3\n"


async def test_a_mark_at_the_beginning_of_a_file_stays_there(desk: Desk) -> None:
    mark = bytes([0xEF, 0xBB, 0xBF])
    file = await shown(desk, "a.txt", mark + b"one\n")

    await desk.call("edit", path="a.txt", old="one", new="two")

    assert file.read_bytes() == mark + b"two\n"


# Files that are not edited


async def test_a_file_that_is_not_there_is_not_edited(desk: Desk) -> None:
    outcome = await desk.call("edit", path="nowhere.txt", old="a", new="b")

    assert outcome.error == ToolError(
        code="NOT_FOUND", reason="There is no file nowhere.txt. To make one, write it."
    )
    assert desk.names() == []


@pytest.mark.parametrize(
    "content", [bytes([0x66, 0xFF, 0xFE, 0x0A]), bytes([0x61, 0x00, 0x62])], ids=["bytes", "nul"]
)
async def test_a_file_that_is_no_text_is_not_edited(desk: Desk, content: bytes) -> None:
    file = desk.work / "blob.bin"
    file.write_bytes(content)
    # No read shows such a file to the model, so the seam is told for this test.
    status = file.stat()
    where = locate(desk.work, "blob.bin")
    permit = desk.fs.permit(Access(desk.session, desk.work, where, "read", "read"))
    desk.fs.shown(permit, Fingerprint(status.st_size, status.st_mtime_ns))

    outcome = await desk.call("edit", path="blob.bin", old="a", new="b")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_TEXT"
    assert file.read_bytes() == content


async def test_a_file_that_was_not_read_is_not_edited_whatever_the_old_text(desk: Desk) -> None:
    file = desk.work / "a.txt"
    file.write_bytes(b"one\n")

    outcome = await desk.call("edit", path="a.txt", old="no such text", new="x")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_READ"
    assert file.read_bytes() == b"one\n"


async def test_what_an_edit_wrote_is_remembered_under_its_own_call(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one two\n")
    first = await desk.call("edit", path="a.txt", old="one", new="1")
    assert first.error is None
    # The edit showed the model the file as it is now, so the reading before it no longer
    # counts for anything. With the edit gone from the conversation, the file is not known.
    assert desk.fs.unshown(desk.session, {"call-1"}) == 0
    assert desk.fs.unshown(desk.session, {"call-2"}) == 1

    second = await desk.call("edit", path="a.txt", old="two", new="2")

    assert second.error is not None
    assert second.error.code == "NOT_READ"
    assert "has since been taken out of the conversation" in second.error.reason
    assert file.read_bytes() == b"1 two\n"


async def test_a_file_that_changed_since_it_was_read_is_not_edited(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\n")
    file.write_bytes(b"one, and more\n")

    outcome = await desk.call("edit", path="a.txt", old="one", new="1")

    assert outcome.error is not None
    assert outcome.error.code == "CHANGED_SINCE_READ"
    assert file.read_bytes() == b"one, and more\n"


async def test_a_file_can_be_edited_again_without_reading_it_again(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one two three\n")

    await desk.call("edit", path="a.txt", old="one", new="1")
    outcome = await desk.call("edit", path="a.txt", old="two", new="2")

    assert outcome.error is None
    assert file.read_bytes() == b"1 2 three\n"


async def test_a_file_that_is_too_large_is_not_edited(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    file = await shown(desk, "a.txt", b"0123456789\n")
    monkeypatch.setattr(edit, "_MAX_BYTES", 10)

    outcome = await desk.call("edit", path="a.txt", old="0", new="zero")

    assert outcome.error is not None
    assert outcome.error.code == "TOO_LARGE"
    assert file.read_bytes() == b"0123456789\n"


async def test_a_directory_is_not_edited(desk: Desk) -> None:
    (desk.work / "docs").mkdir()

    outcome = await desk.call("edit", path="docs", old="a", new="b")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_A_FILE"


async def test_a_session_that_is_read_only_edits_nothing(desk: Desk) -> None:
    file = await shown(desk, "a.txt", b"one\n")
    desk.sandbox.set_mode(desk.session, "read-only")

    outcome = await desk.call("edit", path="a.txt", old="one", new="1")

    assert outcome.error is not None
    assert outcome.error.code == "READ_ONLY"
    assert file.read_bytes() == b"one\n"


async def test_a_file_outside_the_working_directory_is_not_edited(desk: Desk) -> None:
    outside = desk.work.parent / "outside.txt"
    outside.write_bytes(b"one\n")

    outcome = await desk.call("edit", path="../outside.txt", old="one", new="1")

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert outside.read_bytes() == b"one\n"


async def test_a_file_that_cannot_be_written_back_is_an_error_that_says_why(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(*arguments: Any, **options: Any) -> None:
        raise OSError(28, "No space left on device")

    file = await shown(desk, "a.txt", b"one\n")
    monkeypatch.setattr(os, "replace", full)

    outcome = await desk.call("edit", path="a.txt", old="one", new="1")

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_ERROR"
    assert outcome.text == "a.txt cannot be edited: [Errno 28] No space left on device"
    assert file.read_bytes() == b"one\n"


async def test_a_path_that_leads_elsewhere_than_when_the_call_was_checked_is_not_edited(
    desk: Desk,
) -> None:
    await shown(desk, "drafts/a.txt", b"one\n")
    final = await shown(desk, "final/a.txt", b"one\n")
    link_directory(desk.work / "door", desk.work / "drafts")

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(desk.work / "door")
        link_directory(desk.work / "door", desk.work / "final")
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    outcome = await desk.call("edit", path="door/a.txt", old="one", new="1")

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert final.read_bytes() == b"one\n"


# What the checks and the model are told


async def test_an_edit_says_which_file_it_acts_on_by_the_place_of_the_file(desk: Desk) -> None:
    seen: list[tuple[str, str | None]] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append((invocation.tool.effect, invocation.target))
        return None

    await shown(desk, "docs/a.txt", b"one\n")
    desk.tools.guards.add(desk.ctx, "watching", watching)

    await desk.call("edit", path="docs/./a.txt", old="one", new="1")

    assert seen == [("write", "docs/a.txt")]


def test_an_edit_does_not_run_beside_other_calls(desk: Desk) -> None:
    assert not desk.begin("edit", path="a.txt", old="a", new="b").concurrency_safe


def test_the_model_is_told_what_an_edit_needs(desk: Desk) -> None:
    schema = next(schema for schema in desk.tools.schemas() if schema.name == "edit")

    assert "has to be read first" in schema.description
    assert schema.parameters["required"] == ["path", "old", "new"]


# An edit that is cut short


async def test_an_edit_that_is_cancelled_does_not_end_before_its_thread_has(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    (desk.work / "notes.txt").write_bytes(b"one\ntwo\n")
    await desk.have_read("notes.txt")
    held = HeldWrite(monkeypatch)
    editing = asyncio.ensure_future(desk.call("edit", path="notes.txt", old="two", new="three"))
    assert await asyncio.to_thread(held.reached.wait, 20)

    editing.cancel()
    await asyncio.sleep(0.1)
    stayed = not editing.done()
    held.go_on.set()
    with pytest.raises(asyncio.CancelledError):
        await editing

    assert stayed
    assert held.ended.is_set()
    assert (desk.work / "notes.txt").read_bytes() == b"one\ntwo\n"
    assert desk.names() == ["notes.txt"]


async def test_an_edit_puts_its_text_in_the_place_of_the_text_it_was_made_from_and_no_other(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    (desk.work / "notes.txt").write_bytes(b"one\ntwo\n")
    await desk.have_read("notes.txt")
    made_from: list[object] = []
    really_replace: Any = Permit.replace

    def watching(permit: Permit, data: bytes, **options: Any) -> Any:
        made_from.append(options["made_from"])
        return really_replace(permit, data, **options)

    monkeypatch.setattr(Permit, "replace", watching)
    before = (desk.work / "notes.txt").stat()

    outcome = await desk.call("edit", path="notes.txt", old="two", new="three")

    assert outcome.error is None
    assert made_from == [Fingerprint(before.st_size, before.st_mtime_ns)]
