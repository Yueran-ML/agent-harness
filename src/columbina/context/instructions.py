"""Instruction files: what a project and its user want an agent to know, told when a turn begins.

This module is a plugin. Loading it registers the source of context
``instructions`` with the agent service.

A repository says how work in it is done in a file at its top, ``AGENTS.md``
by a convention that several tools share, and a directory below may add to
that in a file of its own. A user keeps what holds for all projects in one
file elsewhere. This plugin finds those files and gives their text to the
model, the widest first: the file of the user, then the files from the root
of the project down to the working directory. The root of the project is the
nearest directory, from the working directory upwards, that holds ``.git``.
Without one, only the working directory is looked at.

When the text changes, the new one follows the conversation, said to take the
place of the one that was given before, so the model knows which instructions
are the current ones. The old text stays where it stands: the beginning of a
conversation that stays the same is what a provider keeps from one request to
the next, and some bind the thinking of their model to it. If the files go
away, the model is told that too.

The files are read for the user and not by the agent, so the sandbox, which
holds an agent in its working directory, is not asked about them. That makes
this plugin a way around the sandbox and the rules unless it is careful, since
what a repository holds was written by somebody else. So two things are seen
to. Only a file that lies where it is looked for is read. A link in that
place is not followed, wherever it leads: it could hand the model a file that
is nobody's business, a key of the user or a file that a rule keeps the agent
from. And all files together may not be longer than a budget. If they are,
the wider ones are left out first, and the most specific one is cut last. The
model is told what was left out, cut or not read.

A file of the same content as one that was given already is not given twice.
A link to another place where an instruction file is looked for is passed
over without a word: what lies there is read there. That is how a repository
that keeps one file under two names looks.
"""

import asyncio
import codecs
import os
from dataclasses import dataclass, field
from pathlib import Path

from pydantic import BaseModel, Field, field_validator

from columbina.agent import AGENTS, CONTEXT_PREFIX, Agent, ContextSource
from columbina.fs import bound, locate
from columbina.fs.paths import usable
from columbina.kernel import Context
from columbina.project import way_down
from columbina.session import EventData, Projection, SessionEvent, UserMessageEvent

__all__ = ["Config", "apply", "inject", "name"]

name = "context-instructions"
inject = (AGENTS,)

_SOURCE = CONTEXT_PREFIX + "instructions"
_GONE = (
    "The instruction files that were given before are no longer there. What they said does "
    "not hold any more."
)
_OPENING = (
    "These files hold instructions for work in this directory. They come from the widest to "
    "the most specific one, and where two of them disagree, the later one counts."
)
_VERBATIM = "\\\\?\\"
"""What Windows may put before the place that a link leads to."""


class Config(BaseModel):
    """The config of the instruction files.

    Attributes:
        names: The names that an instruction file may have. In a directory
            that holds several of them, all are read, in this order.
        user_file: The file with the instructions of the user, which hold
            for every project. One that is not there holds none.
        max_bytes: How long all instructions together may be.
    """

    names: tuple[str, ...] = ("AGENTS.md", "CLAUDE.md")
    user_file: Path | None = None
    max_bytes: int = Field(default=65536, ge=1)

    @field_validator("names")
    @classmethod
    def _names_of_files(cls, names: tuple[str, ...]) -> tuple[str, ...]:
        for file_name in names:
            plain = file_name.strip() and not set(file_name) & {"/", "\\", chr(0)}
            if not plain or file_name in (".", "..") or not usable(file_name):
                raise ValueError(f"{file_name!r} is not the name of a file in a directory")
        return names


@dataclass(frozen=True, slots=True)
class _File:
    """An instruction file.

    Attributes:
        label: What the file is called before the model.
        content: What it holds, or the beginning of that if the file is
            longer than all instructions together may be.
    """

    label: str
    content: bytes


@dataclass(slots=True)
class _Gathered:
    """What was found.

    Attributes:
        files: The files, the widest first.
        unread: The labels of the files that were not read because they are
            links.
    """

    files: list[_File] = field(default_factory=list[_File])
    unread: list[str] = field(default_factory=list[str])


def _last_given(state: bool, event: SessionEvent[EventData]) -> bool:
    data = event.data
    return state or (isinstance(data, UserMessageEvent) and data.source == _SOURCE)


_GIVEN: Projection[bool] = Projection("instructions-given", False, _last_given)
"""Whether instructions were ever given in a session."""


def _named_by(directory: Path, link: Path) -> Path:
    """Return the place that a link in a directory names, going by what the link stores alone."""
    return Path(os.path.normpath(str(directory / link).removeprefix(_VERBATIM)))


def _gather(cwd: Path, config: Config) -> _Gathered:
    """Find and read the instruction files for a working directory. This blocks.

    Raises:
        OSError: A file is there and cannot be read.
    """
    gathered = _Gathered()
    known: set[bytes] = set()

    def take(label: str, content: bytes) -> None:
        if content.removeprefix(codecs.BOM_UTF8).strip() and content not in known:
            known.add(content)
            gathered.files.append(_File(label, content))

    if config.user_file is not None:
        # This file is the user's own, wherever it is and whatever it leads to.
        try:
            with config.user_file.open("rb") as file:
                take(str(config.user_file), file.read(config.max_bytes + 1))
        except FileNotFoundError:
            pass
        except OSError as error:
            raise OSError(
                f"the instruction file {config.user_file} cannot be read: {error}"
            ) from error
    here = locate(cwd, ".").path
    down = way_down(here)
    looked_for = {directory / file_name for directory in down for file_name in config.names}
    for directory in down:
        for file_name in config.names:
            path = directory / file_name
            # The file is called the way the model would write its path.
            steps_up = len(here.parts) - len(directory.parts)
            label = "/".join([*[".."] * steps_up, file_name])
            try:
                link = bound.link_at(path)
                if link is not None:
                    if _named_by(directory, link) not in looked_for:
                        gathered.unread.append(label)
                    continue
                with bound.open_for_reading(path)[0] as file:
                    take(label, file.read(config.max_bytes + 1))
            except (FileNotFoundError, bound.NotAFileError):
                continue
            except OSError as error:
                raise OSError(f"the instruction file {label} cannot be read: {error}") from error
    return gathered


def _said(gathered: _Gathered, max_bytes: int) -> str | None:
    """Return the text for the model, or ``None`` if there is nothing to say."""
    kept: list[str] = []
    left_out: list[str] = []
    room = max_bytes
    # The most specific file is taken first, so that it is the last to give way.
    for file in reversed(gathered.files):
        fits = len(file.content) <= room
        if left_out or (kept and not fits):
            # Once a file was left out, every wider one is, however small it is.
            left_out.append(file.label)
            continue
        # A character that the cut goes through is dropped. Other bytes that are no text show.
        text = file.content[:room].decode("utf-8-sig", errors="replace" if fits else "ignore")
        text = text.replace("\r\n", "\n").strip()
        if not fits:
            text += f"\n[Here the file is cut off: only its first {room} bytes are given.]"
        kept.append(f"==> {file.label} <==\n{text}")
        room -= min(room, len(file.content))
    notes: list[str] = []
    if left_out:
        notes.append(
            f"[Left out, because all instructions together may not be longer than {max_bytes} "
            f"bytes: {', '.join(reversed(left_out))}]"
        )
    if gathered.unread:
        notes.append(
            "[Not read, because it is a link, and a link could lead out of the project: "
            f"{', '.join(gathered.unread)}]"
        )
    parts = [_OPENING, *reversed(kept)] if kept else []
    return "\n\n".join([*parts, *notes]) or None


def apply(ctx: Context, config: Config) -> None:
    """Register the source of context that gives the instruction files."""

    async def render(agent: Agent) -> str | None:
        gathered = await asyncio.to_thread(_gather, agent.cwd, config)
        said = _said(gathered, config.max_bytes)
        if said is None and _GIVEN.of(agent.session):
            return _GONE
        return said

    ctx.use(AGENTS).context.add(ctx, "instructions", ContextSource(render, order=0, kind="replace"))
