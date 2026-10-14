"""Where a program that this one runs by its name may come from: ``columbina.programs``."""

import os
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from columbina import programs
from support.links import link_directory, link_file

WINDOWS = sys.platform == "win32"
NAME = "tool"
PROGRAM = f"{NAME}.exe" if WINDOWS else NAME


def found(search: str, outside: Sequence[Path] = ()) -> str | None:
    return programs.found(NAME, search, outside)


def program_in(directory: Path) -> Path:
    """Put something that passes for the program into a directory."""
    directory.mkdir(parents=True, exist_ok=True)
    program = directory / PROGRAM
    program.write_bytes(b"")
    program.chmod(0o755)
    return program


def search(*entries: object) -> str:
    return os.pathsep.join(str(entry) for entry in entries)


@pytest.fixture
def root(tmp_path: Path) -> Path:
    """A directory to work below, with every link on its path resolved."""
    return Path(os.path.realpath(tmp_path))


def test_a_program_is_looked_for_by_its_own_name(root: Path) -> None:
    program_in(root / "tools")
    other = root / "tools" / ("other.exe" if WINDOWS else "other")
    other.write_bytes(b"")
    other.chmod(0o755)

    assert programs.found("other", search(root / "tools")) == str(other)
    assert programs.found("missing", search(root / "tools")) is None


def test_the_program_is_found_in_a_directory_of_the_search_path(root: Path) -> None:
    program_in(root / "tools")

    assert found(search(root / "empty", root / "tools")) == str(root / "tools" / PROGRAM)


def test_the_first_directory_that_has_the_program_gives_it(root: Path) -> None:
    program_in(root / "one")
    program_in(root / "two")

    assert found(search(root / "two", root / "one")) == str(root / "two" / PROGRAM)


def test_no_program_is_found_where_the_search_path_leads_to_none(root: Path) -> None:
    (root / "tools").mkdir()

    assert found(search(root / "tools", root / "not-there")) is None
    assert found("") is None


def test_an_entry_that_is_no_whole_path_is_passed_over(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Such an entry means a place below the directory that the program was started in, and
    # that is where a repository would put a program of this name.
    program_in(root / "repository")
    program_in(root / "repository" / "bin")
    program_in(root / "tools")
    monkeypatch.chdir(root / "repository")

    there = found(search(".", "bin", "", root / "tools"))

    assert there == str(root / "tools" / PROGRAM)
    assert found(search(".", "bin", "")) is None


def test_a_program_in_a_working_directory_is_not_taken(root: Path) -> None:
    program_in(root / "repository")
    program_in(root / "repository" / "node_modules" / ".bin")
    program_in(root / "tools")
    path = search(
        root / "repository", root / "repository" / "node_modules" / ".bin", root / "tools"
    )

    assert found(path, [root / "repository"]) == str(root / "tools" / PROGRAM)
    assert found(path, [root / "elsewhere"]) == str(root / "repository" / PROGRAM)
    assert found(search(root / "repository"), [root / "repository"]) is None


def test_a_program_that_a_link_leads_into_a_working_directory_for_is_not_taken(root: Path) -> None:
    program_in(root / "repository" / "bin")
    link_directory(root / "tools", root / "repository" / "bin")

    assert found(search(root / "tools"), [root / "repository"]) is None
    assert found(search(root / "tools")) == str(root / "tools" / PROGRAM)


def test_a_working_directory_that_is_named_through_a_link_is_the_same_directory(
    root: Path,
) -> None:
    program_in(root / "repository")
    link_directory(root / "alias", root / "repository")

    assert found(search(root / "repository"), [root / "alias"]) is None


def test_a_link_in_a_working_directory_is_not_followed_to_a_program_outside_it(
    root: Path,
) -> None:
    # Whoever can write the directory can put another file where the link is, and the
    # program is run by its name again and again.
    program_in(root / "real")
    (root / "repository" / "bin").mkdir(parents=True)
    link_file(root / "repository" / "bin" / PROGRAM, root / "real" / PROGRAM)
    path = search(root / "repository" / "bin")

    assert found(path, [root / "repository"]) is None
    assert found(path, [root / "elsewhere"]) == str(root / "repository" / "bin" / PROGRAM)


def test_a_way_to_the_program_that_leads_through_a_working_directory_is_not_taken(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    (root / "tools").mkdir()
    link_file(root / "repository" / PROGRAM, root / "real" / PROGRAM)
    link_file(root / "tools" / PROGRAM, root / "repository" / PROGRAM)

    # The name and the file both lie outside. A link between them does not.
    assert found(search(root / "tools"), [root / "repository"]) is None
    assert found(search(root / "tools")) == str(root / "tools" / PROGRAM)


def test_a_directory_in_a_working_directory_that_leads_out_of_it_is_not_searched(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "repository" / "tools", root / "real")
    path = search(root / "repository" / "tools")

    assert found(path, [root / "repository"]) is None
    assert found(path) == str(root / "repository" / "tools" / PROGRAM)


def test_a_working_directory_is_known_under_every_name_that_leads_to_it(root: Path) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "repository" / "tools", root / "real")
    link_directory(root / "alias", root / "repository")

    # The entry names the working directory by another name, and a link in it leads out.
    assert found(search(root / "alias" / "tools"), [root / "repository"]) is None


def test_a_way_that_lies_outside_every_working_directory_is_taken_as_it_is_written(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "tools", root / "real")
    by_a_detour = root / "tools" / ".." / "tools"

    assert found(search(root / "tools"), [root / "repository"]) == str(root / "tools" / PROGRAM)
    assert found(search(by_a_detour), [root / "repository"]) == str(by_a_detour / PROGRAM)


def test_a_link_to_a_program_by_a_path_that_is_no_whole_path_is_followed(root: Path) -> None:
    program_in(root / "real")
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, Path("..") / "real" / PROGRAM)

    assert found(search(root / "tools")) == str(root / "tools" / PROGRAM)
    assert found(search(root / "tools"), [root / "real"]) is None


def test_a_way_with_more_links_than_are_followed_leads_to_no_program(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    program_in(root / "real")
    link_directory(root / "tools", root / "real")
    assert found(search(root / "tools")) == str(root / "tools" / PROGRAM)

    monkeypatch.setattr(programs, "_HOPS", 0)

    assert found(search(root / "tools")) is None


def test_links_that_lead_round_in_a_circle_lead_to_no_program(root: Path) -> None:
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, root / "tools" / "other")
    link_file(root / "tools" / "other", root / "tools" / PROGRAM)

    assert found(search(root / "tools")) is None


def test_a_link_that_leads_to_nothing_is_no_program(root: Path) -> None:
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, root / "gone" / PROGRAM)

    assert found(search(root / "tools")) is None


def test_a_step_back_in_a_path_is_taken_the_way_the_system_takes_it(root: Path) -> None:
    # Windows takes the steps back out of a path before it looks at anything. Other systems
    # go to the directory above the one that a link led to. So the system is asked which
    # of the two programs it would run.
    program_in(root / "bin")
    program_in(root / "far" / "bin")
    (root / "far" / "place").mkdir()
    link_directory(root / "link", root / "far" / "place")
    entry = root / "link" / ".." / "bin"
    (root / "bin" / "here").write_text("")
    run, other = root / "bin", root / "far" / "bin"
    if not (entry / "here").exists():
        run, other = other, run

    assert found(search(entry), [run]) is None
    assert found(search(entry), [other]) == str(entry / PROGRAM)


def links(monkeypatch: pytest.MonkeyPatch, made_up: dict[Path, str]) -> None:
    """Have some names pass for links, each to what is said of it.

    Not every system lets a test make a link to a file, or a link that says where it leads
    from its own place.
    """

    def leads(here: Path) -> str | None:
        return made_up.get(here)

    monkeypatch.setattr(programs, "_leads", leads)


@pytest.mark.parametrize(
    ("first", "run", "other"), [(True, "bin", "far/bin"), (False, "far/bin", "bin")]
)
def test_the_walk_takes_a_step_back_in_either_of_the_two_ways(
    root: Path, monkeypatch: pytest.MonkeyPatch, first: bool, run: str, other: str
) -> None:
    # Each system has one of the two, so the other is tried with a link that is made up.
    monkeypatch.setattr(programs, "_STEPS_BACK_FIRST", first)
    links(monkeypatch, {root / "link": str(root / "far" / "place")})
    named = root / "link" / ".." / "bin" / PROGRAM

    assert not programs.trusted(named, [root / run])
    assert programs.trusted(named, [root / other])


def test_what_a_link_says_is_counted_from_the_directory_that_the_link_lies_in(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    named = root / "tools" / PROGRAM
    links(
        monkeypatch,
        {
            named: str(root / "links" / "deep" / PROGRAM),
            root / "links" / "deep" / PROGRAM: str(Path("..") / ".." / "real" / PROGRAM),
        },
    )

    assert not programs.trusted(named, [root / "real"])
    assert not programs.trusted(named, [root / "links"])
    assert programs.trusted(named, [root / "elsewhere"])


def test_a_step_back_in_what_a_link_says_leads_to_the_directory_above(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    named = root / "tools" / PROGRAM
    links(monkeypatch, {named: str(Path("..") / "real" / PROGRAM)})

    assert not programs.trusted(named, [root / "real"])
    assert programs.trusted(named, [root / "tools" / "real"])


@pytest.mark.timeout(20)
def test_links_that_lead_round_in_a_circle_are_not_followed_for_ever(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    one, other = root / "one" / PROGRAM, root / "other" / PROGRAM
    links(monkeypatch, {one: str(other), other: str(one)})

    assert not programs.trusted(one, [])


def test_a_name_of_which_the_system_says_in_its_own_way_that_it_is_no_link_is_none(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Windows has names that are neither plain files nor links: a file that is fetched when
    # it is opened, a name that starts a packaged program. Python does not call that an
    # error of the system.
    program_in(root / "tools")

    def no_link(self: Path) -> Path:
        raise ValueError("not a symbolic link")

    monkeypatch.setattr(Path, "readlink", no_link)

    assert found(search(root / "tools")) == str(root / "tools" / PROGRAM)
    assert found(search(root / "tools"), [root / "tools"]) is None


def test_a_working_directory_is_known_by_what_it_is_and_not_only_by_its_name(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A system that takes no notice of case may leave a name the way it was written. Then
    # one directory has more names than can be compared.
    program_in(root / "Repository" / "bin")
    shouting = Path(str(root / "Repository" / "bin").upper())
    if not shouting.exists():
        pytest.skip("this system tells names apart by their case")

    with monkeypatch.context() as as_written:
        as_written.setattr(os.path, "realpath", os.path.abspath)
        within = found(str(shouting), [root / "Repository"])
        beside = found(str(shouting), [root / "elsewhere"])

    assert within is None
    assert beside == str(shouting / PROGRAM)


def test_a_directory_that_goes_by_another_name_is_known_by_its_numbers(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # One directory with two names and no link between them: only a system that takes no
    # notice of case and leaves names as they were written has that. So its answers are
    # made up here.
    program_in(root / "tools")
    (root / "work").mkdir()
    told = Path.stat

    def renamed(self: Path, *, follow_symlinks: bool = True) -> os.stat_result:
        asked = root / "work" if self == root / "tools" else self
        return told(asked, follow_symlinks=follow_symlinks)

    monkeypatch.setattr(Path, "stat", renamed)

    assert found(search(root / "tools"), [root / "work"]) is None
    assert found(search(root / "tools"), [root / "far"]) == str(root / "tools" / PROGRAM)


def test_on_a_file_system_that_numbers_nothing_a_directory_is_known_by_its_name(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    program_in(root / "tools")
    program_in(root / "work" / "bin")
    link_directory(root / "work" / "out", root / "tools")
    link_directory(root / "alias", root / "work")
    numbered = Path.stat

    def unnumbered(self: Path, *, follow_symlinks: bool = True) -> os.stat_result:
        told = numbered(self, follow_symlinks=follow_symlinks)
        return os.stat_result((told.st_mode, 0, 0, *told[3:10]))

    monkeypatch.setattr(Path, "stat", unnumbered)

    # Not every directory is the same one for that.
    assert found(search(root / "tools"), [root / "work"]) == str(root / "tools" / PROGRAM)
    assert found(search(root / "work" / "bin"), [root / "work"]) is None
    assert found(search(root / "work" / "out"), [root / "work"]) is None
    assert found(search(root / "work" / "bin"), [root / "alias"]) is None


def test_whether_a_way_is_trusted_does_not_hang_on_what_lies_at_its_end(root: Path) -> None:
    # Whether there is a program at the end is another question, with an answer of its own.
    (root / "tools").mkdir()

    assert programs.trusted(root / "tools" / PROGRAM, [root / "work"])
    assert not programs.trusted(root / "tools" / PROGRAM, [root / "tools"])


@pytest.mark.parametrize(
    ("read", "plain"),
    [
        ("\\\\?\\C:\\Program Files\\Docker", "C:\\Program Files\\Docker"),
        ("\\\\?\\UNC\\server\\share\\tools", "\\\\server\\share\\tools"),
        ("C:\\tools", "C:\\tools"),
        ("/usr/bin/docker", "/usr/bin/docker"),
        ("../real/docker", "../real/docker"),
    ],
)
def test_what_a_link_of_windows_says_it_leads_to_is_read_as_a_plain_path(
    read: str, plain: str
) -> None:
    to_plain: Any = vars(programs)["_plain"]

    assert to_plain(read) == plain


if sys.platform == "win32":

    def test_on_windows_a_working_directory_is_the_same_however_its_name_is_written(
        root: Path,
    ) -> None:
        program_in(root / "Repository" / "bin")
        shouting = str(root / "Repository" / "bin").upper()

        assert found(shouting, [root / "repository"]) is None
        assert found(shouting, [Path(str(root / "Repository").upper())]) is None
        assert found(shouting) == str(Path(shouting) / PROGRAM)


def test_a_directory_with_the_name_of_the_program_is_no_program(root: Path) -> None:
    (root / "tools" / PROGRAM).mkdir(parents=True)

    assert found(search(root / "tools")) is None


def test_quotes_around_an_entry_of_the_search_path_do_not_hide_it(root: Path) -> None:
    program_in(root / "my tools")

    assert found(f'"{root / "my tools"}"') == str(root / "my tools" / PROGRAM)


if sys.platform != "win32":

    def test_a_file_that_cannot_be_run_is_no_program(root: Path) -> None:
        program_in(root / "tools").chmod(0o644)

        assert found(search(root / "tools")) is None
