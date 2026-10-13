"""The provider that runs commands in a container: what it works out before it asks the engine."""

import contextlib
import errno
import json
import os
import stat
import sys
from collections.abc import Generator
from pathlib import Path
from typing import Any

import pytest
from pydantic import ValidationError

from columbina.sandbox import SandboxUnavailableError, docker
from columbina.sandbox.docker import DockerConfig, Engine
from support.links import link_directory, link_file, unlink_directory

WINDOWS = sys.platform == "win32"
PROGRAM = "docker.exe" if WINDOWS else "docker"

DESKTOP = (
    '{"version":"29.8.2","system":"Docker Desktop",'
    '"security":["name=seccomp,profile=builtin","name=cgroupns"],"os":"linux"}'
)
"""What the engine of this project's own machine says of itself."""


def said(**changed: object) -> str:
    """What an engine says of itself, with something about it changed."""
    return json.dumps({**json.loads(DESKTOP), **changed})


def program_in(directory: Path) -> Path:
    """Put something that passes for the docker program into a directory."""
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


@pytest.fixture
def work(root: Path) -> Path:
    directory = root / "work"
    directory.mkdir()
    return directory


def bind(source: Path, target: str, *more: str) -> str:
    return ",".join(
        [f"type=bind,source={source},target={target}", *more, "bind-recursive=disabled"]
    )


# The program


def test_the_program_is_found_in_a_directory_of_the_search_path(root: Path) -> None:
    program_in(root / "tools")

    assert docker.found(search(root / "empty", root / "tools")) == str(root / "tools" / PROGRAM)


def test_the_first_directory_that_has_the_program_gives_it(root: Path) -> None:
    program_in(root / "one")
    program_in(root / "two")

    assert docker.found(search(root / "two", root / "one")) == str(root / "two" / PROGRAM)


def test_no_program_is_found_where_the_search_path_leads_to_none(root: Path) -> None:
    (root / "tools").mkdir()

    assert docker.found(search(root / "tools", root / "not-there")) is None
    assert docker.found("") is None


def test_an_entry_that_is_no_whole_path_is_passed_over(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Such an entry means a place below the directory that the program was started in, and
    # that is where a repository would put a program of this name.
    program_in(root / "repository")
    program_in(root / "repository" / "bin")
    program_in(root / "tools")
    monkeypatch.chdir(root / "repository")

    there = docker.found(search(".", "bin", "", root / "tools"))

    assert there == str(root / "tools" / PROGRAM)
    assert docker.found(search(".", "bin", "")) is None


def test_a_program_in_a_working_directory_is_not_taken(root: Path) -> None:
    program_in(root / "repository")
    program_in(root / "repository" / "node_modules" / ".bin")
    program_in(root / "tools")
    path = search(
        root / "repository", root / "repository" / "node_modules" / ".bin", root / "tools"
    )

    assert docker.found(path, [root / "repository"]) == str(root / "tools" / PROGRAM)
    assert docker.found(path, [root / "elsewhere"]) == str(root / "repository" / PROGRAM)
    assert docker.found(search(root / "repository"), [root / "repository"]) is None


def test_a_program_that_a_link_leads_into_a_working_directory_for_is_not_taken(root: Path) -> None:
    program_in(root / "repository" / "bin")
    link_directory(root / "tools", root / "repository" / "bin")

    assert docker.found(search(root / "tools"), [root / "repository"]) is None
    assert docker.found(search(root / "tools")) == str(root / "tools" / PROGRAM)


def test_a_working_directory_that_is_named_through_a_link_is_the_same_directory(
    root: Path,
) -> None:
    program_in(root / "repository")
    link_directory(root / "alias", root / "repository")

    assert docker.found(search(root / "repository"), [root / "alias"]) is None


def test_a_link_in_a_working_directory_is_not_followed_to_a_program_outside_it(
    root: Path,
) -> None:
    # Whoever can write the directory can put another file where the link is, and the
    # program is run by its name again and again.
    program_in(root / "real")
    (root / "repository" / "bin").mkdir(parents=True)
    link_file(root / "repository" / "bin" / PROGRAM, root / "real" / PROGRAM)
    path = search(root / "repository" / "bin")

    assert docker.found(path, [root / "repository"]) is None
    assert docker.found(path, [root / "elsewhere"]) == str(root / "repository" / "bin" / PROGRAM)


def test_a_way_to_the_program_that_leads_through_a_working_directory_is_not_taken(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    (root / "tools").mkdir()
    link_file(root / "repository" / PROGRAM, root / "real" / PROGRAM)
    link_file(root / "tools" / PROGRAM, root / "repository" / PROGRAM)

    # The name and the file both lie outside. A link between them does not.
    assert docker.found(search(root / "tools"), [root / "repository"]) is None
    assert docker.found(search(root / "tools")) == str(root / "tools" / PROGRAM)


def test_a_directory_in_a_working_directory_that_leads_out_of_it_is_not_searched(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "repository" / "tools", root / "real")
    path = search(root / "repository" / "tools")

    assert docker.found(path, [root / "repository"]) is None
    assert docker.found(path) == str(root / "repository" / "tools" / PROGRAM)


def test_a_working_directory_is_known_under_every_name_that_leads_to_it(root: Path) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "repository" / "tools", root / "real")
    link_directory(root / "alias", root / "repository")

    # The entry names the working directory by another name, and a link in it leads out.
    assert docker.found(search(root / "alias" / "tools"), [root / "repository"]) is None


def test_a_way_that_lies_outside_every_working_directory_is_taken_as_it_is_written(
    root: Path,
) -> None:
    program_in(root / "real")
    (root / "repository").mkdir()
    link_directory(root / "tools", root / "real")
    by_a_detour = root / "tools" / ".." / "tools"

    assert docker.found(search(root / "tools"), [root / "repository"]) == str(
        root / "tools" / PROGRAM
    )
    assert docker.found(search(by_a_detour), [root / "repository"]) == str(by_a_detour / PROGRAM)


def test_a_link_to_a_program_by_a_path_that_is_no_whole_path_is_followed(root: Path) -> None:
    program_in(root / "real")
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, Path("..") / "real" / PROGRAM)

    assert docker.found(search(root / "tools")) == str(root / "tools" / PROGRAM)
    assert docker.found(search(root / "tools"), [root / "real"]) is None


def test_a_way_with_more_links_than_are_followed_leads_to_no_program(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    program_in(root / "real")
    link_directory(root / "tools", root / "real")
    assert docker.found(search(root / "tools")) == str(root / "tools" / PROGRAM)

    monkeypatch.setattr(docker, "_HOPS", 0)

    assert docker.found(search(root / "tools")) is None


def test_links_that_lead_round_in_a_circle_lead_to_no_program(root: Path) -> None:
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, root / "tools" / "other")
    link_file(root / "tools" / "other", root / "tools" / PROGRAM)

    assert docker.found(search(root / "tools")) is None


def test_a_link_that_leads_to_nothing_is_no_program(root: Path) -> None:
    (root / "tools").mkdir()
    link_file(root / "tools" / PROGRAM, root / "gone" / PROGRAM)

    assert docker.found(search(root / "tools")) is None


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

    assert docker.found(search(entry), [run]) is None
    assert docker.found(search(entry), [other]) == str(entry / PROGRAM)


def links(monkeypatch: pytest.MonkeyPatch, made_up: dict[Path, str]) -> None:
    """Have some names pass for links, each to what is said of it.

    Not every system lets a test make a link to a file, or a link that says where it leads
    from its own place.
    """

    def leads(here: Path) -> str | None:
        return made_up.get(here)

    monkeypatch.setattr(docker, "_leads", leads)


@pytest.mark.parametrize(
    ("first", "run", "other"), [(True, "bin", "far/bin"), (False, "far/bin", "bin")]
)
def test_the_walk_takes_a_step_back_in_either_of_the_two_ways(
    root: Path, monkeypatch: pytest.MonkeyPatch, first: bool, run: str, other: str
) -> None:
    # Each system has one of the two, so the other is tried with a link that is made up.
    monkeypatch.setattr(docker, "_STEPS_BACK_FIRST", first)
    links(monkeypatch, {root / "link": str(root / "far" / "place")})
    named = root / "link" / ".." / "bin" / PROGRAM

    assert not docker.trusted(named, [root / run])
    assert docker.trusted(named, [root / other])


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

    assert not docker.trusted(named, [root / "real"])
    assert not docker.trusted(named, [root / "links"])
    assert docker.trusted(named, [root / "elsewhere"])


def test_a_step_back_in_what_a_link_says_leads_to_the_directory_above(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    named = root / "tools" / PROGRAM
    links(monkeypatch, {named: str(Path("..") / "real" / PROGRAM)})

    assert not docker.trusted(named, [root / "real"])
    assert docker.trusted(named, [root / "tools" / "real"])


@pytest.mark.timeout(20)
def test_links_that_lead_round_in_a_circle_are_not_followed_for_ever(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    one, other = root / "one" / PROGRAM, root / "other" / PROGRAM
    links(monkeypatch, {one: str(other), other: str(one)})

    assert not docker.trusted(one, [])


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

    assert docker.found(search(root / "tools")) == str(root / "tools" / PROGRAM)
    assert docker.found(search(root / "tools"), [root / "tools"]) is None


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
        within = docker.found(str(shouting), [root / "Repository"])
        beside = docker.found(str(shouting), [root / "elsewhere"])

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

    assert docker.found(search(root / "tools"), [root / "work"]) is None
    assert docker.found(search(root / "tools"), [root / "far"]) == str(root / "tools" / PROGRAM)


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
    assert docker.found(search(root / "tools"), [root / "work"]) == str(root / "tools" / PROGRAM)
    assert docker.found(search(root / "work" / "bin"), [root / "work"]) is None
    assert docker.found(search(root / "work" / "out"), [root / "work"]) is None
    assert docker.found(search(root / "work" / "bin"), [root / "alias"]) is None


def test_whether_a_way_is_trusted_does_not_hang_on_what_lies_at_its_end(root: Path) -> None:
    # Whether there is a program at the end is another question, with an answer of its own.
    (root / "tools").mkdir()

    assert docker.trusted(root / "tools" / PROGRAM, [root / "work"])
    assert not docker.trusted(root / "tools" / PROGRAM, [root / "tools"])


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
    to_plain: Any = vars(docker)["_plain"]

    assert to_plain(read) == plain


if sys.platform == "win32":

    def test_on_windows_a_working_directory_is_the_same_however_its_name_is_written(
        root: Path,
    ) -> None:
        program_in(root / "Repository" / "bin")
        shouting = str(root / "Repository" / "bin").upper()

        assert docker.found(shouting, [root / "repository"]) is None
        assert docker.found(shouting, [Path(str(root / "Repository").upper())]) is None
        assert docker.found(shouting) == str(Path(shouting) / PROGRAM)


def test_a_directory_with_the_name_of_the_program_is_no_program(root: Path) -> None:
    (root / "tools" / PROGRAM).mkdir(parents=True)

    assert docker.found(search(root / "tools")) is None


def test_quotes_around_an_entry_of_the_search_path_do_not_hide_it(root: Path) -> None:
    program_in(root / "my tools")

    assert docker.found(f'"{root / "my tools"}"') == str(root / "my tools" / PROGRAM)


if sys.platform != "win32":

    def test_a_file_that_cannot_be_run_is_no_program(root: Path) -> None:
        program_in(root / "tools").chmod(0o644)

        assert docker.found(search(root / "tools")) is None

    def test_a_protected_name_that_is_neither_file_nor_directory_is_told_and_not_mounted(
        work: Path,
    ) -> None:
        os.mkfifo(work / ".git")

        assert docker.shape(work) == ((".git", "other"),)
        assert docker.mounts("workspace-write", work, "/workspace") == [bind(work, "/workspace")]

    def test_a_working_directory_with_a_line_break_in_its_name_gets_no_container(
        root: Path,
    ) -> None:
        work = root / "a\r\nb"
        work.mkdir()

        with pytest.raises(SandboxUnavailableError, match="has a line break in it"):
            docker.mounts("workspace-write", work, work.as_posix())

    def test_a_way_to_what_is_protected_with_a_line_break_in_it_gets_no_container(
        work: Path,
    ) -> None:
        # Mounted as it would be read, a harmless directory beside it would be read-only
        # and what the link names would not.
        (work / "me\r\nta" / "git").mkdir(parents=True)
        (work / "me\nta").mkdir()
        (work / ".git").symlink_to(work / "me\r\nta" / "git")

        with pytest.raises(SandboxUnavailableError, match="has a line break in it"):
            docker.mounts("workspace-write", work, "/workspace")


# The engine


def test_the_engine_is_asked_for_what_tells_whether_it_holds_a_command() -> None:
    assert docker.asked("/usr/bin/docker") == [
        "/usr/bin/docker",
        "info",
        "--format",
        '{"version":{{json .ServerVersion}},"system":{{json .OperatingSystem}},'
        '"security":{{json .SecurityOptions}},"os":{{json .OSType}}}',
    ]


@pytest.mark.parametrize(
    ("answer", "engine"),
    [
        (DESKTOP, Engine("29.8.2", "Docker Desktop")),
        (
            said(version="25.0.0", system="Ubuntu 24.04.1 LTS"),
            Engine("25.0.0", "Ubuntu 24.04.1 LTS"),
        ),
        (said(security=None), Engine("29.8.2", "Docker Desktop")),
        (
            # An option that only has such a word in it is no mapping of users.
            said(security=["name=seccomp,profile=rootless-userns.json"]),
            Engine("29.8.2", "Docker Desktop"),
        ),
        (
            said(security=["name=apparmor", "name=seccomp,profile=builtin"], extra=1),
            Engine("29.8.2", "Docker Desktop"),
        ),
    ],
)
def test_an_engine_that_holds_commands_is_known_by_what_it_says(
    answer: str, engine: Engine
) -> None:
    assert docker.supported(answer) == engine


@pytest.mark.parametrize(
    ("answer", "why"),
    [
        (said(version="24.0.7"), "is version 24.0.7, and version 25 or newer is needed"),
        (said(version="dev"), "is version dev, and version 25 or newer is needed"),
        (said(version="5"), "is version 5, and version 25 or newer is needed"),
        (said(os="windows"), "runs containers of the system 'windows', and commands need a Linux"),
        (said(security=["name=seccomp,profile=builtin", "name=rootless"]), "(rootless)"),
        (said(security=["name=userns"]), "(userns)"),
        ("", "did not say what it is"),
        ("template: :1: can't evaluate field ServerVersion", "did not say what it is"),
        ('["29.8.2"]', "did not say what it is"),
        ('{"version":"29.8.2"}', "did not say what it is"),
        (said(version=29), "did not say what it is"),
        (said(security="none"), "did not say what it is"),
    ],
)
def test_an_engine_that_does_not_hold_commands_is_refused_in_words_that_say_why(
    answer: str, why: str
) -> None:
    with pytest.raises(SandboxUnavailableError) as refused:
        docker.supported(answer)

    assert why in str(refused.value)


def test_a_user_mapping_is_named_as_what_the_sandbox_does_not_work_with() -> None:
    with pytest.raises(SandboxUnavailableError) as refused:
        docker.supported(said(security=["name=rootless"]))

    assert str(refused.value) == (
        "the Docker engine maps the users of a container to other users of this machine "
        "(rootless), and this sandbox does not work with such an engine"
    )


# Who a command is


def test_on_linux_a_command_is_the_user_of_this_program() -> None:
    assert docker.user_of_commands("linux", 1001, 121) == "1001:121"


@pytest.mark.parametrize("platform", ["win32", "darwin"])
def test_where_files_come_through_a_share_a_command_is_some_user_that_is_not_root(
    platform: str,
) -> None:
    assert docker.user_of_commands(platform, 501, 20) == "1000:1000"
    assert docker.user_of_commands(platform, 0, 0) == "1000:1000"


def test_a_program_that_runs_as_root_on_linux_has_no_container() -> None:
    # Its commands would be the user that keeps the container alive, and could stop that.
    with pytest.raises(SandboxUnavailableError, match="runs as root"):
        docker.user_of_commands("linux", 0, 0)


# What a container is given of this machine


@pytest.mark.parametrize(
    ("field", "written"),
    [
        ("source=/home/me/work", "source=/home/me/work"),
        ("source=C:\\Users\\me\\my work", "source=C:\\Users\\me\\my work"),
        ("source=/home/me/a, b", '"source=/home/me/a, b"'),
        ('source=/home/me/say "hi"', '"source=/home/me/say ""hi"""'),
        ("source=/home/me/two\nlines", '"source=/home/me/two\nlines"'),
        ("source=/home/me/one\rline", '"source=/home/me/one\rline"'),
    ],
)
def test_a_field_of_a_mount_is_written_the_way_the_engine_reads_it(
    field: str, written: str
) -> None:
    assert docker.quoted(field) == written


def test_a_session_that_is_read_only_gets_its_directory_read_only(work: Path) -> None:
    (work / ".git").mkdir()

    assert docker.mounts("read-only", work, "/workspace") == [bind(work, "/workspace", "readonly")]


def test_a_session_with_full_access_gets_its_directory_and_nothing_over_it(work: Path) -> None:
    (work / ".git").mkdir()

    assert docker.mounts("danger-full-access", work, "/workspace") == [bind(work, "/workspace")]


def test_a_directory_with_nothing_protected_in_it_is_one_mount(work: Path) -> None:
    (work / "src").mkdir()
    (work / ".gitignore").write_text("")

    assert docker.mounts("workspace-write", work, "/workspace") == [bind(work, "/workspace")]


def test_what_is_protected_at_the_top_is_mounted_over_the_directory_read_only(work: Path) -> None:
    (work / ".git").mkdir()
    (work / ".columbina").mkdir()
    (work / "sub" / ".git").mkdir(parents=True)

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / ".columbina", "/workspace/.columbina", "readonly"),
        bind(work / ".git", "/workspace/.git", "readonly"),
    ]


def test_a_protected_name_that_is_a_file_is_mounted_as_a_file(work: Path) -> None:
    # A working tree of a repository has a file of that name, which says where the history is.
    (work / ".git").write_text("gitdir: /elsewhere\n")

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / ".git", "/workspace/.git", "readonly"),
    ]


def test_a_protected_name_is_known_however_it_is_written(work: Path) -> None:
    (work / ".GIT").mkdir()

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / ".GIT", "/workspace/.GIT", "readonly"),
    ]


def test_a_protected_name_that_leads_elsewhere_in_the_directory_protects_that_place(
    work: Path,
) -> None:
    (work / "kept" / "hooks").mkdir(parents=True)
    link_directory(work / ".git", work / "kept")

    # The link itself is no source: the engine would follow it wherever it leads.
    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "kept", "/workspace/kept", "readonly"),
    ]


def test_a_protected_name_that_leads_below_another_directory_protects_the_first_one_on_the_way(
    work: Path,
) -> None:
    (work / "meta" / "git" / "hooks").mkdir(parents=True)
    link_directory(work / ".git", work / "meta" / "git")

    # With only what the link names read-only, a command could move `meta` aside and make
    # another one. The link would name what the command put there.
    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "meta", "/workspace/meta", "readonly"),
    ]


def test_a_protected_name_that_leads_to_nothing_yet_protects_the_way_there(work: Path) -> None:
    (work / "meta" / "git").mkdir(parents=True)
    link_directory(work / ".git", work / "meta" / "git")
    (work / "meta" / "git").rmdir()

    # Or a command would make what the link names.
    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "meta", "/workspace/meta", "readonly"),
    ]


def unseen(monkeypatch: pytest.MonkeyPatch, *names: str) -> None:
    """Have the system refuse to look at whatever goes by one of some names.

    Windows does that with a link that a command made in a container. It answers with an
    error that is none of those for a thing that is not there.
    """
    looked = Path.stat

    def refuses(self: Path, *, follow_symlinks: bool = True) -> os.stat_result:
        if self.name in names:
            raise OSError(22, "the system cannot look at this file")
        return looked(self, follow_symlinks=follow_symlinks)

    monkeypatch.setattr(Path, "stat", refuses)


def test_a_protected_name_that_cannot_be_looked_at_makes_the_whole_directory_read_only(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Whatever it is or leads to: it cannot be told, so it cannot be mounted over.
    (work / ".git").mkdir()
    unseen(monkeypatch, ".git")

    assert docker.shape(work) == ((".git", "unknown"),)
    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace", "readonly")
    ]
    assert docker.mounts("danger-full-access", work, "/workspace") == [bind(work, "/workspace")]


def test_a_way_to_what_is_protected_that_cannot_be_looked_at_makes_the_whole_read_only(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "meta" / "git").mkdir(parents=True)
    link_directory(work / ".git", work / "meta" / "git")
    unseen(monkeypatch, "meta")

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace", "readonly")
    ]


def test_what_cannot_be_looked_at_below_a_directory_that_is_protected_changes_nothing(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "meta" / "git").mkdir(parents=True)
    link_directory(work / ".git", work / "meta" / "git")
    unseen(monkeypatch, "git")

    assert docker.shape(work) == ((".git", f"link to unknown {work / 'meta' / 'git'}"),)
    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "meta", "/workspace/meta", "readonly"),
    ]


def test_a_protected_name_that_leads_out_of_the_directory_to_what_cannot_be_looked_at_is_nothing(
    work: Path, root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (root / "elsewhere").mkdir()
    link_directory(work / ".git", root / "elsewhere")
    unseen(monkeypatch, "elsewhere")

    assert docker.mounts("workspace-write", work, "/workspace") == [bind(work, "/workspace")]


@pytest.mark.parametrize("ending", ["\n", "\r\n", "\r"])
def test_a_path_with_a_line_break_in_it_is_not_given_to_docker(ending: str) -> None:
    # Docker reads the fields of a mount the way a line of CSV is read, and that takes the
    # two characters at the end of a line of Windows for one, also between quotes. So the
    # mount would be of another path than the one that was written.
    laid: Any = vars(docker)["_laid"]
    broken = Path(f"/work/a{ending}b")

    with pytest.raises(SandboxUnavailableError) as refused:
        laid("workspace-write", broken, "/workspace", [])

    assert str(refused.value) == (
        f"the path {str(broken)!r} has a line break in it, and Docker does not read such a path "
        "the way it is written. No container is made"
    )
    with pytest.raises(SandboxUnavailableError, match="has a line break in it"):
        laid("workspace-write", Path("/work"), f"/work/a{ending}b", [])
    with pytest.raises(SandboxUnavailableError, match="has a line break in it"):
        docker.kept("docker", DockerConfig(), "name", [], broken)


def test_a_protected_name_that_leads_out_of_the_directory_brings_nothing_in(
    work: Path, root: Path
) -> None:
    (root / "elsewhere").mkdir()
    (root / "elsewhere" / "secret.txt").write_text("s")
    link_directory(work / ".git", root / "elsewhere")

    mounted = docker.mounts("workspace-write", work, "/workspace")

    assert mounted == [bind(work, "/workspace")]
    assert "elsewhere" not in "".join(mounted)


def test_a_protected_name_that_leads_to_the_directory_itself_protects_all_of_it(
    work: Path,
) -> None:
    link_directory(work / ".git", work)

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace", "readonly")
    ]


def test_two_protected_names_that_lead_to_one_place_are_one_mount(work: Path) -> None:
    (work / "kept").mkdir()
    link_directory(work / ".git", work / "kept")
    link_directory(work / ".columbina", work / "kept")

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "kept", "/workspace/kept", "readonly"),
    ]


def test_a_protected_name_that_leads_to_nothing_is_no_mount(work: Path) -> None:
    (work / "gone").mkdir()
    link_directory(work / ".git", work / "gone")
    (work / "gone").rmdir()

    assert docker.mounts("workspace-write", work, "/workspace") == [bind(work, "/workspace")]


def test_a_protected_name_that_is_a_link_to_a_file_protects_the_file(work: Path) -> None:
    (work / "pointer").write_text("gitdir: /elsewhere\n")
    link_file(work / ".git", "pointer")

    assert docker.mounts("workspace-write", work, "/workspace") == [
        bind(work, "/workspace"),
        bind(work / "pointer", "/workspace/pointer", "readonly"),
    ]


@pytest.mark.parametrize("protected", [".git", ".columbina", ".GIT"])
def test_a_working_directory_inside_a_protected_directory_is_read_only_as_a_whole(
    root: Path, protected: str
) -> None:
    inner = root / protected / "hooks"
    inner.mkdir(parents=True)

    assert docker.mounts("workspace-write", inner, "/workspace") == [
        bind(inner, "/workspace", "readonly")
    ]
    assert docker.mounts("danger-full-access", inner, "/workspace") == [bind(inner, "/workspace")]


def test_a_directory_is_mounted_where_the_container_is_to_have_it(root: Path) -> None:
    work = root / "a, b"
    (work / ".git").mkdir(parents=True)
    target = work.as_posix()

    assert docker.mounts("workspace-write", work, target) == [
        f'type=bind,"source={work}","target={target}",bind-recursive=disabled',
        f'type=bind,"source={work / ".git"}","target={target}/.git",readonly,'
        "bind-recursive=disabled",
    ]


# A program that is not there


def test_a_program_that_is_on_the_search_path_leaves_nothing_to_say(root: Path) -> None:
    program_in(root / "tools")

    assert docker.absent(DockerConfig(), search(root / "tools"), [root / "work"]) is None


def test_no_program_on_the_search_path_is_said_with_what_to_do_about_it(root: Path) -> None:
    program_in(root / "work" / "bin")

    for path in (search(root / "empty"), search(root / "work" / "bin")):
        assert docker.absent(DockerConfig(), path, [root / "work"]) == (
            "commands are to run in containers, and no docker program was found on the search "
            "path. No command will run until there is one: install Docker, or name the program "
            "in the config of the sandbox"
        )


def test_a_program_that_the_config_names_is_looked_for_where_it_is_named(root: Path) -> None:
    named = program_in(root / "tools")
    gone = root / "nowhere" / PROGRAM

    assert docker.absent(DockerConfig(program=str(named)), "", []) is None
    assert docker.absent(DockerConfig(program=str(gone)), search(root / "tools"), []) == (
        f"commands are to run in containers with the docker program {gone}, which is not "
        "there. No command will run until it is"
    )


# What a model is told


def test_a_model_is_told_where_its_commands_run() -> None:
    assert docker.told(DockerConfig(), "linux") == {
        "commands run in": (
            "a Linux container of the image python:3.12, as a user that is not root. Of this "
            "machine the container has the working directory and nothing else, at the same "
            "path as here. It has no network. What a command installs outside the working "
            "directory is gone when the container is, and nothing can be installed into the "
            "directories of the system."
        )
    }


def test_on_windows_a_model_is_told_the_path_that_the_container_has_the_directory_at() -> None:
    said = docker.told(DockerConfig(image="debian:bookworm-slim"), "win32")["commands run in"]

    assert said.startswith(
        "a Linux container of the image debian:bookworm-slim, as a user that is not root. "
    )
    assert (
        "the working directory and nothing else, at /workspace. The file tools take a path "
        "that begins with /workspace for the file of this machine. It has no network."
    ) in said


def test_a_model_is_told_of_the_network_that_a_container_has() -> None:
    said = docker.told(DockerConfig(network="bridge"), "linux")["commands run in"]

    assert "path as here. It has the network of Docker. What a command installs" in said
    assert "no network" not in said


# Which directory a container has

ITSELF, TOKEN_AT, KNOWN = (2049, 1200), (39, 77), (39, 78)
"""The numbers of a working directory, of the directory of a token, and of one in it."""


@pytest.mark.parametrize(
    ("said", "same"),
    [
        # As the kernel of this machine has them.
        (["2049:1200", "39:77", "39:78"], True),
        # Through a share: every inode shifted alike, and devices of its own.
        (["73:1202", "73:79", "73:80"], True),
        (["2049:1201", "39:77", "39:78"], False),
        # The same number on another device is another directory.
        (["2050:1200", "39:77", "39:78"], False),
        (["73:1203", "73:79", "73:80"], False),
        # The two that are known disagree: the numbers of this container are its own.
        (["73:1202", "73:79", "73:85"], None),
        (["2049:1200", "39:77", "39:83"], None),
        (["2049:1200", "39:77"], None),
        (["2049:1200", "39:77", "39:78", "1:1"], None),
        (["writable", "39:77", "39:78"], None),
        (["2049:1200", "39:77", "39:x"], None),
        (["2049:1200", "39:77", "-39:78"], None),
        ([], None),
    ],
)
def test_a_container_is_known_to_have_a_directory_by_the_numbers_of_it(
    said: list[str], same: bool | None
) -> None:
    assert docker.given(said, ITSELF, TOKEN_AT, KNOWN) is same


def test_without_the_numbers_of_this_machine_nothing_is_compared() -> None:
    said = ["2049:1200", "39:77", "39:78"]

    assert docker.given(said, None, TOKEN_AT, KNOWN) is None
    assert docker.given(said, ITSELF, None, KNOWN) is None
    assert docker.given(said, ITSELF, TOKEN_AT, None) is None


def reached(path: Path) -> tuple[int, int] | None:
    by_no_link: Any = vars(docker)["_reached"]
    return by_no_link(path)


def not_reached(path: Path) -> OSError:
    """Return what the system says of a path that is not reached as a directory."""
    try:
        reached(path)
    except OSError as error:
        return error
    raise AssertionError(f"{path} was reached")


def says_no_directory(error: OSError) -> bool:
    # Linux says of a link that it is no directory, and others say that links were in the way.
    return isinstance(error, NotADirectoryError) or error.errno in (errno.ELOOP, errno.EMLINK)


def test_a_directory_is_reached_with_the_numbers_that_the_system_gives_it(work: Path) -> None:
    told = work.stat()

    assert reached(work) == (told.st_dev, told.st_ino)


def test_what_is_not_there_is_not_reached(work: Path) -> None:
    assert isinstance(not_reached(work / "gone"), FileNotFoundError)
    assert isinstance(not_reached(work / "gone" / "deeper"), FileNotFoundError)


def test_a_directory_behind_a_link_is_not_reached(root: Path) -> None:
    # Whoever can write the directory above can make its name lead anywhere.
    (root / "real" / "inner").mkdir(parents=True)
    link_directory(root / "alias", root / "real")

    assert says_no_directory(not_reached(root / "alias"))
    assert says_no_directory(not_reached(root / "alias" / "inner"))
    assert reached(root / "real" / "inner") is not None


def test_a_file_is_not_reached_as_a_directory(work: Path) -> None:
    (work / "notes.txt").write_text("")

    assert says_no_directory(not_reached(work / "notes.txt"))


# What a command may have changed


def test_a_directory_with_no_protected_name_has_no_shape(work: Path) -> None:
    (work / "src").mkdir()
    (work / ".gitignore").write_text("")

    assert docker.shape(work) == ()


def test_the_shape_says_what_each_protected_name_is(work: Path, root: Path) -> None:
    (root / "elsewhere").mkdir()
    (work / ".git").mkdir()
    (work / ".columbina").write_text("")

    assert docker.shape(work) == ((".columbina", "file"), (".git", "directory"))

    (work / ".git").rmdir()
    link_directory(work / ".git", root / "elsewhere")

    assert docker.shape(work) == (
        (".columbina", "file"),
        (".git", f"link to directory {root / 'elsewhere'}"),
    )


def test_a_protected_name_that_a_command_made_changes_the_shape(work: Path) -> None:
    before = docker.shape(work)

    (work / ".git").mkdir()

    assert docker.shape(work) != before


def test_a_directory_put_in_the_place_of_a_link_changes_the_shape(work: Path) -> None:
    (work / "kept").mkdir()
    link_directory(work / ".git", work / "kept")
    before = docker.shape(work)

    unlink_directory(work / ".git")
    (work / ".git").mkdir()

    assert before == ((".git", f"link to directory {work / 'kept'}"),)
    assert docker.shape(work) == ((".git", "directory"),)


def test_the_shape_is_the_same_in_whatever_order_the_disk_lists_the_names(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / ".git").mkdir()
    (work / ".columbina").mkdir()
    listed = docker.shape(work)
    really_scan = os.scandir

    @contextlib.contextmanager
    def backwards(path: Path) -> Generator[list[os.DirEntry[str]]]:
        with really_scan(path) as entries:
            yield list(entries)[::-1]

    monkeypatch.setattr(os, "scandir", backwards)

    assert docker.shape(work) == listed


def test_a_link_that_leads_to_nothing_is_still_told_as_a_link(work: Path) -> None:
    (work / "gone").mkdir()
    link_directory(work / ".git", work / "gone")
    (work / "gone").rmdir()

    assert docker.shape(work) == ((".git", f"link to nothing {work / 'gone'}"),)


def test_what_a_link_leads_to_coming_to_be_changes_the_shape(work: Path) -> None:
    # The link is the same and names the same place, and yet a command made what it names.
    (work / "gone").mkdir()
    link_directory(work / ".git", work / "gone")
    (work / "gone").rmdir()
    before = docker.shape(work)

    (work / "gone").mkdir()

    assert docker.shape(work) == ((".git", f"link to directory {work / 'gone'}"),)
    assert docker.shape(work) != before


# Doors that a mount leaves open


def test_a_directory_without_a_socket_leaves_nothing_against_a_full_hold(work: Path) -> None:
    (work / "src" / "deep").mkdir(parents=True)
    (work / "src" / "deep" / "a.py").write_text("")
    (work / "notes.txt").write_text("")

    assert docker.sockets(work, 1000, 60.0) is None
    assert docker.sockets(work, 4, 60.0) is None


def test_a_directory_with_more_entries_than_are_looked_at_is_not_known_to_be_free(
    work: Path,
) -> None:
    (work / "src").mkdir()
    for number in range(5):
        (work / "src" / f"{number}.py").write_text("")

    # Six entries: the directory and what is in it.
    assert docker.sockets(work, 6, 60.0) is None
    assert docker.sockets(work, 5, 60.0) == (
        "the working directory is too large to be looked through for sockets"
    )


def test_a_look_that_takes_too_long_is_cut_short(work: Path) -> None:
    (work / "notes.txt").write_text("")

    assert docker.sockets(work, 1000, 0.0) == (
        "the working directory is too large to be looked through for sockets"
    )


def test_an_empty_directory_is_looked_through_in_no_time(work: Path) -> None:
    assert docker.sockets(work, 0, 0.0) is None


class Entry:
    """What a directory lists, as far as the look for sockets asks about it."""

    def __init__(self, mode: int, *, link: bool = False) -> None:
        self._mode = mode
        self._link = link

    def is_file(self, *, follow_symlinks: bool = True) -> bool:
        return stat.S_ISREG(self._mode) and not self._link

    def is_symlink(self) -> bool:
        return self._link

    def stat(self, *, follow_symlinks: bool = True) -> os.stat_result:
        return os.stat_result((self._mode, 0, 0, 0, 0, 0, 0, 0, 0, 0))


def test_a_socket_is_what_the_system_says_is_one() -> None:
    # Not every system lets a test make one, so the answers of the system are made up here.
    is_socket: Any = vars(docker)["_is_socket"]

    assert is_socket(Entry(stat.S_IFSOCK | 0o600))
    assert not is_socket(Entry(stat.S_IFIFO | 0o600))
    assert not is_socket(Entry(stat.S_IFREG | 0o600))
    # What a link leads to is not in the container, whatever it is.
    assert not is_socket(Entry(stat.S_IFSOCK | 0o600, link=True))


def test_a_socket_is_told_by_where_it_lies_in_the_directory(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "run" / "deep").mkdir(parents=True)
    (work / "run" / "deep" / "agent.sock").write_text("")
    (work / "notes.txt").write_text("")

    def by_its_name(entry: os.DirEntry[str]) -> bool:
        return entry.name == "agent.sock"

    monkeypatch.setattr(docker, "_is_socket", by_its_name)

    assert docker.sockets(work, 1000, 60.0) == (
        "a socket lies in the working directory, at run/deep/agent.sock, and a command can "
        "talk to whatever listens there"
    )


def test_a_part_that_the_system_will_not_list_is_not_known_to_be_free(
    work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (work / "locked").mkdir()
    (work / "notes.txt").write_text("")
    really_scan: Any = os.scandir

    def refusing(path: Path) -> Any:
        if Path(path).name == "locked":
            raise PermissionError(13, "Permission denied")
        return really_scan(path)

    monkeypatch.setattr(os, "scandir", refusing)

    assert docker.sockets(work, 1000, 60.0) == (
        "a part of the working directory could not be looked through for sockets"
    )


def test_what_a_link_leads_to_is_not_looked_through(work: Path, root: Path) -> None:
    # It is not in the container either.
    (root / "elsewhere").mkdir()
    for number in range(5):
        (root / "elsewhere" / f"{number}.txt").write_text("")
    link_file(work / "link", root / "elsewhere")

    assert docker.sockets(work, 1, 60.0) is None


if sys.platform != "win32":

    def test_a_socket_is_found_wherever_it_lies(work: Path) -> None:
        import socket

        (work / "run" / "deep").mkdir(parents=True)
        (work / "run" / "notes.txt").write_text("")
        listening = socket.socket(socket.AF_UNIX)
        listening.bind(str(work / "run" / "deep" / "agent.sock"))
        try:
            found = docker.sockets(work, 1000, 60.0)
        finally:
            listening.close()

        assert found == (
            "a socket lies in the working directory, at run/deep/agent.sock, and a command "
            "can talk to whatever listens there"
        )

    def test_a_pipe_with_a_name_is_no_socket(work: Path) -> None:
        os.mkfifo(work / "pipe")

        assert docker.sockets(work, 1000, 60.0) is None

    def test_a_socket_that_a_link_leads_to_is_not_in_the_container(work: Path, root: Path) -> None:
        import socket

        listening = socket.socket(socket.AF_UNIX)
        listening.bind(str(root / "agent.sock"))
        try:
            (work / "agent.sock").symlink_to(root / "agent.sock")
            found = docker.sockets(work, 1000, 60.0)
        finally:
            listening.close()

        assert found is None

    @pytest.mark.skipif(os.geteuid() == 0, reason="root reads every directory")
    def test_a_part_that_cannot_be_looked_into_is_not_known_to_be_free(work: Path) -> None:
        (work / "locked").mkdir()
        (work / "locked").chmod(0o000)
        try:
            found = docker.sockets(work, 1000, 60.0)
        finally:
            (work / "locked").chmod(0o755)

        assert found == "a part of the working directory could not be looked through for sockets"


# The commands


def test_a_container_is_started_with_nothing_but_what_it_needs(work: Path, root: Path) -> None:
    mounted = [bind(work, "/workspace"), bind(work / ".git", "/workspace/.git", "readonly")]

    argv = docker.kept("/bin/docker", DockerConfig(), "columbina-0a1b2c", mounted, root / "probe")

    assert argv == [
        "/bin/docker",
        "run",
        "--rm",
        "--interactive",
        "--pull",
        "never",
        "--init",
        "--user",
        "0:0",
        "--name",
        "columbina-0a1b2c",
        "--label",
        "columbina=1",
        "--network",
        "none",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        "2048",
        "--mount",
        mounted[0],
        "--mount",
        mounted[1],
        "--mount",
        bind(root / "probe", "/run/columbina-probe", "readonly"),
        "--workdir",
        "/",
        "--entrypoint",
        "sh",
        "python:3.12",
        "-c",
        "if command -v bash >/dev/null 2>&1; "
        'then echo "ready $(cat /run/columbina-probe/token 2>/dev/null)"; '
        "else echo missing; fi; exec cat",
    ]


def test_what_the_config_says_of_a_container_is_passed_on(root: Path) -> None:
    config = DockerConfig(
        image="ghcr.io/me/tools@sha256:0123", network="bridge", pids_limit=64, memory="4g", cpus=1.5
    )

    argv = docker.kept("/bin/docker", config, "columbina-0a1b2c", [], root / "probe")

    limits = argv[argv.index("--network") : argv.index("--mount")]
    assert limits == [
        "--network",
        "bridge",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        "64",
        "--memory",
        "4g",
        "--cpus",
        "1.5",
    ]
    assert argv[argv.index("--entrypoint") + 2] == "ghcr.io/me/tools@sha256:0123"
    assert argv.count("--mount") == 1


def test_a_command_is_run_as_its_user_in_the_working_directory_with_a_home_of_its_own() -> None:
    argv = docker.called(
        "/bin/docker",
        DockerConfig(),
        "columbina-0a1b2c",
        "1001:121",
        "/home/me/my work",
        ["bash", "-c", "echo 'a  b'"],
    )

    assert argv == [
        "/bin/docker",
        "exec",
        "--interactive",
        "--user",
        "1001:121",
        "--workdir",
        "/home/me/my work",
        "--env",
        "LANG=C.UTF-8",
        "--env",
        "TERM=dumb",
        "--env",
        "HOME=/tmp/columbina-home",
        "columbina-0a1b2c",
        "sh",
        "-c",
        'mkdir -p "$HOME" && exec "$@"',
        "sh",
        "bash",
        "-c",
        "echo 'a  b'",
    ]


def test_a_command_gets_the_variables_of_the_config_and_keeps_its_home() -> None:
    config = DockerConfig(env={"CI": "1", "LANG": "en_US.UTF-8", "HOME": "/root", "NOTE": "a b=c"})

    argv = docker.called("/bin/docker", config, "name", "1000:1000", "/workspace", ["true"])

    given = [argv[at + 1] for at, word in enumerate(argv) if word == "--env"]
    assert given == [
        "LANG=en_US.UTF-8",
        "TERM=dumb",
        "CI=1",
        "HOME=/tmp/columbina-home",
        "NOTE=a b=c",
    ]


def test_what_commands_left_is_stopped_as_their_user_with_what_a_shell_has_built_in() -> None:
    argv = docker.ended("/bin/docker", "columbina-0a1b2c", "1001:121")

    assert argv[:7] == ["/bin/docker", "exec", "--user", "1001:121", "columbina-0a1b2c", "sh", "-c"]
    assert argv[8:] == ["sh", "1001"]
    script = argv[7]
    assert "kill -KILL -1" in script
    assert "echo none; exit 0" in script
    assert "echo left; exit 0" in script
    # No program is started to do it: a command may have left no room for one.
    words = {word for line in script.splitlines() for word in line.replace(";", " ").split()}
    assert not words & {"sleep", "cat", "grep", "ps", "id", "sed", "awk", "seq", "stat"}


def test_the_directory_is_looked_at_as_the_user_of_commands() -> None:
    assert docker.usable("/bin/docker", "columbina-0a1b2c", "1001:121", "/home/me/work") == [
        "/bin/docker",
        "exec",
        "--user",
        "1001:121",
        "--workdir",
        "/home/me/work",
        "columbina-0a1b2c",
        "sh",
        "-c",
        "test -r . && test -x . && { if test -w .; then echo writable; else echo readable; fi; "
        "stat -c %d:%i . /run/columbina-probe /run/columbina-probe/known 2>/dev/null; true; }",
    ]


def test_a_container_is_asked_for_and_removed_by_its_whole_name() -> None:
    assert docker.there("/bin/docker", "columbina-0a1b2c") == [
        "/bin/docker",
        "ps",
        "--all",
        "--filter",
        "name=^columbina-0a1b2c$",
        "--format",
        "{{.Names}}",
    ]
    assert docker.removed("/bin/docker", "columbina-0a1b2c") == [
        "/bin/docker",
        "rm",
        "--force",
        "columbina-0a1b2c",
    ]


# What went wrong, in words


@pytest.mark.parametrize(
    "wrote",
    [
        "docker: Error response from daemon: No such image: python:3.12\n\n"
        "Run 'docker run --help' for more information",
        "Unable to find image 'python:3.12' locally\ndocker: Error response from daemon: denied",
    ],
)
def test_an_image_that_is_not_there_is_told_with_the_command_that_fetches_it(wrote: str) -> None:
    assert docker.unavailable("python:3.12", wrote) == (
        "the image python:3.12 is not on this machine, and no image is fetched for a command. "
        "Fetch it with: docker pull python:3.12"
    )


@pytest.mark.parametrize(
    "wrote",
    [
        "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; "
        "check if the path is correct and if the daemon is running",
        "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. "
        "Is the docker daemon running?",
        "error during connect: Get http://%2F%2F.%2Fpipe%2Fdocker_engine/v1.47/info",
        "ERROR: Docker Desktop is stopped: the docker daemon is not running",
    ],
)
def test_an_engine_that_does_not_answer_is_told_as_one_to_start(wrote: str) -> None:
    assert docker.unavailable("python:3.12", wrote) == (
        "the Docker engine does not answer. Start Docker Desktop, or the service of Docker, "
        "and run the command again"
    )


def test_any_other_failure_is_told_in_the_first_words_of_docker() -> None:
    wrote = (
        '\ndocker: Error response from daemon: invalid mount config for type "bind": '
        "bind source path does not exist: /home/me/gone.\n"
        "See 'docker run --help'.\n"
    )

    assert docker.unavailable("python:3.12", wrote) == (
        "Docker could not start a container: docker: Error response from daemon: invalid "
        'mount config for type "bind": bind source path does not exist: /home/me/gone.'
    )
    assert docker.unavailable("python:3.12", " \n") == (
        "Docker could not start a container: it did not say why"
    )
    long = docker.unavailable("python:3.12", "x" * 1000)
    assert long == "Docker could not start a container: " + "x" * 300


# The config


def test_a_container_has_no_network_and_the_usual_image_unless_the_config_says_so() -> None:
    config = DockerConfig()

    assert (config.program, config.image, config.network, config.env) == (
        None,
        "python:3.12",
        "none",
        {},
    )
    assert (config.pids_limit, config.memory, config.cpus) == (2048, None, None)
    assert (config.start_timeout_seconds, config.idle_seconds) == (60.0, 1800.0)


def test_the_program_of_the_config_is_named_by_its_whole_path(root: Path) -> None:
    whole = str(root / PROGRAM)

    assert DockerConfig(program=whole).program == whole
    for part in ("docker", "bin/docker", "./docker"):
        with pytest.raises(ValidationError, match="named by its whole path"):
            DockerConfig(program=part)


@pytest.mark.parametrize(
    ("key", "value"),
    [
        ("image", "--privileged"),
        ("image", "python 3.12"),
        ("image", ""),
        ("network", "host"),
        ("env", {"A B": "1"}),
        ("env", {"1A": "1"}),
        ("env", {"A=B": "1"}),
        ("env", {"A": "one\0two"}),
        ("pids_limit", 15),
        ("memory", "lots"),
        ("memory", "0g"),
        ("cpus", 0),
        ("start_timeout_seconds", 0),
        ("idle_seconds", 0),
        ("privileged", True),
    ],
)
def test_a_config_that_makes_no_container_is_refused(key: str, value: object) -> None:
    with pytest.raises(ValidationError):
        DockerConfig.model_validate({key: value})


@pytest.mark.parametrize(
    ("key", "value"),
    [
        ("image", "debian:bookworm-slim"),
        ("image", "localhost:5000/me/tools:1.0"),
        ("network", "bridge"),
        ("env", {"_A1": ""}),
        ("pids_limit", 16),
        ("memory", "512m"),
        ("memory", "1073741824"),
        ("cpus", 0.5),
    ],
)
def test_a_config_that_docker_understands_is_taken(key: str, value: object) -> None:
    assert getattr(DockerConfig.model_validate({key: value}), key) == value
