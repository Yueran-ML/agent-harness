"""The shell tool: a command, run by the shell of this platform."""

import asyncio
import json
import logging
import os
import sys
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
import columbina.session
import columbina.spill.local
import columbina.spill.policy
import columbina.tools
from columbina import process
from columbina.builtin_tools import read, shell
from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.permissions import PERMISSIONS, Permissions
from columbina.permissions.commands import split_bash, split_powershell
from columbina.process import Completed
from columbina.sandbox import SANDBOX, Mode, Sandbox, Strength
from columbina.session import SESSIONS, Session
from columbina.spill import SPILL, Saved
from columbina.token_meter import estimate
from columbina.tools import TOOLS, Deny, Invocation, ToolOutcome, ToolRun, Tools
from support.processes import gone

WINDOWS = sys.platform == "win32"
PYTHON = sys.executable

# The same deeds in the words of each shell.
SLEEP = "Start-Sleep -Seconds 60" if WINDOWS else "sleep 60"
WHERE = "(Get-Location).Path" if WINDOWS else "pwd"
READ_A_LINE = (
    "$line = [Console]::In.ReadLine(); if ($null -eq $line) { 'no input' }"
    if WINDOWS
    else "read line || echo 'no input'"
)
SECRET = (
    '"[$env:COLUMBINA_TEST_TOKEN][$env:COLUMBINA_TEST_PLAIN]"'
    if WINDOWS
    else ('echo "[$COLUMBINA_TEST_TOKEN][$COLUMBINA_TEST_PLAIN]"')
)
CALL = "& " if WINDOWS else ""


def python(code: str) -> str:
    """The command that runs ``code`` in a Python of its own, in the words of this shell."""
    return f"{CALL}'{PYTHON}' -c \"{code}\""


@dataclass
class Bench:
    """A session in which the shell tool may run whatever it is given."""

    ctx: Context
    tools: Tools
    sandbox: Sandbox
    permissions: Permissions
    session: Session
    work: Path
    _calls: int = 0

    async def call(self, command: str, **more: Any) -> ToolOutcome:
        self._calls += 1
        call_id = f"call-{self._calls}"
        arguments = json.dumps({"command": command, **more})
        asked = ToolCallBlock(call_id=call_id, name=shell.NAME, arguments=arguments)
        return await self.tools.run(self.ctx, asked, ToolRun(call_id, self.work, self.session))

    async def said(self, command: str, **more: Any) -> str:
        """Run a command that is to work, and return what the model reads."""
        outcome = await self.call(command, **more)
        assert outcome.error is None, outcome.text
        return outcome.text


async def loaded(
    ctx: Context,
    tmp_path: Path,
    work: str = "work",
    sandbox: dict[str, str] | None = None,
    **permissions: Any,
) -> Bench:
    directory = tmp_path / work
    directory.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, sandbox)
    await ctx.plugin(columbina.approval)
    await ctx.plugin(columbina.permissions, permissions or {"allow": [shell.NAME]})
    await ctx.plugin(shell)
    return Bench(
        ctx=ctx,
        tools=ctx.use(TOOLS),
        sandbox=ctx.use(SANDBOX),
        permissions=ctx.use(PERMISSIONS),
        session=ctx.use(SESSIONS).create(ctx, directory),
        work=directory,
    )


@pytest.fixture
async def bench(ctx: Context, tmp_path: Path) -> Bench:
    return await loaded(ctx, tmp_path)


def watched(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """Write down how every process is run."""
    runs: list[dict[str, Any]] = []
    really_run: Any = process.run

    async def running(argv: Any, **options: Any) -> Any:
        runs.append({"argv": list(argv), **options})
        return await really_run(argv, **options)

    monkeypatch.setattr(process, "run", running)
    return runs


# What comes back


async def test_the_output_of_a_command_comes_back_with_its_exit_code(bench: Bench) -> None:
    assert await bench.said("echo hello") == "hello\n[exit code 0]"


async def test_a_command_that_says_nothing_has_its_exit_code_alone(bench: Bench) -> None:
    assert await bench.said("exit 0") == "[exit code 0]"


async def test_another_exit_code_than_zero_is_no_failure_of_the_call(bench: Bench) -> None:
    outcome = await bench.call("echo before; exit 3")

    assert outcome.error is None
    assert outcome.text == "before\n[exit code 3]"


async def test_the_exit_code_of_a_program_that_the_command_ran_is_passed_on(bench: Bench) -> None:
    assert await bench.said(python("import sys; sys.exit(5)")) == "[exit code 5]"


async def test_what_goes_to_the_error_stream_comes_back_too(bench: Bench) -> None:
    said = await bench.said(python("import sys; print('to the error stream', file=sys.stderr)"))

    assert said == "to the error stream\n[exit code 0]"


async def test_text_of_any_script_comes_back_as_it_was_written(bench: Bench) -> None:
    moon = chr(0x6708) + chr(0x795E)
    (bench.work / f"{moon}.txt").write_text("x")

    assert await bench.said(f"echo '{moon}'") == f"{moon}\n[exit code 0]"
    assert moon in await bench.said("ls" if not WINDOWS else "Get-ChildItem -Name")
    assert await bench.said(python(f"print('{moon}')")) == f"{moon}\n[exit code 0]"


async def test_a_line_of_output_ends_the_way_the_model_writes_lines(bench: Bench) -> None:
    said = await bench.said("echo one; echo two")

    assert said == "one\ntwo\n[exit code 0]"
    assert "\r" not in said


async def test_the_exit_code_stands_on_a_line_of_its_own_however_the_output_ends(
    bench: Bench,
) -> None:
    said = await bench.said(python("import sys; sys.stdout.write('no line break at the end')"))

    assert said == "no line break at the end\n[exit code 0]"


async def test_of_much_output_the_beginning_and_the_end_come_back(bench: Bench) -> None:
    said = await bench.said(python("print('first'); print('x' * 200000); print('last')"))

    assert said.startswith("first\n")
    assert said.endswith("last\n[exit code 0]")
    assert "bytes of output are left out here" in said
    assert len(said) < 70_000


async def test_a_command_of_several_lines_runs_as_one(bench: Bench) -> None:
    several = (
        "function Twice($x) {\n    $x * 2\n}\nif ($true) {\n    Twice 21\n}"
        if WINDOWS
        else "twice() {\n  echo $(($1 * 2))\n}\nif true; then\n  twice 21\nfi"
    )

    assert await bench.said(several) == "42\n[exit code 0]"


# The whole of a long output

ROWS = 2500
MUCH = f"for number in range({ROWS}): print('%06d ' % number + 'x' * 72)"
"""Writes two hundred thousand characters, eighty to a line."""
WHOLE = "".join(f"{number:06d} {'x' * 72}\n" for number in range(ROWS))
HINT = "Read a part of it with the read tool, by offset and limit, or search it with grep."


class Kept:
    """What a place that keeps things in memory was given for one command."""

    def __init__(self, place: "Place", name: str) -> None:
        self.place = place
        self.name = name
        self.written = bytearray()
        self.closed = 0

    def write(self, data: bytes) -> None:
        if self.place.room is not None and len(self.written) + len(data) > self.place.room:
            raise OSError(28, "No space left on device")
        self.written += data

    def close(self) -> Saved:
        self.closed += 1
        if self.place.closing is not None:
            raise self.place.closing
        return Saved(f"kept:{self.name}", len(self.written), "Read it.")


class Place:
    """A place for what is too large for a conversation. It keeps it in memory, and can fail.

    Args:
        opens: Whether a file can be made.
        room: How many bytes a file takes, if not any number.
        closing: What closing a file raises, if anything.
    """

    def __init__(
        self, *, opens: bool = True, room: int | None = None, closing: Exception | None = None
    ) -> None:
        self.opens = opens
        self.room = room
        self.closing = closing
        self.files: list[Kept] = []

    def save_text(self, session: Session, cwd: Path, name: str, text: str) -> Saved:
        raise AssertionError("the tool saves no text")

    def open(self, session: Session, cwd: Path, name: str) -> Kept:
        if not self.opens:
            raise PermissionError(13, "Permission denied")
        self.files.append(Kept(self, name))
        return self.files[-1]


def files_under(root: Path) -> list[Path]:
    return sorted(path for path in root.rglob("*") if path.is_file())


def unix(written: bytes) -> str:
    """What was written, with lines that end the way the model writes them.

    A part of it may end between the two characters that end a line on Windows.
    """
    return written.replace(b"\r", b"").decode()


async def test_the_whole_of_a_long_output_is_saved_and_the_result_says_where(
    bench: Bench, tmp_path: Path
) -> None:
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})

    outcome = await bench.call(python(MUCH))

    assert outcome.error is None, outcome.text
    (file,) = files_under(root)
    written = file.read_bytes()
    assert unix(written) == WHOLE
    assert file.name.endswith(f"-{shell.NAME}-output.txt")
    assert file.parent.name == bench.session.id
    said = outcome.text
    assert said.startswith(WHOLE[:8000])
    assert " bytes of output are left out here]\n" in said
    assert said.endswith(
        f"{WHOLE[-8000:]}[exit code 0]\n"
        f"(The whole output has {len(written)} bytes and is in {file}. {HINT})"
    )
    # Whatever keeps a result within bounds is told that the whole is saved, and where.
    assert outcome.value == shell.ShellResult(said, Saved(str(file), len(written), HINT, "output"))


async def test_an_output_that_memory_keeps_whole_is_saved_nowhere(
    bench: Bench, tmp_path: Path
) -> None:
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})

    outcome = await bench.call(python("print('x' * 60000)"))

    assert outcome.text == "x" * 60000 + "\n[exit code 0]"
    assert outcome.value == shell.ShellResult(outcome.text, None)
    assert files_under(root) == []


async def test_a_long_output_that_is_cut_to_size_is_saved_once_and_says_so_once(
    bench: Bench, tmp_path: Path
) -> None:
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})
    await bench.ctx.plugin(columbina.spill.policy)

    said = await bench.said(python(MUCH))

    (file,) = files_under(root)
    assert unix(file.read_bytes()) == WHOLE
    assert 12_000 < estimate(said) <= 12_500
    # Each end is what half of the limit pays for, which is less than memory had kept.
    assert said.startswith(WHOLE[:4000])
    assert "x\n[... a part is left out here ...]\n0" in said
    assert "bytes of output are left out" not in said
    assert said.count("The whole ") == 1
    assert said.endswith(
        f"{WHOLE[-4000:]}[exit code 0]\n"
        f"(The whole output has {file.stat().st_size} bytes and is in {file}. {HINT})"
    )


async def test_a_long_line_of_output_can_be_read_where_it_was_saved(
    bench: Bench, tmp_path: Path
) -> None:
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})

    said = await bench.said(python("print('x' * 200000)"))

    (file,) = files_under(root)
    lines = unix(file.read_bytes()).split("\n")
    # The read tool shows two thousand characters of a line.
    assert [len(line) for line in lines] == [2000] * 100 + [0]
    assert said.endswith(f"{HINT} Lines of more than 2000 characters are broken there.)")


async def test_no_more_of_an_output_is_saved_than_the_read_tool_looks_at(
    bench: Bench, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A place may break long lines, which adds a byte to every two thousand characters.
    # What is saved must stay within reach of the read tool with those bytes added.
    looked_at: int = vars(read)["_SCAN_BYTES"]
    limit = shell.MAX_SAVED_BYTES
    assert limit + limit // columbina.spill.local.FOLD_AT + 1 <= looked_at
    assert limit >= looked_at - 64 * 1024
    monkeypatch.setattr(shell, "MAX_SAVED_BYTES", 100_000)
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})

    outcome = await bench.call(python(MUCH))

    (file,) = files_under(root)
    written = file.read_bytes()
    assert len(written) == 100_000
    assert WHOLE.startswith(unix(written))
    # The command ran to its end all the same, and the model has the end of what it wrote.
    assert outcome.text.endswith(
        f"{WHOLE[-8000:]}[exit code 0]\n"
        f"(Only the first 100000 bytes of the output are saved, in {file}. {HINT})"
    )
    assert outcome.value == shell.ShellResult(outcome.text, None)


async def test_an_output_that_is_saved_in_part_does_not_count_as_saved(
    bench: Bench, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # What the result is cut down to is then saved like that of any other tool: the end of
    # the output is in no other place.
    monkeypatch.setattr(shell, "MAX_SAVED_BYTES", 100_000)
    root = tmp_path / "home" / "spill"
    await bench.ctx.plugin(columbina.spill.local, {"root": root})
    await bench.ctx.plugin(columbina.spill.policy)

    said = await bench.said(python(MUCH))

    part, result = sorted(files_under(root), key=lambda path: path.name.endswith("-result.txt"))
    assert part.name.endswith(f"-{shell.NAME}-output.txt")
    assert result.name.endswith(f"-{shell.NAME}-result.txt")
    kept = result.read_text(encoding="utf-8")
    assert kept.startswith(WHOLE[:8000])
    assert f"{WHOLE[-8000:]}[exit code 0]\n(Only the first 100000 bytes of the output" in kept
    assert estimate(said) <= 12_500
    assert f"(Only the first 100000 bytes of the output are saved, in {part}. {HINT})\n" in said
    assert said.endswith(
        f"(The whole result has {result.stat().st_size} bytes and is in {result}. {HINT})"
    )


async def test_the_place_is_asked_for_at_every_call(bench: Bench) -> None:
    first, second = Place(), Place()

    without = await bench.said(python(MUCH))
    offered = bench.ctx.provide(SPILL, first)
    with_the_first = await bench.said(python(MUCH))
    await offered.dispose()
    without_again = await bench.said(python(MUCH))
    bench.ctx.provide(SPILL, second)
    with_the_second = await bench.said(python(MUCH))

    assert without == without_again
    assert without.endswith(f"{WHOLE[-8000:]}[exit code 0]")
    assert with_the_first == with_the_second
    assert [len(place.files) for place in (first, second)] == [1, 1]
    (file,) = second.files
    assert unix(bytes(file.written)) == WHOLE
    assert file.closed == 1
    assert with_the_second == (
        f"{without}\n(The whole output has {len(file.written)} bytes and is in "
        f"kept:{shell.NAME}-output.txt. Read it.)"
    )


async def test_a_place_that_cannot_be_opened_does_not_keep_the_command_from_running(
    bench: Bench,
) -> None:
    place = Place(opens=False)
    bench.ctx.provide(SPILL, place)

    outcome = await bench.call(python(MUCH))

    assert outcome.error is None
    assert outcome.text.startswith(WHOLE[:8000])
    assert outcome.text.endswith(
        f"{WHOLE[-8000:]}[exit code 0]\n(The whole output could not be saved.)"
    )
    assert outcome.value == shell.ShellResult(outcome.text, None)


async def test_a_place_that_fills_up_is_said_to_hold_what_it_took(bench: Bench) -> None:
    place = Place(room=100_000)
    bench.ctx.provide(SPILL, place)

    outcome = await bench.call(python(MUCH))

    (file,) = place.files
    assert 0 < len(file.written) <= 100_000
    assert WHOLE.startswith(unix(bytes(file.written)))
    assert file.closed == 1
    assert outcome.text.endswith(
        f"{WHOLE[-8000:]}[exit code 0]\n(Only the first {len(file.written)} bytes of the "
        f"output are saved, in kept:{shell.NAME}-output.txt. Read it.)"
    )
    assert outcome.value == shell.ShellResult(outcome.text, None)


async def test_a_place_that_takes_nothing_is_said_to_hold_nothing(bench: Bench) -> None:
    place = Place(room=0)
    bench.ctx.provide(SPILL, place)

    said = await bench.said(python(MUCH))

    (file,) = place.files
    assert (bytes(file.written), file.closed) == (b"", 1)
    assert said.endswith(f"{WHOLE[-8000:]}[exit code 0]\n(The whole output could not be saved.)")


@pytest.mark.parametrize(
    "refusal", [OSError(5, "Input/output error"), RuntimeError("a fault of the place")]
)
async def test_an_output_whose_file_cannot_be_closed_is_said_not_to_be_saved(
    bench: Bench, caplog: pytest.LogCaptureFixture, refusal: Exception
) -> None:
    place = Place(closing=refusal)
    bench.ctx.provide(SPILL, place)

    outcome = await bench.call(python(MUCH))

    assert outcome.error is None
    assert outcome.text.endswith(
        f"{WHOLE[-8000:]}[exit code 0]\n(The whole output could not be saved.)"
    )
    assert outcome.value == shell.ShellResult(outcome.text, None)
    (record,) = (r for r in caplog.records if r.name == "columbina.builtin_tools.shell")
    assert record.levelno == logging.ERROR
    assert record.getMessage() == f"the whole output of a command could not be saved: {refusal}"
    # What the system refuses is said in a line. A fault of the place is one to be found.
    assert (record.exc_info is None) is isinstance(refusal, OSError)
    assert record.exc_info is None or record.exc_info[1] is refusal


async def test_the_output_of_a_command_that_was_stopped_is_saved_as_far_as_it_came(
    bench: Bench, monkeypatch: pytest.MonkeyPatch
) -> None:
    place = Place()
    bench.ctx.provide(SPILL, place)

    async def stopped(argv: Any, **options: Any) -> Completed:
        options["overflow"]().write(b"x" * 70_000)
        return Completed(
            exit_code=None,
            timed_out=True,
            output="xx\n[5996 bytes of output are left out here]\nxx",
            output_bytes=70_000,
            omitted_bytes=5996,
            saved_bytes=70_000,
        )

    monkeypatch.setattr(process, "run", stopped)

    said = await bench.said("echo hi", timeout_seconds=5)

    (file,) = place.files
    assert file.closed == 1
    assert said == (
        "xx\n[5996 bytes of output are left out here]\nxx\n"
        "[the command was stopped after 5 seconds]\n"
        f"(The whole output has 70000 bytes and is in kept:{shell.NAME}-output.txt. Read it.)"
    )


@pytest.mark.parametrize("closing", [None, RuntimeError("a fault of the place")])
async def test_a_call_that_is_cancelled_closes_what_it_was_saving(
    bench: Bench, closing: Exception | None
) -> None:
    # Whatever closing comes to, the call ends cancelled, and not as a failure of the tool.
    place = Place(closing=closing)
    bench.ctx.provide(SPILL, place)
    writing = (
        "import sys, time; "
        f"sys.stdout.write(''.join('%06d ' % n + 'x' * 72 + chr(10) for n in range({ROWS}))); "
        "sys.stdout.flush(); time.sleep(600)"
    )
    calling = asyncio.ensure_future(bench.call(python(writing)))
    deadline = time.monotonic() + 30
    while not place.files or len(place.files[0].written) < 150_000:
        assert time.monotonic() < deadline
        await asyncio.sleep(0.05)
    (file,) = place.files
    assert file.closed == 0

    calling.cancel()
    with pytest.raises(asyncio.CancelledError):
        await calling

    assert file.closed == 1


# Where and with what a command runs


async def test_a_command_runs_in_the_working_directory_whatever_its_path(
    ctx: Context, tmp_path: Path
) -> None:
    bench = await loaded(ctx, tmp_path, work="a directory with spaces")

    said = await bench.said(WHERE)

    assert Path(said.splitlines()[0]) == bench.work


async def test_a_command_gets_no_input_and_does_not_wait_for_any(bench: Bench) -> None:
    began = time.monotonic()

    assert await bench.said(READ_A_LINE) == "no input\n[exit code 0]"
    assert await bench.said(python("import sys; print(repr(sys.stdin.read()))")) == (
        "''\n[exit code 0]"
    )
    assert time.monotonic() - began < 20


async def test_what_sounds_like_a_secret_is_not_in_the_environment_of_a_command(
    bench: Bench, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_TEST_TOKEN", "a secret")
    monkeypatch.setenv("COLUMBINA_TEST_PLAIN", "plain")

    assert await bench.said(SECRET) == "[][plain]\n[exit code 0]"


async def test_every_call_is_a_shell_of_its_own(bench: Bench) -> None:
    (bench.work / "sub").mkdir()

    await bench.said("cd sub")
    said = await bench.said(WHERE)

    assert Path(said.splitlines()[0]) == bench.work


# How long a command may run


async def test_a_command_that_takes_too_long_is_stopped_and_the_model_is_told(bench: Bench) -> None:
    began = time.monotonic()

    outcome = await bench.call(f"echo started; {SLEEP}", timeout_seconds=2)

    assert outcome.error is None
    assert outcome.text == "started\n[the command was stopped after 2 seconds]"
    assert time.monotonic() - began < 20


@pytest.mark.parametrize(
    ("asked", "given"), [(None, 120.0), (5, 5.0), (0.5, 0.5), (600, 600.0), (100_000, 600.0)]
)
async def test_a_command_may_run_as_long_as_the_call_says_up_to_a_limit(
    bench: Bench, monkeypatch: pytest.MonkeyPatch, asked: float | None, given: float
) -> None:
    runs = watched(monkeypatch)
    more = {} if asked is None else {"timeout_seconds": asked}

    await bench.said("echo hi", **more)

    assert [run["timeout_seconds"] for run in runs] == [given]


def test_the_call_is_given_longer_than_its_command_so_that_it_can_stop_what_was_started(
    bench: Bench,
) -> None:
    tool = bench.tools.registry.get(shell.NAME)

    assert tool is not None
    assert tool.timeout_seconds == shell.MAX_SECONDS + 30


async def test_a_time_that_is_no_time_is_refused(bench: Bench) -> None:
    for wrong in (0, -1):
        outcome = await bench.call("echo hi", timeout_seconds=wrong)

        assert outcome.error is not None
        assert outcome.error.code == "INVALID_ARGS"


# What a command leaves behind


async def test_what_a_command_started_is_stopped_when_the_command_ends(bench: Bench) -> None:
    starting = (
        "import subprocess, sys; "
        "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)']); "
        "print(child.pid)"
    )

    said = await bench.said(python(starting))

    assert await gone(int(said.splitlines()[0]))


async def test_a_call_that_is_cancelled_leaves_no_process_behind(
    bench: Bench, tmp_path: Path
) -> None:
    note = tmp_path / "pid"
    waiting = (
        "import os, time; "
        f"open(r'{note}.part', 'w').write(str(os.getpid())); "
        f"os.replace(r'{note}.part', r'{note}'); "
        "time.sleep(600)"
    )
    calling = asyncio.ensure_future(bench.call(python(waiting)))
    deadline = time.monotonic() + 30
    while not note.exists():
        assert time.monotonic() < deadline
        await asyncio.sleep(0.05)
    pid = int(note.read_text())

    calling.cancel()
    with pytest.raises(asyncio.CancelledError):
        await calling

    assert await gone(pid)


# A command that does not run


async def test_a_command_that_cannot_be_started_is_a_failure_of_the_call(
    bench: Bench, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def failing(argv: Any, **options: Any) -> Any:
        raise OSError("the program 'x' could not be started: [WinError 267]")

    monkeypatch.setattr(process, "run", failing)

    outcome = await bench.call("echo hi")

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_ERROR"
    assert outcome.text == (
        "The command could not be run: the program 'x' could not be started: [WinError 267]"
    )


async def test_a_command_that_is_too_long_is_refused_before_anything_runs(
    bench: Bench, monkeypatch: pytest.MonkeyPatch
) -> None:
    runs = watched(monkeypatch)
    monkeypatch.setattr(shell, "MAX_COMMAND_BYTES", 20)
    moon = chr(0x6708)

    fits = await bench.call("echo " + "x" * 15)
    too_long = await bench.call("echo " + "x" * 16)
    in_bytes = await bench.call("echo " + moon * 6)

    assert fits.error is None
    for refused in (too_long, in_bytes):
        assert refused.error is not None
        assert refused.error.code == "COMMAND_TOO_LONG"
        assert "longer than 20 bytes" in refused.text
    assert len(runs) == 1


async def test_a_command_is_not_run_unheld_where_something_is_to_hold_it(
    ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bench = await loaded(ctx, tmp_path, sandbox={"provider": "container"})
    runs = watched(monkeypatch)

    outcome = await bench.call("echo hi")

    assert outcome.error is not None
    assert outcome.error.code == "SANDBOX_UNAVAILABLE"
    assert outcome.text == (
        "The command was not run: the sandbox provider 'container' is not there, so no "
        "command can be run."
    )
    assert runs == []


async def test_a_command_runs_the_way_the_sandbox_hands_it_back(
    ctx: Context, tmp_path: Path
) -> None:
    class Marking:
        strength: Strength = "full"

        def confine(self, argv: Sequence[str], *, mode: Mode, cwd: Path) -> Sequence[str]:
            return [PYTHON, "-c", f"print('held in {mode} mode, {len(argv)} words')"]

    bench = await loaded(ctx, tmp_path, sandbox={"provider": "marking"})
    bench.sandbox.providers.add(ctx, "marking", Marking())
    words = 6 if WINDOWS else 5

    assert await bench.said("echo hi") == (
        f"held in workspace-write mode, {words} words\n[exit code 0]"
    )


# What the checks and the permissions are told


async def test_a_call_says_that_it_runs_a_command_and_which(bench: Bench) -> None:
    seen: list[tuple[str, str | None]] = []

    async def watching(context: Context, invocation: Invocation) -> Deny | None:
        seen.append((invocation.tool.effect, invocation.target))
        return None

    bench.tools.guards.add(bench.ctx, "watching", watching)

    await bench.call("echo  hi")

    assert seen == [("run", "echo  hi")]


def test_a_command_does_not_run_beside_other_calls(bench: Bench) -> None:
    asked = ToolCallBlock(call_id="c", name=shell.NAME, arguments='{"command": "echo hi"}')

    assert not bench.tools.begin(
        bench.ctx, asked, ToolRun("c", bench.work, bench.session)
    ).concurrency_safe


async def test_the_tool_tells_the_permissions_how_its_commands_are_taken_apart(
    ctx: Context, tmp_path: Path
) -> None:
    bench = await loaded(ctx, tmp_path, allow=[f"{shell.NAME}(echo *)"])

    assert bench.permissions.splitters.get(shell.NAME) is (
        split_powershell if WINDOWS else split_bash
    )
    assert (await bench.call("echo hi; echo there")).error is None
    asked = await bench.call("echo hi; exit 3")
    assert asked.error is not None
    assert asked.error.code == "APPROVAL_UNAVAILABLE"


def test_the_model_is_told_which_shell_it_writes_for(bench: Bench) -> None:
    schema = next(schema for schema in bench.tools.schemas() if schema.name == shell.NAME)

    assert ("Windows PowerShell 5.1" in schema.description) is WINDOWS
    assert ("bash" in schema.description) or WINDOWS
    assert "a process of its own" in schema.description
    assert schema.parameters["required"] == ["command"]


# What the tool needs


@pytest.mark.parametrize("missing", ["sandbox", "permissions"])
async def test_without_the_sandbox_or_the_permissions_there_is_no_tool_that_runs_commands(
    ctx: Context, tmp_path: Path, missing: str
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    if missing != "sandbox":
        await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    if missing != "permissions" and missing != "sandbox":
        await ctx.plugin(columbina.permissions)

    ctx.plugin(shell)
    await ctx.settled()

    assert ctx.use(TOOLS).registry.get(shell.NAME) is None


async def test_the_tool_leaves_when_the_permissions_leave(ctx: Context, tmp_path: Path) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    permissions = await ctx.plugin(columbina.permissions)
    await ctx.plugin(shell)
    tools = ctx.use(TOOLS)
    assert tools.registry.get(shell.NAME) is not None

    await permissions.dispose()

    assert tools.registry.get(shell.NAME) is None


async def test_without_the_shell_of_the_platform_the_tool_does_not_load(
    ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def nowhere() -> None:
        return None

    monkeypatch.setattr(shell, "_shell", nowhere)

    with pytest.raises(RuntimeError, match="is not installed"):
        await loaded(ctx, tmp_path)


if WINDOWS:

    async def test_powershell_is_started_by_its_full_path_with_the_command_on_its_input(
        bench: Bench, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        # A repository could hold a program named powershell.exe, and Windows looks in the
        # directory that this program was started in before it looks where the system is.
        runs = watched(monkeypatch)

        await bench.said("echo hi")

        argv = runs[0]["argv"]
        system = Path(os.environ["SYSTEMROOT"])
        assert Path(argv[0]) == system / "System32/WindowsPowerShell/v1.0/powershell.exe"
        assert argv[1:5] == ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
        assert "echo hi" not in " ".join(argv)
        assert runs[0]["stdin"].startswith(b"echo hi\n")

    async def test_a_cmdlet_that_fails_last_gives_the_exit_code_one(bench: Bench) -> None:
        said = await bench.said("Get-Item 'no such file here'")

        assert said.endswith("[exit code 1]")
        assert "no such file here" in said
        assert "CLIXML" not in said

    async def test_a_failure_that_something_follows_is_not_the_exit_code(bench: Bench) -> None:
        said = await bench.said("Get-Item 'no such file here'; cmd /c exit 4; echo done")

        assert said.endswith("done\n[exit code 0]")

    async def test_an_error_that_stops_the_command_gives_the_exit_code_one(bench: Bench) -> None:
        assert (await bench.said("throw 'stop here'")).endswith("[exit code 1]")
        assert (await bench.said("if ($true) {")).endswith("[exit code 1]")
        assert (await bench.said("no-such-command-anywhere")).endswith("[exit code 1]")

    async def test_a_program_that_fails_last_gives_its_own_exit_code(bench: Bench) -> None:
        assert await bench.said("cmd /c exit 6") == "[exit code 6]"

    async def test_a_long_command_runs(bench: Bench) -> None:
        long = (
            "echo '" + "x" * 100_000 + "' | Measure-Object -Character | ForEach-Object Characters"
        )

        assert await bench.said(long) == "100000\n[exit code 0]"

    async def test_python_is_told_to_write_in_utf8_unless_the_user_said_otherwise(
        bench: Bench, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        runs = watched(monkeypatch)
        monkeypatch.delenv("PYTHONIOENCODING", raising=False)

        await bench.said("echo hi")
        monkeypatch.setenv("PYTHONIOENCODING", "gbk")
        await bench.said("echo hi")

        assert [run["env"]["PYTHONIOENCODING"] for run in runs] == ["utf-8", "gbk"]

    def test_powershell_is_looked_for_where_the_system_is(
        monkeypatch: pytest.MonkeyPatch, tmp_path: Path
    ) -> None:
        find: Any = vars(shell)["_shell"]
        monkeypatch.setenv("SYSTEMROOT", str(tmp_path))

        assert find() is None

        home = tmp_path / "System32" / "WindowsPowerShell" / "v1.0"
        home.mkdir(parents=True)
        (home / "powershell.exe").write_bytes(b"")

        assert find() == str(home / "powershell.exe")

else:

    async def test_bash_is_given_the_command_as_an_argument_and_no_input(
        bench: Bench, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        runs = watched(monkeypatch)

        await bench.said("echo hi")

        assert runs[0]["argv"][1:] == ["--noprofile", "--norc", "-c", "echo hi"]
        assert Path(runs[0]["argv"][0]).name == "bash"
        assert runs[0]["stdin"] is None

    async def test_the_exit_code_of_the_last_command_is_passed_on(bench: Bench) -> None:
        assert await bench.said("false; true") == "[exit code 0]"
        assert await bench.said("true; false") == "[exit code 1]"
