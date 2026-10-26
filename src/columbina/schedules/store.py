"""The tasks that run at set times: what they are, when each is due, and the file that holds them.

``schedules.json`` in the home of the program holds the tasks. Each runs a prompt in a working
directory, every so many minutes, every hour at a minute, every day at a time, or every week on a
day at a time, by the clock of this machine. A task that was due while nothing ran it is run once
when something runs again, or passed over, as the task says. :func:`due` says when that is.

The file is written whole, and read again before every change, so that the program that runs the
tasks and the one that changes them do not undo each other.
"""

import datetime
import json
import secrets
from collections.abc import Callable
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError, model_validator

from columbina import files

__all__ = [
    "FILE",
    "GRACE_MS",
    "Schedule",
    "Schedules",
    "SchedulesError",
    "changed",
    "due",
    "read",
    "slot_before",
]

FILE = "schedules.json"
"""The file of the tasks, in the home of the program."""

GRACE_MS = 30 * 60 * 1000
"""How late a task whose time was missed may run where it says to pass over what was missed."""

Every = Literal["minutes", "hour", "day", "week"]
_WEEKDAYS = ("mon", "tue", "wed", "thu", "fri", "sat", "sun")


class SchedulesError(Exception):
    """The file of the tasks cannot be read or written, or a task cannot be, and why."""


class Schedule(BaseModel):
    """A task that runs at set times.

    Attributes:
        id: What it is known by.
        name: What the user calls it.
        cwd: The working directory that it runs in.
        prompt: What the agent is asked.
        every: How often: ``minutes``, ``hour``, ``day`` or ``week``.
        minutes: Every how many minutes, for ``minutes``.
        at: The time, ``HH:MM``, for ``day`` and ``week``, or ``:MM`` for ``hour``.
        weekday: The day, ``mon`` to ``sun``, for ``week``.
        missed: Whether a time that was missed is made up for once, ``run``, or passed over.
        enabled: Whether it runs at all.
        created: When it was made, in milliseconds since 1970.
        last: When it last ran, or was passed over, in milliseconds since 1970.
        last_session: The session of its last run.
        last_end: How its last run ended: how the turn ended, or what kept it from running.
    """

    model_config = ConfigDict(extra="forbid")

    id: str = Field(default_factory=lambda: secrets.token_hex(4), pattern=r"^[0-9a-z-]{1,40}$")
    name: str = Field(min_length=1, max_length=80)
    cwd: Path
    prompt: str = Field(min_length=1)
    every: Every
    minutes: int | None = Field(default=None, ge=5)
    at: str | None = Field(default=None, pattern=r"^(([01][0-9]|2[0-3]))?:[0-5][0-9]$")
    weekday: Literal["mon", "tue", "wed", "thu", "fri", "sat", "sun"] | None = None
    missed: Literal["run", "skip"] = "run"
    enabled: bool = True
    created: int = 0
    last: int | None = None
    last_session: str | None = None
    last_end: str | None = None

    @model_validator(mode="after")
    def _when_is_said(self) -> "Schedule":
        if self.every == "minutes" and self.minutes is None:
            raise ValueError("a task every so many minutes says how many, 5 at least")
        if self.every == "hour" and (self.at is None or not self.at.startswith(":")):
            raise ValueError("a task every hour says at which minute, as :MM")
        if self.every in ("day", "week") and (self.at is None or self.at.startswith(":")):
            raise ValueError("a task every day or week says at what time, as HH:MM")
        if self.every == "week" and self.weekday is None:
            raise ValueError("a task every week says on which day, mon to sun")
        return self


class Schedules(BaseModel):
    """What the file holds."""

    model_config = ConfigDict(extra="forbid")

    version: Literal[1] = 1
    schedules: list[Schedule] = Field(default_factory=list[Schedule])


def read(home: Path) -> Schedules:
    """Return the tasks, none where there is no file.

    Raises:
        SchedulesError: The file is there and cannot be read or is not what this program writes.
    """
    path = home / FILE
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return Schedules()
    except (OSError, UnicodeDecodeError) as error:
        raise SchedulesError(f"{path} cannot be read: {error}") from None
    try:
        return Schedules.model_validate(json.loads(text))
    except (ValueError, ValidationError) as error:
        raise SchedulesError(f"{path} is not what this program writes: {error}") from None


def changed(home: Path, change: Callable[[Schedules], Schedules]) -> Schedules:
    """Read the tasks, change them, write them down, and return them as they are now.

    Raises:
        SchedulesError: The file cannot be read or written.
    """
    after = change(read(home))
    try:
        files.replaced(home / FILE, after.model_dump_json(indent=2) + "\n")
    except OSError as error:
        raise SchedulesError(f"{home / FILE} cannot be written: {error}") from None
    return after


def _local(moment_ms: int) -> datetime.datetime:
    return datetime.datetime.fromtimestamp(moment_ms / 1000).astimezone()


def slot_before(schedule: Schedule, now_ms: int) -> int:
    """Return the last time at or before now at which a task was to run, in milliseconds."""
    now = _local(now_ms)
    if schedule.every == "minutes":
        step = (schedule.minutes or 0) * 60_000
        since = schedule.last if schedule.last is not None else schedule.created
        return since + step * max((now_ms - since) // step, 0)
    assert schedule.at is not None
    hour, minute = schedule.at.split(":")
    if schedule.every == "hour":
        slot = now.replace(minute=int(minute), second=0, microsecond=0)
        if slot > now:
            slot -= datetime.timedelta(hours=1)
    else:
        slot = now.replace(hour=int(hour), minute=int(minute), second=0, microsecond=0)
        if schedule.every == "week":
            assert schedule.weekday is not None
            slot -= datetime.timedelta(
                days=(slot.weekday() - _WEEKDAYS.index(schedule.weekday)) % 7
            )
            if slot > now:
                slot -= datetime.timedelta(days=7)
        elif slot > now:
            slot -= datetime.timedelta(days=1)
    return round(slot.timestamp() * 1000)


def due(schedule: Schedule, now_ms: int) -> Literal["run", "skip"] | None:
    """Say whether a task is to run now, to be passed over, or neither.

    A task runs at its time and not before it was made. One whose time came while nothing ran it
    is made up for once, or passed over where it says so and its time is more than
    :data:`GRACE_MS` ago.
    """
    if not schedule.enabled:
        return None
    slot = slot_before(schedule, now_ms)
    since = schedule.last if schedule.last is not None else schedule.created
    if slot <= since:
        return None
    if schedule.missed == "skip" and now_ms - slot > GRACE_MS:
        return "skip"
    return "run"
