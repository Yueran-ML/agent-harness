"""A task of the benchmark: what a model is asked, where it begins, and how its work is judged.

A task is a directory. ``task.yml`` says what it is::

    id: code-off-by-one
    category: code            # code, files, sql or compute
    prompt: |
      The tests in tests/ fail. ...
    timeout_seconds: 600      # how long one attempt may take
    max_steps: 40             # how many steps the agent may take

Beside it, ``workspace/`` is what the working directory holds when an
attempt begins, ``reference/`` is what a right answer looks like, and
``checks/`` and ``private/`` hold what the work is judged by. The model sees
``workspace/`` alone. What the other three hold depends on the category, see
:mod:`bench.grade`.
"""

from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from columbina import plain_yaml

__all__ = ["Category", "Loaded", "Task", "TaskError", "load", "tasks"]

Category = Literal["code", "files", "sql", "compute"]
"""What kind of work a task asks for."""


class Task(BaseModel):
    """What ``task.yml`` says of a task.

    Attributes:
        id: Its name, which is also the name of its directory.
        category: What kind of work it asks for.
        prompt: What the model is asked to do.
        timeout_seconds: How long one attempt may take.
        max_steps: How many steps the agent may take in it.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    id: str = Field(pattern=r"^[a-z0-9][a-z0-9-]{0,63}$")
    category: Category
    prompt: str = Field(min_length=1)
    timeout_seconds: float = Field(default=600, gt=0)
    max_steps: int = Field(default=40, gt=0)


class TaskError(Exception):
    """A task cannot be read. The words name the task and say why."""


@dataclass(frozen=True)
class Loaded:
    """A task and where it lies.

    Attributes:
        task: What ``task.yml`` says.
        directory: The directory of the task.
    """

    task: Task
    directory: Path

    @property
    def workspace(self) -> Path:
        """What the working directory holds when an attempt begins."""
        return self.directory / "workspace"


def load(directory: Path) -> Loaded:
    """Read the task in a directory.

    Raises:
        TaskError: Its ``task.yml`` cannot be read or is no task, its id is
            not the name of its directory, or it has no workspace.
    """
    file = directory / "task.yml"
    try:
        said = plain_yaml.load(file.read_text(encoding="utf-8"))
    except OSError as error:
        raise TaskError(f"{file} cannot be read: {error.strerror or error}") from error
    except plain_yaml.PlainYamlError as error:
        raise TaskError(f"{file} is not YAML: {error}") from error
    if not isinstance(said, dict):
        raise TaskError(f"{file} is not a mapping")
    try:
        task = Task.model_validate(said)
    except ValidationError as error:
        raise TaskError(f"{file} is no task: {error}") from error
    if task.id != directory.name:
        raise TaskError(f"the task {task.id} is in a directory called {directory.name}")
    if not (directory / "workspace").is_dir():
        raise TaskError(f"the task {task.id} has no workspace")
    return Loaded(task, directory)


def tasks(root: Path) -> list[Loaded]:
    """Read every task below a directory, ordered by id."""
    found = [load(file.parent) for file in root.rglob("task.yml")]
    return sorted(found, key=lambda loaded: loaded.task.id)
