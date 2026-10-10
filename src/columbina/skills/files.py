"""Skill files: what a skill is on disk, and where skills are looked for.

A skill is a directory with a file ``SKILL.md`` in it. The file begins with a
few lines of YAML between two lines of three dashes. They say what the skill
is called and what it is for. What follows them is the skill itself: text
for a model that is about to do the kind of task that the skill is for.
Other files may lie beside it, and the text may point to them. ::

    release-notes/
        SKILL.md
        codenames.txt

Skills are looked for in several places, the roots, in a fixed order. A
directory in a root is a skill if it holds a ``SKILL.md``. The name of the
directory is the name of the skill, and the file has to say the same name.
Of two skills of one name, the one whose root comes first is the skill. The
other one is passed over, and that is written down.

A root of a project is taken as it lies there. A link in it is not
followed, be it the root itself, the directory of a skill or the file: what a
repository holds was written by somebody else, and a link could lead to a
place that is nobody's business. A root of the user is the user's own, and
links in it are followed.

A skill that cannot be used does not stand in the way of the others, and
neither does one that cannot be read. Why is written down, in words for
whoever wrote the skill. Those words show little of what the file holds: a
file of a few lines can give a value that fills a disk when it is written
out.

Nothing here knows of agents or of tools. It reads, and says what it found.
"""

import os
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import yaml

from columbina.fs import bound

__all__ = [
    "FILE",
    "MAX_BYTES",
    "MAX_NAME",
    "Found",
    "Invalid",
    "Root",
    "Scan",
    "Skill",
    "SkillError",
    "body",
    "merged",
    "parsed",
    "roots",
    "scan",
]

FILE = "SKILL.md"
"""The name of the file that makes a directory a skill."""

MAX_BYTES = 1024 * 1024
"""How long a skill file may be."""

MAX_NAME = 64
"""How long the name of a skill may be."""

_MAX_ENTRIES = 1000
"""How many entries of a root are looked at."""

_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
_DASHES = "---"
_MODEL = "disable-model-invocation"
_USER = "user-invocable"
_MEANT = {"disablemodelinvocation": _MODEL, "modelinvocable": _MODEL, "userinvocable": _USER}
"""The key that a spelling was meant to be, going by its letters alone."""

_SHOWN = 60
"""How many characters of a value from a file are shown to whoever wrote it."""

_SAID = 300
"""How many characters of what the reader of YAML says are passed on."""

_LINK = "it lies behind a link, and links are not followed in a project"


class SkillError(Exception):
    """A skill cannot be used. The text says why, for whoever wrote the skill."""


@dataclass(frozen=True, slots=True)
class Root:
    """A place where skills are looked for.

    Attributes:
        path: The directory whose directories are skills.
        label: What the place is called before a person.
        own: Whether the place is the user's own. Links are followed there,
            and nowhere else.
    """

    path: Path
    label: str
    own: bool


@dataclass(frozen=True, slots=True)
class Skill:
    """A skill that was found.

    Attributes:
        name: What it is called.
        description: What it is for, as its file says.
        directory: Where it lies, with every link followed.
        root: The root that it was found in.
        model_invocable: Whether a model may load it by itself.
        user_invocable: Whether a person may hand it to the model.
    """

    name: str
    description: str
    directory: Path
    root: Root
    model_invocable: bool = True
    user_invocable: bool = True


@dataclass(frozen=True, slots=True)
class Invalid:
    """Something in a root that looks like a skill and cannot be used, or cannot be read.

    Attributes:
        directory: Where it lies.
        root: The root that it was found in.
        reason: Why it cannot be used.
    """

    directory: Path
    root: Root
    reason: str


@dataclass(frozen=True, slots=True)
class Scan:
    """What one root holds.

    Attributes:
        root: The root.
        skills: The skills in it, by name.
        invalid: What looks like a skill and cannot be used.
    """

    root: Root
    skills: tuple[Skill, ...] = ()
    invalid: tuple[Invalid, ...] = ()


@dataclass(frozen=True, slots=True)
class Found:
    """What all roots hold together.

    Attributes:
        skills: The skills, by name. Of two of one name, this is the one
            whose root comes first.
        shadowed: The skills that were passed over for one of the same name.
        invalid: What looks like a skill and cannot be used.
    """

    skills: Mapping[str, Skill]
    shadowed: tuple[Skill, ...] = ()
    invalid: tuple[Invalid, ...] = ()


def _split(text: str) -> tuple[str, str]:
    """Take a skill file apart: the lines between the dashes, and what follows them."""
    lines = text.split("\n")
    if lines[0].rstrip() != _DASHES:
        raise SkillError(f"{FILE} does not begin with a line of three dashes")
    for at in range(1, len(lines)):
        if lines[at].rstrip() == _DASHES:
            return "\n".join(lines[1:at]), "\n".join(lines[at + 1 :])
    raise SkillError(f"the lines at the beginning of {FILE} are not closed by three dashes")


def _cut(text: str, limit: int) -> str:
    return text if len(text) <= limit else f"{text[:limit]}..."


def _shown(value: object) -> str:
    """Return what a file gives in a few characters, whatever it is."""
    # Lists that hold one another take a few lines in a file and no end of them written out.
    if isinstance(value, list):
        return "a list"
    if isinstance(value, dict):
        return "keys and values of its own"
    return _cut(repr(value), _SHOWN)


def _meant(key: object) -> str | None:
    """Return the key that a spelling is a slip of the pen for, or ``None`` if it is none.

    A slip is one of the two keys that say who can reach a skill, written in
    another case, with other marks between its words, or with a k. Passed
    over like a key that another tool knows, it would change who can reach
    the skill without a word.
    """
    if not isinstance(key, str):
        return None
    letters = key.lower().replace("-", "").replace("_", "").replace("invok", "invoc")
    meant = _MEANT.get(letters)
    return None if key == meant else meant


def _text(fields: Mapping[object, object], key: str) -> str:
    """Return the text that a key gives, which has to be there and to say something."""
    if key not in fields:
        raise SkillError(f"the key {key!r} is missing")
    value = fields[key]
    if value is None or (isinstance(value, str) and not value.strip()):
        raise SkillError(f"the key {key!r} says nothing")
    if not isinstance(value, str):
        # YAML reads digits as a number, and yes as true. Quotes make a text of them.
        raise SkillError(
            f"the key {key!r} gives {_shown(value)}, which is no text. Put it in quotes"
        )
    return value


def _flag(fields: Mapping[object, object], key: str, default: bool) -> bool:
    value = fields.get(key, default)
    if not isinstance(value, bool):
        raise SkillError(f"the key {key!r} takes true or false, not {_shown(value)}")
    return value


def _followed(path: Path) -> Path:
    """Return where a path leads, with every link followed.

    Raises:
        OSError: Nothing is there, or the links on the way go in a circle.
    """
    # Not ``Path.resolve``: before Python 3.13 it answers a circle of links with an error
    # that is no ``OSError``, and nobody who reads files is ready for that one.
    return Path(os.path.realpath(path, strict=True))


def parsed(content: bytes, directory: str) -> tuple[str, bool, bool, str]:
    """Read a skill file.

    Args:
        content: What the file holds.
        directory: The name of the directory that the file lies in.

    Returns:
        The description, whether a model may load the skill by itself,
        whether a person may hand it to the model, and the text of the
        skill without the empty lines around it.

    Raises:
        SkillError: The file is not one of a skill of that name.
    """
    try:
        text = content.decode("utf-8-sig")
    except UnicodeDecodeError:
        raise SkillError(f"{FILE} is not written in UTF-8") from None
    front, rest = _split(text.replace("\r\n", "\n"))
    try:
        loaded: object = yaml.safe_load(front)
    except RecursionError:
        raise SkillError(f"the lines at the beginning of {FILE} are nested too deeply") from None
    except yaml.YAMLError as error:
        said = _cut(" ".join(str(error).split()), _SAID)
        raise SkillError(f"the lines at the beginning of {FILE} are not YAML: {said}") from None
    except Exception as error:
        # The reader makes values of what it reads, and a value that cannot be made fails
        # with whatever the making of it raises: a date that there is none of, a number of
        # thousands of digits, a tag before something that it does not go with.
        said = _cut(" ".join(str(error).split()), _SAID)
        raise SkillError(
            f"the lines at the beginning of {FILE} give a value that cannot be read: {said}"
        ) from None
    if loaded is None:
        loaded = {}
    if not isinstance(loaded, dict):
        raise SkillError(f"the lines at the beginning of {FILE} do not give keys and values")
    fields = cast("dict[object, object]", loaded)
    for key in fields:
        meant = _meant(key)
        if meant is not None:
            raise SkillError(
                f"the key {_shown(key)} is not known. What is meant is spelled {meant!r}"
            )
    name = _text(fields, "name")
    if len(name) > MAX_NAME or _NAME.fullmatch(name) is None:
        raise SkillError(
            f"the name {_shown(name)} cannot be used: a name is made of lower-case letters, "
            f"digits and single dashes between them, and has {MAX_NAME} characters at most"
        )
    if name != directory:
        raise SkillError(
            f"the file calls the skill {name!r} and its directory is called {directory!r}. "
            "The two have to be the same"
        )
    description = _text(fields, "description")
    model = not _flag(fields, _MODEL, default=False)
    return description, model, _flag(fields, _USER, default=True), rest.strip("\n")


def _read(directory: Path, root: Root, name: str) -> tuple[Skill, str] | None:
    """Read the skill in a directory, or return ``None`` if the directory is no skill.

    Args:
        directory: Where the skill lies, with every link followed.
        root: The root that it was found in.
        name: What the directory is called in that root. Behind a link it
            may be called something else, and that name does not count.

    Raises:
        SkillError: Something is there that looks like a skill and cannot be used.
        OSError: The file is there and cannot be read.
    """
    file = directory / FILE
    try:
        if root.own:
            # The user's own: a file that is a link is read where it leads.
            file = _followed(file)
        opened, _ = bound.open_for_reading(file)
    except (FileNotFoundError, NotADirectoryError):
        # No such file, or the link that was followed leads to a file and not to a directory.
        return None
    except bound.NotAFileError:
        raise SkillError(f"{FILE} is not a file") from None
    except bound.MovedError:
        if root.own:
            # Its links were followed a moment before, so somebody is at work there.
            raise
        raise SkillError(_LINK) from None
    with opened:
        # One byte more than is allowed is asked for, so a file that grows while it is read
        # is found out as well.
        content = opened.read(MAX_BYTES + 1)
    if len(content) > MAX_BYTES:
        raise SkillError(f"{FILE} has more than {MAX_BYTES} bytes")
    description, model, user, text = parsed(content, name)
    return Skill(name, description, directory, root, model, user), text


def _entry(there: Path, root: Root, entry: bound.Listed) -> Skill | None:
    """Return the skill that an entry of a root is, or ``None`` if it is none.

    Raises:
        SkillError: The entry looks like a skill and cannot be used.
        OSError: It cannot be read.
    """
    directory = there
    if entry.kind == "link":
        if not root.own:
            raise SkillError(_LINK)
        try:
            directory = _followed(there)
        except FileNotFoundError:
            raise SkillError("it is a link that leads nowhere") from None
    elif entry.kind != "directory":
        return None
    read = _read(directory, root, entry.name)
    return None if read is None else read[0]


def scan(root: Root) -> Scan:
    """Look for skills in one root.

    A root that is not there holds nothing. An entry that cannot be used or
    cannot be read is written down, and the ones beside it are looked at all
    the same.

    Raises:
        OSError: The root is there and cannot be looked into. That may pass,
            so whoever asks decides what to go by meanwhile.
    """
    base = root.path
    try:
        if root.own:
            base = _followed(base)
        entries, more = bound.listing(base, _MAX_ENTRIES)
    except (FileNotFoundError, NotADirectoryError):
        return Scan(root)
    except bound.MovedError:
        if root.own:
            # Its links were followed a moment before, so somebody is at work there.
            raise
        return Scan(root, invalid=(Invalid(base, root, _LINK),))
    skills: list[Skill] = []
    invalid: list[Invalid] = []
    if more:
        reason = f"it holds more than {_MAX_ENTRIES} entries, and the rest is not looked at"
        invalid.append(Invalid(base, root, reason))
    for entry in entries:
        there = base / entry.name
        try:
            skill = _entry(there, root, entry)
        except SkillError as error:
            invalid.append(Invalid(there, root, str(error)))
        except OSError as error:
            # One skill that cannot be read is no reason to pass over the ones beside it.
            invalid.append(Invalid(there, root, f"it cannot be read: {error.strerror or error}"))
        else:
            if skill is not None:
                skills.append(skill)
    return Scan(root, tuple(skills), tuple(invalid))


def merged(scans: Sequence[Scan]) -> Found:
    """Put together what the roots hold, the first root first."""
    skills: dict[str, Skill] = {}
    shadowed: list[Skill] = []
    invalid: list[Invalid] = []
    for found in scans:
        invalid.extend(found.invalid)
        for skill in found.skills:
            if skill.name in skills:
                shadowed.append(skill)
            else:
                skills[skill.name] = skill
    return Found(dict(sorted(skills.items())), tuple(shadowed), tuple(invalid))


def body(skill: Skill) -> tuple[Skill, str]:
    """Read a skill anew, and return it as it is now with its text.

    Raises:
        SkillError: The skill is no longer there, or can no longer be used.
        OSError: Its file cannot be read.
    """
    read = _read(skill.directory, skill.root, skill.name)
    if read is None:
        raise SkillError(f"{FILE} is no longer there")
    return read


def roots(
    project: Path, home: Path | None, user: Path | None, extra: Sequence[Path]
) -> tuple[Root, ...]:
    """Return the places where skills are looked for, in the order in which they count.

    Args:
        project: The root of the project, with no link on its way: what
            :func:`columbina.fs.locate` gives for a working directory, or a
            directory above it.
        home: The directory of skills in the home of Columbina, if there is one.
        user: The directory of skills that the user shares between tools, if
            there is one.
        extra: Further directories, which count last.
    """
    found = [
        Root(project / ".columbina" / "skills", "project", own=False),
        Root(project / ".agents" / "skills", "project, .agents", own=False),
    ]
    if home is not None:
        found.append(Root(home, "home", own=True))
    if user is not None:
        found.append(Root(user, "user, .agents", own=True))
    found.extend(Root(path, "extra", own=True) for path in extra)
    return tuple(found)
