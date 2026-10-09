"""Finding the file that a path means."""

import os
import sys
import time
from collections.abc import Mapping
from pathlib import Path

import pytest

from columbina.fs import Located, locate, paths
from columbina.tools import ToolFailure
from support.links import link_directory


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    return directory


def real(path: Path) -> Path:
    return Path(os.path.realpath(path))


def test_a_path_is_taken_from_the_working_directory(work: Path) -> None:
    (work / "sub").mkdir()
    (work / "sub" / "a.txt").write_text("a")

    where = locate(work, "sub/a.txt")

    assert where == Located(
        path=real(work / "sub" / "a.txt"), written="sub/a.txt", inside=True, relative="sub/a.txt"
    )


def test_a_path_that_is_given_in_full_is_found_the_same_way(work: Path) -> None:
    (work / "a.txt").write_text("a")

    where = locate(work, str(work / "a.txt"))

    assert (where.inside, where.relative) == (True, "a.txt")


@pytest.mark.parametrize("written", ["sub/../a.txt", "./a.txt", "sub/./../a.txt", "sub//../a.txt"])
def test_detours_inside_the_working_directory_lead_to_the_same_file(
    work: Path, written: str
) -> None:
    (work / "sub").mkdir()
    (work / "a.txt").write_text("a")

    where = locate(work, written)

    assert (where.path, where.inside, where.relative) == (real(work / "a.txt"), True, "a.txt")


def test_the_working_directory_itself_is_a_dot(work: Path) -> None:
    where = locate(work, ".")

    assert (where.inside, where.relative) == (True, ".")


def test_a_file_that_is_not_there_yet_has_its_place_too(work: Path) -> None:
    where = locate(work, "new/deep/file.txt")

    assert (where.inside, where.relative) == (True, "new/deep/file.txt")
    assert where.path == real(work) / "new" / "deep" / "file.txt"


def test_a_path_that_climbs_out_is_outside(work: Path, tmp_path: Path) -> None:
    (tmp_path / "secret.txt").write_text("s")

    where = locate(work, "../secret.txt")

    assert not where.inside
    assert where.path == real(tmp_path / "secret.txt")
    assert where.relative == real(tmp_path / "secret.txt").as_posix()


def test_a_directory_whose_name_begins_like_the_working_directory_is_outside(
    work: Path, tmp_path: Path
) -> None:
    (tmp_path / "work-other").mkdir()

    assert not locate(work, "../work-other/a.txt").inside


def test_a_link_that_leads_out_is_followed_before_the_path_is_judged(
    work: Path, tmp_path: Path
) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.txt").write_text("s")
    link_directory(work / "link", outside)

    where = locate(work, "link/secret.txt")

    assert not where.inside
    assert where.path == real(outside / "secret.txt")


def test_a_link_inside_the_working_directory_gives_the_name_of_what_it_leads_to(
    work: Path,
) -> None:
    (work / "real").mkdir()
    (work / "real" / "a.txt").write_text("a")
    link_directory(work / "alias", work / "real")

    where = locate(work, "alias/a.txt")

    assert (where.inside, where.relative) == (True, "real/a.txt")


def test_links_that_lead_to_links_are_followed_to_the_end(work: Path, tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    link_directory(work / "second", outside)
    link_directory(work / "first", work / "second")

    where = locate(work, "first/a.txt")

    assert not where.inside
    assert where.path == real(outside) / "a.txt"


def test_a_working_directory_that_is_reached_through_a_link_holds_its_files(
    work: Path, tmp_path: Path
) -> None:
    (work / "a.txt").write_text("a")
    link_directory(tmp_path / "door", work)

    where = locate(tmp_path / "door", "a.txt")

    assert (where.inside, where.relative, where.path) == (True, "a.txt", real(work / "a.txt"))


@pytest.mark.parametrize("written", ["", "   ", "\t"])
def test_no_path_is_refused(work: Path, written: str) -> None:
    with pytest.raises(ToolFailure, match="No path was given") as refused:
        locate(work, written)

    assert refused.value.code == "INVALID_PATH"


def test_a_path_with_a_character_that_no_path_holds_is_refused(work: Path) -> None:
    with pytest.raises(ToolFailure, match="no path can contain") as refused:
        locate(work, "a" + chr(0) + ".txt")

    assert refused.value.code == "INVALID_PATH"


if sys.platform == "win32":
    BACK = chr(92)
    VERBATIM = BACK * 2 + "?" + BACK

    def faked(monkeypatch: pytest.MonkeyPatch, links: Mapping[Path | str, str]) -> None:
        """Make the given paths links for the walk, without making them on the disk."""
        table = {os.path.normcase(path): target for path, target in links.items()}

        def target_of(path: str) -> str | None:
            return table.get(os.path.normcase(path))

        monkeypatch.setattr(paths, "_link_target", target_of)

    def test_a_name_is_given_the_case_it_has_on_the_disk(work: Path) -> None:
        (work / "Notes.txt").write_text("n")

        where = locate(work, "NOTES.TXT")

        assert (where.inside, where.relative) == (True, "Notes.txt")

    def test_two_names_that_are_one_only_in_lower_case_are_two_directories(
        tmp_path: Path,
    ) -> None:
        # Python lowers the capital I with a dot to an i and a dot above. The disk does not.
        small = tmp_path / ("i" + chr(0x307))
        capital = tmp_path / chr(0x130)
        small.mkdir()
        capital.mkdir()
        (capital / "secret.txt").write_text("s")

        where = locate(small, "../" + chr(0x130) + "/secret.txt")

        assert not where.inside
        assert where.path == real(capital / "secret.txt")

    def test_the_case_of_the_working_directory_does_not_count(work: Path) -> None:
        (work / "a.txt").write_text("a")
        shouted = Path(str(work).upper())

        assert locate(shouted, "a.txt").inside
        assert locate(work, str(shouted / "a.txt")).relative == "a.txt"

    @pytest.mark.parametrize(
        "written", ["notes.txt.", "notes.txt ", "sub./a.txt", "sub /a.txt", "..."]
    )
    def test_a_name_that_ends_with_a_dot_or_a_space_is_refused(work: Path, written: str) -> None:
        with pytest.raises(ToolFailure, match="a dot or a space") as refused:
            locate(work, written)

        assert refused.value.code == "INVALID_PATH"

    @pytest.mark.parametrize("written", ["notes.txt:stream", "notes.txt::$DATA", "sub:x/a.txt"])
    def test_a_colon_inside_a_name_is_refused(work: Path, written: str) -> None:
        with pytest.raises(ToolFailure, match="a colon inside a name") as refused:
            locate(work, written)

        assert refused.value.code == "INVALID_PATH"

    def test_a_drive_without_a_slash_is_refused(work: Path) -> None:
        with pytest.raises(ToolFailure, match="a drive has to be followed by a slash"):
            locate(work, "C:notes.txt")

    @pytest.mark.parametrize(
        "written",
        ["NUL", "nul", "con.txt", "COM1", "sub/aux", "lpt9.log", "CONIN$", "com" + chr(0xB9)],
    )
    def test_the_name_of_a_device_is_refused(work: Path, written: str) -> None:
        with pytest.raises(ToolFailure, match="is the name of a device") as refused:
            locate(work, written)

        assert refused.value.code == "INVALID_PATH"

    @pytest.mark.parametrize(
        "written", ["console.txt", "com.txt", "nullable", "aux2/a.txt", "com10", "prn_notes"]
    )
    def test_a_name_that_only_looks_like_a_device_is_a_file(work: Path, written: str) -> None:
        assert locate(work, written).inside

    @pytest.mark.parametrize(
        "written",
        [
            "//server/share/a.txt",
            BACK * 2 + "server" + BACK + "share",
            "/" + BACK + "server/share",
            "//?/C:/Windows/win.ini",
            "//./NUL",
        ],
    )
    def test_a_path_that_names_another_machine_is_refused_before_anything_is_asked(
        work: Path, written: str, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        def asked(path: object) -> str:
            raise AssertionError("the file system was asked")

        monkeypatch.setattr(os.path, "realpath", asked)

        with pytest.raises(ToolFailure, match="names another machine") as refused:
            locate(work, written)

        assert refused.value.code == "INVALID_PATH"

    def test_a_path_from_the_top_of_the_drive_is_outside(work: Path) -> None:
        where = locate(work, BACK + "Windows")

        assert not where.inside
        assert where.path == Path(real(work).drive + "/Windows")

    @pytest.mark.parametrize(
        "target",
        [
            "//evil/share",
            BACK * 2 + "evil" + BACK + "share",
            VERBATIM + "UNC" + BACK + "evil" + BACK + "share",
            VERBATIM + "unc" + BACK + "evil" + BACK + "share",
            BACK * 2 + "." + BACK + "pipe" + BACK + "evil",
            VERBATIM + "GLOBALROOT" + BACK + "Device" + BACK + "Mup" + BACK + "evil" + BACK + "s",
            VERBATIM + "pipe" + BACK + "evil",
            VERBATIM + "Device" + BACK + "HarddiskVolume1",
        ],
    )
    def test_a_link_that_leads_to_another_machine_is_not_followed(
        work: Path, monkeypatch: pytest.MonkeyPatch, target: str
    ) -> None:
        faked(monkeypatch, {real(work) / "docs": target})
        began = time.monotonic()

        with pytest.raises(ToolFailure, match="a link on the way leads to another machine") as no:
            locate(work, "docs/readme.md")

        assert no.value.code == "INVALID_PATH"
        assert time.monotonic() - began < 2

    @pytest.mark.parametrize("spelling", ["plain", "verbatim", "relative", "from the top"])
    def test_what_lies_behind_a_link_is_looked_at_too(
        work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, spelling: str
    ) -> None:
        elsewhere = real(tmp_path) / "elsewhere"
        targets = {
            "plain": str(elsewhere),
            "verbatim": VERBATIM + str(elsewhere),
            "relative": ".." + BACK + "elsewhere",
            "from the top": str(elsewhere).removeprefix(elsewhere.drive),
        }
        faked(
            monkeypatch,
            {real(work) / "docs": targets[spelling], elsewhere / "next": "//evil/share"},
        )

        with pytest.raises(ToolFailure, match="leads to another machine"):
            locate(work, "docs/next/readme.md")

    def test_a_step_back_is_taken_from_the_text_before_any_link_is_followed(
        work: Path, tmp_path: Path
    ) -> None:
        # This is how Windows reads a path, unlike the systems where a step back leaves
        # the place that a link led to.
        elsewhere = tmp_path / "elsewhere"
        (elsewhere / "deep").mkdir(parents=True)
        (elsewhere / "bait.txt").write_text("where the link leads")
        (work / "bait.txt").write_text("in the working directory")
        link_directory(work / "docs", elsewhere / "deep")

        where = locate(work, "docs/../bait.txt")

        assert (where.inside, where.relative) == (True, "bait.txt")
        assert where.path.read_text() == "in the working directory"

    def test_the_links_that_are_looked_at_are_those_that_the_system_will_follow(
        work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        elsewhere = real(tmp_path) / "elsewhere"
        faked(
            monkeypatch,
            {real(work) / "docs": str(elsewhere / "deep"), real(work) / "bait": "//evil/share"},
        )

        # After the step back, bait is the one in the working directory, and it leads away.
        with pytest.raises(ToolFailure, match="leads to another machine"):
            locate(work, "docs/../bait/readme.md")

    def test_a_link_beside_the_target_of_another_is_not_on_the_way(
        work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        elsewhere = real(tmp_path) / "elsewhere"
        faked(
            monkeypatch,
            {real(work) / "docs": str(elsewhere / "deep"), elsewhere / "bait": "//evil/share"},
        )

        where = locate(work, "docs/../bait/readme.md")

        assert (where.inside, where.relative) == (True, "bait/readme.md")

    def test_a_link_to_a_volume_without_a_letter_is_followed_like_one_to_a_drive(
        work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        volume = VERBATIM + "Volume{01234567-89ab-cdef-0123-456789abcdef}"
        faked(
            monkeypatch,
            {
                str(real(work) / "docs"): volume + BACK + "data",
                volume + BACK + "data" + BACK + "next": "//evil/share",
            },
        )

        with pytest.raises(ToolFailure, match="leads to another machine"):
            locate(work, "docs/next/readme.md")

    def test_a_link_to_a_volume_without_a_letter_is_no_reason_to_refuse(
        work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        volume = VERBATIM + "Volume{01234567-89ab-cdef-0123-456789abcdef}"
        faked(monkeypatch, {str(real(work) / "docs"): volume + BACK + "data"})

        # The link is not on the disk, so the system finds the path where it was written.
        assert locate(work, "docs/readme.md").inside

    def test_links_that_lead_in_a_circle_are_given_up(
        work: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        faked(monkeypatch, {real(work) / "loop": str(real(work) / "loop")})

        with pytest.raises(ToolFailure, match="too many links") as refused:
            locate(work, "loop/a.txt")

        assert refused.value.code == "INVALID_PATH"

    def test_a_link_that_cannot_be_read_is_left_to_the_system(
        work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        outside = tmp_path / "outside"
        outside.mkdir()
        link_directory(work / "link", outside)

        def unreadable(path: object) -> str:
            raise OSError("the link cannot be read")

        monkeypatch.setattr(os, "readlink", unreadable)

        assert not locate(work, "link/a.txt").inside
