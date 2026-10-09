"""What a run wrote or sent, put the way it would read on any machine.

A session log and the requests of a run name the working directory, and a run
that uses the shell tool names the shell of its platform. A snapshot that is
compared on every machine cannot hold either. Here such text is brought to
one form before it is compared or stored:

- The working directory is written ``/work``.
- If asked for, the tool that runs commands is called ``shell``, and the line
  of the runtime facts that says which shell it is says that it is the shell
  of the platform. What that line and that name are on each platform has
  tests of its own.

What stands on the screen of a conversation is brought to one form by
:func:`neutral_screen`.

A run also begins the same way everywhere: :func:`copy_of` gives it a working
directory whose files are all of one age.
"""

import os
import re
import shutil
from collections.abc import Sequence
from pathlib import Path
from typing import Any, cast

import columbina
from columbina.llm import LlmRequest

__all__ = ["copy_of", "neutral", "neutral_requests", "neutral_screen"]

WORK = "/work"
_RUN_BY = re.compile(r"^commands are run by: .*$", flags=re.MULTILINE)
_TOOK = re.compile(
    r"^(  \S+ (?:[A-Z_]+, )?)(?:\d+ ms|\d+\.\d s|\d+ min \d+ s)(?=,|$)", flags=re.MULTILINE
)
"""The time that a call took, in the line that tells of its result."""
_ONE_MOMENT = 1_700_000_000
"""When every file of a copy was last changed, as far as the copy says."""


def copy_of(directory: Path, to: Path) -> Path:
    """Copy a working directory for a run, and return where the copy is.

    Every file of the copy was last changed at the same moment. A search
    gives the newest files first, and which file of a checkout is the newest
    differs from machine to machine.
    """
    shutil.copytree(directory, to, ignore=shutil.ignore_patterns("__pycache__"))
    for path in to.rglob("*"):
        os.utime(path, (_ONE_MOMENT, _ONE_MOMENT))
    return to


def neutral(value: object, work: Path, shell: str | None = None) -> Any:
    """Return plain data with the working directory, and the shell if one is named, made neutral.

    Args:
        value: Data as JSON holds it: mappings, lists, text, numbers.
        work: The working directory of the run.
        shell: The name of the tool that runs commands, if it is to be
            written the same on every platform.
    """
    if isinstance(value, str):
        text = value.replace(str(work), WORK)
        if shell is not None:
            text = _RUN_BY.sub("commands are run by: the shell of this platform", text)
        return text
    if isinstance(value, list):
        return [neutral(item, work, shell) for item in cast("list[object]", value)]
    if isinstance(value, dict):
        items = cast("dict[str, object]", value)
        changed = {key: neutral(item, work, shell) for key, item in items.items()}
        # A call of a tool and what a model is told about a tool both carry its name.
        if shell is not None and changed.get("name") == shell:
            changed["name"] = "shell"
        return changed
    return value


def neutral_screen(lines: Sequence[str], work: Path, shell: str) -> str:
    """Return what stands on a screen, with what differs between machines and runs made neutral.

    Beside the working directory, that is the version of the program in the
    line that a conversation begins with, the name of the tool that runs
    commands where a call of it is shown or asked about, and how long each
    call took.

    Args:
        lines: The lines of the screen.
        work: The working directory of the run.
        shell: The name of the tool that runs commands.
    """
    text = "\n".join(lines).replace(str(work), WORK) + "\n"
    text = text.replace(f"Columbina {columbina.__version__} ", "Columbina VERSION ", 1)
    named = re.escape(shell)
    text = re.sub(rf"^(. ){named}( |$)", r"\1shell\2", text, flags=re.MULTILINE)
    text = re.sub(rf"^{named}( asks for a yes)", r"shell\1", text, flags=re.MULTILINE)
    return _TOOK.sub(r"\1TIME", text)


def neutral_requests(
    requests: Sequence[LlmRequest], work: Path, shell: str | None = None
) -> list[LlmRequest]:
    """Return the requests of a run with what differs between machines made neutral."""
    return [
        LlmRequest.model_validate(neutral(request.model_dump(mode="json"), work, shell))
        for request in requests
    ]
