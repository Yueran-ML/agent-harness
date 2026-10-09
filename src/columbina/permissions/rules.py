"""Rules about calls of tools: how they are written, and what they speak about.

A rule is the name of a tool, or the name with a pattern in parentheses::

    grep
    read(.env)
    write(docs/**)
    powershell(git status*)
    mcp__github__*

**Which tools.** The name may hold ``*``, which stands for any run of
characters. The names ``read``, ``write`` and ``run`` speak about every tool
that does that, whatever it is called: ``write(docs/**)`` is about the edit
tool too, and ``read(.env)`` is about a search that would open the file. So a
rule against reading a file cannot be got around by reading it with another
tool.

**Which calls.** A rule without a pattern speaks about every call of its
tools. A pattern is matched against what the call acts on, which the tool
says: a command for a tool that runs one, and a file for the others.

*A pattern for commands.* ``*`` stands for any run of characters, and every
other character for itself. Such a rule goes by the text of a command and
knows nothing of what the command does. It keeps nobody from anything who
means to get around it.

*A pattern for files* is made of names with ``/`` between them. In a name,
``*`` stands for any run of characters. A name that is ``**`` stands for any
number of directories, also none. Beyond that:

- A pattern speaks about what it names and about everything below it, so
  ``read(secrets)`` is about every file in that directory.
- A pattern of one name, or one that begins with ``**``, is looked for at
  every depth: ``.env`` is ``**/.env``. In the working directory that is
  every depth below it. For a file outside, it is every depth of the full
  path of the file.
- A pattern of several names, or one that begins with ``./``, starts at the
  working directory: ``./.env`` is the one at the top, and ``../notes`` is a
  directory beside the working directory. Two dots are matched by two dots
  and by nothing else, so no star leads out of the working directory.
- A pattern that begins with ``/``, with a drive, or with ``~`` for the home
  directory names files by their full path, wherever the working directory
  is.

Where the disk takes upper and lower case for the same, on Windows and macOS,
so does a pattern for files.

Patterns are matched by hand, one character at a time, and not with regular
expressions: a pattern with many stars takes time in proportion to its length
times the length of the text, and no more.
"""

import itertools
import os
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from functools import cache
from pathlib import Path
from typing import Literal

from columbina.fs import Located
from columbina.fs.patterns import sequence_fits
from columbina.tools import Effect

__all__ = [
    "IGNORES_CASE",
    "Place",
    "Rule",
    "covers_command",
    "covers_file",
    "parse_rule",
    "place_of",
    "wild",
]

IGNORES_CASE = sys.platform in ("win32", "darwin")
"""Whether the disks of this platform usually take two names for one that differ in case."""

_CLASSES = ("read", "write", "run")
"""The names that speak about every tool with that effect."""

_folded = str.lower if IGNORES_CASE else str
"""Brings a name to the form in which it is compared with another."""


@dataclass(frozen=True, slots=True)
class Rule:
    """One rule.

    Attributes:
        tool: The name of the tools it speaks about. It may hold ``*``.
        pattern: What a call has to act on for the rule to speak about it,
            or ``None`` if the rule speaks about every call.
    """

    tool: str
    pattern: str | None = None

    def __str__(self) -> str:
        return self.tool if self.pattern is None else f"{self.tool}({self.pattern})"


@dataclass(frozen=True, slots=True)
class Place:
    """Where a file is, in the two ways of writing it that a pattern can mean.

    Attributes:
        relative: The path from the working directory, with forward slashes.
            The working directory itself is ``.``, and a file outside it
            begins with ``..``. It is ``None`` for a file that no path leads
            to from there, which is one on another drive.
        absolute: The full path, with forward slashes.
    """

    relative: str | None
    absolute: str


def place_of(where: Located) -> Place:
    """Return the two ways of writing where a located file is."""
    if where.inside:
        return Place(where.relative, where.path.as_posix())
    try:
        relative = Path(os.path.relpath(where.path, where.root)).as_posix()
    except ValueError:
        # Another drive: nothing leads there from the working directory.
        relative = None
    return Place(relative, where.path.as_posix())


def parse_rule(text: str) -> Rule:
    """Read a rule.

    Raises:
        ValueError: The text is no rule. The message says what is wrong.
    """
    written = text.strip()
    name, opening, rest = written.partition("(")
    if not opening:
        pattern = None
    elif not rest.endswith(")"):
        raise ValueError(f"the rule {text!r} opens a parenthesis and does not close it at its end")
    else:
        pattern = rest[:-1].strip()
        if not pattern:
            raise ValueError(f"the rule {text!r} has nothing between its parentheses")
    name = name.strip()
    if not name:
        raise ValueError(f"the rule {text!r} names no tool")
    if not all(letter.isalnum() or letter in "_-.*" for letter in name):
        raise ValueError(
            f"the rule {text!r} names a tool with a character that no name of a tool holds"
        )
    return Rule(name, pattern)


def wild(pattern: str, text: str) -> bool:
    """Tell whether a text fits a pattern in which ``*`` stands for any run of characters."""
    at = 0
    star = -1
    resumed = 0
    reached = 0
    while reached < len(text):
        if at < len(pattern) and pattern[at] == "*":
            # Try the star as nothing first, and remember where to come back to.
            star = at
            resumed = reached
            at += 1
        elif at < len(pattern) and pattern[at] == text[reached]:
            at += 1
            reached += 1
        elif star >= 0:
            # Let the last star take one character more.
            at = star + 1
            resumed += 1
            reached = resumed
        else:
            return False
    return all(letter == "*" for letter in pattern[at:])


def _is_absolute(path: str) -> bool:
    return path.startswith("/") or (len(path) > 1 and path[1] == ":" and path[0].isalpha())


def _names(path: str) -> tuple[str, ...]:
    """Split a path at its slashes. What a full path starts from is a name of its own."""
    start: tuple[str, ...] = ()
    if path.startswith("//"):
        start = ("//",)
    elif path.startswith("/"):
        start = ("/",)
    return (*start, *(name for name in path.split("/") if name not in ("", ".")))


_Start = Literal["full path", "every depth", "working directory"]
"""Where a pattern for files starts."""


@cache
def _file_pattern(pattern: str) -> tuple[tuple[str | None, ...], _Start]:
    """Return the parts of a pattern for files, and where the pattern starts.

    A part that is ``None`` stands for any number of names.
    """
    written = pattern.strip().replace("\\", "/")
    if written == "~" or written.startswith("~/"):
        written = Path.home().as_posix() + written[1:]
    names = _names(written)
    start: _Start = "working directory"
    if _is_absolute(written):
        start = "full path"
    elif len(names) == 1 and names != ("..",) and not written.startswith("./"):
        # One name alone is looked for at every depth.
        start = "every depth"
        names = ("**", *names)
    elif names[:1] == ("**",):
        start = "every depth"
    # What speaks about a directory speaks about all that is in it.
    names = (*names, "**")
    parts: list[str | None] = []
    for name in names:
        part = None if name == "**" else _folded(name)
        if part is not None or not parts or parts[-1] is not None:
            # Of several parts for any number of names in a row, one is enough.
            parts.append(part)
    return tuple(parts), start


def _path_names(path: str) -> tuple[str, ...]:
    return _names(_folded(path))


def _steps_back(names: Sequence[str | None]) -> int:
    """Return how many times a path or a pattern begins with two dots."""
    return len(list(itertools.takewhile(lambda name: name == "..", names)))


def fits_file(pattern: str, place: Place) -> bool:
    """Tell whether a pattern for files speaks about a file."""
    parts, start = _file_pattern(pattern)
    relative = None if place.relative is None else _path_names(place.relative)
    inside = relative is not None and relative[:1] != ("..",)
    if start == "full path" or (start == "every depth" and not inside):
        return sequence_fits(parts, _path_names(place.absolute), wild)
    if relative is None:
        return False
    # Two dots stand for themselves: no star takes a step back.
    back = _steps_back(relative)
    return _steps_back(parts) == back and sequence_fits(parts[back:], relative[back:], wild)


def _speaks_of(rule: Rule, name: str, effect: Effect) -> bool:
    """Tell whether a rule is about a tool, by its name or by what it does."""
    return wild(rule.tool, name) or (rule.tool == effect and effect in _CLASSES)


def covers_command(rule: Rule, name: str, command: str) -> bool:
    """Tell whether a rule speaks about a command that a tool is to run.

    Args:
        rule: The rule.
        name: The name of the tool.
        command: The command, or a part of it that is a command by itself.
    """
    if not _speaks_of(rule, name, "run"):
        return False
    return rule.pattern is None or wild(rule.pattern, command.strip())


def covers_file(rule: Rule, name: str, effect: Effect, place: Place | None) -> bool:
    """Tell whether a rule speaks about what a tool does to a file.

    Args:
        rule: The rule.
        name: The name of the tool.
        effect: What the tool does.
        place: Where the file is, or ``None`` for a call that names no file.
            Only a rule without a pattern speaks about such a call.
    """
    if not _speaks_of(rule, name, effect):
        return False
    if rule.pattern is None:
        return True
    return place is not None and fits_file(rule.pattern, place)
