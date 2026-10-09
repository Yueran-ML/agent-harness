"""Where text goes that is too large for a conversation: files under the home of the program."""

import asyncio
import itertools
import json
import os
import stat
import sys
import time
from pathlib import Path

import pytest

import columbina.fs
import columbina.session
import columbina.spill.local
import columbina.tools
from columbina.builtin_tools import read
from columbina.fs import FS, Access, locate
from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.session import SESSIONS, Session
from columbina.session.store import workspace_key
from columbina.spill import SPILL, Saved, SpillStore
from columbina.spill.local import FOLD_AT, Fold, Local, sweep
from columbina.tools import TOOLS, ToolRun

DAY = 24 * 60 * 60


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    return directory


@pytest.fixture
async def session(ctx: Context, work: Path) -> Session:
    await ctx.plugin(columbina.session)
    return ctx.use(SESSIONS).create(ctx, work, session_id="s-1")


@pytest.fixture
def store(tmp_path: Path) -> Local:
    return Local(tmp_path / "home" / "spill")


def kept(saved: Saved) -> str:
    return Path(saved.locator).read_text(encoding="utf-8")


# Saving a text


def test_a_text_is_saved_under_the_working_directory_and_the_session(
    store: Local, session: Session, work: Path, tmp_path: Path
) -> None:
    saved = store.save_text(session, work, "grep-result.txt", "one\ntwo\n")

    where = Path(saved.locator)
    assert where.parent == tmp_path / "home" / "spill" / workspace_key(work) / "s-1"
    assert where.name.endswith("-grep-result.txt")
    assert len(where.name) == len("0123abcd-grep-result.txt")
    assert kept(saved) == "one\ntwo\n"
    assert saved.bytes == 8
    assert where.is_absolute()


def test_what_is_told_about_a_saved_text_says_how_to_read_it(
    store: Local, session: Session, work: Path
) -> None:
    saved = store.save_text(session, work, "out.txt", "one\n")

    assert saved.hint == (
        "Read a part of it with the read tool, by offset and limit, or search it with grep."
    )


def test_two_texts_of_the_same_name_are_two_files(
    store: Local, session: Session, work: Path
) -> None:
    first = store.save_text(session, work, "out.txt", "first")
    second = store.save_text(session, work, "out.txt", "second")

    assert first.locator != second.locator
    assert (kept(first), kept(second)) == ("first", "second")


def test_a_file_that_is_there_is_never_written_over(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The names are drawn at random. Should one come up twice, another one is drawn.
    drawn = iter(["aaaaaaaa", "aaaaaaaa", "bbbbbbbb"])
    monkeypatch.setattr("columbina.spill.local._drawn", lambda: next(drawn))

    first = store.save_text(session, work, "out.txt", "first")
    second = store.save_text(session, work, "out.txt", "second")

    assert Path(first.locator).name == "aaaaaaaa-out.txt"
    assert Path(second.locator).name == "bbbbbbbb-out.txt"
    assert kept(first) == "first"


def test_names_that_never_stop_coming_up_twice_are_an_error(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("columbina.spill.local._drawn", lambda: "aaaaaaaa")
    store.save_text(session, work, "out.txt", "first")

    with pytest.raises(OSError, match="no free name"):
        store.save_text(session, work, "out.txt", "second")


@pytest.mark.parametrize(
    ("given", "used"),
    [
        ("../../evil.txt", ".._.._evil.txt"),
        ("a b/c:d.txt", "a_b_c_d.txt"),
        ("", "output"),
        ("...", "output"),
        ("x" * 200, "x" * 80),
        ("CON", "CON"),
    ],
)
def test_the_name_that_is_asked_for_is_made_fit_for_a_file(
    store: Local, session: Session, work: Path, given: str, used: str
) -> None:
    saved = store.save_text(session, work, given, "text")

    where = Path(saved.locator)
    # A few random letters stand before it, so no name of a device or of a directory is left.
    assert where.name[9:] == used
    assert where.parent.name == "s-1"


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such permissions")
def test_only_the_owner_gets_at_what_is_saved(store: Local, session: Session, work: Path) -> None:
    saved = store.save_text(session, work, "out.txt", "secret")

    where = Path(saved.locator)
    assert stat.S_IMODE(where.stat().st_mode) == 0o600
    assert stat.S_IMODE(where.parent.stat().st_mode) == 0o700
    assert stat.S_IMODE(where.parent.parent.stat().st_mode) == 0o700
    assert stat.S_IMODE(store.root.stat().st_mode) == 0o700


def test_what_is_made_for_a_saved_text_is_asked_for_its_owner_alone(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A directory that is made together with those above it is the only one of them to
    # get the mode that is asked for, so each is made by itself. Not every system keeps
    # such modes. What it is asked for can be seen on all of them.
    made: dict[Path, int] = {}
    opened: dict[Path, int] = {}
    really_make = os.mkdir
    really_open = os.open

    def making(path: str | Path, mode: int = 0o777) -> None:
        really_make(path, mode)
        made[Path(path)] = mode

    def opening(path: str | Path, flags: int, mode: int = 0o777) -> int:
        descriptor = really_open(path, flags, mode)
        opened[Path(path)] = mode
        return descriptor

    monkeypatch.setattr(os, "mkdir", making)
    monkeypatch.setattr(os, "open", opening)

    saved = store.save_text(session, work, "out.txt", "secret")

    monkeypatch.undo()
    where = Path(saved.locator)
    assert made == {
        # What lies above the root is not for this program to decide.
        store.root.parent: 0o777,
        store.root: 0o700,
        where.parent.parent: 0o700,
        where.parent: 0o700,
    }
    assert opened == {where: 0o600}


def test_a_character_that_cannot_be_written_is_replaced(
    store: Local, session: Session, work: Path
) -> None:
    saved = store.save_text(session, work, "out.txt", f"a{chr(0xD800)}b")

    assert kept(saved) == "a?b"


def test_a_text_that_cannot_be_written_whole_leaves_no_file(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(descriptor: int, data: memoryview) -> int:
        raise OSError(28, "No space left on device")

    monkeypatch.setattr("columbina.spill.local._put", full)

    with pytest.raises(OSError, match="No space left"):
        store.save_text(session, work, "out.txt", "text")

    monkeypatch.undo()
    assert list((store.directory(work) / "s-1").iterdir()) == []


def test_a_disk_that_takes_a_piece_in_parts_is_given_all_of_it(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def a_little(descriptor: int, data: memoryview) -> int:
        return os.write(descriptor, data[:3])

    monkeypatch.setattr("columbina.spill.local._put", a_little)

    saved = store.save_text(session, work, "out.txt", "0123456789")

    monkeypatch.undo()
    assert kept(saved) == "0123456789"
    assert saved.bytes == 10


# Long lines


def test_a_line_that_the_read_tool_could_not_show_whole_is_broken(
    store: Local, session: Session, work: Path
) -> None:
    # The tool shows so many characters of a line and counts the rest. What is saved is
    # read with that tool, so the rest of a long line would be out of reach.
    assert FOLD_AT == read.MAX_LINE_CHARACTERS
    long = "x" * (2 * FOLD_AT + 5)
    text = f"short\n{long}\nlast"

    saved = store.save_text(session, work, "out.txt", text)

    lines = kept(saved).split("\n")
    assert [len(line) for line in lines] == [5, FOLD_AT, FOLD_AT, 5, 4]
    assert "".join(lines[1:4]) == long
    assert saved.hint.endswith(f" Lines of more than {FOLD_AT} characters are broken there.")


def test_a_line_of_exactly_that_length_stays_whole(
    store: Local, session: Session, work: Path
) -> None:
    text = "x" * FOLD_AT + "\n"

    saved = store.save_text(session, work, "out.txt", text)

    assert kept(saved) == text
    assert "broken" not in saved.hint


def test_a_carriage_return_before_a_line_break_does_not_make_a_line_too_long(
    store: Local, session: Session, work: Path
) -> None:
    # The read tool takes it for a part of the line break, and shows the line whole.
    text = ("x" * FOLD_AT + "\r\n") * 2

    saved = store.save_text(session, work, "out.txt", text)

    assert Path(saved.locator).read_bytes() == text.encode()
    assert "broken" not in saved.hint


def test_a_carriage_return_that_no_line_break_follows_counts_like_any_character(
    store: Local, session: Session, work: Path
) -> None:
    full = "x" * FOLD_AT
    text = f"{full}\ry\n{full}\r\r\n"

    saved = store.save_text(session, work, "out.txt", text)

    assert Path(saved.locator).read_bytes() == f"{full}\r\ny\n{full}\r\n\r\n".encode()
    assert "broken" in saved.hint


def broken_one_by_one(text: str, at: int) -> str:
    """What the rule says, a character at a time: a break before each that is one too many."""
    out: list[str] = []
    length = 0
    for character in text:
        if character == "\n":
            length = 0
        elif length < at or (length == at and character == "\r"):
            length += 1
        else:
            out.append("\n")
            length = 1
        out.append(character)
    return "".join(out)


def test_the_same_breaks_are_made_however_a_text_is_cut(monkeypatch: pytest.MonkeyPatch) -> None:
    # With lines of three characters, every text of up to eight is tried, cut in every place.
    monkeypatch.setattr("columbina.spill.local.FOLD_AT", 3)
    for size in range(9):
        for characters in itertools.product("a\r\n", repeat=size):
            text = "".join(characters)
            expected = broken_one_by_one(text, 3)
            for line in expected.split("\n"):
                assert len(line) <= 3 or (len(line) == 4 and line.endswith("\r")), text
            for cut in range(size + 1):
                fold = Fold()
                assert fold(text[:cut]) + fold(text[cut:]) == expected, (text, cut)
                assert fold.broken is (expected != text), (text, cut)


def test_a_text_that_comes_a_character_at_a_time_is_broken_in_the_same_places(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("columbina.spill.local.FOLD_AT", 3)
    for characters in itertools.product("a\r\n", repeat=7):
        text = "".join(characters)
        fold = Fold()

        assert "".join(fold(character) for character in text) == broken_one_by_one(text, 3)


# Writing as it comes


def test_what_comes_in_pieces_is_written_as_it_comes(
    store: Local, session: Session, work: Path
) -> None:
    file = store.open(session, work, "powershell-output.txt")
    file.write(b"one\n")
    during = Path(file.locator).read_bytes()
    file.write(b"two\xff\n")

    saved = file.close()

    assert during == b"one\n"
    assert Path(saved.locator).read_bytes() == b"one\ntwo\xff\n"
    assert saved.bytes == 9
    assert saved.locator == file.locator
    assert Path(saved.locator).name.endswith("-powershell-output.txt")
    assert "broken" not in saved.hint


def test_a_long_line_that_comes_in_pieces_is_broken_like_one_that_comes_whole(
    store: Local, session: Session, work: Path
) -> None:
    moon = chr(0x6708)
    text = f"short\r\n{moon * (2 * FOLD_AT + 5)}\r\nlast"
    whole = store.save_text(session, work, "out.txt", text)
    data = text.encode()

    file = store.open(session, work, "out.txt")
    # Seven bytes at a time: most pieces end in the middle of a character.
    for start in range(0, len(data), 7):
        file.write(data[start : start + 7])
    saved = file.close()

    written = Path(saved.locator).read_bytes()
    assert written == Path(whole.locator).read_bytes()
    assert [len(line) for line in written.decode().split("\n")] == [6, FOLD_AT, FOLD_AT, 6, 4]
    assert saved.bytes == len(written) == len(data) + 2
    assert saved.hint == whole.hint
    assert saved.hint.endswith(f" Lines of more than {FOLD_AT} characters are broken there.")


def test_a_character_that_comes_in_two_pieces_is_one_character(
    store: Local, session: Session, work: Path
) -> None:
    data = (chr(0x6708) * FOLD_AT).encode()

    file = store.open(session, work, "out.txt")
    file.write(data[:1000])
    file.write(data[1000:])
    saved = file.close()

    # As many characters as a line may have, so no break, though they are thrice the bytes.
    assert len(data[:1000].decode(errors="ignore").encode()) < 1000
    assert Path(saved.locator).read_bytes() == data
    assert "broken" not in saved.hint


def test_a_byte_that_is_no_text_is_written_as_it_came_and_counts_as_a_character(
    store: Local, session: Session, work: Path
) -> None:
    file = store.open(session, work, "out.txt")
    file.write(b"\xff" * (FOLD_AT + 1) + b"\n\xe6\x9c")
    during = Path(file.locator).read_bytes()

    saved = file.close()

    assert during == b"\xff" * FOLD_AT + b"\n\xff\n"
    # Half a character at the very end is no text either. It is written when nothing can follow.
    assert Path(saved.locator).read_bytes() == during + b"\xe6\x9c"
    assert saved.bytes == FOLD_AT + 5
    assert "broken" in saved.hint


def test_a_file_can_be_closed_twice_and_not_written_after(
    store: Local, session: Session, work: Path
) -> None:
    file = store.open(session, work, "out.txt")
    file.write(b"one")

    first = file.close()

    assert file.close() == first
    with pytest.raises(ValueError, match="closed"):
        file.write(b"two")


def test_a_file_whose_end_cannot_be_written_is_closed_all_the_same(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(descriptor: int, data: memoryview) -> int:
        raise OSError(28, "No space left on device")

    closed: list[int] = []
    really_close = os.close

    def closing(descriptor: int) -> None:
        closed.append(descriptor)
        really_close(descriptor)

    file = store.open(session, work, "out.txt")
    file.write(b"one\xe6")
    monkeypatch.setattr("columbina.spill.local._put", full)
    monkeypatch.setattr(os, "close", closing)

    with pytest.raises(OSError, match="No space left"):
        file.close()
    again = file.close()

    monkeypatch.undo()
    # Closed once: the number of a file that was given back may be that of another by now.
    assert len(closed) == 1
    assert again.bytes == 3
    assert Path(again.locator).read_bytes() == b"one"
    with pytest.raises(ValueError, match="closed"):
        file.write(b"two")


def test_a_text_whose_file_cannot_be_closed_leaves_no_file(
    store: Local, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    closed: list[int] = []
    really_close = os.close

    def failing(descriptor: int) -> None:
        closed.append(descriptor)
        really_close(descriptor)
        raise OSError(5, "Input/output error")

    monkeypatch.setattr(os, "close", failing)

    with pytest.raises(OSError, match="Input/output error"):
        store.save_text(session, work, "out.txt", "text")

    monkeypatch.undo()
    assert list((store.directory(work) / "s-1").iterdir()) == []
    # Closed once, although the system said that it failed: the number is given back by then.
    assert len(closed) == 1


# Reading it


async def seam_with(ctx: Context, root: Path) -> SpillStore:
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.spill.local, {"root": root})
    return ctx.use(SPILL)


async def test_the_model_may_read_what_was_saved_for_its_working_directory(
    ctx: Context, session: Session, work: Path, tmp_path: Path
) -> None:
    store = await seam_with(ctx, tmp_path / "home" / "spill")
    saved = store.save_text(session, work, "out.txt", "text")
    seam = ctx.use(FS)

    def judged(path: str, kind: str = "read") -> str | None:
        access = Access(session, work, locate(work, path), kind, "read")  # pyright: ignore[reportArgumentType]
        refusal = seam.judge(access)
        return None if refusal is None else refusal.code

    assert judged(saved.locator) is None
    assert judged(str(Path(saved.locator).parent)) is None
    # The directory of the working directory itself can be listed and searched.
    assert judged(str(Path(saved.locator).parent.parent)) is None
    assert judged(saved.locator, "write") == "OUTSIDE_WORKING_DIRECTORY"


async def test_the_read_tool_shows_every_character_of_what_was_saved(
    ctx: Context, session: Session, work: Path, tmp_path: Path
) -> None:
    # This is what the breaks are for. The lines end the way Windows ends them.
    store = await seam_with(ctx, tmp_path / "home" / "spill")
    await ctx.plugin(columbina.tools)
    await ctx.plugin(read)
    full, long = "x" * FOLD_AT, "y" * (2 * FOLD_AT + 500)
    file = store.open(session, work, "powershell-output.txt")
    file.write(f"{full}\r\n{long}\r\nlast\r\n".encode())
    saved = file.close()

    asked = ToolCallBlock(call_id="c", name="read", arguments=json.dumps({"path": saved.locator}))
    outcome = await ctx.use(TOOLS).run(ctx, asked, ToolRun("c", work, session))

    assert outcome.error is None, outcome.text
    shown = [line.split("\t", 1)[1] for line in outcome.text.split("\n")]
    assert shown == [full, long[:FOLD_AT], long[FOLD_AT : 2 * FOLD_AT], long[2 * FOLD_AT :], "last"]
    assert "more characters" not in outcome.text


async def test_what_was_saved_for_another_working_directory_is_out_of_reach(
    ctx: Context, session: Session, work: Path, tmp_path: Path
) -> None:
    store = await seam_with(ctx, tmp_path / "home" / "spill")
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    saved = store.save_text(session, elsewhere, "out.txt", "theirs")
    seam = ctx.use(FS)

    access = Access(session, work, locate(work, saved.locator), "read", "read")

    refusal = seam.judge(access)
    assert refusal is not None
    assert refusal.code == "OUTSIDE_WORKING_DIRECTORY"
    # Nor the directory that holds what was saved for everybody.
    above = Access(session, work, locate(work, str(tmp_path / "home" / "spill")), "read", "glob")
    assert seam.judge(above) is not None


async def test_without_a_seam_of_the_file_system_text_is_saved_all_the_same(
    ctx: Context, session: Session, work: Path, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.spill.local, {"root": tmp_path / "home" / "spill"})

    saved = ctx.use(SPILL).save_text(session, work, "out.txt", "text")

    assert kept(saved) == "text"


# Old files


def aged(path: Path, days: float) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("old", encoding="utf-8")
    then = time.time() - days * DAY
    os.utime(path, (then, then))
    return path


def test_files_that_are_older_than_they_are_kept_are_removed(tmp_path: Path) -> None:
    root = tmp_path / "spill"
    old = aged(root / "ws" / "s-1" / "aaaa-out.txt", 31)
    fresh = aged(root / "ws" / "s-1" / "bbbb-out.txt", 29)
    alone = aged(root / "ws" / "s-2" / "cccc-out.txt", 40)

    removed = sweep(root, keep_days=30)

    assert removed == 2
    assert not old.exists()
    assert fresh.exists()
    # A directory that is left empty goes too, and one that still holds something stays.
    assert not alone.parent.exists()
    assert fresh.parent.exists()


def test_a_working_directory_without_anything_left_goes_too(tmp_path: Path) -> None:
    root = tmp_path / "spill"
    aged(root / "ws" / "s-1" / "aaaa-out.txt", 31)

    sweep(root, keep_days=30)

    assert not (root / "ws").exists()
    assert root.exists()


def test_where_nothing_was_ever_saved_nothing_is_removed(tmp_path: Path) -> None:
    assert sweep(tmp_path / "never", keep_days=30) == 0


def test_only_what_lies_where_saved_text_lies_is_removed(tmp_path: Path) -> None:
    # Whatever else somebody put into the directory is none of the business of this.
    root = tmp_path / "spill"
    stray = aged(root / "notes.txt", 400)
    shallow = aged(root / "ws" / "notes.txt", 400)
    deep = aged(root / "ws" / "s-1" / "more" / "notes.txt", 400)

    assert sweep(root, keep_days=30) == 0
    assert stray.exists()
    assert shallow.exists()
    assert deep.exists()


def test_a_link_is_not_followed(tmp_path: Path) -> None:
    root = tmp_path / "spill"
    outside = aged(tmp_path / "outside" / "precious.txt", 400)
    (root / "ws").mkdir(parents=True)
    try:
        (root / "ws" / "s-1").symlink_to(outside.parent, target_is_directory=True)
    except OSError:
        pytest.skip("this user cannot make links")

    assert sweep(root, keep_days=30) == 0
    assert outside.exists()


async def test_old_files_are_removed_when_the_program_starts(ctx: Context, tmp_path: Path) -> None:
    root = tmp_path / "home" / "spill"
    old = aged(root / "ws" / "s-1" / "aaaa-out.txt", 31)
    fresh = aged(root / "ws" / "s-1" / "bbbb-out.txt", 1)

    await ctx.plugin(columbina.spill.local, {"root": root, "keep_days": 30})
    async with asyncio.timeout(5):
        while old.exists():  # noqa: ASYNC110 - the removal is done in a thread, which sets no event
            await asyncio.sleep(0.01)

    assert fresh.exists()


async def test_old_files_that_cannot_be_looked_through_do_not_keep_the_program_from_starting(
    ctx: Context,
    session: Session,
    work: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    def broken(root: Path, *, keep_days: float) -> int:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr("columbina.spill.local.sweep", broken)
    root = tmp_path / "home" / "spill"

    await ctx.plugin(columbina.spill.local, {"root": root})
    async with asyncio.timeout(5):
        while "could not be looked through" not in caplog.text:  # noqa: ASYNC110 - as above
            await asyncio.sleep(0.01)

    assert kept(ctx.use(SPILL).save_text(session, work, "out.txt", "text")) == "text"


async def test_a_program_that_ends_at_once_does_not_wait_for_the_old_files(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.spill.local, {"root": tmp_path / "home" / "spill"})

    await ctx.fiber.dispose()


async def test_files_are_kept_for_thirty_days_unless_the_config_says_otherwise(
    ctx: Context, tmp_path: Path
) -> None:
    assert columbina.spill.local.Config(root=tmp_path).keep_days == 30
