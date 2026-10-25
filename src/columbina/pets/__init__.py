"""What a task is doing, written down for a desktop pet.

This module is a plugin. Loading it with a file in its config has the
conversations of the program told in that file, as docs/pets.md describes it:
a pet of the desktop application reads it, and so can any program of somebody
else. Without a file it does nothing.

The file says one of a few states, and a short label beside it:

- ``thinking`` while a turn runs and no tool does;
- ``review`` while a tool runs, labelled with its name;
- ``waiting`` while the user is asked for a yes, labelled with the tool;
- ``done`` when a turn ended well, and ``failed``, labelled with its code,
  when one failed, was blocked or was cut off by the length of a reply;
- ``idle`` at rest: at first, after a turn that was stopped, a while after
  ``done`` or ``failed``, and when the program ends.

Only what a person talks with is told: a subagent works inside a call of its
parent, which shows as that call. Nothing of a conversation is written but the
name of a tool and the code of a failure. The file is written whole each time,
and when the state changes alone: a model may think for minutes, and the file
says so all that while. Whoever reads it takes the program to be at rest where
its process, which the file names, is gone.
"""

import asyncio
import json
import logging
import os
import time
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, field_validator

from columbina import files
from columbina.approval import ApprovalAskedEvent, ApprovalDecidedEvent
from columbina.kernel import Context
from columbina.session import (
    SESSION_EVENT,
    Committed,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)

__all__ = ["Config", "State", "Teller", "apply", "name"]

name = "pets"

logger = logging.getLogger("columbina.pets")

State = Literal["idle", "thinking", "review", "waiting", "done", "failed"]
"""What the file says a task is doing."""

_FAILED = ("error", "blocked", "max-tokens")
"""The ends of a turn that are told as a failure. A turn that was stopped is none."""

_LABEL_MOST = 80


class Config(BaseModel):
    """The config of the pets.

    Attributes:
        file: Where the state is written. Empty or left out, it is written nowhere.
        rest_after: How many seconds after a turn is done, or failed, the state is ``idle`` again.
    """

    model_config = ConfigDict(extra="forbid")

    file: Path | None = None
    rest_after: float = 15.0

    @field_validator("file", mode="before")
    @classmethod
    def _nowhere(cls, value: object) -> object:
        # A variable that names no file names nowhere, and not the working directory.
        return None if value == "" else value


class Teller:
    """Follows the conversations, and writes what they do when that changes."""

    def __init__(self, path: Path, rest_after: float) -> None:
        self._path = path
        self._rest_after = rest_after
        self._said: tuple[State, str] | None = None
        self._resting: asyncio.TimerHandle | None = None
        # The tool of each question that is open, by its call.
        self._asked: dict[str, str] = {}

    def heard(self, committed: Committed) -> None:
        """Hear an event of a session, and tell what it changes."""
        if committed.session.delegation_depth != 0:
            return
        data = committed.event.data
        if isinstance(data, TurnStartEvent):
            self._tell("thinking")
        elif isinstance(data, ToolCallEvent):
            self._tell("review", data.name)
        elif isinstance(data, ApprovalAskedEvent):
            self._asked[data.call_id] = data.tool
            self._tell("waiting", data.tool)
        elif isinstance(data, ApprovalDecidedEvent):
            tool = self._asked.pop(data.call_id, "")
            # A call that was let through runs now. Any other one does not.
            if data.outcome == "allowed-once":
                self._tell("review", tool)
            else:
                self._tell("thinking")
        elif isinstance(data, ToolResultEvent):
            self._tell("thinking")
        elif isinstance(data, TurnEndEvent):
            self._asked.clear()
            if data.reason == "completed":
                self._tell("done")
            elif data.reason in _FAILED:
                self._tell("failed", data.code or "")
            else:
                self._tell("idle")

    def rest(self) -> None:
        """Say that the program is at rest, as it ends."""
        self._tell("idle")

    def _tell(self, state: State, label: str = "") -> None:
        label = label.replace("\n", " ")[:_LABEL_MOST]
        if (state, label) == self._said:
            return
        self._said = (state, label)
        if self._resting is not None:
            self._resting.cancel()
            self._resting = None
        if state in ("done", "failed"):
            self._resting = asyncio.get_running_loop().call_later(self._rest_after, self.rest)
        document = {
            "version": 1,
            "state": state,
            "label": label,
            "at": int(time.time() * 1000),
            "pid": os.getpid(),
        }
        try:
            files.replaced(self._path, json.dumps(document) + "\n")
        except OSError as error:
            # A pet that is told nothing is no reason to stop the work.
            logger.warning(
                "the state for the pets cannot be written to %s: %s", self._path, error.strerror
            )


def apply(ctx: Context, config: Config) -> None:
    """Follow the conversations, where a file to tell them in is named."""
    if config.file is None:
        return
    teller = Teller(config.file, config.rest_after)
    ctx.on(SESSION_EVENT, teller.heard)
    ctx.effect(lambda: teller.rest)
