"""The sandbox: the mode of a session, what it lets tools do to files, and who confines commands."""

import itertools
from collections.abc import Sequence
from pathlib import Path

import pytest
from pydantic import ValidationError

import columbina.fs
import columbina.sandbox
import columbina.session
from columbina.fs import FS, Access, Fs, Kind, locate
from columbina.kernel import Context, Plugin
from columbina.sandbox import (
    SANDBOX,
    Confined,
    Mode,
    Sandbox,
    SandboxModeEvent,
    SandboxUnavailableError,
    Strength,
)
from columbina.session import SESSIONS, Session, Sessions

MODES: tuple[Mode, ...] = ("read-only", "workspace-write", "danger-full-access")
KINDS: tuple[Kind, ...] = ("read", "write")
PATHS = (
    "a.txt",
    "sub/a.txt",
    "new/deep/file.txt",
    ".",
    ".git/config",
    ".git",
    "sub/.git/hooks/pre-commit",
    ".columbina/permissions.yml",
    ".GIT/config",
    ".gitignore",
    "git/config",
    "../outside.txt",
    "../.git/config",
)


async def program(ctx: Context, tmp_path: Path, **config: str) -> tuple[Sessions, Fs, Sandbox]:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, config)
    return ctx.use(SESSIONS), ctx.use(FS), ctx.use(SANDBOX)


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    (directory / "sub").mkdir(parents=True)
    (directory / "a.txt").write_text("a")
    (tmp_path / "outside.txt").write_text("o")
    return directory


def modes_of(session: Session) -> list[str]:
    return [e.data.mode for e in session.events if isinstance(e.data, SandboxModeEvent)]


# The mode of a session


async def test_a_session_has_the_mode_of_the_config_until_its_log_names_one(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path, mode="read-only")
    session = sessions.create(ctx, work)

    assert sandbox.mode(session) == "read-only"
    assert sandbox.recorded(session) is None

    sandbox.set_mode(session, "danger-full-access")

    assert sandbox.mode(session) == "danger-full-access"
    assert sandbox.recorded(session) == "danger-full-access"
    assert modes_of(session) == ["danger-full-access"]


async def test_the_mode_is_workspace_write_unless_the_config_says_otherwise(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path)

    assert sandbox.mode(sessions.create(ctx, work)) == "workspace-write"


async def test_a_mode_that_the_log_names_already_is_not_written_again(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path)
    session = sessions.create(ctx, work)

    sandbox.set_mode(session, "workspace-write")
    sandbox.set_mode(session, "workspace-write")
    sandbox.set_mode(session, "read-only")
    sandbox.set_mode(session, "read-only")
    sandbox.set_mode(session, "workspace-write")

    assert modes_of(session) == ["workspace-write", "read-only", "workspace-write"]


async def test_the_mode_of_the_config_is_written_once_it_is_set_by_name(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    # The log then names the mode, whatever a later config says.
    sessions, _, sandbox = await program(ctx, tmp_path, mode="read-only")
    session = sessions.create(ctx, work)

    sandbox.set_mode(session, "read-only")

    assert modes_of(session) == ["read-only"]


async def test_a_mode_that_does_not_exist_is_refused(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path)
    session = sessions.create(ctx, work)
    mode: Mode = "write-everything"  # pyright: ignore[reportAssignmentType]

    with pytest.raises(ValidationError):
        sandbox.set_mode(session, mode)

    assert modes_of(session) == []
    with pytest.raises(ValidationError):
        await ctx.plugin(columbina.sandbox, {"mode": "write-everything"})


async def test_a_session_that_is_taken_up_again_has_the_mode_it_had(
    tmp_path: Path, work: Path
) -> None:
    first = Context()
    sessions, _, sandbox = await program(first, tmp_path)
    session = sessions.create(first, work, session_id="s-1")
    sandbox.set_mode(session, "read-only")
    await session.flush()
    await first.fiber.dispose()

    second = Context()
    try:
        sessions, _, sandbox = await program(second, tmp_path)
        resumed = sessions.resume(second, work, "s-1")

        assert sandbox.mode(resumed) == "read-only"
    finally:
        await second.fiber.dispose()


async def test_a_session_with_a_mode_can_be_read_by_a_program_without_the_sandbox(
    tmp_path: Path, work: Path
) -> None:
    first = Context()
    sessions, _, sandbox = await program(first, tmp_path)
    session = sessions.create(first, work, session_id="s-1")
    sandbox.set_mode(session, "read-only")
    await session.flush()
    await first.fiber.dispose()

    second = Context()
    try:
        await second.plugin(columbina.session, {"root": tmp_path / "sessions"})

        resumed = second.use(SESSIONS).resume(second, work, "s-1")

        assert not resumed.closed
    finally:
        await second.fiber.dispose()


# What the mode lets tools do to files


def by_definition(mode: Mode, kind: Kind, written: str) -> str | None:
    """The error code that an access is refused with, straight from what the modes mean."""
    inside = not written.startswith("..")
    protected = any(part.lower() in (".git", ".columbina") for part in written.split("/"))
    if kind == "read":
        return None if inside or mode == "danger-full-access" else "OUTSIDE_WORKING_DIRECTORY"
    if mode == "read-only":
        return "READ_ONLY"
    if mode == "danger-full-access":
        return None
    if protected:
        return "PROTECTED"
    return None if inside else "OUTSIDE_WORKING_DIRECTORY"


@pytest.fixture
async def seam(ctx: Context, tmp_path: Path, work: Path) -> tuple[Fs, Sandbox, Session]:
    sessions, fs, sandbox = await program(ctx, tmp_path)
    return fs, sandbox, sessions.create(ctx, work)


@pytest.mark.parametrize(("mode", "kind", "written"), list(itertools.product(MODES, KINDS, PATHS)))
def test_every_mode_allows_what_it_means_and_nothing_else(
    seam: tuple[Fs, Sandbox, Session], work: Path, mode: Mode, kind: Kind, written: str
) -> None:
    fs, sandbox, session = seam
    sandbox.set_mode(session, mode)

    refusal = fs.judge(Access(session, work, locate(work, written), kind, "probe"))

    assert (None if refusal is None else refusal.code) == by_definition(mode, kind, written)


def test_a_write_in_a_session_that_is_read_only_is_refused_in_words_that_say_so(
    seam: tuple[Fs, Sandbox, Session], work: Path
) -> None:
    fs, sandbox, session = seam
    sandbox.set_mode(session, "read-only")

    refusal = fs.judge(Access(session, work, locate(work, "a.txt"), "write", "probe"))

    assert refusal is not None
    assert refusal.reason == "The session is read-only, so a.txt cannot be written."


def test_a_write_into_a_protected_directory_is_refused_in_words_that_name_it(
    seam: tuple[Fs, Sandbox, Session], work: Path
) -> None:
    fs, _, session = seam

    refusal = fs.judge(Access(session, work, locate(work, "sub/.git/config"), "write", "probe"))

    assert refusal is not None
    assert refusal.reason == "sub/.git/config is in .git, which is not written in this mode."


def test_a_change_of_the_mode_counts_from_the_next_access_on(
    seam: tuple[Fs, Sandbox, Session], work: Path
) -> None:
    fs, sandbox, session = seam
    access = Access(session, work, locate(work, "a.txt"), "write", "probe")
    assert fs.judge(access) is None

    sandbox.set_mode(session, "read-only")

    assert fs.judge(access) is not None


async def test_the_mode_of_one_session_is_not_the_mode_of_another(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, fs, sandbox = await program(ctx, tmp_path)
    strict, free = sessions.create(ctx, work), sessions.create(ctx, work)
    sandbox.set_mode(strict, "read-only")
    where = locate(work, "a.txt")

    assert fs.judge(Access(strict, work, where, "write", "probe")) is not None
    assert fs.judge(Access(free, work, where, "write", "probe")) is None


async def test_what_the_sandbox_registered_leaves_with_it(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.fs)
    fiber = await ctx.plugin(columbina.sandbox)
    fs = ctx.use(FS)
    session = ctx.use(SESSIONS).create(ctx, work)
    access = Access(session, work, locate(work, "a.txt"), "write", "probe")
    assert fs.judge(access) is None

    await fiber.dispose()

    refusal = fs.judge(access)
    assert refusal is not None
    assert refusal.code == "NOT_WRITABLE"


# Who confines commands


class Jail:
    """A provider for these tests, which puts a word in front of a command."""

    def __init__(self, strength: Strength) -> None:
        self.strength: Strength = strength
        self.asked: list[tuple[Mode, Path]] = []

    def confine(self, argv: Sequence[str], *, mode: Mode, cwd: Path) -> Sequence[str]:
        self.asked.append((mode, cwd))
        return ["jail", f"--{mode}", *argv]


async def test_the_provider_that_holds_nothing_returns_a_command_as_it_is(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path)
    session = sessions.create(ctx, work)

    confined = sandbox.confine(["python", "check.py"], session=session, cwd=work)

    assert confined == Confined(argv=("python", "check.py"), strength="none")
    assert sandbox.strength() == "none"


async def test_a_provider_that_another_plugin_brings_confines_in_the_mode_of_the_session(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    jail = Jail("full")

    def apply(inner: Context, config: object) -> None:
        inner.use(SANDBOX).providers.add(inner, "jail", jail)

    sessions, _, sandbox = await program(ctx, tmp_path, provider="jail")
    session = sessions.create(ctx, work)
    sandbox.set_mode(session, "read-only")
    await ctx.plugin(Plugin("jail", apply, inject=(SANDBOX,)))

    confined = sandbox.confine(["python", "check.py"], session=session, cwd=work)

    assert confined == Confined(argv=("jail", "--read-only", "python", "check.py"), strength="full")
    assert jail.asked == [("read-only", work)]
    assert sandbox.strength() == "full"


async def test_a_command_is_not_run_unconfined_in_the_place_of_a_provider_that_is_not_there(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, _, sandbox = await program(ctx, tmp_path, provider="jail")
    session = sessions.create(ctx, work)

    with pytest.raises(SandboxUnavailableError, match="'jail' is not there"):
        sandbox.confine(["python", "check.py"], session=session, cwd=work)

    assert sandbox.strength() == "none"


async def test_a_provider_leaves_with_the_plugin_that_brought_it(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(SANDBOX).providers.add(inner, "jail", Jail("partial"))

    sessions, _, sandbox = await program(ctx, tmp_path, provider="jail")
    session = sessions.create(ctx, work)
    fiber = await ctx.plugin(Plugin("jail", apply, inject=(SANDBOX,)))
    assert sandbox.strength() == "partial"

    await fiber.dispose()

    assert list(sandbox.providers) == ["none"]
    with pytest.raises(SandboxUnavailableError):
        sandbox.confine(["python", "check.py"], session=session, cwd=work)
