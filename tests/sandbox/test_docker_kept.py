"""The provider that runs commands in a container: keeping one, running in it, letting it go.

A stand-in takes the place of the ``docker`` program here, so that these
tests run on machines that have no engine. A container is a process of the
stand-in, and a command in it runs on this machine. What the stand-in cannot
show is whether a command is held: that is tried with a real engine.
"""

import asyncio
import csv
import json
import logging
import os
import re
import sys
import time
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

import columbina.fs
import columbina.sandbox
import columbina.session
from columbina.kernel import Context
from columbina.process import Completed
from columbina.sandbox import SANDBOX, Mode, SandboxUnavailableError, Strength, docker
from columbina.sandbox.docker import Docker, DockerConfig
from columbina.session import SESSIONS, Session, Sessions
from support.processes import alive, end, gone, python

pytestmark = pytest.mark.timeout(90)

WINDOWS = sys.platform == "win32"
FAKE = Path(__file__).resolve().parents[1] / "support" / "fake_docker.py"
FIRST = ["info", "run", "usable", "command", "stop"]
"""What the engine is asked for the first command of a session."""

NEW_ONE = (
    " The next command gets a new container, without what was installed outside the working "
    "directory."
)
ENDED = "The container ended while this command ran, so the command may have been cut short."
LEFT = (
    "What this command left running in the container could not be stopped, so the container "
    "was removed."
)
RESHAPED = (
    "This command changed what .git or .columbina is at the top of the working directory."
    f"{NEW_ONE} There a command cannot write what is new of the two."
)

STAYS = "import time; time.sleep(60)"
"""A command that goes on for a minute."""


class Engine:
    """The stand-in for the docker program, as a test sees and steers it."""

    def __init__(self, home: Path, program: Path) -> None:
        self.home = home
        self.program = str(program)
        self.fails: set[str] = set()
        self.held: set[str] = set()
        self.holding = asyncio.Event()
        self.go_on = asyncio.Event()

    def told(self, **how: object) -> None:
        """Say how the engine is to behave from now on."""
        (self.home / "told.json").write_text(json.dumps(how), encoding="utf-8")

    def _noted(self) -> list[dict[str, Any]]:
        noted = sorted((self.home / "calls").glob("*.json"))
        return [json.loads(note.read_text(encoding="utf-8")) for note in noted]

    def calls(self) -> list[list[str]]:
        """The arguments of every call of the program so far."""
        return [note["argv"] for note in self._noted() if "argv" in note]

    def verbs(self) -> list[str]:
        """What the program was called for, a word for each call."""
        return [verb(argv) for argv in self.calls()]

    def stops(self) -> list[int]:
        """How many commands ran in a container at each stop."""
        return [note["running"] for note in self._noted() if "stop" in note]

    def containers(self) -> dict[str, int]:
        """The containers that are there: their names, and the process that each one is."""
        there: dict[str, int] = {}
        for known in sorted((self.home / "containers").glob("*.json")):
            pid = json.loads(known.read_text(encoding="utf-8"))["pid"]
            if alive(pid):
                there[known.stem] = pid
        return there

    def of(self, word: str) -> list[list[str]]:
        """The arguments of the calls of one kind."""
        return [argv for argv in self.calls() if verb(argv) == word]

    def started(self) -> list[int]:
        """The processes of the commands that were started in containers."""
        return [note["started"] for note in self._noted() if "started" in note]

    def given(self) -> list[list[str]]:
        """Which variables of a test each call of the program was given."""
        return [note["given"] for note in self._noted() if "argv" in note]


def verb(argv: list[str]) -> str:
    """Say in a word what the docker program was called for."""
    if argv[0] != "exec":
        return argv[0]
    script = argv[argv.index("-c") + 1]
    if "kill -KILL -1" in script:
        return "stop"
    return "usable" if "test -w ." in script else "command"


def mounted(argv: list[str]) -> list[str]:
    return [argv[at + 1] for at, word in enumerate(argv) if word == "--mount"]


@pytest.fixture
async def engine(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Engine:
    home = tmp_path / "engine"
    home.mkdir()
    program = tmp_path / "bin" / ("docker.exe" if WINDOWS else "docker")
    program.parent.mkdir()
    program.write_bytes(b"")
    program.chmod(0o755)
    monkeypatch.setenv("COLUMBINA_FAKE_DOCKER", str(home))
    # Whoever runs these tests: as root on Linux nothing would be put into a container.
    monkeypatch.setattr(docker, "_me", lambda: (1000, 1000))
    made = Engine(home, program)
    loop = asyncio.get_running_loop()
    really_start: Any = loop.subprocess_exec

    async def start(factory: Any, name: str, *arguments: str, **options: Any) -> Any:
        if name != made.program:
            return await really_start(factory, name, *arguments, **options)
        what = verb(list(arguments))
        if what in made.fails:
            raise OSError(8, "Exec format error")
        started = await really_start(factory, sys.executable, str(FAKE), *arguments, **options)
        if what in made.held:
            made.holding.set()
            await made.go_on.wait()
        return started

    monkeypatch.setattr(loop, "subprocess_exec", start)
    return made


@pytest.fixture
async def provider(engine: Engine) -> AsyncIterator[Docker]:
    made = Docker(DockerConfig(program=engine.program))
    yield made
    engine.go_on.set()
    await made.close()


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    return ctx.use(SESSIONS)


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = Path(os.path.realpath(tmp_path)) / "work"
    directory.mkdir()
    return directory


@pytest.fixture
def session(ctx: Context, sessions: Sessions, work: Path) -> Session:
    return sessions.create(ctx, work)


async def ran(
    provider: Docker,
    session: Session,
    work: Path,
    code: str,
    *arguments: object,
    mode: Mode = "workspace-write",
    stdin: bytes | None = None,
    seconds: float = 30.0,
    expected: Strength = "none",
) -> Completed:
    """Run a small program of Python as a command of a session."""
    return await provider.run(
        python(code, *(str(argument) for argument in arguments)),
        session=session,
        mode=mode,
        cwd=work,
        stdin=stdin,
        timeout_seconds=seconds,
        overflow=None,
        max_overflow_bytes=1 << 20,
        expected=expected,
    )


async def until(what: Any, within: float = 20.0) -> None:
    """Wait until something is so."""
    deadline = time.monotonic() + within
    while not what():
        assert time.monotonic() < deadline, "it did not come to be"
        await asyncio.sleep(0.05)


async def begun(engine: Engine, before: int) -> int:
    """Wait until a command runs in a container, and return which process it is.

    The stand-in says so. A command that is stopped early may not live long
    enough to say it itself.
    """
    await until(lambda: len(engine.started()) > before)
    return engine.started()[before]


def quick(monkeypatch: pytest.MonkeyPatch, **seconds: float) -> None:
    """Make the provider look and give up sooner than it does for a person."""
    for name, value in seconds.items():
        monkeypatch.setattr(docker, f"_{name.upper()}_SECONDS", value)


# A container for a session


async def test_the_first_command_of_a_session_makes_a_container_and_runs_in_it(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    done = await ran(provider, session, work, "import os; print(os.getcwd()); raise SystemExit(3)")

    assert (done.exit_code, done.timed_out, done.notes) == (3, False, ())
    assert Path(done.output.strip()).samefile(work)
    assert engine.verbs() == FIRST
    (name,) = engine.containers()
    assert re.fullmatch(r"columbina-[0-9a-f]{12}", name)


async def test_the_next_command_of_the_session_runs_in_the_same_container(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "print('one')")
    before = engine.containers()

    done = await ran(provider, session, work, "print('two')")

    assert done.output.strip() == "two"
    assert engine.verbs() == [*FIRST, "command", "stop"]
    assert engine.containers() == before


async def test_every_session_has_a_container_of_its_own(
    ctx: Context, provider: Docker, engine: Engine, sessions: Sessions, work: Path
) -> None:
    one, two = sessions.create(ctx, work), sessions.create(ctx, work)

    await ran(provider, one, work, "pass")
    await ran(provider, two, work, "pass")

    # The engine is asked once what it is, and a container is made for each.
    assert engine.verbs() == [*FIRST, "run", "usable", "command", "stop"]
    assert len(engine.containers()) == 2


async def test_a_container_is_given_the_directory_the_way_its_mode_says(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    (work / ".git").mkdir()
    target = "/workspace" if WINDOWS else work.as_posix()

    await ran(provider, session, work, "pass")

    (started,) = engine.of("run")
    *given, probe = mounted(started)
    assert given == docker.mounts("workspace-write", work, target)
    assert probe.endswith(",target=/run/columbina-probe,readonly,bind-recursive=disabled")
    (command,) = engine.of("command")
    assert command[command.index("--workdir") + 1] == target
    assert command[command.index("--user") + 1] == docker.user_of_commands(
        sys.platform, *vars(docker)["_me"]()
    )


@pytest.mark.parametrize("mode", ["read-only", "danger-full-access"])
async def test_a_session_whose_mode_has_changed_gets_another_container(
    provider: Docker, engine: Engine, session: Session, work: Path, mode: Mode
) -> None:
    (work / ".git").mkdir()
    target = "/workspace" if WINDOWS else work.as_posix()
    await ran(provider, session, work, "pass")
    ((old, keeper),) = engine.containers().items()

    done = await ran(provider, session, work, "print('there')", mode=mode)

    assert done.output.strip() == "there"
    # The old one is removed and seen to be gone before the new one is made.
    assert engine.verbs() == [*FIRST, "rm", "ps", "run", "usable", "command", "stop"]
    assert engine.of("rm")[0][-1] == old
    assert await gone(keeper)
    assert old not in engine.containers()
    assert mounted(engine.of("run")[1])[:-1] == docker.mounts(mode, work, target)


async def test_a_session_that_works_in_another_directory_gets_another_container(
    provider: Docker, engine: Engine, session: Session, work: Path, tmp_path: Path
) -> None:
    other = Path(os.path.realpath(tmp_path)) / "other"
    other.mkdir()
    await ran(provider, session, work, "pass")

    done = await ran(provider, session, other, "import os; print(os.getcwd())")

    assert Path(done.output.strip()).samefile(other)
    assert engine.verbs() == [*FIRST, "rm", "ps", "run", "usable", "command", "stop"]
    assert len(engine.containers()) == 1


async def test_a_directory_that_is_named_through_a_link_is_the_same_directory(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")

    await ran(provider, session, work / "sub" / "..", "pass")

    assert engine.verbs().count("run") == 1


async def test_a_command_that_makes_a_protected_name_costs_the_container_and_says_so(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    target = "/workspace" if WINDOWS else work.as_posix()

    made = await ran(provider, session, work, "import os; os.mkdir('.git'); print('made')")
    after = await ran(provider, session, work, "print('after')")

    assert made.output.strip() == "made"
    assert made.notes == (RESHAPED,)
    assert after.notes == ()
    assert engine.verbs() == [*FIRST, "rm", "ps", "run", "usable", "command", "stop"]
    # In the new container what the command made cannot be written.
    assert mounted(engine.of("run")[1])[:-1] == docker.mounts("workspace-write", work, target)
    assert f"target={target}/.git,readonly" in mounted(engine.of("run")[1])[1]


async def test_a_protected_name_that_came_from_elsewhere_costs_the_container_before_a_command(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")

    (work / ".columbina").mkdir()
    done = await ran(provider, session, work, "print('after')")

    # It was not this command that made it, so the command has nothing to say of it.
    assert (done.output.strip(), done.notes) == ("after", ())
    assert engine.verbs() == [*FIRST, "rm", "ps", "run", "usable", "command", "stop"]


@pytest.mark.parametrize("mode", ["read-only", "danger-full-access"])
async def test_where_nothing_is_mounted_over_the_directory_a_protected_name_changes_nothing(
    provider: Docker, engine: Engine, session: Session, work: Path, mode: Mode
) -> None:
    await ran(provider, session, work, "pass", mode=mode)

    (work / ".git").mkdir()
    done = await ran(provider, session, work, "pass", mode=mode)

    assert done.notes == ()
    assert engine.verbs().count("run") == 1


async def test_a_command_does_not_make_containers_without_end(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Something goes on changing what is protected in the directory.
    looks: list[int] = []

    def never_the_same(cwd: Path) -> tuple[tuple[str, str], ...]:
        looks.append(len(looks))
        return ((".git", f"link to somewhere else each time, {len(looks)}"),)

    monkeypatch.setattr(docker, "_seen", never_the_same)

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "open('ran', 'w').close()")

    assert str(refused.value) == (
        "3 containers in a row were of no use for this command. Of the last one: what is "
        "protected in its directory is not what it was"
    )
    assert engine.verbs().count("run") == 3
    assert engine.containers() == {}
    assert not (work / "ran").exists()


async def test_a_container_that_has_ended_is_replaced_for_the_next_command(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")
    ((old, keeper),) = engine.containers().items()
    end(keeper)
    assert await gone(keeper)
    # The loop hears of the end of a program a moment after the system knows of it.
    await asyncio.sleep(0.5)

    done = await ran(provider, session, work, "print('there')")

    assert done.output.strip() == "there"
    assert engine.verbs().count("run") == 2
    assert list(engine.containers()) != [old]


async def test_a_container_that_ends_under_a_command_is_told_with_the_result(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")
    (keeper,) = engine.containers().values()
    ends_it = (
        "import os, signal, sys, time\n"
        "os.kill(int(sys.argv[1]), signal.SIGTERM)\n"
        "time.sleep(1.0)\n"
        "print('went on')\n"
    )

    done = await ran(provider, session, work, ends_it, keeper)
    after = await ran(provider, session, work, "print('after')")

    assert done.notes == (ENDED + NEW_ONE,)
    assert (after.output.strip(), after.notes) == ("after", ())
    assert engine.verbs().count("run") == 2


# Letting containers go


async def test_closing_lets_every_container_go(
    ctx: Context, engine: Engine, sessions: Sessions, work: Path
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    for _ in range(2):
        await ran(provider, sessions.create(ctx, work), work, "pass")
    kept = engine.containers()
    probes = [Path(docker_fields(mounted(argv)[-1])["source"]) for argv in engine.of("run")]
    assert all(probe.is_dir() for probe in probes)

    await provider.close()

    assert engine.containers() == {}
    for keeper in kept.values():
        assert await gone(keeper)
    assert sorted(argv[-1] for argv in engine.of("rm")) == sorted(kept)
    assert not any(probe.exists() for probe in probes)


def docker_fields(mount: str) -> dict[str, str]:
    """Read one mount the way the engine does."""
    read: dict[str, str] = {}
    for field in next(csv.reader([mount])):
        key, _, value = field.partition("=")
        read[key] = value
    return read


async def test_a_provider_that_is_closed_runs_no_command(
    engine: Engine, session: Session, work: Path
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await provider.close()

    with pytest.raises(SandboxUnavailableError, match="the sandbox is closed"):
        await ran(provider, session, work, "pass")

    assert engine.calls() == []


async def test_the_container_of_a_session_that_is_over_is_let_go(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, look=0.05)
    await ran(provider, session, work, "pass")
    assert len(engine.containers()) == 1

    session.close()

    await until(lambda: engine.verbs() == [*FIRST, "rm", "ps"])
    assert engine.containers() == {}


async def test_a_container_in_which_no_command_ran_for_a_while_is_let_go(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    quick(monkeypatch, look=0.05)
    provider = Docker(DockerConfig(program=engine.program, idle_seconds=0.3))
    await ran(provider, session, work, "pass")

    await until(lambda: engine.containers() == {})
    done = await ran(provider, session, work, "print('again')")

    assert done.output.strip() == "again"
    assert engine.verbs().count("run") == 2
    await provider.close()


async def test_a_container_in_which_commands_go_on_running_is_kept(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    quick(monkeypatch, look=0.05)
    provider = Docker(DockerConfig(program=engine.program, idle_seconds=1.5))

    for _ in range(4):
        await ran(provider, session, work, "pass")
        await asyncio.sleep(0.6)

    # Longer than a container may be idle has gone by, but never without a command.
    assert engine.verbs().count("run") == 1
    await provider.close()


async def test_a_container_is_not_let_go_while_a_command_runs_in_it(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    quick(monkeypatch, look=0.05)
    provider = Docker(DockerConfig(program=engine.program, idle_seconds=0.2))
    await ran(provider, session, work, "pass")

    done = await ran(provider, session, work, "import time; time.sleep(1.5); print('done')")

    assert (done.output.strip(), done.exit_code, done.notes) == ("done", 0, ())
    assert engine.verbs() == [*FIRST, "command", "stop"]
    await provider.close()


async def test_a_container_that_ended_by_itself_is_cleared_away(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, look=0.05)
    await ran(provider, session, work, "pass")
    (keeper,) = engine.containers().values()

    end(keeper)

    await until(lambda: "ps" in engine.verbs())
    assert engine.verbs() == [*FIRST, "rm", "ps"]


async def test_closing_lets_the_containers_go_whatever_became_of_the_look(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    quick(monkeypatch, look=0.05)
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    kept: Any = provider

    async def fails(container: object, why: str) -> None:
        raise RuntimeError("the look fell over")

    kept._let_go = fails
    session.close()
    await asyncio.sleep(0.5)
    del kept._let_go

    await provider.close()

    assert engine.containers() == {}
    assert engine.verbs() == [*FIRST, "rm", "ps"]


async def test_a_container_that_is_being_made_when_the_provider_is_closed_is_let_go(
    engine: Engine, session: Session, work: Path
) -> None:
    engine.held.add("run")
    provider = Docker(DockerConfig(program=engine.program))
    asked = asyncio.ensure_future(ran(provider, session, work, "print('never')"))
    await engine.holding.wait()

    closing = asyncio.ensure_future(provider.close())
    await asyncio.sleep(0.2)
    waits = not closing.done()
    engine.go_on.set()
    await closing

    assert waits
    with pytest.raises(SandboxUnavailableError, match="the sandbox is closed"):
        await asked
    assert engine.containers() == {}
    assert engine.verbs()[-2:] == ["rm", "ps"]


async def test_a_container_that_the_engine_will_not_remove_is_told_to_the_user(
    engine: Engine, session: Session, work: Path, caplog: pytest.LogCaptureFixture
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    (name,) = engine.containers()
    engine.told(rm="fails", ps="stuck")

    with caplog.at_level(logging.WARNING, logger="columbina.sandbox.docker"):
        await provider.close()

    assert caplog.messages == [
        f"the container {name} may still be there. Remove it with: docker rm -f {name}"
    ]


async def test_a_container_that_is_gone_is_let_go_without_a_word(
    engine: Engine, session: Session, work: Path, caplog: pytest.LogCaptureFixture
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")

    with caplog.at_level(logging.WARNING, logger="columbina.sandbox.docker"):
        await provider.close()

    assert caplog.messages == []


async def test_an_engine_that_no_longer_answers_runs_no_container_to_warn_of(
    engine: Engine, session: Session, work: Path, caplog: pytest.LogCaptureFixture
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    engine.told(engine="down")

    with caplog.at_level(logging.WARNING, logger="columbina.sandbox.docker"):
        await provider.close()

    assert caplog.messages == []
    assert engine.containers() == {}


async def test_an_engine_that_cannot_say_whether_a_container_is_gone_leaves_it_in_doubt(
    engine: Engine, session: Session, work: Path, caplog: pytest.LogCaptureFixture
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    (name,) = engine.containers()
    engine.told(ps="fails")

    with caplog.at_level(logging.WARNING, logger="columbina.sandbox.docker"):
        await provider.close()

    assert caplog.messages == [
        f"the container {name} may still be there. Remove it with: docker rm -f {name}"
    ]


async def test_closing_while_a_command_runs_ends_the_command_with_its_container(
    engine: Engine, session: Session, work: Path
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    running = asyncio.ensure_future(ran(provider, session, work, STAYS))
    command = await begun(engine, 0)

    await provider.close()
    done = await asyncio.wait_for(running, timeout=30)

    assert done.notes == (ENDED + NEW_ONE,)
    assert await gone(command)
    assert engine.containers() == {}
    # The container was let go once, by whoever closed.
    assert engine.verbs().count("rm") == 1


async def test_what_keeps_a_container_is_ended_if_it_does_not_end_by_itself(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    quick(monkeypatch, keeper=0.5)
    engine.told(run="stays")
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    (keeper,) = engine.containers().values()
    engine.told(run="stays", rm="fails")

    await provider.close()

    assert await gone(keeper)


async def test_a_program_that_cannot_be_started_when_a_container_is_let_go_is_no_failure(
    engine: Engine, session: Session, work: Path, caplog: pytest.LogCaptureFixture
) -> None:
    provider = Docker(DockerConfig(program=engine.program))
    await ran(provider, session, work, "pass")
    (keeper,) = engine.containers().values()
    engine.fails.add("rm")

    with caplog.at_level(logging.WARNING, logger="columbina.sandbox.docker"):
        await provider.close()

    # Its input is closed all the same, which ends it. Whether the engine still has it
    # could not be asked, so the user is told.
    assert await gone(keeper)
    assert len(caplog.messages) == 1
    assert "may still be there" in caplog.messages[0]


# No container


@pytest.mark.parametrize(
    ("how", "why", "asked"),
    [
        (
            {"engine": "down"},
            "the Docker engine does not answer. Start Docker Desktop, or the service of Docker, "
            "and run the command again",
            ["info"],
        ),
        (
            {"info": '{"version":"29.8.2","system":"x","security":["name=rootless"],"os":"linux"}'},
            "the Docker engine maps the users of a container to other users of this machine "
            "(rootless), and this sandbox does not work with such an engine",
            ["info"],
        ),
        (
            {"info": "Client: Podman Engine"},
            "the Docker engine did not say what it is, so it is not known whether it holds a "
            "command",
            ["info"],
        ),
        (
            {"info": "template: :1:12: can't evaluate field ServerVersion", "info_exit": 1},
            "the Docker engine did not say what it is, so it is not known whether it holds a "
            "command",
            ["info"],
        ),
        (
            # What it wrote would do, but it says that it failed.
            {"info_exit": 1},
            "the Docker engine did not say what it is, so it is not known whether it holds a "
            "command",
            ["info"],
        ),
        (
            {"run": "no-image"},
            "the image python:3.12 is not on this machine, and no image is fetched for a "
            "command. Fetch it with: docker pull python:3.12",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"run": "missing"},
            "the image python:3.12 has no bash, and commands are run with bash. Name an image "
            "that has it in the config of the sandbox",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"run": "another-token"},
            "the Docker engine does not see the files of this machine, so it cannot give a "
            "container the working directory. It may run on another machine, or the directory "
            "of temporary files is not shared with it",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"run": "breaks"},
            "Docker could not start a container: docker: Error response from daemon: the "
            "engine is out of sorts",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"run": "odd"},
            "Docker could not start a container: hello",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"run": "endless"},
            "Docker could not start a container: it wrote a line without end where it was to "
            "say that it is ready",
            ["info", "run", "rm", "ps"],
        ),
        (
            {"usable": "readable"},
            "who cannot write the working directory there, although this program can. The "
            "engine maps users in a way that this sandbox does not work with",
            ["info", "run", "usable", "rm", "ps"],
        ),
        (
            {"usable": ""},
            "who cannot read the working directory there. The engine maps users in a way that "
            "this sandbox does not work with",
            ["info", "run", "usable", "rm", "ps"],
        ),
        (
            # The look failed, whatever it wrote before it did.
            {"usable": "writable", "usable_exit": 1},
            "who cannot read the working directory there. The engine maps users in a way that "
            "this sandbox does not work with",
            ["info", "run", "usable", "rm", "ps"],
        ),
    ],
)
async def test_without_a_container_that_holds_it_a_command_is_not_run(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    how: dict[str, object],
    why: str,
    asked: list[str],
) -> None:
    engine.told(**how)
    ran_it = work / "ran"

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "open('ran', 'w').close()")

    assert why in str(refused.value)
    assert engine.verbs() == asked
    # Nothing ran, on this machine or anywhere, and nothing is left of the attempt.
    assert not ran_it.exists()
    assert engine.containers() == {}


async def test_a_directory_that_was_to_be_read_only_and_can_be_written_holds_nothing(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    engine.told(usable="writable")

    with pytest.raises(SandboxUnavailableError, match="was to be read-only in the container"):
        await ran(provider, session, work, "pass", mode="read-only")

    assert engine.verbs() == ["info", "run", "usable", "rm", "ps"]
    assert engine.containers() == {}


async def test_a_container_that_does_not_come_up_in_time_is_given_up(
    engine: Engine, session: Session, work: Path
) -> None:
    engine.told(run="silent")
    provider = Docker(DockerConfig(program=engine.program, start_timeout_seconds=1.5))

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "pass")

    assert str(refused.value) == "the container did not come up within 1.5 seconds"
    assert engine.verbs() == ["info", "run", "rm", "ps"]
    assert engine.containers() == {}
    await provider.close()


async def test_a_working_directory_that_is_not_there_is_not_made_by_the_engine(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work / "gone", "pass")

    assert str(refused.value) == f"the working directory {work / 'gone'} is not there"
    assert engine.verbs() == ["info"]
    assert not (work / "gone").exists()


async def test_the_program_that_the_config_names_has_to_be_there(
    engine: Engine, session: Session, work: Path, tmp_path: Path
) -> None:
    named = str(tmp_path / "nowhere" / "docker")
    provider = Docker(DockerConfig(program=named))

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "pass")

    assert str(refused.value) == f"the docker program that the config names, {named}, is not there"
    assert engine.calls() == []


async def test_without_a_name_in_the_config_the_program_is_found_on_the_search_path(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # In front of what is there: the stand-in looks for programs of the system itself.
    here = str(Path(engine.program).parent)
    monkeypatch.setenv("PATH", os.pathsep.join([here, os.environ["PATH"]]))
    provider = Docker(DockerConfig())

    done = await ran(provider, session, work, "print('found')")

    assert done.output.strip() == "found"
    await provider.close()


async def test_a_program_in_the_working_directory_is_not_run(
    engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A repository may bring a program of that name, and its directory may be on the path.
    (work / "bin").mkdir()
    planted = work / "bin" / Path(engine.program).name
    planted.write_bytes(b"")
    planted.chmod(0o755)
    monkeypatch.setenv("PATH", str(work / "bin"))
    provider = Docker(DockerConfig())

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "pass")

    assert str(refused.value) == (
        "no docker program was found on the search path. Install Docker, or name the program "
        "in the config of the sandbox. A program that lies in the working directory is not "
        "taken"
    )
    assert engine.calls() == []


async def test_a_program_in_the_directory_that_this_program_was_started_in_is_not_run(
    engine: Engine, session: Session, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    started = tmp_path / "started"
    (started / "bin").mkdir(parents=True)
    planted = started / "bin" / Path(engine.program).name
    planted.write_bytes(b"")
    planted.chmod(0o755)
    monkeypatch.chdir(started)
    monkeypatch.setenv("PATH", str(started / "bin"))
    provider = Docker(DockerConfig())

    with pytest.raises(SandboxUnavailableError, match="no docker program was found"):
        await ran(provider, session, work, "pass")

    assert engine.calls() == []


if sys.platform != "win32":

    async def test_a_directory_that_this_program_cannot_write_either_is_no_fault_of_the_engine(
        provider: Docker, engine: Engine, session: Session, work: Path
    ) -> None:
        engine.told(usable="readable")
        work.chmod(0o555)
        try:
            done = await ran(provider, session, work, "print('read')")
        finally:
            work.chmod(0o755)

        assert done.output.strip() == "read"


@pytest.mark.parametrize(
    ("call", "asked"), [("info", []), ("run", ["info"]), ("usable", ["info", "run", "rm", "ps"])]
)
async def test_a_program_that_cannot_be_started_makes_no_container(
    provider: Docker, engine: Engine, session: Session, work: Path, call: str, asked: list[str]
) -> None:
    engine.fails.add(call)

    with pytest.raises(SandboxUnavailableError, match="could not be started"):
        await ran(provider, session, work, "pass")

    assert engine.verbs() == asked
    assert engine.containers() == {}


async def test_an_engine_that_was_refused_is_refused_again(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    engine.told(info='{"version":"29.8.2","system":"x","security":["name=rootless"],"os":"linux"}')

    for _ in range(2):
        with pytest.raises(SandboxUnavailableError, match="rootless"):
            await ran(provider, session, work, "open('ran', 'w').close()")

    # It is not taken for known because it was asked once.
    assert engine.verbs() == ["info", "info"]
    assert not (work / "ran").exists()


async def test_after_a_failure_the_next_command_tries_again(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    engine.told(engine="down")
    with pytest.raises(SandboxUnavailableError):
        await ran(provider, session, work, "pass")
    engine.told()

    done = await ran(provider, session, work, "print('up')")

    assert done.output.strip() == "up"
    assert engine.verbs() == ["info", *FIRST]


# Running a command


async def test_a_command_gets_its_input_and_the_variables_that_the_config_names(
    engine: Engine, session: Session, work: Path
) -> None:
    provider = Docker(DockerConfig(program=engine.program, env={"CI": "1"}))
    says = (
        "import os, sys\n"
        "print(sys.stdin.read().upper())\n"
        "print([os.environ.get(name) for name in ('HOME', 'LANG', 'TERM', 'CI')])\n"
    )

    done = await ran(provider, session, work, says, stdin=b"hello")

    assert done.output.splitlines() == ["HELLO", "['/tmp/columbina-home', 'C.UTF-8', 'dumb', '1']"]
    await provider.close()


async def test_what_asks_the_engine_gets_no_secret_of_this_program(
    provider: Docker, engine: Engine, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_TEST_TOKEN", "a-secret")
    monkeypatch.setenv("COLUMBINA_TEST_PLAIN", "no-secret")

    await ran(provider, session, work, "pass")
    await provider.close()

    assert engine.verbs() == [*FIRST, "rm", "ps"]
    assert engine.given() == [["COLUMBINA_TEST_PLAIN"]] * 7


async def test_the_commands_of_a_session_run_one_after_another(
    provider: Docker, session: Session, work: Path
) -> None:
    takes_a_while = (
        "import sys, time\n"
        "with open('log', 'a') as log:\n"
        "    print('begins', sys.argv[1], file=log, flush=True)\n"
        "    time.sleep(0.5)\n"
        "    print('ends', sys.argv[1], file=log, flush=True)\n"
    )

    await asyncio.gather(
        ran(provider, session, work, takes_a_while, "a"),
        ran(provider, session, work, takes_a_while, "b"),
    )

    said = (work / "log").read_text().split()
    assert said[0::2] == ["begins", "ends", "begins", "ends"]
    assert sorted(said[1::2]) == ["a", "a", "b", "b"]


async def test_the_commands_of_two_sessions_run_side_by_side(
    ctx: Context, provider: Docker, sessions: Sessions, work: Path
) -> None:
    waits_for_the_other = (
        "import os, sys, time\n"
        "open(sys.argv[1], 'w').close()\n"
        "deadline = time.time() + 20\n"
        "while not os.path.exists(sys.argv[2]):\n"
        "    assert time.time() < deadline\n"
        "    time.sleep(0.05)\n"
        "print('met')\n"
    )
    one, two = sessions.create(ctx, work), sessions.create(ctx, work)

    first, second = await asyncio.gather(
        ran(provider, one, work, waits_for_the_other, "one-is-here", "two-is-here"),
        ran(provider, two, work, waits_for_the_other, "two-is-here", "one-is-here"),
    )

    assert (first.output.strip(), second.output.strip()) == ("met", "met")


@pytest.mark.parametrize("how", ["left", "fails", "cannot be asked"])
async def test_a_container_in_which_something_would_not_stop_is_removed(
    provider: Docker, engine: Engine, session: Session, work: Path, how: str
) -> None:
    await ran(provider, session, work, "pass")
    engine.told(stop=how)
    if how == "cannot be asked":
        engine.fails.add("stop")

    done = await ran(provider, session, work, "print('said')")
    engine.told()
    engine.fails.clear()
    after = await ran(provider, session, work, "print('after')")
    asked = [] if how == "cannot be asked" else ["stop"]

    assert (done.output.strip(), done.exit_code) == ("said", 0)
    assert done.notes == (LEFT + NEW_ONE,)
    assert after.notes == ()
    assert engine.verbs() == [
        *FIRST,
        *["command", *asked, "rm", "ps"],
        *["run", "usable", "command", "stop"],
    ]


async def test_a_stop_that_does_not_come_back_does_not_hold_the_command(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, stop=1.0)
    await ran(provider, session, work, "pass")
    engine.told(stop="hangs")
    began = time.monotonic()

    done = await ran(provider, session, work, "print('said')")

    assert time.monotonic() - began < 20
    assert done.notes == (LEFT + NEW_ONE,)
    assert engine.containers() == {}


async def test_a_caller_that_leaves_while_things_are_stopped_does_not_cut_that_short(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, stop=2.0)
    await ran(provider, session, work, "pass")
    engine.told(stop="hangs")
    running = asyncio.ensure_future(ran(provider, session, work, "print('said')"))
    await until(lambda: engine.verbs().count("stop") == 2)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=30)
    # The stop did not come back, so the container was removed, although nobody waited.
    assert engine.verbs()[-2:] == ["rm", "ps"]
    assert engine.containers() == {}


async def test_a_working_directory_that_went_away_costs_the_container(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    inner = work / "inner"
    inner.mkdir()
    await ran(provider, session, inner, "pass")

    inner.rmdir()

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, inner, "pass")
    assert str(refused.value) == f"the working directory {inner} is not there"
    assert engine.verbs() == [*FIRST, "rm", "ps"]
    assert engine.containers() == {}


async def test_a_command_whose_time_is_up_is_stopped_in_its_container(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")
    kept, before = engine.containers(), len(engine.started())

    done = await ran(provider, session, work, STAYS, seconds=1.0)

    assert (done.timed_out, done.exit_code, done.notes) == (True, None, ())
    (command,) = engine.started()[before:]
    assert await gone(command)
    # It was stopped from inside while the program that asked for it still waited, and
    # the container is the one for the next command.
    assert 1 in engine.stops()[1:]
    assert "rm" not in engine.verbs()
    assert engine.containers() == kept


async def test_a_command_that_the_engine_starts_late_is_stopped_all_the_same(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    # The engine has taken the command and not started it when its time is up.
    await ran(provider, session, work, "pass")
    before = len(engine.started())
    engine.told(late=1.5)

    done = await ran(provider, session, work, "import time; time.sleep(60)", seconds=0.5)

    assert (done.timed_out, done.notes) == (True, ())
    stops = engine.stops()[1:]
    assert stops[0] == 0
    assert 1 in stops
    # It did start, late, and was stopped as soon as it was there.
    (command,) = engine.started()[before:]
    assert await gone(command)
    assert "rm" not in engine.verbs()


async def test_a_command_that_could_not_be_stopped_when_its_time_was_up_costs_the_container(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, stop=1.0)
    await ran(provider, session, work, "pass")
    before = len(engine.started())
    engine.told(stop="left")
    began = time.monotonic()

    done = await ran(provider, session, work, STAYS, seconds=0.5)

    assert time.monotonic() - began < 15
    assert (done.timed_out, done.notes) == (True, (LEFT + NEW_ONE,))
    # With the container goes everything in it.
    for command in engine.started()[before:]:
        assert await gone(command)
    assert engine.containers() == {}


async def test_a_command_whose_caller_is_cancelled_is_stopped_in_its_container(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")
    kept, before = engine.containers(), len(engine.started())
    running = asyncio.ensure_future(ran(provider, session, work, STAYS))
    command = await begun(engine, before)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=30)
    assert await gone(command)
    assert "rm" not in engine.verbs()
    assert engine.containers() == kept


async def test_a_cancelled_command_that_could_not_be_stopped_costs_the_container(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    quick(monkeypatch, stop=1.0)
    await ran(provider, session, work, "pass")
    before = len(engine.started())
    engine.told(stop="left")
    running = asyncio.ensure_future(ran(provider, session, work, STAYS))
    command = await begun(engine, before)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=30)
    # The caller has left, and the container was removed all the same.
    assert engine.verbs()[-2:] == ["rm", "ps"]
    assert await gone(command)
    assert engine.containers() == {}


async def test_a_command_that_was_cancelled_as_it_was_handed_over_costs_the_container(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    # Whether the engine took the command cannot be known, so nothing may go on in there.
    await ran(provider, session, work, "pass")
    engine.held.add("command")
    running = asyncio.ensure_future(ran(provider, session, work, "print('never read')"))
    await engine.holding.wait()

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=30)
    engine.go_on.set()
    # The program that was to hand the command over may have written itself down at any
    # moment, or have been ended before it ran at all.
    after = [verb for verb in engine.verbs()[len(FIRST) :] if verb != "command"]
    assert after == ["rm", "ps"]
    assert engine.containers() == {}


async def test_a_container_is_made_to_its_end_although_whoever_asked_for_it_has_left(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    engine.held.add("run")
    first = asyncio.ensure_future(ran(provider, session, work, "print('first')"))
    await engine.holding.wait()

    first.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(first, timeout=30)
    engine.go_on.set()
    done = await ran(provider, session, work, "print('second')")

    # The second command found the container that the first one had asked for.
    assert done.output.strip() == "second"
    assert engine.verbs() == FIRST
    assert len(engine.containers()) == 1


async def test_a_command_that_cannot_be_handed_to_the_engine_is_not_run(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    await ran(provider, session, work, "pass")
    engine.fails.add("command")

    with pytest.raises(SandboxUnavailableError, match="could not be started"):
        await ran(provider, session, work, "open('ran', 'w').close()")

    assert not (work / "ran").exists()


async def test_a_long_output_goes_where_the_caller_says(
    provider: Docker, session: Session, work: Path
) -> None:
    written = bytearray()

    class Place:
        def write(self, data: bytes) -> None:
            written.extend(data)

    done = await provider.run(
        python("print('x' * 200_000)"),
        session=session,
        mode="workspace-write",
        cwd=work,
        stdin=None,
        timeout_seconds=30,
        overflow=Place,
        max_overflow_bytes=1 << 20,
        expected="none",
    )

    assert done.omitted_bytes > 0
    assert bytes(written).strip() == b"x" * 200_000


# Waiting for something to its end


async def test_what_is_waited_for_to_its_end_gives_what_it_gave() -> None:
    to_the_end: Any = vars(docker)["_to_the_end"]

    async def gives() -> str:
        await asyncio.sleep(0)
        return "given"

    async def fails() -> None:
        raise ValueError("no")

    assert await to_the_end(gives()) == "given"
    with pytest.raises(ValueError, match="no"):
        await to_the_end(fails())


async def test_what_is_waited_for_to_its_end_is_not_cut_short_by_a_cancellation(
    caplog: pytest.LogCaptureFixture,
) -> None:
    to_the_end: Any = vars(docker)["_to_the_end"]
    go_on = asyncio.Event()
    finished: list[str] = []

    async def takes_its_time() -> str:
        await go_on.wait()
        finished.append("done")
        return "lost"

    waiting = asyncio.ensure_future(to_the_end(takes_its_time()))
    await asyncio.sleep(0.05)
    waiting.cancel()
    await asyncio.sleep(0.05)
    waiting.cancel()
    await asyncio.sleep(0.05)
    still_at_it = not waiting.done() and finished == []
    go_on.set()

    quiet = caplog.at_level(logging.ERROR, logger="columbina.sandbox.docker")
    with quiet, pytest.raises(asyncio.CancelledError):
        await waiting
    assert still_at_it
    assert finished == ["done"]
    assert caplog.messages == []


async def test_a_failure_of_what_was_waited_for_is_written_down_if_its_caller_has_left(
    caplog: pytest.LogCaptureFixture,
) -> None:
    to_the_end: Any = vars(docker)["_to_the_end"]
    go_on = asyncio.Event()

    async def fails_later() -> None:
        await go_on.wait()
        raise ValueError("the engine fell over")

    waiting = asyncio.ensure_future(to_the_end(fails_later()))
    await asyncio.sleep(0.05)
    waiting.cancel()
    await asyncio.sleep(0.05)
    go_on.set()

    heard = caplog.at_level(logging.ERROR, logger="columbina.sandbox.docker")
    with heard, pytest.raises(asyncio.CancelledError):
        await waiting
    assert caplog.messages == ["what was done for a container failed after its caller had left"]
    assert "the engine fell over" in caplog.text


# What the provider says of itself


def test_the_user_of_this_program_is_asked_of_the_system() -> None:
    me: Any = vars(docker)["_me"]

    if sys.platform == "win32":
        # Windows has no such numbers, and none are needed there.
        assert me() == (0, 0)
    else:
        assert me() == (os.getuid(), os.getgid())


def test_commands_in_a_container_are_written_for_bash() -> None:
    assert Docker(DockerConfig()).shell == "bash"


@pytest.mark.parametrize(("network", "strength"), [("none", "full"), ("bridge", "partial")])
async def test_a_container_with_a_network_holds_a_command_in_part_only(
    engine: Engine, session: Session, work: Path, network: Any, strength: str
) -> None:
    # It can send away what it reads.
    provider = Docker(DockerConfig(program=engine.program, network=network))

    before = provider.strength(session)
    await ran(provider, session, work, "pass")

    assert (before, provider.strength(session)) == (strength, strength)
    await provider.close()


async def test_what_was_found_in_the_directory_of_a_session_lowers_how_well_it_is_held(
    ctx: Context,
    provider: Docker,
    engine: Engine,
    sessions: Sessions,
    work: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    other = Path(os.path.realpath(tmp_path)) / "other"
    other.mkdir()
    looked: list[Path] = []

    def doubt(cwd: Path) -> str | None:
        looked.append(cwd)
        return "a socket lies in the working directory, at agent.sock" if cwd == work else None

    monkeypatch.setattr(docker, "_doubt", doubt)
    one, two = sessions.create(ctx, work), sessions.create(ctx, other)

    before = provider.strength(one)
    await ran(provider, one, work, "pass")
    await ran(provider, two, other, "pass")

    # Nothing is known of a directory before a container was made for it.
    assert before == "full"
    assert (provider.strength(one), provider.strength(two)) == ("partial", "full")
    assert looked == [work, other]


async def test_a_new_container_is_held_as_well_as_its_own_directory_allows(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    found = ["a socket lies in the working directory, at agent.sock", None]

    def doubt(cwd: Path) -> str | None:
        return found.pop(0)

    monkeypatch.setattr(docker, "_doubt", doubt)

    await ran(provider, session, work, "pass")
    first = provider.strength(session)
    await ran(provider, session, work, "pass", mode="read-only")

    assert (first, provider.strength(session)) == ("partial", "full")


async def test_a_command_that_was_let_through_for_a_full_hold_runs_where_the_hold_is_full(
    provider: Docker, engine: Engine, session: Session, work: Path
) -> None:
    done = await ran(provider, session, work, "print('held')", expected="full")

    assert done.output.strip() == "held"
    assert engine.verbs() == FIRST


async def test_a_command_that_was_let_through_for_a_full_hold_is_not_run_in_a_lesser_one(
    provider: Docker,
    engine: Engine,
    session: Session,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Nothing was known of the directory when the call was decided. Now a container was
    # made for it, and a door was found open.
    def doubt(cwd: Path) -> str | None:
        return "a socket lies in the working directory, at agent.sock"

    monkeypatch.setattr(docker, "_doubt", doubt)

    with pytest.raises(SandboxUnavailableError) as refused:
        await ran(provider, session, work, "open('ran', 'w').close()", expected="full")
    after = await ran(provider, session, work, "print('asked about and allowed')")

    assert str(refused.value) == (
        "it was let through without a question because commands were held fully, and now a "
        "socket lies in the working directory, at agent.sock. Call it again, and it is asked "
        "about"
    )
    assert not (work / "ran").exists()
    # The container is there, and serves the command that comes with a yes.
    assert after.output.strip() == "asked about and allowed"
    assert engine.verbs() == ["info", "run", "usable", "command", "stop"]


async def test_a_container_with_a_network_runs_nothing_that_was_let_through_for_a_full_hold(
    engine: Engine, session: Session, work: Path
) -> None:
    provider = Docker(DockerConfig(program=engine.program, network="bridge"))

    with pytest.raises(SandboxUnavailableError, match="they are held less well now"):
        await ran(provider, session, work, "open('ran', 'w').close()", expected="full")

    assert not (work / "ran").exists()
    await provider.close()


if sys.platform != "win32":

    async def test_a_socket_in_the_working_directory_leaves_its_commands_held_in_part(
        provider: Docker, engine: Engine, session: Session, work: Path
    ) -> None:
        import socket

        (work / "run").mkdir()
        listening = socket.socket(socket.AF_UNIX)
        listening.bind(str(work / "run" / "agent.sock"))
        try:
            await ran(provider, session, work, "pass")
        finally:
            listening.close()

        assert provider.strength(session) == "partial"

else:

    async def test_on_windows_no_socket_is_looked_for(
        provider: Docker, engine: Engine, session: Session, work: Path
    ) -> None:
        doubt: Any = vars(docker)["_doubt"]

        assert doubt(work) is None


# The provider as a part of the sandbox


async def test_the_sandbox_brings_the_provider_and_lets_its_containers_go_when_it_leaves(
    ctx: Context, engine: Engine, sessions: Sessions, work: Path
) -> None:
    await ctx.plugin(columbina.fs)
    config = {"provider": "docker", "docker": {"program": engine.program}}
    loaded = await ctx.plugin(columbina.sandbox, config)
    sandbox = ctx.use(SANDBOX)
    session = sessions.create(ctx, work)

    done = await sandbox.run(python("print('held')"), session=session, cwd=work, timeout_seconds=30)

    assert done.output.strip() == "held"
    assert (sandbox.shell(), sandbox.strength(session)) == ("bash", "full")
    assert len(engine.containers()) == 1

    await loaded.dispose()

    assert engine.containers() == {}


async def test_the_provider_asks_nothing_of_docker_until_a_command_is_to_run_in_a_container(
    ctx: Context, engine: Engine, sessions: Sessions, work: Path
) -> None:
    await ctx.plugin(columbina.fs)
    loaded = await ctx.plugin(columbina.sandbox, {"docker": {"program": engine.program}})
    sandbox = ctx.use(SANDBOX)
    session = sessions.create(ctx, work)

    done = await sandbox.run(python("print('here')"), session=session, cwd=work, timeout_seconds=30)
    await loaded.dispose()

    assert done.output.strip() == "here"
    assert (sandbox.shell(), sandbox.strength(session)) == (None, "none")
    assert engine.calls() == []


async def test_a_config_of_the_provider_that_makes_no_container_keeps_the_sandbox_from_loading(
    ctx: Context, sessions: Sessions
) -> None:
    await ctx.plugin(columbina.fs)

    with pytest.raises(Exception, match="named by its whole path"):
        await ctx.plugin(columbina.sandbox, {"docker": {"program": "docker"}})
