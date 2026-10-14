"""Writing the files of a task that is made: its ``task.yml`` and its text files."""

from pathlib import Path
from typing import Any, cast

import yaml

__all__ = ["task_yml", "text"]


class _Dumper(yaml.SafeDumper):
    """Writes a text of several lines as a block, the way a person writes a prompt."""


def _string(dumper: yaml.SafeDumper, value: str) -> yaml.ScalarNode:
    style = "|" if "\n" in value else None
    # The stubs of PyYAML do not say what the value may be.
    scalar = cast(Any, dumper).represent_scalar
    return cast(yaml.ScalarNode, scalar("tag:yaml.org,2002:str", value, style=style))


_Dumper.add_representer(str, _string)


def task_yml(directory: Path, task: dict[str, Any]) -> None:
    """Write the ``task.yml`` of a task, with its fields in the order given."""
    text(directory / "task.yml", yaml.dump(task, Dumper=_Dumper, sort_keys=False, width=100))


def text(path: Path, content: str) -> None:
    """Write a text file with line ends of one character, making its directory."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8", newline="\n")
