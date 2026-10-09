"""Where text goes that is too large for a conversation: files under the home of the program."""

import asyncio
import os
import stat
import sys
import time
from pathlib import Path

import pytest

import columbina.fs
import columbina.session
import columbina.spill.local
from columbina.builtin_tools import read
from columbina.fs import FS, Access, locate
from columbina.kernel import Context
from columbina.session import SESSIONS, Session
from columbina.session.store import workspace_key
from columbina.spill import SPILL, Saved, SpillStore
from columbina.spill.local import FOLD_AT, Local, sweep

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


def test_a_file_can_be_closed_twice_and_not_written_after(
    store: Local, session: Session, work: Path
) -> None:
    file = store.open(session, work, "out.txt")
    file.write(b"one")

    first = file.close()

    assert file.close() == first
    with pytest.raises(ValueError, match="closed"):
        file.write(b"two")


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
