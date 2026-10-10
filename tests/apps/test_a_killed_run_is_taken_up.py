"""Where stage 5 ends, the other half: a run that is killed can be taken up.

A real ``columbina run`` is started as a process of its own. Its model, which
is scripted, has it run a command that goes on for two minutes. Once the
command has begun, the process is killed the way the system kills: nothing in
it gets to tidy up. Then a second run takes up the session by its id.

The log of the first run stops in the middle of a turn. The second run mends
it before it goes on: the call that has no result gets one that says that
nobody knows what came of it, and the turn is closed as cut short. The model
is told what happened, and the new task is done.

What becomes of the command differs. On Windows it is held in a job, and the
job ends with the program. Elsewhere it is in a group of its own and goes on,
so the test ends it.
"""

import os
import subprocess
import sys
import time
from pathlib import Path

import pytest

from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from columbina.session import (
    EndSeedEvent,
    StepEndEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    UserMessageEvent,
)
from support.launch import profile, stored
from support.processes import alive, end

WAITS = """\
import os
import pathlib
import time

pathlib.Path("begun.txt").write_text(str(os.getpid()), encoding="utf-8")
time.sleep(120)
"""
COMMAND = "python waits.py"
ANSWER = "The command was cut short, so I do not know whether it did its work."


def begun(work: Path, process: "subprocess.Popen[bytes]") -> int:
    """Wait until the command of a run has begun, and return which process it is."""
    told = work / "begun.txt"
    deadline = time.monotonic() + 60
    while True:
        if told.exists() and (written := told.read_text(encoding="utf-8").strip()):
            return int(written)
        assert process.poll() is None, "the run ended before its command began"
        assert time.monotonic() < deadline, "the command did not begin in time"
        time.sleep(0.05)


def ended(pid: int, within: float) -> bool:
    deadline = time.monotonic() + within
    while alive(pid):
        if time.monotonic() > deadline:
            return False
        time.sleep(0.05)
    return True


@pytest.mark.timeout(150)
def test_a_run_that_is_killed_in_the_middle_of_a_command_is_taken_up(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (work / "waits.py").write_text(WAITS, encoding="utf-8")
    runs = {"calls": [{"name": shell.NAME, "arguments": {"command": COMMAND}}]}
    first = profile(tmp_path, runs, {"text": "This is never said."})
    where = ["--profile", first, "--cwd", str(work), "--session-id", "s-1"]
    allowed = ["--allow", f"run({COMMAND})"]
    process = subprocess.Popen(
        [sys.executable, "-m", "columbina", "run", "Wait for it.", *where, *allowed],
        env=os.environ.copy(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    command: int | None = None
    try:
        command = begun(work, process)

        process.kill()

        assert process.wait(timeout=30) != 0
        if sys.platform == "win32":
            # The job that held the command ended with the program.
            assert ended(command, within=10)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)
        if command is not None and alive(command):
            end(command)

    # The log stops where the program did: a call was begun, and nothing came after it.
    cut = stored(home, work).events
    assert isinstance(cut[-1].data, ToolCallEvent)
    assert not any(isinstance(event.data, TurnEndEvent) for event in cut)

    second = profile(tmp_path, {"text": ANSWER})
    again = ["--profile", second, "--cwd", str(work), "--session-id", "s-1"]
    code = main(["run", "What came of it?", *again])

    captured = capsys.readouterr()
    assert (code, captured.out, captured.err) == (0, f"{ANSWER}\n", "")
    events = stored(home, work).events
    assert [event.data for event in events[: len(cut)]] == [event.data for event in cut]
    # The log is mended before anything else is written: the call gets a result, and the
    # step and the turn are closed.
    seed, result, step, turn = (event.data for event in events[len(cut) : len(cut) + 4])
    assert seed == EndSeedEvent(inherited=False)
    assert isinstance(result, ToolResultEvent)
    assert result.message.call_id == cut[-1].data.call_id
    assert result.message.is_error
    assert result.error is not None
    assert result.error.code == "TOOL_OUTCOME_UNKNOWN"
    assert step == StepEndEvent(turn=1, step=1)
    assert turn == TurnEndEvent(turn=1, reason="interrupted")
    # The model of the second run is told what happened before it is given the new task.
    told = [
        (event.data.source, event.data.message.content[0].text)
        for event in events[len(cut) + 4 :]
        if isinstance(event.data, UserMessageEvent)
    ]
    sources = [source for source, _ in told]
    assert sources.index("context/interruption") < sources.index("user")
    said = dict(told)
    assert (
        "The program ended while the turn before this one was under way."
        in said["context/interruption"]
    )
    assert said["user"] == "What came of it?"
    assert events[-1].data == TurnEndEvent(turn=2, reason="completed")
