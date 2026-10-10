"""Kinds of subagent: what one is on disk, and where they are looked for.

An agent can hand a task to a subagent, see :mod:`columbina.subagents`. A
kind of subagent says what such a helper is for and what it works with. It
is a file of Markdown whose name is the name of the kind. The file begins
with a few lines of YAML between two lines of three dashes, and what
follows them is the system prompt of a subagent of that kind. ::

    .columbina/agents/reviewer.md

    ---
    name: reviewer
    description: Reads a change and says what is wrong with it. Changes nothing.
    tools: [read, glob, grep]
    max_steps: 30
    ---
    You review a change that somebody else made. ...

``name`` has to be the name of the file, and ``description`` is what an
agent goes by when it chooses a kind. The other keys may be left out.
``tools`` lists the tools that a subagent of the kind has: without the key
it has all of them, and with an empty list none. ``max_steps`` is how many
steps its one turn may take, and ``model`` the model that it asks. A file
without text after its first lines gives no system prompt.

A key that is not one of these makes the file unusable. Passed over, a key
such as ``tool`` would leave a subagent with every tool where its writer
meant it to have two.

Kinds are looked for in the project and in the home of the program, and of
two of one name the one of the project is the kind. A file in a project is
the work of somebody else. All that it can do is name fewer tools, another
model and a system prompt: what a subagent may do with its tools is decided
elsewhere, and no kind can widen it. A link in a project is not followed,
and one in the home of the program is, see :mod:`columbina.places`.

A kind that cannot be used does not stand in the way of the others. Why is
written down, in words for whoever wrote the file.

Nothing here knows of agents. It reads, and says what it found.
"""

from collections.abc import Collection, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from columbina import frontmatter, places
from columbina.frontmatter import FrontMatterError
from columbina.fs import bound
from columbina.places import Root, RootError

__all__ = [
    "MAX_BYTES",
    "SUFFIX",
    "Found",
    "Invalid",
    "Scan",
    "SubagentType",
    "SubagentTypeError",
    "merged",
    "parsed",
    "roots",
    "scan",
    "usable",
]

SUFFIX = ".md"
"""What the name of a file ends with that holds a kind of subagent."""

MAX_BYTES = 64 * 1024
"""How long such a file may be. What follows its first lines is a system prompt."""

_MAX_ENTRIES = 1000
"""How many entries of a root are looked at."""

_KEYS = ("name", "description", "tools", "max_steps", "model")
_MEANT = {
    "name": "name",
    "description": "description",
    "tools": "tools",
    "tool": "tools",
    "allowedtools": "tools",
    "maxsteps": "max_steps",
    "maxturns": "max_steps",
    "model": "model",
}
"""The key that a spelling was meant to be, going by its letters alone."""

_NAMED = 5
"""How many of the tools that are not there are named."""


class SubagentTypeError(Exception):
    """A kind of subagent cannot be used. The text says why, for whoever wrote its file."""


@dataclass(frozen=True, slots=True)
class SubagentType:
    """A kind of subagent that was found.

    Attributes:
        name: What it is called.
        description: What it is for, as its file says.
        prompt: The system prompt of a subagent of this kind, or ``None`` if
            the file gives none.
        tools: The names of the tools that such a subagent has, or ``None``
            if the file does not say, which means all of them.
        max_steps: How many steps the turn of such a subagent may take, or
            ``None`` if the file does not say.
        model: The model that such a subagent asks, or ``None`` if the file
            does not say.
        file: Where the file lies, with every link followed.
        root: The root that it was found in.
    """

    name: str
    description: str
    prompt: str | None
    tools: tuple[str, ...] | None
    max_steps: int | None
    model: str | None
    file: Path
    root: Root


@dataclass(frozen=True, slots=True)
class Invalid:
    """A file in a root that looks like a kind of subagent and cannot be used or read.

    Attributes:
        file: Where it lies.
        root: The root that it was found in.
        reason: Why it cannot be used.
    """

    file: Path
    root: Root
    reason: str


@dataclass(frozen=True, slots=True)
class Scan:
    """What one root holds.

    Attributes:
        root: The root.
        types: The kinds in it, by name.
        invalid: What looks like a kind and cannot be used.
    """

    root: Root
    types: tuple[SubagentType, ...] = ()
    invalid: tuple[Invalid, ...] = ()


@dataclass(frozen=True, slots=True)
class Found:
    """What all roots hold together.

    Attributes:
        types: The kinds, by name. Of two of one name, this is the one whose
            root comes first.
        shadowed: The kinds that were passed over for one of the same name.
        invalid: What looks like a kind and cannot be used.
    """

    types: Mapping[str, SubagentType]
    shadowed: tuple[SubagentType, ...] = ()
    invalid: tuple[Invalid, ...] = ()


def _meant(key: object) -> str | None:
    """Return the key that a spelling is a slip of the pen for, or ``None`` if it is none."""
    if not isinstance(key, str):
        return None
    return _MEANT.get(key.lower().replace("-", "").replace("_", ""))


def _tools(fields: Mapping[object, object]) -> tuple[str, ...] | None:
    if "tools" not in fields:
        return None
    value = fields["tools"]
    if not isinstance(value, list):
        # Left as all tools, a slip here would hand a subagent what it was not to have.
        raise SubagentTypeError(
            f"the key 'tools' gives {frontmatter.shown(value)}, which is no list of the names "
            "of tools. Write it like [read, grep], or leave the key out for all tools"
        )
    names: list[str] = []
    for one in cast("list[object]", value):
        if not isinstance(one, str) or not one.strip():
            raise SubagentTypeError(
                f"the key 'tools' holds {frontmatter.shown(one)}, which is no name of a tool"
            )
        if one not in names:
            names.append(one)
    return tuple(names)


def _steps(fields: Mapping[object, object]) -> int | None:
    if "max_steps" not in fields:
        return None
    value = fields["max_steps"]
    # True is a number to Python, and no number of steps to anybody.
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise SubagentTypeError(
            f"the key 'max_steps' gives {frontmatter.shown(value)}, which is no whole number "
            "above zero"
        )
    return value


def parsed(content: bytes, name: str, file: Path, root: Root) -> SubagentType:
    """Read the file of a kind of subagent.

    Args:
        content: What the file holds.
        name: What the file is called in its root, without what its name
            ends with.
        file: Where the file lies.
        root: The root that it was found in.

    Raises:
        SubagentTypeError: The file is not one of a kind of that name.
    """
    try:
        fields, text = frontmatter.read(content, name + SUFFIX)
        for key in fields:
            if key in _KEYS:
                continue
            meant = _meant(key)
            hint = (
                f"The keys of a kind of subagent are: {', '.join(_KEYS)}"
                if meant is None
                else f"What is meant is spelled {meant!r}"
            )
            raise SubagentTypeError(f"the key {frontmatter.shown(key)} is not known. {hint}")
        given = frontmatter.name(fields)
        if given != name:
            raise SubagentTypeError(
                f"the file calls the kind {given!r}, and by its own name it would be {name!r}. "
                "The two have to be the same"
            )
        description = frontmatter.text(fields, "description")
        model = frontmatter.text(fields, "model") if "model" in fields else None
        prompt = text if text.strip() else None
        return SubagentType(
            name, description, prompt, _tools(fields), _steps(fields), model, file, root
        )
    except FrontMatterError as error:
        # What is wrong with the beginning of the file is what is wrong with the kind.
        raise SubagentTypeError(str(error)) from None


def _entry(there: Path, root: Root, entry: bound.Listed) -> SubagentType | None:
    """Return the kind that an entry of a root is, or ``None`` if it is none.

    Raises:
        SubagentTypeError: The entry looks like a kind and cannot be used.
        OSError: It cannot be read.
    """
    if not entry.name.endswith(SUFFIX) or entry.name.startswith("."):
        # Not a file of a kind, or one that a program left there for itself.
        return None
    if entry.kind not in ("file", "link"):
        return None
    try:
        file = places.target(root, there, entry)
        opened = places.opened(root, file)
    except RootError as error:
        raise SubagentTypeError(str(error)) from None
    except bound.NotAFileError:
        raise SubagentTypeError("it is not a file") from None
    if opened is None:
        # Gone since the root was listed.
        return None
    with opened:
        # One byte more than is allowed is asked for, so a file that grows while it is read
        # is found out as well.
        content = opened.read(MAX_BYTES + 1)
    if len(content) > MAX_BYTES:
        raise SubagentTypeError(f"it has more than {MAX_BYTES} bytes")
    # It is called what its entry is called. What a link leads to may be called anything.
    return parsed(content, entry.name.removesuffix(SUFFIX), file, root)


def scan(root: Root) -> Scan:
    """Look for kinds of subagent in one root.

    A root that is not there holds nothing. A file that cannot be used or
    cannot be read is written down, and the ones beside it are looked at all
    the same.

    Raises:
        OSError: The root is there and cannot be looked into. That may pass,
            so whoever asks decides what to go by meanwhile.
    """
    try:
        looked = places.listed(root, _MAX_ENTRIES)
    except RootError as error:
        return Scan(root, invalid=(Invalid(root.path, root, str(error)),))
    if looked is None:
        return Scan(root)
    base, entries, more = looked
    types: list[SubagentType] = []
    invalid: list[Invalid] = []
    if more:
        reason = f"it holds more than {_MAX_ENTRIES} entries, and the rest is not looked at"
        invalid.append(Invalid(base, root, reason))
    for entry in entries:
        there = base / entry.name
        try:
            kind = _entry(there, root, entry)
        except SubagentTypeError as error:
            invalid.append(Invalid(there, root, str(error)))
        except OSError as error:
            # One file that cannot be read is no reason to pass over the ones beside it.
            invalid.append(Invalid(there, root, f"it cannot be read: {error.strerror or error}"))
        else:
            if kind is not None:
                types.append(kind)
    return Scan(root, tuple(types), tuple(invalid))


def usable(found: Scan, tools: Collection[str]) -> Scan:
    """Return what a root holds, without the kinds that name tools which are not there.

    A kind is written for the tools that it names. Without one of them a
    subagent of the kind would be something else than its writer meant, so
    the kind is told as one that cannot be used.

    Args:
        found: What the root holds.
        tools: The names of the tools that there are.
    """
    types: list[SubagentType] = []
    invalid = list(found.invalid)
    for kind in found.types:
        missing = [name for name in kind.tools or () if name not in tools]
        if not missing:
            types.append(kind)
            continue
        named = ", ".join(frontmatter.shown(name) for name in missing[:_NAMED])
        more = len(missing) - _NAMED
        rest = f", and {more} more" if more > 0 else ""
        what = "a tool that is" if len(missing) == 1 else "tools that are"
        invalid.append(Invalid(kind.file, found.root, f"it names {what} not there: {named}{rest}"))
    return Scan(found.root, tuple(types), tuple(invalid))


def merged(scans: Sequence[Scan]) -> Found:
    """Put together what the roots hold, the first root first."""
    types: dict[str, SubagentType] = {}
    shadowed: list[SubagentType] = []
    invalid: list[Invalid] = []
    for found in scans:
        invalid.extend(found.invalid)
        for kind in found.types:
            if kind.name in types:
                shadowed.append(kind)
            else:
                types[kind.name] = kind
    return Found(dict(sorted(types.items())), tuple(shadowed), tuple(invalid))


def roots(project: Path, home: Path | None) -> tuple[Root, ...]:
    """Return the places where kinds of subagent are looked for, in the order in which they count.

    Args:
        project: The root of the project, with no link on its way: what
            :func:`columbina.fs.locate` gives for a working directory, or a
            directory above it.
        home: The directory of kinds in the home of Columbina, if there is one.
    """
    found = [Root(project / ".columbina" / "agents", "project", own=False)]
    if home is not None:
        found.append(Root(home, "home", own=True))
    return tuple(found)
