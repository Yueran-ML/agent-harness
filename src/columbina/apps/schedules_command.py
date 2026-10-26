"""What ``columbina schedules`` does: the tasks that run at set times, listed, added and changed.

``list`` says what tasks there are, when each runs and how it last ended. ``add`` adds one,
``remove`` takes one away, and ``enable`` and ``disable`` have one run or not. With ``--json``
what is said is a document of JSON, for another program. The tasks run while the desktop
application is open, see :mod:`columbina.schedules`.
"""

import time
from collections.abc import Callable, Mapping
from pathlib import Path

from pydantic import ValidationError

from columbina.schedules.store import Schedule, Schedules, SchedulesError, changed, read

__all__ = ["ADD", "DISABLE", "ENABLE", "LIST", "REMOVE", "SCHEDULES", "run", "when"]

SCHEDULES = "schedules"
LIST = "list"
ADD = "add"
REMOVE = "remove"
ENABLE = "enable"
DISABLE = "disable"


def when(schedule: Schedule) -> str:
    """Say in a few words when a task runs."""
    if schedule.every == "minutes":
        return f"every {schedule.minutes} minutes"
    if schedule.every == "hour":
        return f"every hour at {schedule.at}"
    if schedule.every == "day":
        return f"every day at {schedule.at}"
    return f"every {schedule.weekday} at {schedule.at}"


def _listed(schedules: Schedules) -> str:
    if not schedules.schedules:
        return "There are no tasks. 'columbina schedules add' adds one."
    lines: list[str] = []
    for one in schedules.schedules:
        state = "" if one.enabled else ", disabled"
        lines.append(f"{one.id}  {one.name}: {when(one)}{state}, in {one.cwd}")
        if one.last is not None:
            at = time.strftime("%Y-%m-%d %H:%M", time.localtime(one.last / 1000))
            lines.append(f"    last at {at}: {one.last_end}")
    return "\n".join(lines)


def run(
    what: str,
    given: Mapping[str, object],
    *,
    home: Path,
    as_json: bool,
    now_ms: int,
    say: Callable[[str], None],
    answer: Callable[[Mapping[str, object]], None],
) -> int:
    """Carry out the command, and return its exit code.

    Args:
        what: ``list``, ``add``, ``remove``, ``enable`` or ``disable``.
        given: What the command line gave: the fields of a task to add, or the ``id`` of one.
        home: The home of the program.
        as_json: Whether what is said is a document of JSON.
        now_ms: The time, for when a task is added.
        say: Says a line to a person.
        answer: Writes a document of JSON.
    """

    def failed(said: str) -> int:
        if as_json:
            answer({"error": said})
        else:
            say(f"columbina: {said}")
        return 2

    try:
        if what == LIST:
            schedules = read(home)
            if as_json:
                answer(schedules.model_dump(mode="json"))
            else:
                say(_listed(schedules))
            return 0
        if what == ADD:
            fields = {key: value for key, value in given.items() if value is not None}
            try:
                made = Schedule.model_validate({**fields, "created": now_ms})
            except ValidationError as error:
                return failed(f"the task cannot be: {error.errors()[0]['msg']}")

            def add(schedules: Schedules) -> Schedules:
                schedules.schedules.append(made)
                return schedules

            changed(home, add)
            if as_json:
                answer(made.model_dump(mode="json"))
            else:
                say(f"Added {made.id}: {made.name}, {when(made)}.")
            return 0
        key = str(given["id"])
        if all(one.id != key for one in read(home).schedules):
            return failed(f"there is no task {key}")

        def change(schedules: Schedules) -> Schedules:
            if what == REMOVE:
                schedules.schedules = [one for one in schedules.schedules if one.id != key]
            for one in schedules.schedules:
                if one.id == key:
                    one.enabled = what == ENABLE
            return schedules

        changed(home, change)
        if as_json:
            answer({"id": key, "done": what})
        else:
            say({REMOVE: "Removed", ENABLE: "Enabled", DISABLE: "Disabled"}[what] + f" {key}.")
        return 0
    except SchedulesError as error:
        return failed(str(error))
