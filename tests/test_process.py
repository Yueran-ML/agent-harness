"""Running a program within limits: its output, its time, and what is left of it afterwards."""

import asyncio
import logging
import os
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.process import Completed, HeadTail, cleaned
from support.processes import alive, end, gone, python

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

STAYS_UNTIL_A_WORD = (
    "import os, sys, time\n"
    "with open(sys.argv[1], 'w') as note:\n"
    "    print(os.getpid(), file=note)\n"
    "while not os.path.exists(sys.argv[2]):\n"
    "    time.sleep(0.02)\n"
    "print('heard', flush=True)\n"
    "sys.exit(5)\n"
)
"""Says which process it is and stays until a file is there. Then it ends by itself."""


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

    assert done == Completed(
        exit_code=3,
        timed_out=False,
        output="hello",
        output_bytes=5,
        omitted_bytes=0,
        saved_bytes=0,
    )


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
    # Where the two streams are one, nothing is told of the error stream by itself.
    assert (done.error_output, done.error_bytes, done.error_omitted_bytes) == ("", 0, 0)


# The two streams apart

SAYS_ON_BOTH = (
    "import sys\n"
    "print('one', flush=True)\n"
    "print('two', file=sys.stderr, flush=True)\n"
    "print('three', flush=True)\n"
    "sys.exit(2)\n"
)


async def test_the_two_streams_of_a_program_can_be_kept_apart(tmp_path: Path) -> None:
    done = await process.run(
        python(SAYS_ON_BOTH), cwd=tmp_path, timeout_seconds=30, streams="apart"
    )

    assert done.output.splitlines() == ["one", "three"]
    assert done.error_output.splitlines() == ["two"]
    assert (done.exit_code, done.timed_out) == (2, False)
    # Each stream is counted by itself.
    assert done.output_bytes == len(done.output.encode())
    assert done.error_bytes == len(done.error_output.encode())
    assert (done.omitted_bytes, done.error_omitted_bytes) == (0, 0)


async def test_each_of_the_two_streams_has_the_limit_to_itself(tmp_path: Path) -> None:
    code = "import sys\nsys.stdout.write('a' * 300)\nsys.stderr.write('x' * 3000 + 'y' * 3000)\n"

    done = await process.run(
        python(code), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, streams="apart"
    )

    # Too much on one stream takes nothing from the other.
    assert done.output == "a" * 300
    assert done.omitted_bytes == 0
    head, note, tail = done.error_output.split("\n")
    assert (head, tail) == ("x" * 1000, "y" * 1000)
    assert note == "[4000 bytes of output are left out here]"
    assert (done.error_bytes, done.error_omitted_bytes) == (6000, 4000)


async def test_a_program_that_writes_nothing_to_one_of_the_streams_is_awaited_all_the_same(
    tmp_path: Path,
) -> None:
    quiet = await process.run(
        python("import sys; sys.stderr.write('only here')"),
        cwd=tmp_path,
        timeout_seconds=30,
        streams="apart",
    )
    assert (quiet.output, quiet.error_output) == ("", "only here")

    loud = await process.run(
        python("print('only here', end='')"), cwd=tmp_path, timeout_seconds=30, streams="apart"
    )
    assert (loud.output, loud.error_output) == ("only here", "")


async def test_what_was_written_to_either_stream_just_before_the_end_is_not_lost(
    tmp_path: Path,
) -> None:
    code = (
        "import os, sys\n"
        "sys.stdout.write('out' * 20000)\n"
        "sys.stderr.write('err' * 20000)\n"
        "sys.stdout.flush()\n"
        "sys.stderr.flush()\n"
        "os._exit(0)\n"
    )

    done = await process.run(
        python(code), cwd=tmp_path, timeout_seconds=30, max_output_bytes=200_000, streams="apart"
    )

    assert (done.output, done.error_output) == ("out" * 20000, "err" * 20000)


async def test_the_error_stream_is_read_to_its_end_although_the_output_was_closed_long_before(
    tmp_path: Path,
) -> None:
    code = (
        "import os, sys, time\n"
        "os.close(1)\n"
        "time.sleep(0.5)\n"
        "sys.stderr.write('late ' * 30000)\n"
        "sys.stderr.flush()\n"
        "os._exit(0)\n"
    )

    done = await process.run(
        python(code), cwd=tmp_path, timeout_seconds=30, max_output_bytes=400_000, streams="apart"
    )

    assert (done.output, done.error_output) == ("", "late " * 30000)


async def test_a_run_goes_on_reading_until_both_streams_have_come_to_their_ends() -> None:
    # What hears a program is asked by itself: a real program ends its two streams at
    # nearly the same moment, and which of them is first is nothing that a test can arrange.
    hears: Any = vars(process)["_Watch"]
    keeps: Any = vars(process)["_Output"]

    apart = hears(keeps(100, None, 0), HeadTail(100))
    # The pipe that the program reads from says nothing about what it has written.
    apart.pipe_connection_lost(0, None)
    apart.pipe_connection_lost(1, None)
    assert not apart.silent.done()
    apart.pipe_connection_lost(2, None)
    assert apart.silent.done()

    late = hears(keeps(100, None, 0), HeadTail(100))
    late.pipe_connection_lost(2, None)
    assert not late.silent.done()
    late.pipe_connection_lost(1, None)
    assert late.silent.done()

    joined = hears(keeps(100, None, 0))
    joined.pipe_connection_lost(1, None)
    assert joined.silent.done()


async def test_input_reaches_a_program_whose_streams_are_kept_apart(tmp_path: Path) -> None:
    code = "import sys; told = sys.stdin.read(); print(told.upper()); sys.stderr.write(told)"

    done = await process.run(
        python(code), cwd=tmp_path, stdin=b"hello", timeout_seconds=30, streams="apart"
    )

    assert (done.output.strip(), done.error_output) == ("HELLO", "hello")


async def test_a_program_whose_streams_are_kept_apart_is_stopped_when_its_time_is_up(
    tmp_path: Path,
) -> None:
    code = "import sys, time; sys.stderr.write('begun'); sys.stderr.flush(); time.sleep(60)"

    began = time.monotonic()
    done = await process.run(python(code), cwd=tmp_path, timeout_seconds=1, streams="apart")

    assert (done.exit_code, done.timed_out) == (None, True)
    assert done.error_output == "begun"
    assert time.monotonic() - began < 20


async def test_the_whole_output_is_kept_elsewhere_only_where_the_streams_are_one(
    tmp_path: Path,
) -> None:
    class Place:
        def write(self, data: bytes) -> None:
            raise AssertionError("nothing is written where the run is refused")

    with pytest.raises(ValueError, match="only where the two streams are one"):
        await process.run(
            python("pass"), cwd=tmp_path, timeout_seconds=30, streams="apart", overflow=Place
        )


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


ENDS_AS_A_COMMAND_STARTS = """
import asyncio, sys, tempfile
from pathlib import Path
from columbina import process


async def main():
    cwd = Path(tempfile.gettempdir())
    running = asyncio.ensure_future(
        process.run([sys.executable, "-c", "pass"], cwd=cwd, timeout_seconds=60)
    )
    for _ in range(int(sys.argv[1])):
        await asyncio.sleep(0)
    running.cancel()
    try:
        await running
    except asyncio.CancelledError:
        pass
    print("main is over", flush=True)


asyncio.run(main())
print("the loop is closed", flush=True)
"""
"""A program that cancels the caller of a command so many turns of the loop after it began.

Then it ends at once. The command is being started at that moment, for some of the numbers.
"""


async def test_a_caller_that_is_cancelled_as_its_program_is_started_leaves_when_that_is_gone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    slow = SlowStart(monkeypatch)
    running = asyncio.ensure_future(
        process.run(
            python(STARTS_ANOTHER_AND_STAYS, str(tmp_path / "started")),
            cwd=tmp_path,
            timeout_seconds=60,
        )
    )
    await asyncio.wait_for(slow.made.wait(), timeout=20)

    running.cancel()
    await asyncio.sleep(0.3)
    waits = not running.done()
    slow.go_on.set()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)

    # The program was made, ended, and seen to end before the cancellation went on. Nothing
    # of it is left for an event loop that may be closed next.
    assert waits
    (made,) = slow.started
    assert made.get_returncode() is not None
    assert not alive(made.get_pid())


if sys.platform != "win32":

    async def test_a_program_that_is_ended_for_a_caller_who_was_cancelled_is_reaped_by_the_loop(
        tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
    ) -> None:
        # Whoever asks the system first whether a program has ended is the one who is told. If
        # that is not the event loop, the loop makes up an exit code and complains.
        really_end: Any = os.killpg

        def end_at_once(group: int, sent: int) -> None:
            # The system may have ended the program by the time this returns. Here it has.
            really_end(group, sent)
            deadline = time.monotonic() + 10
            while alive(group) and time.monotonic() < deadline:
                time.sleep(0.01)

        monkeypatch.setattr(os, "killpg", end_at_once)
        slow = SlowStart(monkeypatch)
        running = asyncio.ensure_future(
            process.run(
                python(STARTS_ANOTHER_AND_STAYS, str(tmp_path / "started")),
                cwd=tmp_path,
                timeout_seconds=60,
            )
        )
        await asyncio.wait_for(slow.made.wait(), timeout=20)
        running.cancel()
        await asyncio.sleep(0.1)

        with caplog.at_level(logging.WARNING, logger="asyncio"):
            slow.go_on.set()
            with pytest.raises(asyncio.CancelledError):
                await asyncio.wait_for(running, timeout=20)

        assert slow.started[0].get_returncode() == -signal.SIGKILL
        assert "already read" not in caplog.text


async def test_a_caller_that_is_cancelled_again_does_not_wait_for_the_start(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(process, "_START_SECONDS", 40.0)
    slow = SlowStart(monkeypatch)
    running = asyncio.ensure_future(
        process.run(
            python(STARTS_ANOTHER_AND_STAYS, str(tmp_path / "started")),
            cwd=tmp_path,
            timeout_seconds=60,
        )
    )
    await asyncio.wait_for(slow.made.wait(), timeout=20)
    running.cancel()
    await asyncio.sleep(0.3)
    waits = not running.done()

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=5)
    assert waits
    # What the start makes is ended all the same, when it is there.
    slow.go_on.set()
    assert await gone(slow.started[0].get_pid())
    await asyncio.sleep(0.5)


async def test_a_start_that_fails_while_its_cancelled_caller_waits_holds_nobody_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    slow = SlowStart(monkeypatch, fails=True)
    running = asyncio.ensure_future(process.run(python("pass"), cwd=tmp_path, timeout_seconds=60))
    await asyncio.sleep(0.05)
    running.cancel()
    await asyncio.sleep(0.05)
    began = time.monotonic()

    with caplog.at_level(logging.ERROR):
        slow.go_on.set()
        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(running, timeout=20)
        await asyncio.sleep(0.05)

    # No program came of it, so there is none to wait for. And the failure was taken note of.
    assert time.monotonic() - began < 3
    assert "never retrieved" not in caplog.text


@pytest.mark.parametrize("turns", [1, 2, 3])
def test_a_program_comes_to_its_end_although_a_command_was_being_started(turns: int) -> None:
    # Python before 3.13 does not come back from closing an event loop in which a program is
    # still being started. A caller that was cancelled waits for the start, so none is.
    done = subprocess.run(
        [sys.executable, "-c", ENDS_AS_A_COMMAND_STARTS, str(turns)],
        capture_output=True,
        text=True,
        timeout=40,
        check=False,
    )

    assert (done.returncode, done.stdout.split("\n")[:2]) == (
        0,
        ["main is over", "the loop is closed"],
    ), done.stderr


async def test_a_program_that_comes_into_being_after_its_caller_was_cancelled_is_ended(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The start takes longer than a caller that was cancelled waits for it.
    monkeypatch.setattr(process, "_START_SECONDS", 0.1)
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
    monkeypatch.setattr(process, "_START_SECONDS", 0.1)
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


# What a caller does before its program is stopped


async def test_a_caller_is_heard_before_its_program_is_stopped(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    note = tmp_path / "started"
    seen: list[tuple[bool, bool]] = []

    async def hears(exited: asyncio.Future[None]) -> None:
        # The program still runs: its caller comes first.
        seen.append((exited.done(), alive(await told(note))))

    done = await process.run(
        python(SAYS_WHO_AND_STAYS, str(note)), cwd=tmp_path, timeout_seconds=1.0, before_stop=hears
    )

    assert seen == [(False, True)]
    assert done.timed_out
    assert await gone(int(done.output.strip()))


async def test_a_caller_can_end_in_its_own_way_what_its_program_stands_for(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    note, word = tmp_path / "started", tmp_path / "word"
    ended: list[bool] = []

    async def says_the_word(exited: asyncio.Future[None]) -> None:
        word.touch()
        await asyncio.wait_for(exited, timeout=30)
        ended.append(exited.done())

    done = await process.run(
        python(STAYS_UNTIL_A_WORD, str(note), str(word)),
        cwd=tmp_path,
        timeout_seconds=1.0,
        before_stop=says_the_word,
    )

    # The program came to its end by itself and said so. Its time was up all the same.
    assert ended == [True]
    assert (done.timed_out, done.exit_code, done.output.strip()) == (True, None, "heard")


async def test_a_program_that_ends_by_itself_is_no_reason_to_hear_its_caller(
    tmp_path: Path,
) -> None:
    heard: list[object] = []

    async def hears(exited: asyncio.Future[None]) -> None:
        heard.append(exited)

    done = await process.run(
        python("print('hi')"), cwd=tmp_path, timeout_seconds=30, before_stop=hears
    )

    assert (done.exit_code, heard) == (0, [])


async def test_a_program_that_never_ran_is_no_reason_to_hear_its_caller(tmp_path: Path) -> None:
    heard: list[object] = []

    async def hears(exited: asyncio.Future[None]) -> None:
        heard.append(exited)

    with pytest.raises(OSError, match="could not be started"):
        await process.run(
            ["no-such-program-anywhere"], cwd=tmp_path, timeout_seconds=30, before_stop=hears
        )

    assert heard == []


async def test_a_caller_that_is_cancelled_is_heard_before_its_program_is_stopped(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    note = tmp_path / "started"
    seen: list[bool] = []

    async def hears(exited: asyncio.Future[None]) -> None:
        seen.append(alive(await told(note)))

    running = asyncio.ensure_future(
        process.run(
            python(SAYS_WHO_AND_STAYS, str(note)),
            cwd=tmp_path,
            timeout_seconds=60,
            before_stop=hears,
        )
    )
    program = await told(note)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)
    assert seen == [True]
    assert await gone(program)


class Slowly:
    """What a caller does before its program is stopped, taking its time."""

    def __init__(self, program: Path) -> None:
        self.program = program
        self.begun = asyncio.Event()
        self.go_on = asyncio.Event()
        self.finished: list[bool] = []

    async def __call__(self, exited: asyncio.Future[None]) -> None:
        self.begun.set()
        await self.go_on.wait()
        # Whether the program was left alone all the while.
        self.finished.append(alive(await told(self.program)))


async def test_a_second_cancellation_does_not_cut_short_what_the_caller_does_first(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    note = tmp_path / "started"
    slowly = Slowly(note)
    running = asyncio.ensure_future(
        process.run(
            python(SAYS_WHO_AND_STAYS, str(note)),
            cwd=tmp_path,
            timeout_seconds=60,
            before_stop=slowly,
        )
    )
    program = await told(note)
    running.cancel()
    await slowly.begun.wait()

    running.cancel()
    await asyncio.sleep(0.3)
    still_at_it = not running.done() and slowly.finished == []
    slowly.go_on.set()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)
    assert still_at_it
    assert slowly.finished == [True]
    assert await gone(program)


async def test_a_cancellation_that_comes_while_the_caller_is_heard_is_not_lost(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The time of the program was up. Its caller is cancelled while it stops things its own way.
    patient(monkeypatch)
    note = tmp_path / "started"
    slowly = Slowly(note)
    running = asyncio.ensure_future(
        process.run(
            python(SAYS_WHO_AND_STAYS, str(note)),
            cwd=tmp_path,
            timeout_seconds=0.5,
            before_stop=slowly,
        )
    )
    program = await told(note)
    await slowly.begun.wait()

    running.cancel()
    await asyncio.sleep(0.3)
    still_at_it = not running.done()
    slowly.go_on.set()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=20)
    assert still_at_it
    assert slowly.finished == [True]
    assert await gone(program)


async def test_a_caller_that_fails_when_it_is_heard_does_not_keep_its_program_running(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    patient(monkeypatch)

    async def fails(exited: asyncio.Future[None]) -> None:
        raise RuntimeError("the service did not answer")

    with caplog.at_level(logging.ERROR, logger="columbina.process"):
        done = await process.run(
            python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=1.0, before_stop=fails
        )

    assert done.timed_out
    assert await gone(int(done.output.strip()))
    assert "what was to be done before a program is stopped failed" in caplog.text
    assert "the service did not answer" in caplog.text


async def test_a_caller_that_takes_too_long_when_it_is_heard_is_given_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    patient(monkeypatch)
    monkeypatch.setattr(process, "_BEFORE_STOP_SECONDS", 0.3)
    left: list[str] = []

    async def never_ends(exited: asyncio.Future[None]) -> None:
        try:
            await asyncio.sleep(60)
        finally:
            left.append("told to stop")

    began = time.monotonic()
    with caplog.at_level(logging.ERROR, logger="columbina.process"):
        done = await process.run(
            python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=0.5, before_stop=never_ends
        )

    assert done.timed_out
    assert time.monotonic() - began < 20
    assert await gone(int(done.output.strip()))
    assert left == ["told to stop"]
    assert "took longer than 0.3 seconds and was given up" in caplog.text
    # Giving it up is not a failure of it.
    assert "before a program is stopped failed" not in caplog.text


async def test_a_caller_that_does_not_let_go_when_it_is_given_up_does_not_hold_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    patient(monkeypatch)
    monkeypatch.setattr(process, "_BEFORE_STOP_SECONDS", 0.3)
    over = asyncio.Event()

    async def holds_on(exited: asyncio.Future[None]) -> None:
        try:
            await asyncio.sleep(60)
        except asyncio.CancelledError:
            # Told to stop, and going on regardless.
            await asyncio.sleep(3.0)
        over.set()

    began = time.monotonic()
    with caplog.at_level(logging.ERROR, logger="columbina.process"):
        done = await process.run(
            python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=0.5, before_stop=holds_on
        )
    took = time.monotonic() - began

    assert done.timed_out
    assert await gone(int(done.output.strip()))
    assert took < 2.5
    assert caplog.text.count("was given up") == 1
    await asyncio.wait_for(over.wait(), timeout=20)


async def test_a_caller_that_is_cancelled_inside_what_it_does_is_no_cancellation_of_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    patient(monkeypatch)

    async def gives_up(exited: asyncio.Future[None]) -> None:
        raise asyncio.CancelledError

    with caplog.at_level(logging.ERROR, logger="columbina.process"):
        done = await process.run(
            python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=1.0, before_stop=gives_up
        )

    assert done.timed_out
    assert await gone(int(done.output.strip()))
    assert caplog.records == []


async def test_the_caller_has_as_long_as_it_is_given_and_no_longer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    patient(monkeypatch)
    monkeypatch.setattr(process, "_BEFORE_STOP_SECONDS", 1.5)
    took: list[float] = []

    async def takes_a_second(exited: asyncio.Future[None]) -> None:
        began = time.monotonic()
        try:
            await asyncio.sleep(1.0)
        finally:
            took.append(time.monotonic() - began)

    await process.run(
        python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=0.5, before_stop=takes_a_second
    )

    assert len(took) == 1
    assert 0.9 < took[0] < 1.4


async def test_what_the_caller_is_given_is_not_what_the_run_waits_on(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    patient(monkeypatch)

    async def lets_go(exited: asyncio.Future[None]) -> None:
        exited.cancel()

    with caplog.at_level(logging.ERROR):
        done = await process.run(
            python(SAYS_WHO_AND_STAYS), cwd=tmp_path, timeout_seconds=1.0, before_stop=lets_go
        )
        await asyncio.sleep(0.1)

    assert done.timed_out
    assert await gone(int(done.output.strip()))
    # The end of the program was heard as usual, with nothing going wrong on the way.
    assert caplog.records == []


if sys.platform == "win32":
    import ctypes
    import functools
    import itertools
    from collections.abc import Callable, Iterator

    from columbina.descent import descended

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
            ("_kernel32", "GetProcessTimes", 0),
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

    # What a program started that is not in its job

    KERNEL32: Any = vars(process)["_kernel32"]
    """The library of the system as the module has it, whatever a test puts in its place."""

    PYTHON_ITSELF: str = getattr(sys, "_base_executable", sys.executable)
    """The interpreter, not the program of an environment that starts it as another process.

    That program keeps the interpreter in a job of its own, and a process of one job cannot
    be put into another beside it. With the interpreter itself, the process that a test
    starts is the one that says who it is, and each makes the next with its own hands.
    """

    A_FAMILY = (
        "import os, subprocess, sys, time\n"
        "with open(sys.argv[1], 'w') as note:\n"
        "    print(os.getpid(), file=note)\n"
        "if len(sys.argv) > 2:\n"
        "    itself = getattr(sys, '_base_executable', sys.executable)\n"
        "    subprocess.Popen([itself, __file__, *sys.argv[2:]])\n"
        "time.sleep(60)\n"
    )
    """Says which process it is, starts itself once more for each note that is left, and stays."""

    class Family:
        """Programs that were started the ordinary way, each by the one before it.

        A program that is put into a job takes nothing along that it started before. So what
        the first of these started is where a program is that the system started outside the
        job of whoever asked for it: it descends from a process of the job, and the job does
        not hold it.
        """

        def __init__(self, directory: Path, size: int) -> None:
            directory.mkdir()
            script = directory / "family.py"
            script.write_text(A_FAMILY, encoding="utf-8")
            self._notes = [directory / f"member-{index}" for index in range(size)]
            self._open: dict[int, int] = {}
            self._first = subprocess.Popen([PYTHON_ITSELF, str(script), *map(str, self._notes)])
            self.members: list[int] = []

        async def there(self) -> list[int]:
            """Wait until every one of them runs, and return which processes they are."""
            for note in self._notes:
                member = await told(note)
                # Kept open, so that its number names nobody else while the test goes on.
                kept = KERNEL32.OpenProcess(0x1001, False, member)
                assert kept, f"the process {member} cannot be opened"
                self._open[member] = kept
                self.members.append(member)
            assert self.members[0] == self._first.pid
            return self.members

        async def lose(self, member: int) -> None:
            """End one of them, and wait until the machine no longer lists it."""
            KERNEL32.TerminateProcess(self._open[member], 1)
            await unlisted(member)

        def end(self) -> None:
            """End whatever is left of them."""
            for kept in self._open.values():
                KERNEL32.TerminateProcess(kept, 1)
                KERNEL32.CloseHandle(kept)
            self._first.kill()
            self._first.wait()

    async def unlisted(*processes: int) -> None:
        """Wait until the machine lists none of some processes any more."""
        listed: Callable[[], dict[int, int]] = vars(process)["_parents"]
        deadline = time.monotonic() + 20
        while set(processes) & set(listed()):
            assert time.monotonic() < deadline, "a process that was ended is still listed"
            await asyncio.sleep(0.02)

    @pytest.fixture
    def families(tmp_path: Path) -> Iterator[Callable[[int], Family]]:
        """Starts families, and ends at the end of the test what is left of them."""
        started: list[Family] = []

        def one(size: int) -> Family:
            started.append(Family(tmp_path / f"family-{len(started)}", size))
            return started[-1]

        yield one
        for family in started:
            family.end()

    @pytest.fixture
    def jobs() -> Iterator[Callable[[int], Any]]:
        """Puts running programs into jobs, and gives the jobs up at the end of the test."""
        made: list[Any] = []

        def one(pid: int) -> Any:
            made.append(vars(process)["_Tree"](pid))
            return made[-1]

        yield one
        for tree in made:
            tree.close()

    @pytest.fixture
    def bystander() -> Iterator[int]:
        """A program that runs beside a test and descends from nothing that the test holds."""
        beside = subprocess.Popen(python(SLEEPER))
        yield beside.pid
        beside.kill()
        beside.wait()

    def joined(tree: Any, pid: int) -> None:
        """Put one more process into the job of a program, as if the program had started it."""
        reached = KERNEL32.OpenProcess(0x0101, False, pid)
        assert reached, f"the process {pid} cannot be opened"
        try:
            assert KERNEL32.AssignProcessToJobObject(tree._job, reached)
        finally:
            KERNEL32.CloseHandle(reached)

    class Instead:
        """A library of the system, of which one function is another."""

        def __init__(self, real: Any, function: str, other: Callable[..., Any]) -> None:
            self._real = real
            self._function = function
            self._other = other

        def __getattr__(self, name: str) -> Any:
            return self._other if name == self._function else getattr(self._real, name)

    def later(monkeypatch: pytest.MonkeyPatch, maker: int) -> None:
        """Make the first look at the machine miss everything that descends from a process.

        As if that process had started it a moment after the look.
        """
        look: Callable[[], dict[int, int]] = vars(process)["_parents"]
        looked: list[int] = []
        younger = itertools.count(1)

        def missing() -> dict[int, int]:
            seen = look()
            looked.append(len(seen))
            if len(looked) == 1:
                for late in descended({maker: 0}, seen, lambda _: next(younger)):
                    del seen[late]
            return seen

        monkeypatch.setattr(process, "_parents", missing)

    async def test_what_a_program_started_outside_its_job_is_ended_with_it(
        families: Callable[[int], Family], jobs: Callable[[int], Any], bystander: int
    ) -> None:
        family = families(3)
        members = await family.there()

        jobs(members[0]).stop()

        for member in members:
            assert await gone(member)
        # What does not descend from the program is nothing of it.
        assert alive(bystander)

    async def test_a_program_that_has_ended_still_counts_as_the_maker_of_what_it_left(
        families: Callable[[int], Family], jobs: Callable[[int], Any]
    ) -> None:
        # So it is when a command ends by itself and something that it started goes on.
        family = families(3)
        members = await family.there()
        tree = jobs(members[0])
        await family.lose(members[0])

        tree.stop()

        for member in members:
            assert await gone(member)

    @pytest.mark.parametrize("room", [64, 1])
    async def test_what_any_process_of_the_job_started_outside_it_is_ended(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
        room: int,
    ) -> None:
        # Nothing leads from the program to a process of its job whose maker has ended. Here
        # such a process is one that is put into the job by hand. With room for one process,
        # the system has to be asked a second time for who is in the job.
        monkeypatch.setattr(process, "_ROOM", room)
        own, other = families(1), families(2)
        mine = await own.there()
        members = await other.there()
        tree = jobs(mine[0])
        joined(tree, members[0])

        tree.stop()

        for member in members:
            assert await gone(member)

    async def test_what_left_the_job_is_looked_for_before_the_job_is_ended(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        # One is found through the processes that made it, and ending the job ends those.
        # Here the job is not said to be ended before the machine has stopped listing its
        # processes, so whoever looks only then finds nothing that leads to the last one.
        family = families(3)
        members = await family.there()
        tree = jobs(members[0])
        joined(tree, members[1])
        listed: Callable[[], dict[int, int]] = vars(process)["_parents"]

        def ending(job: int, code: int) -> int:
            ended = KERNEL32.TerminateJobObject(job, code)
            deadline = time.monotonic() + 20
            while {members[0], members[1]} & set(listed()) and time.monotonic() < deadline:
                time.sleep(0.02)
            return ended

        monkeypatch.setattr(process, "_kernel32", Instead(KERNEL32, "TerminateJobObject", ending))

        tree.stop()

        assert await gone(members[2])

    async def test_a_listed_number_that_names_no_process_of_the_job_is_not_taken(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        # A process that was listed may be gone when its number is opened, and the number
        # may name another by then. Here the system says of everyone that it is not in the
        # job, so only the program itself counts, which is kept open from the start.
        own, other = families(2), families(2)
        mine = await own.there()
        members = await other.there()
        tree = jobs(mine[0])
        joined(tree, members[0])
        monkeypatch.setattr(process, "_kernel32", Instead(KERNEL32, "IsProcessInJob", lambda *_: 1))

        tree.stop()

        for member in mine:
            assert await gone(member)
        assert alive(members[1])

    async def test_a_job_that_does_not_say_who_is_in_it_still_has_its_program(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        own, other = families(2), families(2)
        mine = await own.there()
        members = await other.there()
        tree = jobs(mine[0])
        joined(tree, members[0])
        failing = Failing(KERNEL32, "QueryInformationJobObject", 0)
        monkeypatch.setattr(process, "_kernel32", failing)

        tree.stop()

        for member in mine:
            assert await gone(member)
        assert alive(members[1])

    @pytest.mark.parametrize(
        ("function", "result"),
        [
            ("OpenProcess", 0),
            ("GetProcessTimes", 0),
            ("CreateToolhelp32Snapshot", ctypes.c_void_p(-1).value),
        ],
    )
    async def test_what_cannot_be_looked_at_is_left_alone(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
        function: str,
        result: int,
    ) -> None:
        family = families(2)
        members = await family.there()
        tree = jobs(members[0])
        monkeypatch.setattr(process, "_kernel32", Failing(KERNEL32, function, result))

        tree.stop()

        # Nobody is ended on a guess, and stopping does not fail over it.
        assert alive(members[1])

    async def test_what_was_started_while_the_first_were_ended_is_found_by_another_look(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        family = families(3)
        members = await family.there()
        later(monkeypatch, members[1])

        jobs(members[0]).stop()

        for member in members:
            assert await gone(member)

    async def test_it_is_not_looked_without_end_for_what_keeps_being_started(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.setattr(process, "_SWEEPS", 1)
        family = families(3)
        members = await family.there()
        later(monkeypatch, members[1])

        jobs(members[0]).stop()

        assert await gone(members[1])
        assert alive(members[2])

    class Counting:
        """The library of the system, which writes down what it opens and what is closed."""

        def __init__(self, real: Any, fails: str | None = None) -> None:
            self._real = real
            self._fails = fails
            self.open: set[int] = set()

        def __getattr__(self, name: str) -> Any:
            function: Callable[..., Any] = getattr(self._real, name)
            if name == self._fails:
                return self._failed
            if name in ("OpenProcess", "CreateToolhelp32Snapshot"):
                return functools.partial(self._opened, function)
            if name == "CloseHandle":
                return functools.partial(self._closed, function)
            return function

        def _failed(self, *arguments: object) -> int:
            return 0

        def _opened(self, opens: Callable[..., int | None], *arguments: object) -> int | None:
            handle = opens(*arguments)
            if handle:
                self.open.add(handle)
            return handle

        def _closed(self, closes: Callable[[int], int], handle: int) -> int:
            self.open.discard(handle)
            return closes(handle)

    @pytest.mark.parametrize("fails", [None, "GetProcessTimes"])
    async def test_nothing_that_was_opened_to_stop_a_program_stays_open(
        families: Callable[[int], Family],
        jobs: Callable[[int], Any],
        monkeypatch: pytest.MonkeyPatch,
        fails: str | None,
    ) -> None:
        # A process that is kept open is kept by the system although it has ended. The
        # second time, no process says when it was made, so each is let go at once.
        own, other = families(2), families(2)
        mine = await own.there()
        members = await other.there()
        tree = jobs(mine[0])
        joined(tree, members[0])
        counting = Counting(KERNEL32, fails)
        monkeypatch.setattr(process, "_kernel32", counting)

        tree.stop()

        assert counting.open == set()

    def test_a_program_is_held_with_the_time_it_was_made(
        jobs: Callable[[int], Any], bystander: int
    ) -> None:
        # The system counts time in tenths of a microsecond since the year 1601. A program
        # that was started within this test was made within a minute of now.
        now = time.time_ns() // 100 + 116_444_736_000_000_000
        minute = 60 * 10_000_000

        tree = jobs(bystander)

        assert now - minute < tree._made < now + minute


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
