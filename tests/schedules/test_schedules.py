"""Tasks that run at set times."""

import asyncio
import datetime
import logging
from pathlib import Path
from typing import Any

import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina import schedules
from columbina.agent import AgentError, Agents
from columbina.kernel import Context
from columbina.llm.adapters.scripted import Reply
from columbina.schedules import SCHEDULE_DONE, Done, Runner
from columbina.schedules.store import (
    FILE,
    GRACE_MS,
    Schedule,
    Schedules,
    SchedulesError,
    changed,
    due,
    read,
    slot_before,
)
from support.loop import MODEL, Harness


def at(year: int, month: int, day: int, hour: int, minute: int) -> int:
    """A moment by the clock of this machine, in milliseconds."""
    moment = datetime.datetime(year, month, day, hour, minute).astimezone()
    return round(moment.timestamp() * 1000)


MONDAY_9 = at(2026, 10, 26, 9, 0)
HOUR = 60 * 60 * 1000


def task(**changes: Any) -> Schedule:
    fields: dict[str, Any] = {
        "name": "news",
        "cwd": "/work",
        "prompt": "Sum up the news.",
        "every": "day",
        "at": "09:00",
        "created": MONDAY_9 - 3 * 24 * HOUR,
    }
    return Schedule.model_validate({**fields, **changes})


# When a task is due


def test_a_task_of_every_day_is_due_at_its_time_once() -> None:
    daily = task(last=MONDAY_9 - 24 * HOUR)

    assert due(daily, MONDAY_9 - 60_000) is None
    assert due(daily, MONDAY_9) == "run"
    assert due(daily.model_copy(update={"last": MONDAY_9}), MONDAY_9 + HOUR) is None


def test_a_task_is_not_due_for_a_time_before_it_was_made() -> None:
    assert due(task(created=MONDAY_9 + 1000), MONDAY_9 + 2 * HOUR) is None


def test_a_missed_time_is_made_up_for_or_passed_over() -> None:
    late = MONDAY_9 + GRACE_MS + 60_000
    made_up = task(last=MONDAY_9 - 24 * HOUR)
    passed = task(last=MONDAY_9 - 24 * HOUR, missed="skip")

    assert due(made_up, late) == "run"
    assert due(passed, late) == "skip"
    assert due(passed, MONDAY_9 + 60_000) == "run"


def test_a_task_that_is_disabled_is_never_due() -> None:
    assert due(task(enabled=False), MONDAY_9) is None


def test_a_task_of_every_hour_is_due_at_its_minute() -> None:
    hourly = task(every="hour", at=":15", last=at(2026, 10, 26, 8, 15))

    assert slot_before(hourly, at(2026, 10, 26, 9, 10)) == at(2026, 10, 26, 8, 15)
    assert due(hourly, at(2026, 10, 26, 9, 10)) is None
    assert due(hourly, at(2026, 10, 26, 9, 15)) == "run"


def test_a_task_of_every_week_is_due_on_its_day() -> None:
    weekly = task(every="week", weekday="wed", at="18:30", last=at(2026, 10, 21, 18, 30))

    assert slot_before(weekly, at(2026, 10, 28, 18, 0)) == at(2026, 10, 21, 18, 30)
    assert due(weekly, at(2026, 10, 28, 18, 30)) == "run"
    assert slot_before(weekly, at(2026, 10, 30, 9, 0)) == at(2026, 10, 28, 18, 30)


def test_a_task_of_every_few_minutes_is_due_after_them() -> None:
    often = task(every="minutes", minutes=30, at=None, last=MONDAY_9)

    assert due(often, MONDAY_9 + 29 * 60_000) is None
    assert due(often, MONDAY_9 + 30 * 60_000) == "run"
    assert due(task(every="minutes", minutes=30, at=None), MONDAY_9) == "run"


@pytest.mark.parametrize(
    ("changes", "said"),
    [
        ({"every": "minutes", "at": None}, "how many, 5 at least"),
        ({"every": "hour", "at": "09:00"}, "at which minute"),
        ({"every": "day", "at": ":30"}, "at what time"),
        ({"every": "week"}, "on which day"),
    ],
)
def test_a_task_says_when_it_runs(changes: dict[str, Any], said: str) -> None:
    with pytest.raises(ValueError, match=said):
        task(**changes)


# The file


def test_the_tasks_are_kept_in_a_file(tmp_path: Path) -> None:
    assert read(tmp_path) == Schedules()

    def add(schedules: Schedules) -> Schedules:
        schedules.schedules.append(task(id="t1"))
        return schedules

    changed(tmp_path, add)

    assert [one.id for one in read(tmp_path).schedules] == ["t1"]


@pytest.mark.parametrize(
    ("content", "said"), [(b"{", "is not what this program writes"), (b"\xff", "cannot be read")]
)
def test_a_file_that_cannot_be_read_is_said(tmp_path: Path, content: bytes, said: str) -> None:
    (tmp_path / FILE).write_bytes(content)

    with pytest.raises(SchedulesError, match=said):
        read(tmp_path)


def test_a_file_that_cannot_be_written_is_said(tmp_path: Path) -> None:
    (tmp_path / "not a directory").write_text("", encoding="utf-8")

    with pytest.raises(SchedulesError, match="cannot be written"):
        changed(tmp_path / "not a directory", lambda schedules: schedules)


# Running


async def runner(
    harness: Harness, home: Path, *kept: Schedule, permissions: bool = True
) -> tuple[Runner, list[Done]]:
    if permissions:
        await harness.ctx.plugin(columbina.fs)
        await harness.ctx.plugin(columbina.sandbox)
        await harness.ctx.plugin(columbina.approval)
        await harness.ctx.plugin(columbina.permissions)
    changed(home, lambda schedules: Schedules(schedules=list(kept)))
    done: list[Done] = []
    harness.ctx.on(SCHEDULE_DONE, done.append)
    made = Runner(
        harness.ctx,
        schedules.Config(home=home, model=MODEL),
        harness.agents,
    )
    made.clock = lambda: MONDAY_9 + 60_000
    return made, done


async def test_a_task_that_is_due_runs_in_a_session_of_its_own(
    harness: Harness, tmp_path: Path
) -> None:
    harness.model.add(Reply(text="All quiet."))
    made, done = await runner(
        harness, tmp_path, task(id="t0", enabled=False), task(id="t1", cwd=harness.cwd)
    )

    await made.look()
    await made.look()
    await asyncio.wait_for(made.settled(), 10)

    (only,) = done
    assert (only.id, only.name, only.end) == ("t1", "news", "completed")
    kept = read(tmp_path).schedules[1]
    assert kept.last == MONDAY_9 + 60_000
    assert kept.last_session == only.session_id
    assert kept.last_end == "completed"
    assert len(harness.model.requests) == 1


async def test_a_task_whose_time_was_missed_is_passed_over(
    harness: Harness, tmp_path: Path
) -> None:
    made, done = await runner(harness, tmp_path, task(id="t1", missed="skip"))
    made.clock = lambda: MONDAY_9 + GRACE_MS + 60_000

    await made.look()

    assert done == []
    assert read(tmp_path).schedules[0].last_end == "passed over: its time was missed"


async def test_a_task_whose_model_fails_ends_as_its_turn_did(
    harness: Harness, tmp_path: Path
) -> None:
    made, done = await runner(harness, tmp_path, task(id="t1", cwd=harness.cwd), permissions=False)
    bad = schedules.Config(home=tmp_path, model=MODEL.model_copy(update={"provider": "nowhere"}))
    made._config = bad  # pyright: ignore[reportPrivateUsage]

    await made.look()
    await asyncio.wait_for(made.settled(), 10)

    assert done[0].end == "error"


async def test_a_task_that_cannot_run_says_why(
    harness: Harness, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    made, done = await runner(harness, tmp_path, task(id="t1", cwd=harness.cwd))

    async def refused(*args: object, **more: object) -> object:
        raise AgentError("no room")

    monkeypatch.setattr(Agents, "create", refused)

    await made.look()
    await asyncio.wait_for(made.settled(), 10)

    assert done[0].end == "it could not run: no room"
    assert done[0].session_id is None


async def test_a_task_that_is_stopped_with_the_program_says_so(
    harness: Harness, tmp_path: Path
) -> None:
    harness.model.add(Reply(text="slow", hold=True))
    made, done = await runner(harness, tmp_path, task(id="t1", cwd=harness.cwd))

    await made.look()
    await asyncio.wait_for(harness.model.held(), 5)
    await made.stop()

    assert done[0].end == "stopped: the program ended"


async def test_tasks_that_cannot_be_read_are_not_run(
    harness: Harness, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    made, _ = await runner(harness, tmp_path)
    (tmp_path / FILE).write_text("{", encoding="utf-8")

    with caplog.at_level(logging.WARNING, "columbina.schedules"):
        await made.look()

    assert "the tasks cannot be looked at" in caplog.text


async def test_what_cannot_be_written_down_is_said(
    harness: Harness,
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    made, _ = await runner(harness, tmp_path, task(id="t1", missed="skip"))
    made.clock = lambda: MONDAY_9 + GRACE_MS + 60_000

    def refused(home: Path, change: object) -> Schedules:
        raise SchedulesError("the disk is full")

    monkeypatch.setattr(schedules, "changed", refused)
    with caplog.at_level(logging.WARNING, "columbina.schedules"):
        await made.look()

    assert "cannot be written down: the disk is full" in caplog.text


async def test_the_plugin_looks_until_the_program_ends(ctx: Context, tmp_path: Path) -> None:
    import columbina.agent
    import columbina.session

    await ctx.plugin(columbina.session, {"root": str(tmp_path / "sessions")})
    await ctx.plugin(columbina.agent)
    await ctx.plugin(
        schedules, {"home": str(tmp_path), "model": MODEL.model_dump(), "seconds": 0.01}
    )
    await asyncio.sleep(0.05)

    await ctx.fiber.dispose()
