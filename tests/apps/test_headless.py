"""The headless frontend: what happens around the one task that it runs."""

import asyncio
import io
import json
import subprocess
import sys

import pytest
from pydantic import BaseModel

import columbina
import columbina.apps.headless
from columbina.agent import PRE_STEP, Reject, StepDecision, StepInfo
from columbina.apps import FRONTEND
from columbina.kernel import Next
from columbina.llm import LlmFailure, TextBlock
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent, TurnEndEvent
from columbina.session.store import SESSION_FILE
from columbina.tools import Tool, ToolRun
from support.loop import MODEL, Harness


async def frontend(harness: Harness, **config: object) -> columbina.apps.Frontend:
    """Load the headless frontend on top of the loop of the harness."""
    await harness.ctx.plugin(
        columbina.apps.headless,
        {"model": MODEL.model_dump(), "cwd": harness.cwd, "task": "Do it.", **config},
    )
    return harness.ctx.use(FRONTEND)


async def test_a_run_that_is_interrupted_ends_its_turn_before_it_goes(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    runner = await frontend(harness, session_id="s-1")

    running = asyncio.ensure_future(runner.run())
    await asyncio.wait_for(harness.model.held(), timeout=5)
    running.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=5)

    stored = harness.sessions.read(harness.cwd, "s-1")
    assert stored.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert list(harness.agents.live) == []
    assert capsys.readouterr().out == ""


async def test_a_run_whose_log_cannot_be_written_says_so(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    harness.model.add(Reply(text="thinking", hold=True))
    runner = await frontend(harness, session_id="s-1")
    running = asyncio.ensure_future(runner.run())
    await asyncio.wait_for(harness.model.held(), timeout=5)
    monkeypatch.setattr("columbina.session.store.encode_event", full)

    harness.model.release()
    code = await asyncio.wait_for(running, timeout=5)
    monkeypatch.undo()

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == ""
    assert captured.err.startswith("columbina: the agent has stopped: ")
    assert "No space left on device" in captured.err


async def test_an_id_that_cannot_name_a_session_stops_the_run(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    runner = await frontend(harness, session_id="../escape")

    code = await runner.run()

    captured = capsys.readouterr()
    assert code == 1
    assert captured.err.startswith("columbina: '../escape' is not a usable session id")
    assert harness.model.requests == []


async def test_a_stored_session_that_cannot_be_read_stops_the_run(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    directory = harness.sessions.directory(harness.cwd, "s-1")
    directory.mkdir(parents=True)
    (directory / SESSION_FILE).write_text("not a session\n", encoding="utf-8")
    runner = await frontend(harness, session_id="s-1")

    code = await runner.run()

    captured = capsys.readouterr()
    assert code == 1
    assert "line 1: the first line is not a session header" in captured.err
    assert harness.model.requests == []


async def test_without_a_store_a_session_id_names_a_new_session(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.sessions.root = None
    harness.model.add(Reply(text="Done."))
    runner = await frontend(harness, session_id="s-1")

    code = await runner.run()

    assert code == 0
    assert capsys.readouterr().out == "Done.\n"


async def test_a_turn_that_was_blocked_is_reported_with_its_reason(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    async def spent(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        return Reject(reason="the budget is spent")

    harness.ctx.on(PRE_STEP, spent)
    runner = await frontend(harness)

    code = await runner.run()

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == ""
    assert captured.err == "columbina: the turn ended early: blocked (the budget is spent)\n"


# What is printed


ESC = chr(0x1B)
STAND_IN = chr(0xFFFD)
OVERRIDE = chr(0x202E)
CSI = chr(0x9B)
HOSTILE = f"Done.{ESC}[2J{ESC}]52;c;aGk={chr(7)} {OVERRIDE}txt.exe {CSI}2J"
SHOWN = f"Done.{STAND_IN}[2J{STAND_IN}]52;c;aGk={STAND_IN} {STAND_IN}txt.exe {STAND_IN}2J"


class Screen(io.StringIO):
    """A standard output that is a terminal."""

    def isatty(self) -> bool:
        return True


async def test_an_answer_that_goes_to_a_terminal_cannot_steer_it(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.model.add(Reply(text=HOSTILE))
    runner = await frontend(harness)
    screen = Screen()
    monkeypatch.setattr(sys, "stdout", screen)

    code = await runner.run()

    assert code == 0
    assert screen.getvalue() == SHOWN + "\n"


async def test_an_answer_that_goes_to_a_file_or_a_program_is_what_the_model_wrote(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text=HOSTILE))
    runner = await frontend(harness)

    code = await runner.run()

    assert code == 0
    assert capsys.readouterr().out == HOSTILE + "\n"


async def test_what_is_said_about_a_run_cannot_steer_a_terminal(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    failed = LlmFailure(message=f"no{ESC}[2Jthing", code="SERVER", status=500)
    harness.model.add(Reply(failure=failed))
    runner = await frontend(harness)

    code = await runner.run()

    captured = capsys.readouterr()
    assert code == 1
    assert ESC not in captured.err
    assert (
        captured.err == f"columbina: the turn ended with an error: SERVER: no{STAND_IN}[2Jthing\n"
    )


async def test_the_lines_of_json_write_such_characters_by_their_numbers(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    said = f"{chr(0x5B8C)}{chr(0x6210)} {OVERRIDE}txt.exe {CSI}2J{chr(0x7F)}"
    harness.model.add(Reply(text=said))
    runner = await frontend(harness, json_lines=True)

    code = await runner.run()

    lines = capsys.readouterr().out.splitlines()
    assert code == 0
    # Other languages stay readable. What a terminal would act on is written by its number.
    wanted = f'"text": "{chr(0x5B8C)}{chr(0x6210)} \\u202etxt.exe \\u009b2J\\u007f"'
    assert wanted in lines[-1]
    assert not any(mark in line for line in lines for mark in (OVERRIDE, CSI, chr(0x7F)))
    assert json.loads(lines[-1])["text"] == said


class NoArgs(BaseModel):
    pass


async def three_lines(args: NoArgs, run: ToolRun) -> str:
    return "one\ntwo\nthree"


async def test_a_result_that_is_replaced_later_is_reported_once(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.tools.add(
        harness.ctx,
        Tool(name="lines", description="Gives lines.", parameters=NoArgs, execute=three_lines),
    )
    harness.model.add(Reply(calls=(Call(name="lines"),)), Reply(text="Done."))

    async def shorten(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        if info.step == 2:
            # What makes a conversation shorter cuts a result down where it stands.
            session = info.agent.session
            whole = session.events[session.surface[-1]]
            assert isinstance(whole.data, ToolResultEvent)
            message = whole.data.message.model_copy(update={"content": (TextBlock(text="o"),)})
            shorter = whole.data.model_copy(update={"message": message})
            session.append(shorter, replaces=[whole.seq])
        return await proceed(info)

    harness.ctx.on(PRE_STEP, shorten)
    runner = await frontend(harness, json_lines=True)

    code = await runner.run()

    lines = [json.loads(line) for line in capsys.readouterr().out.splitlines()]
    assert code == 0
    assert [line["text"] for line in lines if line["type"] == "tool_result"] == ["one\ntwo\nthree"]


def test_the_package_can_be_run_as_a_module() -> None:
    done = subprocess.run(
        [sys.executable, "-m", "columbina", "--version"],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )

    assert done.returncode == 0
    assert done.stdout == f"columbina {columbina.__version__}\n"
    assert done.stderr == ""
