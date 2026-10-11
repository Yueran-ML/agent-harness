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

        {"version": 1,
         "keys": {"deepseek": {"key": "sk-...", "sent_to": "https://api.deepseek.com"}}}

    A key lies in that file as it is. It is not encrypted, so whoever can
    read the file has the keys, and the file has to be its user's alone. On
    POSIX it is made with the mode 600, and a file that the group or the
    others can read is not used. Windows has no such modes. The access
    rules of the directory count there, and those of a user's own directory
    keep other users out.

    The file is written whole or not at all: next to the old one first, and
    then put in its place. Where the file is a link, the file that it leads
    to is the one that is read and the one that is written.

Where a key goes
    A key is stored with the place that it was typed in for: the scheme,
    the machine and the port of the address of its service, see
    :func:`origin_of`. It is handed out for a request to that place and to
    no other. The name of a service says where its requests go only as long
    as nothing changes the address behind the name, and a variable of the
    environment can do that. A key that was typed in for one service is not
    sent to wherever the name leads on another day.

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
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, ValidationError

__all__ = [
    "CredentialsError",
    "forget",
    "keep",
    "key_for",
    "origin_of",
    "sendable",
    "stored",
    "trimmed",
]

_HAS_MODES = sys.platform != "win32"
"""Whether a file has a mode that says who may read it."""

_PORTS = {"http": 80, "https": 443}
"""The port that an address of a scheme leads to where it names none."""


class CredentialsError(Exception):
    """The file of stored keys cannot be used, or a key cannot be kept in it or had from it."""


class _Entry(BaseModel):
    """A key, and where it is sent."""

    model_config = ConfigDict(extra="forbid")

    key: str
    sent_to: str


class _Kept(BaseModel):
    """What the file of keys holds."""

    model_config = ConfigDict(extra="forbid")

    version: Literal[1]
    keys: dict[str, _Entry]


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


def origin_of(address: str) -> str:
    """Return the place that an address leads to: its scheme, its machine and its port.

    Two addresses that lead to the same place give the same text, however
    they are written: the case of the letters, a port that is the usual one
    for the scheme, and whatever path follows do not count. An address that
    cannot be taken apart is its own place, as it is written.

    Args:
        address: The address of a service.
    """
    written = address.strip()
    try:
        parts = urlsplit(written)
        host, port = parts.hostname, parts.port
    except ValueError:
        return written
    # The scheme and the machine come back in small letters.
    scheme = parts.scheme
    if not scheme or not host:
        return written
    if ":" in host:
        host = f"[{host}]"
    if port is None or port == _PORTS.get(scheme):
        return f"{scheme}://{host}"
    return f"{scheme}://{host}:{port}"


def _too_open(mode: int) -> bool:
    """Whether a file of this mode can be read or changed by its group or by the others."""
    return bool(stat.S_IMODE(mode) & 0o077)


def _read(path: Path, *, whoever_can_read_it: bool) -> dict[str, _Entry]:
    """Return what the file holds, or nothing if there is no file.

    Args:
        path: The file.
        whoever_can_read_it: Whether to read a file that is not its user's
            alone. One who is about to write the file anew may: the new file
            is made for its user alone.
    """
    # Opened without waiting. Something that is no file, a pipe for one, would otherwise keep
    # whoever opens it waiting for somebody to write into it.
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
    try:
        descriptor = os.open(path, flags)
        try:
            # Asked of what is open, and not of its name: what is read is then the file that
            # the answers were about. And asked before anything is made of it: a directory
            # can be opened on some systems, and a pipe on all that have them.
            mode = os.fstat(descriptor).st_mode
            if not stat.S_ISREG(mode):
                raise CredentialsError(
                    f"the file of stored keys, {path}, is no file, so it is not used"
                )
            if _HAS_MODES and not whoever_can_read_it and _too_open(mode):
                raise CredentialsError(
                    f"the file of stored keys, {path}, can be read by other users of this "
                    f"machine, so it is not used. Make it yours alone with: chmod 600 {path}"
                )
            with os.fdopen(descriptor, "rb", closefd=False) as file:
                raw = file.read()
        finally:
            os.close(descriptor)
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


def _write(path: Path, keys: dict[str, _Entry]) -> None:
    """Put a file of these keys in the place of the one that is there, whole or not at all."""
    held = {name: entry.model_dump() for name, entry in keys.items()}
    text = json.dumps({"version": 1, "keys": held}, indent=2, sort_keys=True) + "\n"
    try:
        # Where the file is a link, the file that it leads to is written, as it is the one
        # that is read. Put in the place of the link, the new file would leave the old one
        # lying where it was, with every key in it.
        target = path.resolve()
        beside = target.with_name(f".{target.name}.{secrets.token_hex(8)}.tmp")
        target.parent.mkdir(parents=True, exist_ok=True)
        # Made anew, and for its user alone before a key is written into it.
        descriptor = os.open(beside, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as file:
                file.write(text)
            beside.replace(target)
        finally:
            beside.unlink(missing_ok=True)
    except (OSError, RuntimeError) as error:
        why = error.strerror if isinstance(error, OSError) else str(error)
        raise CredentialsError(
            f"the file of stored keys, {path}, cannot be written: {why}"
        ) from None


def stored(path: Path) -> dict[str, str]:
    """Return which services a key is stored for, each with the place that its key is sent to.

    No key is handed out here: whoever only shows what is stored has no use
    for one.

    Args:
        path: The file of keys.

    Returns:
        The place of each key, see :func:`origin_of`, by the name of its
        service. There are none if there is no file.

    Raises:
        CredentialsError: The file cannot be read, is not what
            ``columbina setup`` writes, or is not its user's alone.
    """
    return {name: entry.sent_to for name, entry in _read(path, whoever_can_read_it=False).items()}


def key_for(path: Path, provider: str, origin: str) -> str | None:
    """Return the key that is stored for a service, for a request to a place.

    Args:
        path: The file of keys.
        provider: The name of the service.
        origin: The place that the request goes to, see :func:`origin_of`.

    Returns:
        The key, or ``None`` if none is stored for the service.

    Raises:
        CredentialsError: The file cannot be read, is not what
            ``columbina setup`` writes, or is not its user's alone. Or a key
            is stored for the service, and it was stored for another place.
    """
    entry = _read(path, whoever_can_read_it=False).get(provider)
    if entry is None:
        return None
    if entry.sent_to != origin:
        raise CredentialsError(
            f"the key that is stored for {provider} was stored for {entry.sent_to}, and the "
            f"requests for {provider} now go to {origin}. It is not sent there. Enter a key "
            "for this address with 'columbina setup', or put it into the variable of the "
            "service"
        )
    return entry.key


def keep(path: Path, provider: str, key: str, origin: str) -> None:
    """Store a key for a service, in the place of the one that is stored for it.

    The file is written anew and for its user alone, whatever it was before.

    Args:
        path: The file of keys.
        provider: The name of the service.
        key: The key. The blanks and line breaks around it are not stored.
        origin: The place that the key is for, see :func:`origin_of`. It is
            handed out for requests to this place and to no other.

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
    entry = _Entry(key=key, sent_to=origin)
    _write(path, _read(path, whoever_can_read_it=True) | {provider: entry})


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
