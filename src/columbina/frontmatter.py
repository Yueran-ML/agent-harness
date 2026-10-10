"""Files that begin with keys and values, and go on with text for a model.

A skill is written as such a file, and so is a kind of subagent. The file
begins with a few lines of YAML between two lines of three dashes. They say
what the thing is called and what it is for. What follows them is the text. ::

    ---
    name: release-notes
    description: Writes the notes of a release from the commits since the last one.
    ---
    Read the commits since the last tag, and sort them by what they change.

Such a file may be the work of somebody else: it lies in a repository that
was cloned. So it is read with care. What is wrong with it is said in words
for whoever wrote it, and those words show little of what the file holds: a
file of a few lines can give a value that fills a disk when it is written
out. Whatever the reader of YAML makes of a file, it ends here as a value or
as an error of this module, and as nothing else.

Nothing here knows what the keys of a skill or of a kind of subagent are.
This reads the lines, and hands out what a key gives if it is what the key
is to give.
"""

import re
from collections.abc import Mapping
from typing import cast

import yaml

__all__ = ["MAX_NAME", "FrontMatterError", "cut", "flag", "name", "read", "shown", "text"]

MAX_NAME = 64
"""How long a name may be that a file gives itself."""

_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
_DASHES = "---"

_SHOWN = 60
"""How many characters of a value from a file are shown to whoever wrote it."""

_SAID = 300
"""How many characters of what the reader of YAML says are passed on."""


class FrontMatterError(Exception):
    """A file is not what it is to be. The text says why, for whoever wrote the file."""


def _split(content: str, file: str) -> tuple[str, str]:
    """Take a file apart: the lines between the dashes, and what follows them."""
    lines = content.split("\n")
    if lines[0].rstrip() != _DASHES:
        raise FrontMatterError(f"{file} does not begin with a line of three dashes")
    for at in range(1, len(lines)):
        if lines[at].rstrip() == _DASHES:
            return "\n".join(lines[1:at]), "\n".join(lines[at + 1 :])
    raise FrontMatterError(f"the lines at the beginning of {file} are not closed by three dashes")


def cut(given: str, limit: int) -> str:
    """Return a text, or its beginning if it is longer than it may be."""
    return given if len(given) <= limit else f"{given[:limit]}..."


def shown(value: object) -> str:
    """Return what a file gives in a few characters, whatever it is."""
    # Lists that hold one another take a few lines in a file and no end of them written out.
    if isinstance(value, list):
        return "a list"
    if isinstance(value, dict):
        return "keys and values of its own"
    return cut(repr(value), _SHOWN)


def read(content: bytes, file: str) -> tuple[dict[object, object], str]:
    """Read a file: the keys and values at its beginning, and the text that follows them.

    Args:
        content: What the file holds.
        file: What the file is called before whoever wrote it.

    Returns:
        What the keys give, and the text without the empty lines around it.

    Raises:
        FrontMatterError: The file does not begin with lines that give keys
            and values.
    """
    try:
        decoded = content.decode("utf-8-sig")
    except UnicodeDecodeError:
        raise FrontMatterError(f"{file} is not written in UTF-8") from None
    front, rest = _split(decoded.replace("\r\n", "\n"), file)
    try:
        loaded: object = yaml.safe_load(front)
    except RecursionError:
        raise FrontMatterError(
            f"the lines at the beginning of {file} are nested too deeply"
        ) from None
    except yaml.YAMLError as error:
        said = cut(" ".join(str(error).split()), _SAID)
        raise FrontMatterError(
            f"the lines at the beginning of {file} are not YAML: {said}"
        ) from None
    except Exception as error:
        # The reader makes values of what it reads, and a value that cannot be made fails
        # with whatever the making of it raises: a date that there is none of, a number of
        # thousands of digits, a tag before something that it does not go with.
        said = cut(" ".join(str(error).split()), _SAID)
        raise FrontMatterError(
            f"the lines at the beginning of {file} give a value that cannot be read: {said}"
        ) from None
    if loaded is None:
        loaded = {}
    if not isinstance(loaded, dict):
        raise FrontMatterError(f"the lines at the beginning of {file} do not give keys and values")
    return cast("dict[object, object]", loaded), rest.strip("\n")


def text(fields: Mapping[object, object], key: str) -> str:
    """Return the text that a key gives, which has to be there and to say something.

    Raises:
        FrontMatterError: The key is missing, or gives no text.
    """
    if key not in fields:
        raise FrontMatterError(f"the key {key!r} is missing")
    value = fields[key]
    if value is None or (isinstance(value, str) and not value.strip()):
        raise FrontMatterError(f"the key {key!r} says nothing")
    if not isinstance(value, str):
        # YAML reads digits as a number, and yes as true. Quotes make a text of them.
        raise FrontMatterError(
            f"the key {key!r} gives {shown(value)}, which is no text. Put it in quotes"
        )
    return value


def flag(fields: Mapping[object, object], key: str, default: bool) -> bool:
    """Return the yes or no that a key gives, or what holds if the key is not there.

    Raises:
        FrontMatterError: The key gives something else.
    """
    value = fields.get(key, default)
    if not isinstance(value, bool):
        raise FrontMatterError(f"the key {key!r} takes true or false, not {shown(value)}")
    return value


def name(fields: Mapping[object, object]) -> str:
    """Return the name that a file gives itself.

    A name is told to a model and written by one, and it is the name of a
    file or of a directory. So it is kept to what all of them take.

    Raises:
        FrontMatterError: There is no name, or it is none that can be used.
    """
    given = text(fields, "name")
    if len(given) > MAX_NAME or _NAME.fullmatch(given) is None:
        raise FrontMatterError(
            f"the name {shown(given)} cannot be used: a name is made of lower-case letters, "
            f"digits and single dashes between them, and has {MAX_NAME} characters at most"
        )
    return given
