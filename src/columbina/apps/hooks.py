"""What ``columbina hooks`` does: the hooks of a directory, listed, trusted, and trusted no more.

A hook is a command that the program runs by itself at certain moments of a
session, see :mod:`columbina.hooks`. The hooks that the user wrote down
count as they are. The hooks that a project brings were written by somebody
else, and count only once the user has said that they may. That is said
here, and taken back here.

``list`` tells which commands would run at which moments in a working
directory, and from which of the two files each comes. It tells what was
passed over in a file, and whether the file of the project is trusted as it
is now. ``trust`` writes the file of the project into the record of trust,
by its place and by what it holds, and shows the commands that run from
then on. ``distrust`` takes the file out of the record again.

None of this holds a conversation, and none of it runs a hook. All that it
needs is the config of the hooks: where the file of the user and the record
of trust lie. The files are found and read the way a session finds and
reads them, by :mod:`columbina.hooks.files`, so what is trusted here is what
runs there.

What comes back is text for a person. It repeats what the files hold, and
the file of a project is the work of somebody else. Whatever of a command
cannot be seen is named in it, so that what is read is what would run, and
whoever prints the text makes it fit for a terminal first.
"""

import secrets
from pathlib import Path
from typing import Literal

from columbina.apps.terminal_text import spelled_out
from columbina.hooks import Config
from columbina.hooks.config import (
    EVENTS,
    TOOL_EVENTS,
    HookFileError,
    Hooks,
    chooses_tools,
    distrust,
    parsed,
    trust,
    trusted,
)
from columbina.hooks.files import (
    Found,
    file_in,
    of_the_project,
    of_the_user,
    project_of,
    record_at,
)
from columbina.hooks.run import MAX_COMMAND

__all__ = ["DISTRUST", "LIST", "TRUST", "HooksError", "What", "run"]

LIST = "list"
TRUST = "trust"
DISTRUST = "distrust"

What = Literal["list", "trust", "distrust"]
"""What is to be done with the hooks."""

_USER = "the user"
_PROJECT = "this project"


class HooksError(Exception):
    """What was asked for cannot be done. The text says why, and begins without a capital."""


def _seconds(timeout: float) -> str:
    return f"{timeout:g} {'second' if timeout == 1 else 'seconds'}"


def _spelled(text: str) -> list[str]:
    """Return a text from a hook file line by line, with whatever cannot be seen named."""
    # Only a line feed breaks a line here. Whatever else would is named instead.
    return [spelled_out(line) for line in text.split("\n")]


def _hooks(hooks: Hooks) -> list[str]:
    """Tell the hooks of a file: when each runs, for which tools, how long it may take, and what."""
    lines: list[str] = []
    for event in EVENTS:
        for group in hooks.groups.get(event, ()):
            matcher = group.matcher
            if event not in TOOL_EVENTS:
                # Where no tool is at work, a matcher says nothing.
                tools = ""
            elif chooses_tools(matcher):
                # A matcher is one line to whoever reads it, whatever it holds.
                tools = f", for the tools {'<U+000A>'.join(_spelled(matcher))}"
            else:
                tools = ", for every tool"
            for hook in group.hooks:
                lines.append(f"  at {event}{tools}, within {_seconds(hook.timeout)}:")
                lines.extend(f"      {line}" for line in _spelled(hook.command))
                if len(hook.command) > MAX_COMMAND:
                    lines.append(
                        f"      (This command has more than {MAX_COMMAND} characters. One "
                        "that long is not run.)"
                    )
    return lines


def _passed_over(hooks: Hooks) -> list[str]:
    """Tell what was passed over when a file was read, if anything was."""
    if not hooks.warnings:
        return []
    return ["Not all of the file is used:", *(f"  {warning}" for warning in hooks.warnings)]


def _held(found: Found, whose: str) -> Hooks | str | None:
    """Return what a hook file holds, why nothing of it counts, or ``None`` if it is not there."""
    if found.problem is not None:
        return f"the hook file of {whose}, {found.path}, cannot be read: {found.problem}"
    if found.content is None:
        return None
    try:
        return parsed(found.content)
    except HookFileError as error:
        return f"the hook file of {whose}, {found.path}, is not used: {error}"


def _told(found: Found, whose: str, absent: str) -> tuple[list[str], bool]:
    """Tell what a hook file holds, and whether it holds hooks at all."""
    held = _held(found, whose)
    if held is None:
        return [f"{absent} It would be {found.path}."], False
    if isinstance(held, str):
        return [held[:1].upper() + held[1:]], False
    if not held.groups:
        told = f"The hook file of {whose}, {found.path}, holds no hooks."
        return [told, *_passed_over(held)], False
    told = f"The hooks of {whose}, from {found.path}:"
    return [told, *_hooks(held), *_passed_over(held)], True


def _listed(config: Config, cwd: Path) -> str:
    """Tell the hooks that count for a working directory, and the ones that would."""
    if config.user_file is None:
        lines = ["The profile names no hook file of the user."]
    else:
        lines, _ = _told(of_the_user(config.user_file), _USER, "The user has no hook file.")
    lines.append("")
    if not config.project:
        lines.append("The profile does not read the hook file of a project.")
        return "\n".join(lines)
    theirs = of_the_project(project_of(cwd))
    told, runs = _told(theirs, _PROJECT, "This project has no hook file.")
    lines.extend(told)
    if runs:
        assert theirs.content is not None
        if trusted(record_at(config.trust_file), theirs.path, theirs.content):
            lines.append("The file is trusted as it is now, so these hooks run.")
        elif config.trust_file is None:
            lines.append(
                "The profile names no record of trust, so no file of a project is trusted "
                "and none of these hooks runs."
            )
        else:
            lines.append(
                "The file is not trusted as it is now, so none of these hooks runs. "
                "'columbina hooks trust' trusts it."
            )
    return "\n".join(lines)


def _record_of(config: Config) -> Path:
    """Return where the record of trust lies, for one who is about to change it.

    Raises:
        HooksError: The profile reads no hook file of a project, or names no
            record.
    """
    if not config.project:
        raise HooksError(
            "the profile does not read the hook file of a project, so there is nothing to trust"
        )
    if config.trust_file is None:
        raise HooksError(
            "the profile names no record of trust, so the hook file of no project can be trusted"
        )
    return config.trust_file


def _kept(path: Path, record: bytes) -> None:
    """Put a record of trust in the place of the one that is there, whole or not at all.

    Two that change the record at the same moment do not see each other.
    What the first of them wrote is gone then, and its file has to be
    trusted once more.

    Raises:
        HooksError: The record cannot be written.
    """
    beside = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        try:
            beside.write_bytes(record)
            beside.replace(path)
        finally:
            beside.unlink(missing_ok=True)
    except OSError as error:
        raise HooksError(f"the record of trust, {path}, cannot be written: {error}") from error


def _trusted(config: Config, cwd: Path) -> str:
    """Write the hook file of a project into the record of trust, and tell what runs from now on.

    Raises:
        HooksError: There is no such file, it cannot be read or used, or the
            record cannot be written.
    """
    record = _record_of(config)
    theirs = of_the_project(project_of(cwd))
    held = _held(theirs, _PROJECT)
    if held is None:
        raise HooksError(
            "this project has no hook file, so there is nothing to trust. It would be "
            f"{theirs.path}"
        )
    if isinstance(held, str):
        raise HooksError(held)
    assert theirs.content is not None
    # What is trusted is what was read here and is shown below, whatever the file holds by now.
    _kept(record, trust(record_at(record), theirs.path, theirs.content))
    trusted_now = f"The hook file of this project, {theirs.path}, is trusted as it is now."
    if held.groups:
        lines = [f"{trusted_now} These hooks run from now on:", *_hooks(held)]
    else:
        lines = [f"{trusted_now} It holds no hooks."]
    lines.extend(_passed_over(held))
    lines.append(
        "When the file changes, its hooks stop until it is trusted again. "
        "'columbina hooks distrust' takes the trust back."
    )
    return "\n".join(lines)


def _distrusted(config: Config, cwd: Path) -> str:
    """Take the hook file of a project out of the record of trust, whatever the file holds.

    Raises:
        HooksError: The record cannot be written.
    """
    record = _record_of(config)
    path = file_in(project_of(cwd))
    left = distrust(record_at(record), path)
    if left is None:
        return f"The hook file of this project, {path}, was not trusted."
    _kept(record, left)
    return f"The hook file of this project, {path}, is trusted no more. Its hooks do not run."


def run(config: Config, cwd: Path, what: What) -> str:
    """Do one of the three things, and return what is to be printed.

    This reads the disk, and the two that change the record of trust write
    to it.

    Args:
        config: The config of the hooks, as the profile has it.
        cwd: The working directory whose hooks are meant.
        what: What to do.

    Raises:
        HooksError: What was asked for cannot be done.
    """
    if what == LIST:
        return _listed(config, cwd)
    if what == TRUST:
        return _trusted(config, cwd)
    return _distrusted(config, cwd)
