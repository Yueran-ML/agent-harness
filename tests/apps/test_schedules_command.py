"""``columbina schedules``: the tasks that run at set times, listed, added and changed."""

import datetime
import json
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.schedules.store import FILE, Schedule, Schedules, changed

MONDAY_9 = round(datetime.datetime(2026, 10, 26, 9, 0).astimezone().timestamp() * 1000)


def task(**changes: Any) -> Schedule:
    fields: dict[str, Any] = {
        "name": "news",
        "cwd": "/work",
        "prompt": "Sum up the news.",
        "every": "day",
        "at": "09:00",
    }
    return Schedule.model_validate({**fields, **changes})


def test_tasks_are_added_listed_and_changed_from_the_command_line(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    del home
    assert main(["schedules", "list"]) == 0
    assert "There are no tasks." in capsys.readouterr().out
    assert main(
        ["schedules", "add", "--name", "news", "--cwd", str(tmp_path), "--prompt", "Sum up.",
         "--every", "week", "--weekday", "mon", "--at", "09:00", "--json"]
    ) == 0  # fmt: skip
    made = json.loads(capsys.readouterr().out)
    assert (made["every"], made["weekday"], made["enabled"]) == ("week", "mon", True)
    key = made["id"]
    assert main(["schedules", "disable", key]) == 0
    assert capsys.readouterr().out == f"Disabled {key}.\n"
    assert main(["schedules", "list"]) == 0
    assert f"{key}  news: every mon at 09:00, disabled, in " in capsys.readouterr().out
    assert main(["schedules", "enable", key, "--json"]) == 0
    assert json.loads(capsys.readouterr().out) == {"id": key, "done": "enable"}
    assert main(["schedules", "remove", key]) == 0
    assert capsys.readouterr().out == f"Removed {key}.\n"
    assert main(["schedules", "list", "--json"]) == 0
    assert json.loads(capsys.readouterr().out)["schedules"] == []


def test_what_the_command_line_cannot_do_is_said(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["schedules", "remove", "none"]) == 2
    assert "there is no task none" in capsys.readouterr().out
    assert main(
        ["schedules", "add", "--name", "x", "--cwd", str(tmp_path), "--prompt", "p",
         "--every", "minutes", "--minutes", "1", "--json"]
    ) == 2  # fmt: skip
    assert "the task cannot be" in json.loads(capsys.readouterr().out)["error"]
    home.mkdir(parents=True, exist_ok=True)
    (home / FILE).write_text("{", encoding="utf-8")
    assert main(["schedules", "list"]) == 2


def test_a_task_that_ran_is_listed_with_how_it_ended(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    changed(
        home,
        lambda schedules: Schedules(
            schedules=[
                task(id="t1", every="hour", at=":05", last=MONDAY_9, last_end="completed"),
                task(id="t2", every="minutes", minutes=10, at=None),
            ]
        ),
    )

    assert main(["schedules", "list"]) == 0

    out = capsys.readouterr().out.splitlines()
    assert out[0].startswith("t1  news: every hour at :05, in ")
    assert out[1].endswith(": completed")
    assert out[2].startswith("t2  news: every 10 minutes, in ")


def test_a_task_is_added_for_a_person_and_others_are_left_as_they_are(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    del home
    changed_one = ["schedules", "add", "--name", "other", "--cwd", str(tmp_path)]
    assert main([*changed_one, "--prompt", "p", "--every", "hour", "--at", ":30", "--json"]) == 0
    other = json.loads(capsys.readouterr().out)["id"]
    daily = ["schedules", "add", "--name", "news", "--cwd", str(tmp_path), "--prompt", "Sum up."]

    assert main([*daily, "--every", "day", "--at", "08:00"]) == 0
    said = capsys.readouterr().out
    key = said.split()[1].rstrip(":")
    assert said == f"Added {key}: news, every day at 08:00.\n"
    assert main(["schedules", "disable", key]) == 0
    assert main(["schedules", "list", "--json"]) == 0
    kept = {
        one["id"]: one["enabled"]
        for one in json.loads(capsys.readouterr().out.splitlines()[-1])["schedules"]
    }
    assert kept == {other: True, key: False}
