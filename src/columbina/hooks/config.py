"""Hook files: the commands that a person wants run at certain moments of a session.

A hook is a command. The program runs it when something happens: a session
begins, the user says something, a tool is about to be called or has been,
the agent is about to stop. What the command answers can hold a call back,
have somebody asked about it, or tell the model something. What happens at
each of these moments is in :mod:`columbina.hooks`. This module only reads
the files in which hooks are written down, and keeps the record of which
files are trusted. It knows nothing of agents or of tools.

A file looks like this. ::

    {"hooks": {"PostToolUse": [
        {"matcher": "Edit|Write",
         "hooks": [{"type": "command", "command": "ruff format", "timeout": 30}]}
    ]}}

Under the name of a moment stands a list of groups. A group has the
commands to run and may say for which tools: its matcher. The form is the
one that other tools use for their hooks, so a file that was written for one
of them can be used here.

A file is read as far as it can be. A moment that is not known, a kind of
hook that is not a command and a group that is written wrongly are passed
over, and each is told in a warning: one mistake does not switch off the
hooks beside it. A file that is no JSON at all, or holds something else than
it should at the top, is not used.

No matcher, an empty one or ``*`` means every tool. Any other matcher is a
regular expression, and it has to match the whole name of a tool. So names
with bars between them, such as ``Edit|Write``, mean just these tools: a
matcher ``Read`` that also took every tool with these four letters in its
name would be a trap.

A tool has two names here: its own, and the one that the other tools call
their tool of that kind. A matcher may use either.

There are two files, the user's and the one of a project. A hook is a
command, and it may run as soon as a session begins. Starting the program
in a repository that somebody else wrote must not run what that somebody
wrote into it. So the file of a project counts only after the user has said
that it may: its place and the SHA-256 of what it holds are written into a
record in the home of the program. When the file changes, the record does
not fit it any more and its hooks stop until the user has looked again.
The record goes by the place as well as by the content, since the same
words mean other commands in another directory.
"""

import hashlib
import json
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import cast

__all__ = [
    "ALIASES",
    "DEFAULT_TIMEOUT",
    "EVENTS",
    "MAX_FILE_BYTES",
    "MAX_TIMEOUT",
    "TOOL_EVENTS",
    "Group",
    "Hook",
    "HookFileError",
    "Hooks",
    "digest",
    "matches",
    "names_of",
    "parsed",
    "trust",
    "trusted",
]

EVENTS = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop")
"""The moments at which hooks run."""

TOOL_EVENTS = ("PreToolUse", "PostToolUse")
"""The moments that have to do with one tool. Only there does a matcher count."""

DEFAULT_TIMEOUT = 60.0
"""How many seconds a hook may take if its file does not say."""

MAX_TIMEOUT = 600.0
"""How many seconds a hook may take at most."""

MAX_FILE_BYTES = 256 * 1024
"""A hook file that is larger is not read."""

ALIASES: Mapping[str, str] = {
    "bash": "Bash",
    "powershell": "Bash",
    "read": "Read",
    "write": "Write",
    "edit": "Edit",
    "glob": "Glob",
    "grep": "Grep",
    "todo_write": "TodoWrite",
    "skill": "Skill",
    "subagent": "Task",
}
"""What other tools call the tool that does the work of one of ours."""

_EVERY = ("", "*")
"""The matchers that are for every tool, beside no matcher at all."""

_SHOWN = 60
_VERSION = 1


class HookFileError(Exception):
    """A hook file cannot be used at all. The text says why."""


@dataclass(frozen=True, slots=True)
class Hook:
    """One command to run.

    Attributes:
        command: The command, for the shell.
        timeout: How many seconds it may take.
    """

    command: str
    timeout: float = DEFAULT_TIMEOUT


@dataclass(frozen=True, slots=True)
class Group:
    """Commands that run together, and the tools that they run for.

    Attributes:
        matcher: Which tools the group is for, as its file says it. ``None``
            if the file says nothing, which means all of them.
        hooks: The commands, in the order in which they run.
    """

    matcher: str | None
    hooks: tuple[Hook, ...]


@dataclass(frozen=True, slots=True)
class Hooks:
    """What a hook file holds.

    Attributes:
        groups: The groups of each moment, by its name. A moment without
            groups is not in it.
        warnings: What was passed over when the file was read, and why.
    """

    groups: Mapping[str, tuple[Group, ...]] = field(default_factory=dict[str, tuple[Group, ...]])
    warnings: tuple[str, ...] = ()


def _shown(value: object) -> str:
    """Return something from a file in a few characters, whatever it is."""
    if isinstance(value, list):
        return "a list"
    if isinstance(value, dict):
        return "an object"
    shown = json.dumps(value, ensure_ascii=False)
    return shown if len(shown) <= _SHOWN else f"{shown[:_SHOWN]}..."


def _hook(written: object, where: str, warnings: list[str]) -> Hook | None:
    if not isinstance(written, dict):
        warnings.append(f"{where} is {_shown(written)} and no object, and is passed over")
        return None
    entry = cast("dict[str, object]", written)
    kind = entry.get("type", "command")
    if kind != "command":
        warnings.append(
            f'{where} is of the type {_shown(kind)}. Only hooks of the type "command" are '
            "run, and this one is passed over"
        )
        return None
    command = entry.get("command")
    if not isinstance(command, str) or not command.strip():
        warnings.append(f"{where} names no command, and is passed over")
        return None
    timeout = entry.get("timeout", DEFAULT_TIMEOUT)
    # True is a number to Python, and no number of seconds to anybody.
    if isinstance(timeout, bool) or not isinstance(timeout, int | float) or not timeout > 0:
        warnings.append(
            f"{where} has the timeout {_shown(timeout)}, which is no number of seconds above "
            "zero. The hook is passed over"
        )
        return None
    if timeout > MAX_TIMEOUT:
        warnings.append(
            f"{where} has the timeout {_shown(timeout)}. A hook may take {MAX_TIMEOUT:g} "
            "seconds at most, and that is what this one gets"
        )
        timeout = MAX_TIMEOUT
    return Hook(command, float(timeout))


def _group(written: object, where: str, warnings: list[str]) -> Group | None:
    if not isinstance(written, dict):
        warnings.append(f"{where} is {_shown(written)} and no object, and is passed over")
        return None
    entry = cast("dict[str, object]", written)
    matcher = entry.get("matcher")
    if matcher is not None and not isinstance(matcher, str):
        warnings.append(
            f"{where} has the matcher {_shown(matcher)}, which is no text. It is passed over"
        )
        return None
    if matcher is not None and matcher not in _EVERY:
        try:
            re.compile(matcher)
        except (re.error, RecursionError, OverflowError) as error:
            warnings.append(
                f"{where} has the matcher {_shown(matcher)}, which is no regular expression "
                f"({error}). It is passed over"
            )
            return None
    listed = entry.get("hooks")
    if not isinstance(listed, list):
        warnings.append(f"{where} has no list of hooks, and is passed over")
        return None
    hooks = [
        hook
        for number, one in enumerate(cast("list[object]", listed), start=1)
        if (hook := _hook(one, f"{where}, hook {number}", warnings)) is not None
    ]
    return Group(matcher, tuple(hooks)) if hooks else None


def parsed(content: bytes) -> Hooks:
    """Read what a hook file holds.

    Raises:
        HookFileError: The file cannot be used at all: it is too large, is
            no JSON, or holds something else than an object of moments.
    """
    if len(content) > MAX_FILE_BYTES:
        raise HookFileError(f"it has more than {MAX_FILE_BYTES} bytes")
    try:
        loaded: object = json.loads(content.decode("utf-8-sig"))
    except UnicodeDecodeError:
        raise HookFileError("it is not written in UTF-8") from None
    except RecursionError:
        raise HookFileError("it is nested too deeply") from None
    except ValueError as error:
        raise HookFileError(f"it is not JSON: {error}") from None
    if not isinstance(loaded, dict):
        raise HookFileError(f"it holds {_shown(loaded)} and no object")
    moments = cast("dict[str, object]", loaded).get("hooks", {})
    if not isinstance(moments, dict):
        raise HookFileError(f'its "hooks" is {_shown(moments)} and no object')
    warnings: list[str] = []
    groups: dict[str, tuple[Group, ...]] = {}
    for event, listed in cast("dict[str, object]", moments).items():
        if event not in EVENTS:
            warnings.append(
                f"{_shown(event)} is no moment at which hooks run here, and is passed over. "
                f"The moments are: {', '.join(EVENTS)}"
            )
            continue
        if not isinstance(listed, list):
            warnings.append(
                f"{event} holds {_shown(listed)} and no list of groups, and is passed over"
            )
            continue
        found = [
            group
            for number, one in enumerate(cast("list[object]", listed), start=1)
            if (group := _group(one, f"{event}, group {number}", warnings)) is not None
        ]
        if found:
            groups[event] = tuple(found)
    return Hooks(groups, tuple(warnings))


def names_of(tool: str) -> tuple[str, ...]:
    """Return the names that a matcher may call a tool by: its own, and what others call it."""
    alias = ALIASES.get(tool)
    return (tool,) if alias is None else (tool, alias)


def matches(matcher: str | None, names: Sequence[str]) -> bool:
    """Tell whether a matcher is for a tool that goes by one of these names."""
    if matcher is None or matcher in _EVERY:
        return True
    return any(re.fullmatch(matcher, name) is not None for name in names)


def digest(content: bytes) -> str:
    """Return what tells one content of a hook file from every other."""
    return hashlib.sha256(content).hexdigest()


def _record(record: bytes | None) -> dict[str, str]:
    """Return the files that a record trusts, by their places.

    A record that cannot be read as one trusts none.
    """
    if record is None:
        return {}
    try:
        loaded: object = json.loads(record.decode("utf-8-sig"))
    except (ValueError, RecursionError):
        return {}
    if not isinstance(loaded, dict):
        return {}
    files = cast("dict[str, object]", loaded).get("files")
    if cast("dict[str, object]", loaded).get("version") != _VERSION or not isinstance(files, dict):
        return {}
    return {
        place: sha
        for place, sha in cast("dict[object, object]", files).items()
        if isinstance(place, str) and isinstance(sha, str)
    }


def trusted(record: bytes | None, file: Path, content: bytes) -> bool:
    """Tell whether the user has said that this content of a hook file may be used.

    Args:
        record: What the record of trust holds, or ``None`` if there is none.
        file: Where the hook file lies, with every link followed.
        content: What it holds now.
    """
    return _record(record).get(str(file)) == digest(content)


def trust(record: bytes | None, file: Path, content: bytes) -> bytes:
    """Return the record of trust with this content of a hook file added to it.

    What the record said about the file before is replaced. A record that
    cannot be read is begun anew.
    """
    files = _record(record)
    files[str(file)] = digest(content)
    kept = {"version": _VERSION, "files": dict(sorted(files.items()))}
    return (json.dumps(kept, ensure_ascii=False, indent=2) + "\n").encode()
