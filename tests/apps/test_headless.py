"""The headless frontend: what happens around the one task that it runs."""

import asyncio
import subprocess
import sys

import pytest

import columbina
import columbina.apps.headless
from columbina.agent import PRE_STEP, Reject, StepDecision, StepInfo
from columbina.apps import FRONTEND
from columbina.kernel import Next
from columbina.llm.adapters.scripted import Reply
from columbina.session import TurnEndEvent
from columbina.session.store import SESSION_FILE
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
