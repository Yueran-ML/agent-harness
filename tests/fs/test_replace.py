"""Replacing a file through the seam: what the model was shown, where it was judged."""

import os
import shutil
import stat
import sys
from pathlib import Path
from typing import Any

import pytest

import columbina.fs
from columbina.fs import FS, Access, Fingerprint, Fs, Permit, bound, locate
from columbina.kernel import Context
from columbina.session import CORE_EVENTS, Session
from columbina.tools import ToolFailure
from support.links import link_directory


@pytest.fixture
async def fs(ctx: Context) -> Fs:
    await ctx.plugin(columbina.fs)
    seam = ctx.use(FS)

    def inside(access: Access) -> bool:
        return access.where.inside

    seam.grants.add(ctx, "inside", inside)
    return seam


@pytest.fixture
def session(ctx: Context) -> Session:
    return Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "a.txt").write_bytes(b"old")
    return directory


class Hand:
    """Reads and writes the files of one session the way a tool does."""

    def __init__(self, fs: Fs, session: Session, work: Path) -> None:
        self.fs = fs
        self.session = session
        self.work = work

    def to_write(self, written: str) -> Permit:
        where = locate(self.work, written)
        return self.fs.permit(Access(self.session, self.work, where, "write", "probe"))

    def show(self, written: str) -> Fingerprint:
        """Read a file through the seam, and tell the seam that the model saw it."""
        where = locate(self.work, written)
        permit = self.fs.permit(Access(self.session, self.work, where, "read", "probe"))
        with permit.open() as opened:
            fingerprint = opened.fingerprint
        self.fs.shown(permit, fingerprint)
        return fingerprint

    def seen(self, written: str) -> Fingerprint | None:
        return self.fs.seen(self.session, locate(self.work, written).path)


@pytest.fixture
def hand(fs: Fs, session: Session, work: Path) -> Hand:
    return Hand(fs, session, work)


def state(path: Path) -> Fingerprint:
    status = path.stat()
    return Fingerprint(status.st_size, status.st_mtime_ns)


def names(directory: Path) -> list[str]:
    return sorted(entry.name for entry in directory.iterdir())


# Making a file


def test_a_file_that_is_not_there_is_made(hand: Hand, work: Path) -> None:
    written = hand.to_write("new.txt").replace(b"fresh")

    assert (work / "new.txt").read_bytes() == b"fresh"
    assert written == state(work / "new.txt")
    assert names(work) == ["a.txt", "new.txt"]


def test_directories_that_are_missing_on_the_way_are_made(hand: Hand, work: Path) -> None:
    hand.to_write("deep/er/new.txt").replace(b"fresh")

    assert (work / "deep" / "er" / "new.txt").read_bytes() == b"fresh"
    assert names(work / "deep") == ["er"]
    assert names(work / "deep" / "er") == ["new.txt"]


def test_the_content_is_written_as_it_is_given(hand: Hand, work: Path) -> None:
    content = b"one\r\ntwo\nthree\r" + bytes(range(256))

    hand.to_write("exact.bin").replace(content)

    assert (work / "exact.bin").read_bytes() == content


def test_something_in_the_way_that_is_no_directory_is_not_written_through(
    hand: Hand, work: Path
) -> None:
    with pytest.raises(NotADirectoryError, match="is in the way"):
        hand.to_write("a.txt/inner.txt").replace(b"fresh")

    assert (work / "a.txt").read_bytes() == b"old"


# Reading before writing


def test_a_file_that_the_model_was_never_shown_is_not_replaced(hand: Hand, work: Path) -> None:
    with pytest.raises(ToolFailure, match=r"a\.txt exists and has not been read") as refused:
        hand.to_write("a.txt").replace(b"new")

    assert refused.value.code == "NOT_READ"
    assert (work / "a.txt").read_bytes() == b"old"
    assert names(work) == ["a.txt"]


def test_a_file_that_the_model_was_shown_is_replaced(hand: Hand, work: Path) -> None:
    hand.show("a.txt")

    written = hand.to_write("a.txt").replace(b"new content")

    assert (work / "a.txt").read_bytes() == b"new content"
    assert written == state(work / "a.txt")
    assert names(work) == ["a.txt"]


def test_what_the_agent_wrote_it_may_write_again(hand: Hand, work: Path) -> None:
    hand.to_write("new.txt").replace(b"first")
    hand.to_write("new.txt").replace(b"second, and longer")

    hand.to_write("new.txt").replace(b"third")

    assert (work / "new.txt").read_bytes() == b"third"
    assert hand.seen("new.txt") == state(work / "new.txt")


def test_a_file_that_changed_after_it_was_shown_is_not_replaced(hand: Hand, work: Path) -> None:
    hand.show("a.txt")
    (work / "a.txt").write_bytes(b"changed by someone else")

    with pytest.raises(ToolFailure, match=r"a\.txt has changed since it was read") as refused:
        hand.to_write("a.txt").replace(b"new")

    assert refused.value.code == "CHANGED_SINCE_READ"
    assert (work / "a.txt").read_bytes() == b"changed by someone else"


def test_a_change_that_keeps_the_size_is_told_by_its_time(hand: Hand, work: Path) -> None:
    shown = hand.show("a.txt")
    (work / "a.txt").write_bytes(b"new")
    later = shown.modified_ns + 2_000_000_000
    os.utime(work / "a.txt", ns=(later, later))

    with pytest.raises(ToolFailure) as refused:
        hand.to_write("a.txt").replace(b"mine")

    assert refused.value.code == "CHANGED_SINCE_READ"
    assert (work / "a.txt").read_bytes() == b"new"


def test_a_file_that_is_shown_again_after_a_change_is_replaced(hand: Hand, work: Path) -> None:
    hand.show("a.txt")
    (work / "a.txt").write_bytes(b"changed by someone else")
    hand.show("a.txt")

    hand.to_write("a.txt").replace(b"mine")

    assert (work / "a.txt").read_bytes() == b"mine"


def test_what_one_session_was_shown_does_not_let_another_write(
    hand: Hand, fs: Fs, ctx: Context, work: Path
) -> None:
    other = Hand(fs, Session(ctx, "s-2", {kind.event_type: kind for kind in CORE_EVENTS}), work)
    hand.show("a.txt")

    with pytest.raises(ToolFailure) as refused:
        other.to_write("a.txt").replace(b"new")

    assert refused.value.code == "NOT_READ"
    assert (work / "a.txt").read_bytes() == b"old"


def test_a_file_that_was_shown_and_is_gone_is_made_again(hand: Hand, work: Path) -> None:
    hand.show("a.txt")
    (work / "a.txt").unlink()

    hand.to_write("a.txt").replace(b"back")

    assert (work / "a.txt").read_bytes() == b"back"


def test_a_directory_is_not_replaced(hand: Hand, work: Path) -> None:
    (work / "sub").mkdir()

    with pytest.raises(ToolFailure, match="sub is not a file") as refused:
        hand.to_write("sub").replace(b"new")

    assert refused.value.code == "NOT_A_FILE"
    assert (work / "sub").is_dir()
    assert names(work) == ["a.txt", "sub"]


def test_a_permit_to_read_does_not_replace(
    hand: Hand, fs: Fs, session: Session, work: Path
) -> None:
    permit = fs.permit(Access(session, work, locate(work, "a.txt"), "read", "probe"))

    with pytest.raises(ValueError, match="not for writing"):
        permit.replace(b"new")


# One step


def test_a_replacement_that_fails_leaves_the_file_and_nothing_beside_it(
    hand: Hand, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(*arguments: Any, **options: Any) -> None:
        raise OSError(28, "No space left on device")

    hand.show("a.txt")
    monkeypatch.setattr(os, "replace", full)

    with pytest.raises(OSError, match="No space left"):
        hand.to_write("a.txt").replace(b"new")

    assert (work / "a.txt").read_bytes() == b"old"
    assert names(work) == ["a.txt"]


def test_the_new_content_is_whole_before_it_takes_the_place_of_the_old(
    hand: Hand, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    seen: list[tuple[bytes, bytes]] = []
    really_replace = os.replace

    def watching(source: Any, target: Any, **options: Any) -> None:
        beside = next(entry for entry in work.iterdir() if entry.name != "a.txt")
        seen.append(((work / "a.txt").read_bytes(), beside.read_bytes()))
        really_replace(source, target, **options)

    hand.show("a.txt")
    monkeypatch.setattr(os, "replace", watching)

    hand.to_write("a.txt").replace(b"new content")

    assert seen == [(b"old", b"new content")]
    assert names(work) == ["a.txt"]


# Where it was judged


def test_nothing_is_written_where_a_directory_was_replaced_by_a_link(
    hand: Hand, work: Path, tmp_path: Path
) -> None:
    (work / "d").mkdir()
    (work / "d" / "notes.txt").write_bytes(b"inside")
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    (outside / "notes.txt").write_bytes(b"theirs")
    hand.show("d/notes.txt")
    permit = hand.to_write("d/notes.txt")

    # Between the decision and the writing, the directory becomes a link that leads out.
    shutil.rmtree(work / "d")
    link_directory(work / "d", outside)

    with pytest.raises(ToolFailure, match="was not where it had been") as refused:
        permit.replace(b"mine")

    assert refused.value.code == "MOVED"
    assert (outside / "notes.txt").read_bytes() == b"theirs"
    assert names(outside) == ["notes.txt"]


def test_no_directory_is_made_where_a_directory_was_replaced_by_a_link(
    hand: Hand, work: Path, tmp_path: Path
) -> None:
    (work / "d").mkdir()
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    permit = hand.to_write("d/new/deep.txt")

    shutil.rmtree(work / "d")
    link_directory(work / "d", outside)

    with pytest.raises(ToolFailure) as refused:
        permit.replace(b"mine")

    assert refused.value.code == "MOVED"
    assert names(outside) == []


if sys.platform == "win32":

    def test_the_directory_of_a_file_cannot_be_moved_away_while_the_file_is_replaced(
        hand: Hand, work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        (work / "d").mkdir()
        (work / "d" / "f.txt").write_bytes(b"old")
        hand.show("d/f.txt")
        refusals: list[int | None] = []
        really_replace = os.replace

        def meddling(source: Any, target: Any, **options: Any) -> None:
            for directory in (work / "d", work):
                try:
                    directory.rename(directory.with_name(directory.name + "-moved"))
                except PermissionError as error:
                    refusals.append(error.winerror)
            really_replace(source, target, **options)

        monkeypatch.setattr(os, "replace", meddling)

        hand.to_write("d/f.txt").replace(b"new")

        assert refusals == [32, 5]
        assert (work / "d" / "f.txt").read_bytes() == b"new"

    def test_a_file_that_is_marked_read_only_is_not_replaced(
        hand: Hand, work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        monkeypatch.setattr(bound, "_RENAME_PAUSE", 0.0)
        hand.show("a.txt")
        (work / "a.txt").chmod(stat.S_IREAD)
        try:
            with pytest.raises(PermissionError):
                hand.to_write("a.txt").replace(b"new")
        finally:
            (work / "a.txt").chmod(stat.S_IREAD | stat.S_IWRITE)

        assert (work / "a.txt").read_bytes() == b"old"
        assert names(work) == ["a.txt"]

    def test_a_file_that_is_busy_for_a_moment_is_tried_again(
        hand: Hand, work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        attempts: list[str] = []
        really_replace = os.replace

        def busy_twice(source: Any, target: Any, **options: Any) -> None:
            attempts.append("tried")
            if len(attempts) < 3:
                raise PermissionError(13, "The file is in use")
            really_replace(source, target, **options)

        monkeypatch.setattr(bound, "_RENAME_PAUSE", 0.0)
        monkeypatch.setattr(os, "replace", busy_twice)
        hand.show("a.txt")

        hand.to_write("a.txt").replace(b"new")

        assert len(attempts) == 3
        assert (work / "a.txt").read_bytes() == b"new"

    def test_a_path_is_given_to_the_system_in_the_form_that_takes_any_length() -> None:
        back = chr(92)
        verbatim: Any = vars(bound)["_verbatim"]
        prefix = back * 2 + "?" + back

        assert verbatim(Path("C:/work/d")) == prefix + "C:" + back + "work" + back + "d"
        assert verbatim(Path("//nas/share/d")) == (
            prefix + "UNC" + back + "nas" + back + "share" + back + "d"
        )
        assert verbatim(Path(prefix + "C:" + back + "work")) == prefix + "C:" + back + "work"

    def test_a_directory_that_cannot_be_held_is_an_error_of_the_system(tmp_path: Path) -> None:
        held: Any = vars(bound)["_Held"]

        with pytest.raises(FileNotFoundError):
            held(tmp_path / "not-there")

    def test_a_file_on_a_drive_that_is_not_there_is_an_error_of_the_system(hand: Hand) -> None:
        letters = [letter for letter in "QRSTUVWXYZ" if not Path(f"{letter}:/").exists()]
        if not letters:
            pytest.skip("every drive letter is taken")
        where = locate(hand.work, f"{letters[0]}:/nothing/a.txt")
        permit = Permit(Access(hand.session, hand.work, where, "write", "probe"), hand.fs)

        # The words of the error are in the language of the system. Its number is not.
        with pytest.raises(FileNotFoundError) as refused:
            permit.replace(b"new")

        assert refused.value.winerror == 3

else:

    def test_the_file_lands_in_the_directory_that_was_held_wherever_its_path_leads_by_then(
        hand: Hand, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        (work / "d").mkdir()
        (work / "d" / "f.txt").write_bytes(b"old")
        hand.show("d/f.txt")
        outside = tmp_path / "elsewhere"
        outside.mkdir()
        really_replace = os.replace

        def meddling(source: Any, target: Any, **options: Any) -> None:
            (work / "d").rename(work / "d-moved")
            (work / "d").symlink_to(outside, target_is_directory=True)
            really_replace(source, target, **options)

        monkeypatch.setattr(os, "replace", meddling)

        hand.to_write("d/f.txt").replace(b"new")

        assert (work / "d-moved" / "f.txt").read_bytes() == b"new"
        assert names(outside) == []

    def test_a_link_that_has_taken_the_place_of_a_file_is_not_written_through(
        hand: Hand, work: Path, tmp_path: Path
    ) -> None:
        outside = tmp_path / "theirs.txt"
        outside.write_bytes(b"theirs")
        hand.show("a.txt")
        permit = hand.to_write("a.txt")
        (work / "a.txt").unlink()
        (work / "a.txt").symlink_to(outside)

        with pytest.raises(ToolFailure) as refused:
            permit.replace(b"mine")

        assert refused.value.code == "NOT_A_FILE"
        assert outside.read_bytes() == b"theirs"

    def test_a_file_keeps_its_permissions_when_it_is_replaced(hand: Hand, work: Path) -> None:
        (work / "a.txt").chmod(0o750)
        hand.show("a.txt")

        hand.to_write("a.txt").replace(b"new")

        assert stat.S_IMODE((work / "a.txt").stat().st_mode) == 0o750
