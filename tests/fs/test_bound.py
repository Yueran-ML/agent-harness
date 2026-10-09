"""Opening a file so that what is open is the file that was meant."""

import os
import shutil
import sys
from pathlib import Path
from typing import Any

import pytest

from columbina.fs import bound
from columbina.fs.bound import MovedError, NotAFileError, open_for_reading
from support.links import link_directory


def real(path: Path) -> Path:
    return Path(os.path.realpath(path))


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = real(tmp_path) / "work"
    directory.mkdir()
    return directory


@pytest.fixture
def swapped(work: Path, tmp_path: Path) -> Path:
    """The place of a file whose directory was replaced by a link that leads outside."""
    (work / "d").mkdir()
    (work / "d" / "f.txt").write_text("inside")
    outside = real(tmp_path) / "outside"
    outside.mkdir()
    (outside / "f.txt").write_text("outside")
    checked = real(work / "d" / "f.txt")
    shutil.rmtree(work / "d")
    link_directory(work / "d", outside)
    return checked


def without_an_answer(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make the system say nothing about where an open file is."""

    def where(descriptor: int) -> None:
        return None

    monkeypatch.setattr(bound, "_where", where)


def test_a_file_is_opened_and_can_be_read(work: Path) -> None:
    (work / "a.txt").write_bytes(b"first\nsecond\n")

    file, status = open_for_reading(work / "a.txt")

    with file:
        assert file.read() == b"first\nsecond\n"
    assert status.st_size == 13
    assert status.st_mtime_ns == (work / "a.txt").stat().st_mtime_ns


def test_a_file_that_is_not_there_cannot_be_opened(work: Path) -> None:
    with pytest.raises(FileNotFoundError):
        open_for_reading(work / "nowhere.txt")


def test_a_directory_is_no_file(work: Path) -> None:
    (work / "sub").mkdir()

    with pytest.raises(NotAFileError):
        open_for_reading(work / "sub")


def test_what_was_opened_through_a_link_that_took_the_place_of_a_directory_is_refused(
    swapped: Path,
) -> None:
    with pytest.raises(MovedError, match="somewhere else"):
        open_for_reading(swapped)


def test_a_file_that_is_refused_is_not_left_open(work: Path) -> None:
    (work / "sub").mkdir()
    (work / "sub" / "a.txt").write_text("a")
    opened: list[int] = []
    closed: list[int] = []
    really_open, really_close = os.open, os.close

    def opening(*arguments: Any, **options: Any) -> int:
        descriptor = really_open(*arguments, **options)
        opened.append(descriptor)
        return descriptor

    def closing(descriptor: int) -> None:
        closed.append(descriptor)
        really_close(descriptor)

    checked = real(work / "sub" / "a.txt")
    outside = work.parent / "outside"
    outside.mkdir()
    (outside / "a.txt").write_text("o")
    shutil.rmtree(work / "sub")
    link_directory(work / "sub", outside)
    with pytest.MonkeyPatch.context() as patch:
        patch.setattr(os, "open", opening)
        patch.setattr(os, "close", closing)
        with pytest.raises(MovedError):
            open_for_reading(checked)

    assert opened == closed
    assert len(opened) == 1


def test_without_an_answer_of_the_system_a_file_in_its_place_is_opened(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "a.txt").write_text("a")
    without_an_answer(monkeypatch)

    file, _ = open_for_reading(work / "a.txt")

    with file:
        assert file.read() == b"a"


def test_without_an_answer_of_the_system_a_path_that_now_leads_elsewhere_is_refused(
    swapped: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    without_an_answer(monkeypatch)

    with pytest.raises(MovedError):
        open_for_reading(swapped)


def test_a_path_that_names_nothing_now_does_not_name_the_open_file(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "a.txt").write_text("a")
    without_an_answer(monkeypatch)
    is_at: Any = vars(bound)["_is_at"]
    descriptor = os.open(work / "a.txt", os.O_RDONLY)
    try:
        assert is_at(descriptor, work / "a.txt")
        assert not is_at(descriptor, work / "gone.txt")
    finally:
        os.close(descriptor)


def test_another_file_at_the_place_is_not_the_open_file(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "a.txt").write_text("a")
    (work / "b.txt").write_text("b")
    without_an_answer(monkeypatch)
    is_at: Any = vars(bound)["_is_at"]
    descriptor = os.open(work / "a.txt", os.O_RDONLY)
    try:
        assert not is_at(descriptor, work / "b.txt")
    finally:
        os.close(descriptor)


def test_two_spellings_are_one_place_if_they_name_the_same_entries(work: Path) -> None:
    (work / "sub").mkdir()
    (work / "sub" / "a.txt").write_text("a")
    same: Any = vars(bound)["_same_entries"]

    assert same(str(work / "sub" / "a.txt"), str(work / "sub" / "a.txt"))
    assert not same(str(work / "sub" / "a.txt"), str(work / "sub" / "b.txt"))
    assert not same(str(work / "sub" / "a.txt"), str(work / "a.txt"))
    assert not same(str(work / "sub" / "a.txt"), str(work / "sub" / "deeper" / "a.txt"))


def test_a_link_and_what_it_leads_to_are_not_the_same_entry(work: Path) -> None:
    # The link lies beside what it leads to, so the two ways part at one name only.
    (work / "room").mkdir()
    (work / "room" / "a.txt").write_text("a")
    link_directory(work / "door", work / "room")
    same: Any = vars(bound)["_same_entries"]

    assert (work / "door" / "a.txt").samefile(work / "room" / "a.txt")
    assert (work / "door").samefile(work / "room")
    assert not same(str(work / "door" / "a.txt"), str(work / "room" / "a.txt"))
    assert not same(str(work / "door"), str(work / "room"))


if sys.platform == "win32":
    BACK = chr(92)

    def test_names_that_are_one_only_in_lower_case_are_not_the_same_place(
        tmp_path: Path,
    ) -> None:
        small = real(tmp_path) / ("i" + chr(0x307))
        capital = real(tmp_path) / chr(0x130)
        small.mkdir()
        capital.mkdir()
        (small / "a.txt").write_text("the one that was checked")
        (capital / "a.txt").write_text("another")
        is_at: Any = vars(bound)["_is_at"]
        descriptor = os.open(capital / "a.txt", os.O_RDONLY)
        try:
            assert is_at(descriptor, capital / "a.txt")
            assert not is_at(descriptor, small / "a.txt")
        finally:
            os.close(descriptor)

    def test_another_case_of_a_name_is_the_same_entry(work: Path) -> None:
        (work / "Notes.txt").write_text("n")
        same: Any = vars(bound)["_same_entries"]

        assert same(str(work / "Notes.txt"), str(work / "NOTES.TXT"))

    def test_two_spellings_that_differ_from_the_drive_down_can_be_one_place(work: Path) -> None:
        (work / "Notes.txt").write_text("n")
        same: Any = vars(bound)["_same_entries"]
        spelled = str(work / "Notes.txt")

        assert same(spelled, spelled.swapcase())

    def test_the_case_of_a_name_does_not_make_a_file_another_file(work: Path) -> None:
        (work / "Notes.txt").write_text("n")

        file, _ = open_for_reading(work / "NOTES.TXT")

        with file:
            assert file.read() == b"n"

    def test_a_device_is_no_file() -> None:
        with pytest.raises(NotAFileError, match="not a regular file"):
            open_for_reading(Path("NUL"))

    def test_the_system_says_nothing_about_where_a_pipe_is() -> None:
        where: Any = vars(bound)["_where"]
        reading, writing = os.pipe()
        try:
            assert where(reading) is None
        finally:
            os.close(reading)
            os.close(writing)

    def test_the_way_the_system_reports_a_path_is_turned_into_the_way_people_write_it() -> None:
        plain: Any = vars(bound)["_plain"]
        verbatim = BACK * 2 + "?" + BACK

        assert plain(verbatim + "C:" + BACK + "work") == "C:" + BACK + "work"
        assert plain(verbatim + "UNC" + BACK + "nas" + BACK + "share") == (
            BACK * 2 + "nas" + BACK + "share"
        )
        assert plain("C:" + BACK + "work") == "C:" + BACK + "work"

    def test_a_file_that_cannot_be_opened_for_another_reason_says_so(
        work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        (work / "a.txt").write_text("a")

        def locked(*arguments: Any, **options: Any) -> int:
            raise PermissionError(13, "Permission denied")

        monkeypatch.setattr(os, "open", locked)

        with pytest.raises(PermissionError) as refused:
            open_for_reading(work / "a.txt")

        assert not isinstance(refused.value, NotAFileError)

else:

    def test_a_pipe_is_no_file_and_opening_it_does_not_wait(work: Path) -> None:
        os.mkfifo(work / "pipe")

        with pytest.raises(NotAFileError):
            open_for_reading(work / "pipe")
