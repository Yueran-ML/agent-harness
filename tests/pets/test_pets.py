"""What a task is doing, written down for a desktop pet: ``columbina.pets``."""

import asyncio
import json
import logging
import os
import time
from dataclasses import dataclass
from pathlib import Path

import pytest
from pydantic import BaseModel

import columbina.pets
from columbina.approval import ApprovalAskedEvent, ApprovalDecidedEvent
from columbina.kernel import Context
from columbina.llm import TextBlock, ToolMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.pets import Config, Teller
from columbina.session import (
    SESSION_EVENT,
    Committed,
    EventData,
    SessionEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.tools import Tool, ToolRun
from support.loop import Harness, idle


@dataclass
class _Session:
    delegation_depth: int = 0


@dataclass
class _Committed:
    session: _Session
    event: SessionEvent[EventData]


def heard(teller: Teller, data: EventData, depth: int = 0) -> None:
    teller.heard(_Committed(_Session(depth), SessionEvent(seq=0, time=0, data=data)))  # type: ignore[arg-type]


def told(path: Path) -> tuple[str, str] | None:
    """The state and the label in the file, or nothing where there is no file."""
    if not path.exists():
        return None
    document = json.loads(path.read_text(encoding="utf-8"))
    assert document["version"] == 1
    assert document["pid"] == os.getpid()
    # Written a moment ago, in milliseconds.
    assert abs(document["at"] - time.time() * 1000) < 60_000
    return document["state"], document["label"]


RESULT = ToolResultEvent(message=ToolMessage(call_id="c-1", content=(TextBlock(text="ok"),)))


async def test_a_turn_is_told_as_it_goes_on(tmp_path: Path) -> None:
    file = tmp_path / "deeper" / "pet-state.json"
    teller = Teller(file, rest_after=60)
    seen: list[tuple[str, str] | None] = []
    for data in (
        TurnStartEvent(turn=1),
        ToolCallEvent(call_id="c-1", name="read", arguments="{}"),
        RESULT,
        ToolCallEvent(call_id="c-2", name="shell", arguments="{}"),
        ApprovalAskedEvent(call_id="c-2", tool="shell", summary="Run a command."),
        ApprovalDecidedEvent(call_id="c-2", outcome="allowed-once"),
        RESULT,
        TurnEndEvent(turn=1, reason="completed"),
    ):
        heard(teller, data)
        seen.append(told(file))
    assert seen == [
        ("thinking", ""),
        ("review", "read"),
        ("thinking", ""),
        ("review", "shell"),
        ("waiting", "shell"),
        ("review", "shell"),
        ("thinking", ""),
        ("done", ""),
    ]


async def test_a_call_that_is_not_let_through_is_thought_about_again(tmp_path: Path) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=60)
    heard(teller, ApprovalAskedEvent(call_id="c-1", tool="shell", summary="Run."))
    heard(teller, ApprovalDecidedEvent(call_id="c-1", outcome="rejected"))
    assert told(file) == ("thinking", "")
    # A question that was never told of lets through a call without a name.
    heard(teller, ApprovalDecidedEvent(call_id="c-9", outcome="allowed-once"))
    assert told(file) == ("review", "")


@pytest.mark.parametrize(
    ("reason", "code", "state"),
    [
        ("error", "AUTH", ("failed", "AUTH")),
        ("blocked", "MAX_STEPS", ("failed", "MAX_STEPS")),
        ("max-tokens", None, ("failed", "")),
        ("aborted", None, ("idle", "")),
        ("interrupted", None, ("idle", "")),
    ],
)
async def test_a_turn_ends_as_its_reason_says(
    tmp_path: Path, reason: str, code: str | None, state: tuple[str, str]
) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=60)
    heard(teller, TurnStartEvent(turn=1))
    heard(teller, ApprovalAskedEvent(call_id="c-1", tool="shell", summary="Run."))
    heard(teller, TurnEndEvent(turn=1, reason=reason, code=code))  # type: ignore[arg-type]
    assert told(file) == state
    # The questions of the turn are over with it.
    heard(teller, ApprovalDecidedEvent(call_id="c-1", outcome="allowed-once"))
    assert told(file) == ("review", "")


async def test_a_turn_that_has_ended_rests_a_while_later(tmp_path: Path) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=0.01)
    heard(teller, TurnEndEvent(turn=1, reason="completed"))
    assert told(file) == ("done", "")
    await asyncio.sleep(0.05)
    assert told(file) == ("idle", "")
    # A new turn before the rest takes its place.
    heard(teller, TurnEndEvent(turn=2, reason="error", code="X"))
    heard(teller, TurnStartEvent(turn=3))
    await asyncio.sleep(0.05)
    assert told(file) == ("thinking", "")


async def test_what_a_subagent_does_is_not_told(tmp_path: Path) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=60)
    heard(teller, TurnStartEvent(turn=1), depth=1)
    assert told(file) is None


async def test_a_state_is_written_when_it_changes_alone(tmp_path: Path) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=60)
    heard(teller, ToolCallEvent(call_id="c-1", name="read", arguments="{}"))
    file.unlink()
    heard(teller, ToolCallEvent(call_id="c-2", name="read", arguments="{}"))
    assert told(file) is None
    heard(teller, ToolCallEvent(call_id="c-3", name="grep", arguments="{}"))
    assert told(file) == ("review", "grep")


async def test_a_label_is_one_short_line(tmp_path: Path) -> None:
    file = tmp_path / "pet-state.json"
    teller = Teller(file, rest_after=60)
    heard(teller, ToolCallEvent(call_id="c-1", name="a\nb" + "x" * 100, arguments="{}"))
    assert told(file) == ("review", ("a b" + "x" * 100)[:80])


async def test_a_file_that_cannot_be_written_is_let_go(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    (tmp_path / "a file").write_text("in the way", encoding="utf-8")
    teller = Teller(tmp_path / "a file" / "pet-state.json", rest_after=60)
    with caplog.at_level(logging.WARNING, logger="columbina.pets"):
        heard(teller, TurnStartEvent(turn=1))
    assert "the state for the pets cannot be written" in caplog.text


def test_a_variable_that_names_no_file_names_nowhere() -> None:
    assert Config.model_validate({"file": ""}).file is None
    assert Config.model_validate({}).file is None
    assert Config.model_validate({"file": "C:/x/pet-state.json"}).file == Path(
        "C:/x/pet-state.json"
    )
    with pytest.raises(ValueError, match="extra"):
        Config.model_validate({"file": "", "more": 1})


class _Nothing(BaseModel):
    pass


async def test_the_plugin_tells_the_conversations_of_the_program_and_rests_at_its_end(
    ctx: Context, harness: Harness, tmp_path: Path
) -> None:
    file = tmp_path / "pet-state.json"

    async def look(args: _Nothing, run: ToolRun) -> str:
        return "seen"

    harness.tools.add(
        harness.ctx, Tool(name="look", description="Look.", parameters=_Nothing, execute=look)
    )
    fiber = await harness.ctx.plugin(columbina.pets, {"file": str(file)})
    states: list[tuple[str, str] | None] = []

    def watching(committed: Committed) -> None:
        # The plugin heard the event first, and wrote what it changed.
        now = told(file)
        if now is not None and (not states or states[-1] != now):
            states.append(now)

    harness.ctx.on(SESSION_EVENT, watching)
    agent = await harness.agent(Reply(calls=(Call(name="look"),)), Reply(text="Seen."))
    agent.followup("Look.")
    await idle(agent)
    assert states == [("thinking", ""), ("review", "look"), ("thinking", ""), ("done", "")]
    await fiber.dispose()
    assert told(file) == ("idle", "")


async def test_the_plugin_without_a_file_writes_nothing(harness: Harness, tmp_path: Path) -> None:
    await harness.ctx.plugin(columbina.pets, {"file": ""})
    agent = await harness.agent(Reply(text="Hi."))
    agent.followup("Hello.")
    await idle(agent)
    assert list(tmp_path.glob("**/pet-state.json")) == []
