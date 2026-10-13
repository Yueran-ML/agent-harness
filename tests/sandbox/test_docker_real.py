"""The provider that runs commands in a container, with a real engine.

What a command can reach, and what is left of it, cannot be shown with a
stand-in. These tests start containers. They are passed over where no engine
answers or an image is missing, unless ``COLUMBINA_REQUIRE_DOCKER`` is ``1``:
then that is a failure, so that a machine which is meant to run them cannot
pass without them.

They need two small images, which they do not fetch themselves:
``debian:bookworm-slim`` and ``busybox``.
"""

import asyncio
import os
import subprocess
import sys
import time
from collections.abc import AsyncIterator, Awaitable, Callable
from pathlib import Path
from typing import Any, NoReturn

import pytest

import columbina.session
from columbina.builtin_tools import shell
from columbina.kernel import Context
from columbina.process import Completed
from columbina.sandbox import Mode, SandboxUnavailableError, docker
from columbina.sandbox.docker import Docker, DockerConfig
from columbina.session import SESSIONS, Session, Sessions
from support.links import link_directory

pytestmark = pytest.mark.timeout(180)

WINDOWS = sys.platform == "win32"
IMAGE = "debian:bookworm-slim"
BARE = "busybox"
LISTS_SLEEPERS = "grep -lx sleep /proc/[0-9]*/comm 2>/dev/null | wc -l"
"""Counts the processes of a container that are a ``sleep``, by the name of their program."""


def docker_program() -> str | None:
    """Find the docker program for these tests."""
    named = os.environ.get("COLUMBINA_TEST_DOCKER")
    if named:
        return named
    found = docker.found(os.environ.get("PATH", ""))
    usual = Path(os.environ.get("PROGRAMFILES", ""), "Docker/Docker/resources/bin/docker.exe")
    if found is None and WINDOWS and usual.is_file():
        # A shell that was opened before Docker Desktop was installed does not have it yet.
        return str(usual)
    return found


def asked(program: str, *arguments: str) -> tuple[int, str]:
    """Ask the engine something, and return how that ended and what it said."""
    try:
        done = subprocess.run(
            [program, *arguments], capture_output=True, text=True, timeout=120, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return 1, str(error)
    return done.returncode, done.stdout.strip()


@pytest.fixture(scope="session")
def program() -> str:
    """The docker program, if a real engine with the images of these tests answers it."""

    def without(why: str) -> NoReturn:
        if os.environ.get("COLUMBINA_REQUIRE_DOCKER") == "1":
            pytest.fail(f"COLUMBINA_REQUIRE_DOCKER is set, and {why}")
        pytest.skip(why)

    found = docker_program()
    if found is None:
        without("no docker program was found")
    code, system = asked(found, "info", "--format", "{{.OSType}}")
    if code != 0:
        without("no Docker engine answers")
    if system != "linux":
        without("the Docker engine runs no Linux containers")
    for image in (IMAGE, BARE):
        if asked(found, "image", "inspect", "--format", "{{.Id}}", image)[0] != 0:
            without(f"the image {image} is not on this machine")
    return found


def containers(program: str) -> set[str]:
    """The names of the containers of this program that the engine has, running or not."""
    code, said = asked(
        program, "ps", "--all", "--filter", "label=columbina=1", "--format", "{{.Names}}"
    )
    assert code == 0, said
    return set(said.split())


Make = Callable[..., Docker]


@pytest.fixture
async def make(program: str) -> AsyncIterator[Make]:
    """Makes providers, and sees at the end that none of their containers is left."""
    before = containers(program)
    made: list[Docker] = []

    def one(**config: Any) -> Docker:
        given = {"program": program, "image": IMAGE, **config}
        made.append(Docker(DockerConfig.model_validate(given)))
        return made[-1]

    yield one
    for provider in made:
        await provider.close()
    assert containers(program) <= before


@pytest.fixture
def provider(make: Make) -> Docker:
    return make()


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    return ctx.use(SESSIONS)


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = Path(os.path.realpath(tmp_path)) / "work"
    directory.mkdir()
    (directory / "notes.txt").write_text("hi\n", encoding="utf-8")
    return directory


@pytest.fixture
def session(ctx: Context, sessions: Sessions, work: Path) -> Session:
    return sessions.create(ctx, work)


def inside(work: Path) -> str:
    """Where a container has a working directory."""
    return "/workspace" if WINDOWS else work.as_posix()


async def sh(
    provider: Docker,
    session: Session,
    work: Path,
    command: str,
    *,
    mode: Mode = "workspace-write",
    seconds: float = 60.0,
) -> Completed:
    """Run a command the way the tool for commands hands it over."""
    argv, given = shell.BASH_ELSEWHERE.invocation("bash", command)
    return await provider.run(
        argv,
        session=session,
        mode=mode,
        cwd=work,
        stdin=given,
        timeout_seconds=seconds,
        overflow=None,
        max_overflow_bytes=1 << 20,
        expected="none",
    )


async def said(provider: Docker, session: Session, work: Path, command: str, **more: Any) -> str:
    """Run a command that is to come to its end, and return what it wrote."""
    done = await sh(provider, session, work, command, **more)
    assert not done.timed_out, done.output
    return done.output


Says = Callable[..., Awaitable[str]]


@pytest.fixture
def says(provider: Docker, session: Session, work: Path) -> Says:
    """Runs a command in the container of the session, and returns what it wrote."""

    async def one(command: str, **more: Any) -> str:
        return await said(provider, session, work, command, **more)

    return one


# What a command is and where it runs


async def test_a_command_runs_in_the_working_directory_as_a_user_that_is_not_root(
    provider: Docker, session: Session, work: Path
) -> None:
    done = await sh(provider, session, work, 'id -u; pwd; cat notes.txt; echo "to err" >&2; exit 3')

    user, where, read, wrote = done.output.splitlines()
    assert (done.exit_code, done.timed_out, done.notes) == (3, False, ())
    assert user != "0"
    assert (where, read, wrote) == (inside(work), "hi", "to err")


async def test_what_a_command_writes_to_its_error_stream_comes_in_the_order_it_was_written(
    says: Says,
) -> None:
    # Out of a container the two streams would come on two ways, in no order.
    wrote = await says("for n in $(seq 1 200); do echo out $n; echo err $n >&2; done")

    assert wrote.splitlines() == [f"{to} {n}" for n in range(1, 201) for to in ("out", "err")]


async def test_what_a_command_writes_in_the_working_directory_is_on_this_machine(
    says: Says, work: Path
) -> None:
    await says("mkdir -p sub && printf 'made\\n' > sub/made.txt")

    assert (work / "sub" / "made.txt").read_text() == "made\n"


if sys.platform == "linux":

    async def test_what_a_command_makes_belongs_to_the_user_of_this_program(
        says: Says, work: Path
    ) -> None:
        await says("echo x > mine.txt")

        made = (work / "mine.txt").stat()
        assert (made.st_uid, made.st_gid) == (os.getuid(), os.getgid())

    async def test_a_socket_in_the_working_directory_leaves_its_commands_held_in_part(
        provider: Docker, session: Session, work: Path
    ) -> None:
        # Where this machine and the container have one kernel, a command could talk to
        # whatever listens there, with no network and through a read-only mount.
        import socket

        listening = socket.socket(socket.AF_UNIX)
        listening.bind(str(work / "agent.sock"))
        try:
            before = provider.strength(session)
            wrote = await said(provider, session, work, "test -S agent.sock && echo a socket")
        finally:
            listening.close()

        assert wrote.strip() == "a socket"
        assert (before, provider.strength(session)) == ("full", "partial")


async def test_a_command_gets_a_home_of_its_own_and_nothing_of_the_environment_of_this_program(
    make: Make, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_TEST_PLAIN", "of-this-machine")
    provider = make(env={"CI": "1"})

    wrote = await said(
        provider,
        session,
        work,
        'echo "$HOME|$LANG|$TERM|$CI|${COLUMBINA_TEST_PLAIN:-none}"; test -d "$HOME" && echo home',
    )

    assert wrote.splitlines() == ["/tmp/columbina-home|C.UTF-8|dumb|1|none", "home"]


async def test_a_command_reaches_nothing_of_this_machine_but_the_working_directory(
    says: Says, work: Path
) -> None:
    (work.parent / "beside.txt").write_text("beside\n")

    wrote = await says("cat ../beside.txt 2>&1; echo x > ../planted.txt; echo $?")

    assert "No such file or directory" in wrote
    assert wrote.splitlines()[-1] == "1"
    assert not (work.parent / "planted.txt").exists()


async def test_a_command_cannot_change_the_programs_of_its_container(says: Says) -> None:
    wrote = await says(
        "echo x > /usr/bin/planted 2>&1; echo $?; echo y > /tmp/fine && cat /tmp/fine"
    )

    assert "Permission denied" in wrote
    assert wrote.splitlines()[-2:] == ["1", "y"]


async def test_a_command_has_no_network(says: Says) -> None:
    wrote = await says("(exec 3<>/dev/tcp/1.1.1.1/53) 2>&1; echo code $?")

    assert "Network is unreachable" in wrote
    assert wrote.splitlines()[-1] == "code 1"


async def test_a_container_is_given_no_capability_and_can_gain_none(says: Says) -> None:
    wrote = await says("grep -E '^(CapBnd|CapEff|NoNewPrivs):' /proc/self/status; cat /proc/1/comm")

    assert wrote.split() == [
        "CapEff:",
        "0000000000000000",
        "CapBnd:",
        "0000000000000000",
        "NoNewPrivs:",
        "1",
        "docker-init",
    ]


async def test_the_limits_of_the_config_are_the_limits_of_the_container(
    make: Make, session: Session, work: Path
) -> None:
    provider = make(pids_limit=64, memory="256m", cpus=1.5)

    wrote = await said(
        provider,
        session,
        work,
        "cat /sys/fs/cgroup/pids.max /sys/fs/cgroup/memory.max /sys/fs/cgroup/cpu.max",
    )

    assert wrote.splitlines() == ["64", "268435456", "150000 100000"]


async def test_a_long_command_with_every_kind_of_character_arrives_as_it_was_written(
    says: Says,
) -> None:
    moon = chr(0x6708)
    long = (
        f'printf \'%s|\' "a  b" \'c$d\' "e\\"f" {moon}; echo; '
        "echo '" + "x" * 100_000 + "' | wc -c; read line || echo 'no input'"
    )

    assert (await says(long)).splitlines() == [f'a  b|c$d|e"f|{moon}|', "100001", "no input"]


# What the mode of a session lets a command write


async def test_a_session_that_is_read_only_cannot_write_its_directory(
    says: Says, work: Path
) -> None:
    wrote = await says("cat notes.txt; echo x > made.txt; echo $?", mode="read-only")

    assert wrote.splitlines()[0] == "hi"
    assert "Read-only file system" in wrote
    assert wrote.splitlines()[-1] == "1"
    assert not (work / "made.txt").exists()


async def test_what_is_protected_cannot_be_written_removed_or_moved_aside(
    says: Says, work: Path
) -> None:
    (work / ".git" / "hooks").mkdir(parents=True)
    (work / ".git" / "config").write_text("[core]\n")
    (work / ".columbina").mkdir()

    wrote = await says(
        "echo x > .git/hooks/pre-commit; echo $?; echo x >> .git/config; echo $?; "
        "mv .git aside; echo $?; rm -rf .git; echo $?; echo x > .columbina/rules.yml; echo $?; "
        "echo fine > beside.txt && cat beside.txt"
    )

    codes = [line for line in wrote.splitlines() if line.isdigit()]
    assert codes == ["1", "1", "1", "1", "1"]
    assert wrote.splitlines()[-1] == "fine"
    assert (work / ".git" / "config").read_text() == "[core]\n"
    assert sorted(path.name for path in (work / ".git").iterdir()) == ["config", "hooks"]
    assert list((work / ".git" / "hooks").iterdir()) == []
    assert list((work / ".columbina").iterdir()) == []


async def test_a_protected_name_that_is_a_file_cannot_be_written_or_replaced(
    says: Says, work: Path
) -> None:
    (work / ".git").write_text("gitdir: /elsewhere\n")

    wrote = await says(
        "echo x >> .git; echo $?; echo other > new && mv new .git; echo $?; rm -f .git; echo $?"
    )

    assert [line for line in wrote.splitlines() if line.isdigit()] == ["1", "1", "1"]
    assert (work / ".git").read_text() == "gitdir: /elsewhere\n"


async def test_a_session_with_full_access_can_write_what_is_protected(
    says: Says, work: Path
) -> None:
    (work / ".git").mkdir()

    wrote = await says("echo x > .git/config; echo $?", mode="danger-full-access")

    assert wrote.splitlines() == ["0"]
    assert (work / ".git" / "config").read_text() == "x\n"


async def test_a_protected_name_that_leads_out_of_the_directory_brings_nothing_of_it_in(
    says: Says, work: Path
) -> None:
    # The engine follows a link that it is given as a source. So it is given none.
    outside = work.parent / "outside"
    outside.mkdir()
    (outside / "secret.txt").write_text("the secret\n")
    link_directory(work / ".git", outside)

    wrote = await says("cat .git/secret.txt 2>&1; echo x > .git/planted.txt 2>&1; echo done")

    assert "the secret" not in wrote
    assert "done" in wrote.splitlines()
    assert sorted(path.name for path in outside.iterdir()) == ["secret.txt"]


async def test_what_a_protected_name_leads_to_in_the_directory_cannot_be_written(
    says: Says, work: Path
) -> None:
    (work / "kept").mkdir()
    link_directory(work / ".git", work / "kept")

    wrote = await says("echo x > kept/config; echo $?; echo fine > beside.txt; echo $?")

    assert [line for line in wrote.splitlines() if line.isdigit()] == ["1", "0"]
    assert list((work / "kept").iterdir()) == []


async def test_the_way_to_what_a_protected_name_leads_to_cannot_be_moved_aside(
    says: Says, work: Path
) -> None:
    # With only the end of the way read-only, a command could move the directory above it
    # aside and make another one, and the link would name what the command put there.
    (work / "meta" / "git").mkdir(parents=True)
    (work / "meta" / "git" / "config").write_text("[core]\n")
    link_directory(work / ".git", work / "meta" / "git")

    wrote = await says(
        "mv meta old; echo $?; mkdir -p meta/git/hooks; echo $?; echo x > meta/git/config; "
        "echo $?; echo fine > beside.txt && cat beside.txt"
    )

    assert [line for line in wrote.splitlines() if line.isdigit()] == ["1", "1", "1"]
    assert wrote.splitlines()[-1] == "fine"
    assert sorted(path.name for path in work.iterdir()) == [
        ".git",
        "beside.txt",
        "meta",
        "notes.txt",
    ]
    assert (work / "meta" / "git" / "config").read_text() == "[core]\n"
    assert [path.name for path in (work / "meta" / "git").iterdir()] == ["config"]


async def test_a_command_that_makes_a_protected_name_cannot_write_it_from_the_next_command_on(
    provider: Docker, session: Session, work: Path
) -> None:
    made = await sh(provider, session, work, "mkdir .git && echo made")
    after = await sh(provider, session, work, "echo x > .git/config; echo $?; echo y > fine.txt")

    assert made.output.strip() == "made"
    assert len(made.notes) == 1
    assert made.notes[0].startswith("This command changed what .git or .columbina is")
    assert [line for line in after.output.splitlines() if line.isdigit()] == ["1"]
    assert after.notes == ()
    assert list((work / ".git").iterdir()) == []
    assert (work / "fine.txt").exists()


async def test_a_directory_with_blanks_a_comma_and_other_letters_in_its_name_is_mounted(
    provider: Docker, ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    work = Path(os.path.realpath(tmp_path)) / f"a, b {chr(0x6708)}"
    (work / ".git").mkdir(parents=True)
    session = sessions.create(ctx, work)

    wrote = await said(
        provider, session, work, "echo x > made.txt; echo $?; echo y > .git/no; echo $?"
    )

    assert [line for line in wrote.splitlines() if line.isdigit()] == ["0", "1"]
    assert (work / "made.txt").read_text() == "x\n"


# A container for a session


async def test_the_commands_of_a_session_run_in_one_container_and_those_of_another_in_another(
    ctx: Context, provider: Docker, sessions: Sessions, work: Path
) -> None:
    one, two = sessions.create(ctx, work), sessions.create(ctx, work)

    first = await said(provider, one, work, "hostname; echo kept > /tmp/kept")
    again = await said(provider, one, work, "hostname; cat /tmp/kept")
    other = await said(provider, two, work, "hostname; cat /tmp/kept 2>&1")

    assert again.splitlines() == [first.strip(), "kept"]
    assert other.splitlines()[0] != first.strip()
    assert "No such file" in other


async def test_a_session_whose_mode_has_changed_gets_another_container(
    provider: Docker, session: Session, work: Path
) -> None:
    first = await said(provider, session, work, "hostname")

    other = await said(
        provider, session, work, "hostname; echo x > made.txt; echo $?", mode="read-only"
    )

    assert other.splitlines()[0] != first.strip()
    assert other.splitlines()[-1] == "1"


async def test_a_command_cannot_stop_what_keeps_its_container(says: Says) -> None:
    tries = (
        'for entry in /proc/[0-9]*; do pid=${entry#/proc/}; [ "$pid" = "$$" ] && continue; '
        "kill -STOP $pid 2>/dev/null && echo stopped $pid; kill -KILL $pid 2>/dev/null; done; "
        "hostname"
    )

    first = await says(tries)
    after = await says("hostname")

    assert "stopped" not in first
    assert after.strip() == first.strip()


# What is left of a command


async def test_what_a_command_left_running_is_stopped_wherever_it_went(says: Says) -> None:
    started = await says(
        "sleep 300 & setsid sleep 301 & (setsid bash -c 'sleep 303 & sleep 304' &); "
        # One that does not go when it is asked to.
        "(trap '' TERM; exec sleep 305) & "
        f"sleep 0.5; hostname; {LISTS_SLEEPERS}"
    )
    after = await says(f"hostname; {LISTS_SLEEPERS}")

    name, running = started.split()
    assert int(running) >= 5
    assert after.split() == [name, "0"]


async def test_a_command_whose_time_is_up_is_stopped_with_all_that_it_started(
    provider: Docker, session: Session, work: Path
) -> None:
    name = (await said(provider, session, work, "hostname")).strip()
    began = time.monotonic()

    done = await sh(provider, session, work, "setsid sleep 300 & sleep 300", seconds=1.5)
    after = await said(provider, session, work, f"hostname; {LISTS_SLEEPERS}")

    assert (done.timed_out, done.exit_code, done.notes) == (True, None, ())
    assert time.monotonic() - began < 30
    assert after.split() == [name, "0"]


async def test_a_command_that_forks_without_end_is_stopped(
    make: Make, session: Session, work: Path
) -> None:
    provider = make(pids_limit=64)
    name = (await said(provider, session, work, "hostname")).strip()

    done = await sh(provider, session, work, "f() { f | f & }; f 2>/dev/null; sleep 300", seconds=3)
    after = await said(provider, session, work, "hostname; ls -d /proc/[0-9]* | wc -l")

    assert done.timed_out
    kept, running = after.split()
    assert kept == name
    assert int(running) < 10


async def test_a_command_whose_caller_is_cancelled_is_stopped_in_its_container(
    provider: Docker, session: Session, work: Path
) -> None:
    name = (await said(provider, session, work, "hostname")).strip()
    running = asyncio.ensure_future(sh(provider, session, work, "setsid sleep 300 & sleep 300"))
    await asyncio.sleep(1.5)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=60)
    after = await said(provider, session, work, f"hostname; {LISTS_SLEEPERS}")
    assert after.split() == [name, "0"]


HOLDS_A_CONTAINER = """
import asyncio, sys
from pathlib import Path
from types import SimpleNamespace
from columbina.builtin_tools import shell
from columbina.sandbox.docker import Docker, DockerConfig

async def main():
    provider = Docker(DockerConfig(program=sys.argv[1], image=sys.argv[2]))
    argv, given = shell.BASH_ELSEWHERE.invocation("bash", "hostname")
    done = await provider.run(
        argv, session=SimpleNamespace(id="held", closed=False), mode="read-only",
        cwd=Path(sys.argv[3]), stdin=given, timeout_seconds=60, overflow=None,
        max_overflow_bytes=1 << 20, expected="none",
    )
    print(done.output.strip(), flush=True)
    await asyncio.sleep(600)

asyncio.run(main())
"""


def test_a_container_does_not_outlive_the_program_that_keeps_it(program: str, work: Path) -> None:
    holder = subprocess.Popen(
        [sys.executable, "-c", HOLDS_A_CONTAINER, program, IMAGE, str(work)],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        assert holder.stdout is not None
        name = holder.stdout.readline().strip()
        assert name
        assert asked(program, "ps", "--quiet", "--filter", f"id={name}")[1] != ""
    finally:
        # Ended without a word of farewell, as by a crash.
        holder.kill()
        holder.wait()
        if holder.stdout is not None:
            holder.stdout.close()

    deadline = time.monotonic() + 30
    while asked(program, "ps", "--all", "--quiet", "--filter", f"id={name}")[1] != "":
        assert time.monotonic() < deadline, "the container is still there"
        time.sleep(0.2)


# No container


async def test_an_image_without_bash_is_told_and_leaves_no_container(
    make: Make, session: Session, work: Path
) -> None:
    provider = make(image=BARE)

    with pytest.raises(SandboxUnavailableError) as refused:
        await sh(provider, session, work, "echo hi")

    assert str(refused.value) == (
        "the image busybox has no bash, and commands are run with bash. Name an image that "
        "has it in the config of the sandbox"
    )


async def test_an_image_that_is_not_on_this_machine_is_not_fetched(
    make: Make, session: Session, work: Path
) -> None:
    provider = make(image="columbina/no-such-image:1")

    with pytest.raises(SandboxUnavailableError) as refused:
        await sh(provider, session, work, "echo hi")

    assert str(refused.value) == (
        "the image columbina/no-such-image:1 is not on this machine, and no image is fetched "
        "for a command. Fetch it with: docker pull columbina/no-such-image:1"
    )
