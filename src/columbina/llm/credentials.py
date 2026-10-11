"""API keys: what one looks like, and the file in which a user keeps the ones that were typed in.

What a key looks like
    A key reaches a service as part of a header of the request. What is
    found where a key is kept is not always a key as its service issued it:
    it was copied by hand, and blanks, a line break or a mark of the text
    that it was copied from may have come along. :func:`trimmed` and
    :func:`sendable` are asked of every key before it is sent.

The file of keys
    A user who does not want to set a variable types a key in once, and it
    is kept in one file, by the name of the service that it is for::

        {"version": 1, "keys": {"deepseek": "sk-..."}}

    A key lies in that file as it is. It is not encrypted, so whoever can
    read the file has the keys, and the file has to be its user's alone. On
    POSIX it is made with the mode 600, and a file that the group or the
    others can read is not used. Windows has no such modes. The access
    rules of the directory count there, and those of a user's own directory
    keep other users out.

    The file is written whole or not at all: next to the old one first, and
    then put in its place.

Nothing here says a key. An error names the file and what is wrong with it,
and never what the file holds.
"""

import contextlib
import json
import os
import secrets
import stat
import sys
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, ValidationError

__all__ = ["CredentialsError", "forget", "keep", "key_for", "sendable", "stored", "trimmed"]

_HAS_MODES = sys.platform != "win32"
"""Whether a file has a mode that says who may read it."""


class CredentialsError(Exception):
    """The file of stored keys cannot be used, or a key cannot be kept in it."""


class _Kept(BaseModel):
    """What the file of keys holds."""

    model_config = ConfigDict(extra="forbid")

    version: Literal[1]
    keys: dict[str, str]


def trimmed(key: str) -> str:
    """Return a key without the blanks and line breaks around it.

    They are no part of any key, and they come along easily: a line break at
    the end of a file that holds the key, or a blank at the end of the line
    that sets a variable.
    """
    return key.strip()


def sendable(key: str) -> bool:
    """Whether a key can be sent as it is.

    A key that can is made of the visible characters of ASCII and of nothing
    else. That is all that the header of a request carries, and all that the
    keys of the services are made of. Anything else in a key shows that it
    was not copied whole or not copied alone: a blank or a line break in the
    middle, a quotation mark of a word processor, a mark that cannot be seen.

    Args:
        key: The key, already trimmed.
    """
    return bool(key) and all("!" <= character <= "~" for character in key)


def _too_open(mode: int) -> bool:
    """Whether a file of this mode can be read or changed by its group or by the others."""
    return bool(stat.S_IMODE(mode) & 0o077)


def _read(path: Path, *, whoever_can_read_it: bool) -> dict[str, str]:
    """Return the keys of the file, or none if there is no file.

    Args:
        path: The file.
        whoever_can_read_it: Whether to read a file that is not its user's
            alone. One who is about to write the file anew may: the new file
            is made for its user alone.
    """
    try:
        with path.open("rb") as file:
            # Asked of the file that is open, and not of its name: what is read is then the
            # file that the answer was about.
            mode = os.fstat(file.fileno()).st_mode
            if _HAS_MODES and not whoever_can_read_it and _too_open(mode):
                raise CredentialsError(
                    f"the file of stored keys, {path}, can be read by other users of this "
                    f"machine, so it is not used. Make it yours alone with: chmod 600 {path}"
                )
            raw = file.read()
    except FileNotFoundError:
        return {}
    except OSError as error:
        raise CredentialsError(
            f"the file of stored keys, {path}, cannot be read: {error.strerror}"
        ) from None
    kept: _Kept | None = None
    with contextlib.suppress(ValidationError):
        kept = _Kept.model_validate_json(raw)
    if kept is None:
        # Said out here and in words of its own. The error that was caught repeats what was
        # read, and so it is neither quoted nor held on to.
        raise CredentialsError(
            f"the file of stored keys, {path}, is not what 'columbina setup' writes, so it is "
            "not used. Delete it, and enter the keys again with 'columbina setup'"
        )
    return kept.keys


def _write(path: Path, keys: dict[str, str]) -> None:
    """Put a file of these keys in the place of the one that is there, whole or not at all."""
    text = json.dumps({"version": 1, "keys": keys}, indent=2, sort_keys=True) + "\n"
    beside = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        # Made anew, and for its user alone before a key is written into it.
        descriptor = os.open(beside, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as file:
                file.write(text)
            beside.replace(path)
        finally:
            beside.unlink(missing_ok=True)
    except OSError as error:
        raise CredentialsError(
            f"the file of stored keys, {path}, cannot be written: {error.strerror}"
        ) from None


def stored(path: Path) -> dict[str, str]:
    """Return the keys that the file holds, by the name of the service that each is for.

    Args:
        path: The file of keys.

    Returns:
        The keys, as they lie in the file. There are none if there is no file.

    Raises:
        CredentialsError: The file cannot be read, is not what
            ``columbina setup`` writes, or is not its user's alone.
    """
    return _read(path, whoever_can_read_it=False)


def key_for(path: Path, provider: str) -> str | None:
    """Return the key that is stored for a service, or ``None`` if none is.

    Raises:
        CredentialsError: The file cannot be read, is not what
            ``columbina setup`` writes, or is not its user's alone.
    """
    return stored(path).get(provider)


def keep(path: Path, provider: str, key: str) -> None:
    """Store a key for a service, in the place of the one that is stored for it.

    The file is written anew and for its user alone, whatever it was before.

    Args:
        path: The file of keys.
        provider: The name of the service.
        key: The key. The blanks and line breaks around it are not stored.

    Raises:
        CredentialsError: There is no key or it cannot be sent as it is,
            the file that is there cannot be read or is not what
            ``columbina setup`` writes, or the file cannot be written.
    """
    key = trimmed(key)
    if not key:
        raise CredentialsError("there is no key to keep: what was given is empty")
    if not sendable(key):
        raise CredentialsError(
            "this cannot be a key: there is a character in it that no key has, such as a "
            "blank, a line break or a quotation mark of a word processor"
        )
    _write(path, _read(path, whoever_can_read_it=True) | {provider: key})


def forget(path: Path, provider: str) -> bool:
    """Delete the key that is stored for a service.

    Returns:
        Whether there was one.

    Raises:
        CredentialsError: The file that is there cannot be read or is not
            what ``columbina setup`` writes, or the file cannot be written.
    """
    keys = _read(path, whoever_can_read_it=True)
    if provider not in keys:
        return False
    del keys[provider]
    _write(path, keys)
    return True
