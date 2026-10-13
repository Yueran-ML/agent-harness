"""The provider that runs commands in a container: what it works out before it asks the engine."""

import contextlib
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
        (".git", f"link to {root / 'elsewhere'}"),
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

    assert before == ((".git", f"link to {work / 'kept'}"),)
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

    assert docker.shape(work) == ((".git", f"link to {work / 'gone'}"),)


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
        "test -r . && test -x . && if test -w .; then echo writable; else echo readable; fi",
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
