"""Every task of the benchmark can be judged: its start fails, and its reference passes."""

from pathlib import Path

import pytest

from bench.check import TASKS, check
from bench.task import Loaded, tasks

ALL = tasks(TASKS)


def test_there_are_tasks_of_every_category() -> None:
    assert {loaded.task.category for loaded in ALL} == {"code", "files", "sql", "compute"}


@pytest.mark.parametrize("loaded", ALL, ids=[loaded.task.id for loaded in ALL])
def test_the_task_can_be_judged(loaded: Loaded, tmp_path: Path) -> None:
    assert check(loaded, tmp_path) == []
