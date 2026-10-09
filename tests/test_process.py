"""Running a program within limits: its output, its time, and what is left of it afterwards."""

import asyncio
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.process import Completed, HeadTail, cleaned
from support.processes import end, gone, python

pytestmark = pytest.mark.timeout(60)

SLEEPER = "import time; time.sleep(60)"

STARTS_ANOTHER = (
    "import subprocess, sys\n"
    f"child = subprocess.Popen([sys.executable, '-c', {SLEEPER!r}])\n"
    "print(child.pid, flush=True)\n"
    "if len(sys.argv) > 1:\n"
    "    with open(sys.argv[1], 'w') as note:\n"
    "        print(child.pid, file=note)\n"
)
"""Starts a program that goes on for a minute, says which process that is, and ends."""

STARTS_ANOTHER_AND_STAYS = STARTS_ANOTHER + "import time\ntime.sleep(60)\n"

SAYS_WHO_AND_STAYS = (
    "import os, sys, time\n"
    "print(os.getpid(), flush=True)\n"
    "if len(sys.argv) > 1:\n"
    "    with open(sys.argv[1], 'w') as note:\n"
    "        print(os.getpid(), file=note)\n"
    "time.sleep(60)\n"
)

LEAVES_A_NOTE = "import sys; open(sys.argv[1], 'w').close()"


async def told(note: Path, within: float = 20.0) -> int:
    """Wait for a program to write a process down, and return which one."""
    deadline = time.monotonic() + within
    while time.monotonic() < deadline:
        if note.exists():
            text = note.read_text()
            if text.endswith("\n"):
                return int(text)
        await asyncio.sleep(0.05)
    raise AssertionError("the program did not write down a process")


def unstoppable(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make stopping what a program started do nothing, as if it had got away."""

    def stop(self: object) -> None:
        pass

    tree: Any = vars(process)["_Tree"]
    monkeypatch.setattr(tree, "stop", stop)


def patient(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make the bounded waits long, so that a run ends soon only if things were stopped."""
    monkeypatch.setattr(process, "_REAP_SECONDS", 40.0)
    monkeypatch.setattr(process, "_DRAIN_SECONDS", 40.0)


# What a program says and how it ends


async def test_the_output_and_the_exit_code_of_a_program_are_returned(tmp_path: Path) -> None:
    done = await process.run(
        python("import sys; print('hello', end=''); sys.exit(3)"), cwd=tmp_path, timeout_seconds=30
    )

    assert done == Completed(exit_code=3, timed_out=False, output="hello", omitted_bytes=0)


async def test_what_goes_to_the_error_stream_comes_in_the_order_it_was_written(
    tmp_path: Path,
) -> None:
    code = (
        "import sys\n"
        "print('one', flush=True)\n"
        "print('two', file=sys.stderr, flush=True)\n"
        "print('three', flush=True)\n"
    )

    done = await process.run(python(code), cwd=tmp_path, timeout_seconds=30)

    assert done.output.splitlines() == ["one", "two", "three"]
    assert done.exit_code == 0


async def test_a_program_runs_in_the_directory_it_is_given(tmp_path: Path) -> None:
    done = await process.run(
        python("import os; print(os.getcwd())"), cwd=tmp_path, timeout_seconds=30
    )

    assert Path(done.output.strip()).resolve() == tmp_path.resolve()


async def test_a_program_sees_the_environment_it_is_given(tmp_path: Path) -> None:
    environment = {**os.environ, "COLUMBINA_TEST_API_KEY": "halyard"}

    done = await process.run(
        python("import os; print(os.environ['COLUMBINA_TEST_API_KEY'])"),
        cwd=tmp_path,
        env=environment,
        timeout_seconds=30,
    )

    assert done.output.strip() == "halyard"


async def test_a_program_given_no_environment_gets_this_one_without_its_secrets(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_TEST_API_KEY", "sk-not-for-programs")
    monkeypatch.setenv("COLUMBINA_TEST_WORD", "halyard")
    code = (
        "import os\n"
        "print(os.environ.get('COLUMBINA_TEST_API_KEY'), os.environ.get('COLUMBINA_TEST_WORD'))\n"
    )

    done = await process.run(python(code), cwd=tmp_path, timeout_seconds=30)

    assert done.output.split() == ["None", "halyard"]


async def test_a_program_can_be_given_input(tmp_path: Path) -> None:
    done = await process.run(
        python("import sys; print(sys.stdin.read().upper())"),
        cwd=tmp_path,
        stdin=b"spoken to",
        timeout_seconds=30,
    )

    assert done.output.strip() == "SPOKEN TO"


async def test_a_program_that_reads_without_being_given_input_finds_none(tmp_path: Path) -> None:
    done = await process.run(
        python("import sys; print(repr(sys.stdin.read()))"), cwd=tmp_path, timeout_seconds=30
    )

    assert done.output.strip() == "''"


async def test_a_program_that_does_not_read_its_input_does_no_harm(tmp_path: Path) -> None:
    done = await process.run(
        python("print('deaf')"), cwd=tmp_path, stdin=b"x" * 1_000_000, timeout_seconds=30
    )

    assert done.output.strip() == "deaf"
    assert done.exit_code == 0


async def test_output_that_is_no_utf8_is_shown_with_replacement_characters(tmp_path: Path) -> None:
    done = await process.run(
        python("import sys; sys.stdout.buffer.write(bytes([99, 97, 102, 233]))"),
        cwd=tmp_path,
        timeout_seconds=30,
    )

    assert done.output == "caf" + chr(0xFFFD)


async def test_what_a_program_wrote_just_before_it_ended_is_not_lost(tmp_path: Path) -> None:
    code = "import sys; sys.stdout.write('a' * 2_000_000 + 'END')"

    done = await process.run(python(code), cwd=tmp_path, timeout_seconds=30)

    assert done.output.endswith("END")
    assert done.omitted_bytes == 2_000_003 - 64_000
    assert done.exit_code == 0


async def test_a_program_that_cannot_be_started_raises(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="'no-such-program-of-columbina' could not be started"):
        await process.run(["no-such-program-of-columbina"], cwd=tmp_path, timeout_seconds=30)


async def test_arguments_that_no_program_can_be_given_raise_the_same_way(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="could not be started"):
        await process.run(python("pass" + chr(0)), cwd=tmp_path, timeout_seconds=30)


async def test_running_nothing_is_refused(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="no program"):
        await process.run([], cwd=tmp_path, timeout_seconds=30)


# Too much output


async def test_of_too_much_output_the_beginning_and_the_end_are_kept(tmp_path: Path) -> None:
    code = "import sys; sys.stdout.write('a' * 3000 + 'b' * 5000 + 'c' * 3000)"

    done = await process.run(python(code), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000)

    head, note, tail = done.output.split("\n")
    assert head == "a" * 1000
    assert note == "[9000 bytes of output are left out here]"
    assert tail == "c" * 1000
    assert done.omitted_bytes == 9000
    assert done.exit_code == 0


@pytest.mark.parametrize(("size", "left_out"), [(1999, 0), (2000, 0), (2001, 1), (2002, 2)])
def test_output_is_kept_whole_up_to_the_limit(size: int, left_out: int) -> None:
    kept = HeadTail(2000)

    for start in range(0, size, 7):
        kept.add(b"x" * min(7, size - start))

    assert kept.total == size
    assert kept.omitted == left_out
    assert kept.text().count("x") == size - left_out
    assert ("left out" in kept.text()) is bool(left_out)


@pytest.mark.parametrize(
    ("limit", "data", "text"),
    [
        (4, b"ab" + b"x" * 10 + b"yz", "ab\n[10 bytes of output are left out here]\nyz"),
        (4, b"a\n" + b"x" * 10 + b"yz", "a\n[10 bytes of output are left out here]\nyz"),
        (2, b"abc", "a\n[1 byte of output is left out here]\nc"),
        (1, b"abc", "[2 bytes of output are left out here]\nc"),
        (0, b"abc", "[3 bytes of output are left out here]\n"),
    ],
)
def test_the_line_about_what_is_left_out_stands_on_its_own(
    limit: int, data: bytes, text: str
) -> None:
    kept = HeadTail(limit)

    kept.add(data)

    assert kept.text() == text


def test_a_cut_does_not_leave_half_a_character() -> None:
    # Each accented letter is two bytes, and both cuts fall between the two.
    data = b"ab" + "é".encode() * 10
    kept = HeadTail(10)

    kept.add(data)

    assert kept.text() == "abé\n[14 bytes of output are left out here]\néé"
    assert kept.omitted == 14


def test_output_that_fits_is_not_cut_where_its_two_halves_meet() -> None:
    kept = HeadTail(4)

    kept.add("aéb".encode())

    assert kept.text() == "aéb"
    assert kept.omitted == 0


def test_what_is_kept_does_not_depend_on_how_the_output_arrived() -> None:
    data = bytes(range(256)) * 40
    at_once, in_pieces = HeadTail(1000), HeadTail(1000)

    at_once.add(data)
    for start in range(0, len(data), 333):
        in_pieces.add(data[start : start + 333])

    assert at_once.text() == in_pieces.text()
    assert at_once.omitted == in_pieces.omitted
    assert len(data) - 1000 <= at_once.omitted <= len(data) - 1000 + 6


def test_keeping_less_than_nothing_is_refused() -> None:
    with pytest.raises(ValueError, match="less than nothing"):
        HeadTail(-1)


# Too much time


async def test_a_program_that_takes_too_long_is_stopped(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    began = time.monotonic()

    done = await process.run(python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=1.0)

    assert done.timed_out
    assert done.exit_code is None
    assert time.monotonic() - began < 20
    assert await gone(int(done.output.strip()))


async def test_stopping_a_program_stops_the_programs_it_started(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    began = time.monotonic()

    done = await process.run(python(STARTS_ANOTHER_AND_STAYS), cwd=tmp_path, timeout_seconds=2.0)

    assert done.timed_out
    assert time.monotonic() - began < 20
    assert await gone(int(done.output.strip()))


async def test_cancelling_the_caller_stops_the_program_and_what_it_started(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    note = tmp_path / "started"
    running = asyncio.ensure_future(
        process.run(python(STARTS_ANOTHER_AND_STAYS, str(note)), cwd=tmp_path, timeout_seconds=60)
    )
    started = await told(note)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)
    assert await gone(started)


class SlowStart:
    """Starts programs for a run, and takes its time to say that it has."""

    def __init__(self, monkeypatch: pytest.MonkeyPatch, *, fails: bool = False) -> None:
        loop = asyncio.get_running_loop()
        self.started: list[asyncio.SubprocessTransport] = []
        self.made = asyncio.Event()
        self.go_on = asyncio.Event()
        self.over = asyncio.Event()
        really_start: Any = loop.subprocess_exec

        async def start(*arguments: Any, **options: Any) -> Any:
            try:
                if fails:
                    await self.go_on.wait()
                    raise OSError("the program was not there after all")
                made = await really_start(*arguments, **options)
                self.started.append(made[0])
                self.made.set()
                await self.go_on.wait()
                return made
            finally:
                self.over.set()

        monkeypatch.setattr(loop, "subprocess_exec", start)


async def test_a_program_that_comes_into_being_after_its_caller_was_cancelled_is_ended(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    slow = SlowStart(monkeypatch)
    note = tmp_path / "started"
    running = asyncio.ensure_future(
        process.run(python(STARTS_ANOTHER_AND_STAYS, str(note)), cwd=tmp_path, timeout_seconds=60)
    )
    await asyncio.wait_for(slow.made.wait(), timeout=20)
    descendants: list[int] = []
    if sys.platform != "win32":
        # Where a program runs from the moment it is made, it starts another before long.
        descendants.append(await told(note))

    running.cancel()
    with pytest.raises(asyncio.CancelledError):
        await running
    slow.go_on.set()

    assert await gone(slow.started[0].get_pid())
    for descendant in descendants:
        assert await gone(descendant)
    if not descendants:
        # The program was made suspended, so it started nothing, and it never ran.
        await asyncio.sleep(0.5)
        assert not note.exists()


async def test_a_program_that_could_not_be_started_after_its_caller_was_cancelled_is_no_loose_end(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    slow = SlowStart(monkeypatch, fails=True)
    running = asyncio.ensure_future(process.run(python("pass"), cwd=tmp_path, timeout_seconds=60))
    await asyncio.sleep(0.05)

    running.cancel()
    with pytest.raises(asyncio.CancelledError):
        await running
    with caplog.at_level(logging.ERROR):
        slow.go_on.set()
        await asyncio.wait_for(slow.over.wait(), timeout=5)
        await asyncio.sleep(0.05)

    # The failure was taken note of, so the event loop has nothing to complain about.
    assert "never retrieved" not in caplog.text


# What a program leaves behind


async def test_what_a_program_started_does_not_outlive_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    began = time.monotonic()

    done = await process.run(python(STARTS_ANOTHER), cwd=tmp_path, timeout_seconds=60)

    started = int(done.output.strip())
    assert done.exit_code == 0
    assert not done.timed_out
    assert time.monotonic() - began < 20
    assert await gone(started)


async def test_a_run_does_not_wait_for_output_that_something_left_behind_holds_on_to(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # What the program started keeps the output open, and for once it is not stopped.
    unstoppable(monkeypatch)
    monkeypatch.setattr(process, "_DRAIN_SECONDS", 0.5)
    began = time.monotonic()

    done = await process.run(python(STARTS_ANOTHER), cwd=tmp_path, timeout_seconds=60)

    started = int(done.output.strip())
    try:
        assert done.exit_code == 0
        assert not done.timed_out
        assert done.omitted_bytes == 0
        assert time.monotonic() - began < 20
    finally:
        end(started)


async def test_a_program_that_cannot_be_stopped_does_not_keep_a_run_from_ending(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    unstoppable(monkeypatch)
    monkeypatch.setattr(process, "_REAP_SECONDS", 0.3)
    monkeypatch.setattr(process, "_DRAIN_SECONDS", 0.3)
    began = time.monotonic()

    done = await process.run(python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=0.5)

    assert done.timed_out
    assert done.exit_code is None
    assert time.monotonic() - began < 20
    # Letting go of the program ends it in the end, if only that one process.
    assert await gone(int(done.output.strip()))


async def test_a_caller_cancelled_again_while_a_run_winds_down_is_not_held_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    unstoppable(monkeypatch)
    monkeypatch.setattr(process, "_REAP_SECONDS", 30.0)
    note = tmp_path / "running"
    running = asyncio.ensure_future(
        process.run(python(SAYS_WHO_AND_STAYS, str(note)), cwd=tmp_path, timeout_seconds=60)
    )
    program = await told(note)
    running.cancel()
    await asyncio.sleep(0.2)
    assert not running.done()
    began = time.monotonic()

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)
    assert time.monotonic() - began < 10
    assert await gone(program)


if sys.platform == "win32":
    import ctypes

    class Failing:
        """A library of the system, of which one function fails."""

        def __init__(self, real: Any, function: str, result: int) -> None:
            self._real = real
            self._function = function
            self._result = result

        def __getattr__(self, name: str) -> Any:
            return self._fail if name == self._function else getattr(self._real, name)

        def _fail(self, *arguments: object) -> int:
            ctypes.set_last_error(5)
            return self._result

    @pytest.mark.parametrize(
        ("library", "function", "result"),
        [
            ("_kernel32", "CreateJobObjectW", 0),
            ("_kernel32", "SetInformationJobObject", 0),
            ("_kernel32", "OpenProcess", 0),
            ("_kernel32", "AssignProcessToJobObject", 0),
            ("_ntdll", "NtResumeProcess", -1073741790),
        ],
    )
    async def test_a_program_that_cannot_be_put_into_a_job_does_not_run(
        tmp_path: Path, monkeypatch: pytest.MonkeyPatch, library: str, function: str, result: int
    ) -> None:
        started: list[int] = []
        tree: Any = vars(process)["_Tree"]

        def recording(pid: int) -> Any:
            started.append(pid)
            # Time enough for a program that was let run to leave its note.
            time.sleep(0.5)
            return tree(pid)

        monkeypatch.setattr(process, "_Tree", recording)
        monkeypatch.setattr(process, library, Failing(vars(process)[library], function, result))
        ran = tmp_path / "ran"

        with pytest.raises(OSError, match="could not"):
            await process.run(python(LEAVES_A_NOTE, str(ran)), cwd=tmp_path, timeout_seconds=30)

        assert await gone(started[0])
        assert not ran.exists()

    async def test_a_job_that_cannot_be_ended_is_written_down_and_given_up(
        tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
    ) -> None:
        failing = Failing(vars(process)["_kernel32"], "TerminateJobObject", 0)
        monkeypatch.setattr(process, "_kernel32", failing)
        monkeypatch.setattr(process, "_REAP_SECONDS", 0.3)
        monkeypatch.setattr(process, "_DRAIN_SECONDS", 0.3)

        with caplog.at_level(logging.ERROR, logger="columbina.process"):
            done = await process.run(
                python(STARTS_ANOTHER_AND_STAYS), cwd=tmp_path, timeout_seconds=1.0
            )

        assert done.timed_out
        assert "could not be stopped" in caplog.text
        # Giving the job up ends what is in it, so nothing is left even now.
        assert await gone(int(done.output.strip()))


# What a program is not told


def test_variables_that_sound_like_secrets_are_not_passed_on() -> None:
    environment = {
        "PATH": "/usr/bin",
        "DEEPSEEK_API_KEY": "sk-1",
        "github_token": "t",
        "DB_PASSWORD": "p",
        "Client_Secret": "s",
        "HOME": "/home/me",
        "COLUMBINA_HOME": "/home/me/.columbina",
    }

    assert cleaned(environment) == {
        "PATH": "/usr/bin",
        "HOME": "/home/me",
        "COLUMBINA_HOME": "/home/me/.columbina",
    }
