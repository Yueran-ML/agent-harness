"""API keys: what one looks like, and the file in which a user keeps the ones that were typed in.

What a key looks like
    A key reaches a service as part of a header of the request. What is
    found where a key is kept is not always a key as its service issued it:
    it was copied by hand, and blanks, a line break or a mark of the text
    that it was copied from may have come along. :func:`trimmed` and
    :func:`sendable` are asked of every key before it is sent.

Where a key is kept
    A user who does not want to set a variable types a key in once. It goes
    into the keyring of the system where there is one: the Credential
    Manager of Windows, the Keychain of macOS, the Secret Service of a Linux
    desktop. Such a keyring keeps a key encrypted with what its user logs in
    with. A disk that is carried off, and a copy of the home directory in a
    backup, hold nothing that can be read. A keyring does not keep a key
    from the other programs that its user runs, except on macOS, where
    another program is asked about first.

    A system without a keyring that can be used, a server without a desktop
    for one, leaves the file. Whoever calls :func:`keep` is told which of the
    two took the key.

    A keyring is asked on the side and is waited for a while, and not for
    good. One that is locked asks its user to unlock it. On a machine
    without a screen it asks nobody, and waits for an answer all the same.
    Whoever wants no keyring to be asked sets the variable
    ``PYTHON_KEYRING_BACKEND`` to ``keyring.backends.null.Keyring``.

The file of keys
    One file says which services a key is stored for, and where each key is
    sent. It holds the key itself only where no keyring took it::

        {"version": 1,
         "keys": {"deepseek": {"sent_to": "https://api.deepseek.com"},
                  "mine": {"key": "sk-...", "sent_to": "http://10.0.0.5:8000"}}}

    A key that lies in the file lies there as it is. It is not encrypted, so
    whoever can read the file has it, and the file has to be its user's
    alone. On POSIX it is made with the mode 600, and a file that the group
    or the others can read is not used. Windows has no such modes. The
    access rules of the directory count there, and those of a user's own
    directory keep other users out.

    The file is written whole or not at all: next to the old one first, and
    then put in its place. Where the file is a link, the file that it leads
    to is the one that is read and the one that is written.

    In a keyring the keys of a file are filed under the place of that file.
    Two homes of the program do not share their keys.

Where a key goes
    A key is stored with the place that it was typed in for: the scheme,
    the machine and the port of the address of its service, see
    :func:`origin_of`. It is handed out for a request to that place and to
    no other. The name of a service says where its requests go only as long
    as nothing changes the address behind the name, and a variable of the
    environment can do that. A key that was typed in for one service is not
    sent to wherever the name leads on another day.

Nothing here says a key. An error names the file or the keyring and what is
wrong with it, and never what either holds.
"""

import contextlib
import json
import os
import secrets
import stat
import sys
from collections.abc import Callable
from concurrent.futures import Future
from dataclasses import dataclass
from pathlib import Path
from threading import Thread
from typing import TYPE_CHECKING, Literal, Protocol, TypeVar
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, ValidationError

if TYPE_CHECKING:
    from keyring.backend import KeyringBackend

__all__ = [
    "CredentialsError",
    "Kept",
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


_FILED_UNDER = "columbina"
"""What the keys of a file are filed under in a keyring, before the place of the file."""

_SAID = 160
"""How many characters of what a keyring says are passed on."""

_WAIT = 30.0
"""How many seconds a keyring is waited for.

A keyring that is locked asks its user to unlock it, and a person takes a while to do that.
"""

_CALLED = {
    "keyring.backends.Windows": "the Credential Manager of Windows",
    "keyring.backends.macOS": "the Keychain of macOS",
    "keyring.backends.SecretService": "the Secret Service of this desktop",
    "keyring.backends.libsecret": "the Secret Service of this desktop",
    "keyring.backends.kwallet": "the KWallet of this desktop",
}
"""What a person knows a keyring by, by the module through which it is reached."""

_T = TypeVar("_T")


class CredentialsError(Exception):
    """The stored keys cannot be used, or a key cannot be kept or had."""


class _NoKeyring(Exception):  # noqa: N818 - what a keyring answered, and no error of a caller
    """The keyring of the system did not do what it was asked. The text says why, without a key."""


@dataclass(frozen=True, slots=True)
class Kept:
    """Where a key was kept.

    Attributes:
        keyring: What a person knows the keyring by that took the key, or
            ``None`` if the key lies in the file.
        said: What kept the key out of a keyring, in a few words. Empty
            where a keyring took it.
    """

    keyring: str | None
    said: str = ""


class _Keyring(Protocol):
    """What is asked of the package that reaches the keyring of a system."""

    def get_keyring(self) -> "KeyringBackend": ...

    def get_password(self, service_name: str, username: str, /) -> str | None: ...

    def set_password(self, service_name: str, username: str, password: str, /) -> None: ...

    def delete_password(self, service_name: str, username: str, /) -> None: ...


class _Entry(BaseModel):
    """Where a key is sent, and the key itself where this is the place that holds it."""

    model_config = ConfigDict(extra="forbid")

    sent_to: str
    key: str | None = None


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


def _spelled(secret: str) -> set[str]:
    """Return the ways in which the text of a program is likely to spell a secret."""
    return {secret, json.dumps(secret)[1:-1], repr(secret)[1:-1]}


def _said(error: BaseException, *kept_out: str) -> str:
    """Return what was said when a keyring failed: on one line, not at length, and with no key.

    The text is fit to stand in a sentence of this program: the mark that
    ends its last sentence is left to whoever goes on after it.

    Args:
        error: What the keyring failed with.
        kept_out: What the keyring was given. None of it is repeated,
            however the text spells it.
    """
    said = str(error)
    spellings = {spelling for secret in kept_out for spelling in _spelled(secret)}
    # The longest first. A spelling that is part of a longer one would leave the rest of it.
    for spelling in sorted(spellings, key=len, reverse=True):
        said = said.replace(spelling, "[key]")
    said = " ".join(said.split())
    if len(said) > _SAID:
        return f"{said[:_SAID].rstrip()} [...]"
    return said.rstrip(".!") or type(error).__name__


def _called(vault: "KeyringBackend") -> str:
    """Return what a person knows a keyring by."""
    known = _CALLED.get(type(vault).__module__)
    return f"the keyring of this system ({vault.name})" if known is None else known


def _of_the_keyring(path: Path, ask: Callable[[_Keyring, str], _T], *kept_out: str) -> _T:
    """Ask the keyring of the system for something, and return what it answers.

    Args:
        path: The file of keys whose keys are asked about.
        ask: Asks. It is given what reaches the keyring, and the name
            under which the keys of the file are filed in it.
        kept_out: What the keyring is given. None of it is repeated where
            the keyring fails.

    Raises:
        _NoKeyring: There is no keyring, it did not answer in time, or it
            failed. The text says which, and holds no key.
    """
    try:
        # Loaded here and not at the head of the module: it takes a fifth of a second, and a
        # start of the program that finds its key in a variable has no use for it.
        import keyring
        from keyring.errors import NoKeyringError

        name = f"{_FILED_UNDER} {path.resolve()}"
    except Exception as error:
        raise _NoKeyring(_said(error)) from None
    answer: Future[_T] = Future()

    def run() -> None:
        try:
            answer.set_result(ask(keyring, name))
        except BaseException as error:
            # A keyring is the business of the system, and fails with whatever the system
            # fails with. All of it is for whoever waits for the answer.
            answer.set_exception(error)

    # Asked on the side, and left behind when the program ends: a keyring that never answers
    # would keep the program there with it.
    Thread(target=run, name="keyring", daemon=True).start()
    try:
        error = answer.exception(timeout=_WAIT)
    except TimeoutError:
        raise _NoKeyring(f"the keyring did not answer within {_WAIT:g} seconds") from None
    if error is None:
        return answer.result()
    if isinstance(error, NoKeyringError):
        raise _NoKeyring("there is no keyring here")
    raise _NoKeyring(_said(error, *kept_out))


def _put_back(path: Path, provider: str, before: str | None) -> None:
    """Leave a keyring with what it held for a service before, as far as it lets one."""

    def put(vault: _Keyring, name: str) -> None:
        if before is None:
            vault.delete_password(name, provider)
        else:
            vault.set_password(name, provider, before)

    with contextlib.suppress(_NoKeyring):
        _of_the_keyring(path, put)


def _into_the_keyring(path: Path, provider: str, key: str, origin: str) -> tuple[str, str | None]:
    """Give a key to the keyring of the system.

    Returns:
        What a person knows the keyring by that took the key, and what that
        keyring held for the service until now.

    Raises:
        _NoKeyring: No keyring took the key. The text says why.
    """
    payload = _Entry(key=key, sent_to=origin).model_dump_json()

    def give(vault: _Keyring, name: str) -> tuple[str | None, str | None, str]:
        before = vault.get_password(name, provider)
        vault.set_password(name, provider, payload)
        return before, vault.get_password(name, provider), _called(vault.get_keyring())

    before, back, called = _of_the_keyring(path, give, payload, key)
    if back == payload:
        return called, before
    # A keyring that is set up to keep nothing takes what it is given without a word. Whatever
    # this one made of the key is not left in it.
    _put_back(path, provider, before)
    does = "keeps nothing" if back is None else "does not hand out what it was given"
    raise _NoKeyring(f"the keyring that is set up here {does}")


def _from_the_keyring(path: Path, provider: str) -> tuple[str, str]:
    """Return a key that the keyring of the system keeps, and the place that it is sent to.

    Raises:
        CredentialsError: The keyring cannot be asked, or does not hold what
            was put there.
    """
    back: str | None = None
    refused: str | None = None
    try:
        back = _of_the_keyring(path, lambda vault, name: vault.get_password(name, provider))
    except _NoKeyring as error:
        refused = str(error)
    if refused is not None:
        raise CredentialsError(
            f"the key of {provider} is kept in the keyring of this system, and the keyring "
            f"could not be asked for it: {refused}. Put the key into the variable of the "
            "service, or enter it again with 'columbina setup'"
        )
    entry: _Entry | None = None
    if back is not None:
        with contextlib.suppress(ValidationError):
            entry = _Entry.model_validate_json(back)
    if entry is None or entry.key is None:
        raise CredentialsError(
            f"the key of {provider} is said to be in the keyring of this system, and it is "
            "not there. Enter it again with 'columbina setup'"
        )
    return entry.key, entry.sent_to


def _out_of_the_keyring(path: Path, provider: str) -> bool:
    """Take a key out of the keyring of the system, and return whether there was one.

    Raises:
        _NoKeyring: The keyring could not be asked. The text says why.
    """

    def take(vault: _Keyring, name: str) -> bool:
        if vault.get_password(name, provider) is None:
            return False
        vault.delete_password(name, provider)
        return True

    return _of_the_keyring(path, take)


def _write(path: Path, keys: dict[str, _Entry]) -> None:
    """Put a file of these keys in the place of the one that is there, whole or not at all."""
    held = {name: entry.model_dump(exclude_none=True) for name, entry in keys.items()}
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
            ``columbina setup`` writes, or is not its user's alone. Or the
            key is kept in the keyring of the system, and cannot be had from
            it. Or a key is stored for the service, and it was stored for
            another place.
    """
    entry = _read(path, whoever_can_read_it=False).get(provider)
    if entry is None:
        return None
    if entry.key is None:
        # What the keyring holds says where the key goes. What the file says of that is for
        # whoever lists the keys, and whoever can change the file cannot send a key elsewhere.
        key, sent_to = _from_the_keyring(path, provider)
    else:
        key, sent_to = entry.key, entry.sent_to
    if sent_to != origin:
        raise CredentialsError(
            f"the key that is stored for {provider} was stored for {sent_to}, and the "
            f"requests for {provider} now go to {origin}. It is not sent there. Enter a key "
            "for this address with 'columbina setup', or put it into the variable of the "
            "service"
        )
    return key


def keep(path: Path, provider: str, key: str, origin: str) -> Kept:
    """Store a key for a service, in the place of the one that is stored for it.

    The key goes into the keyring of the system if there is one that takes
    it, and into the file otherwise. The file is written anew and for its
    user alone, whatever it was before.

    Args:
        path: The file of keys.
        provider: The name of the service.
        key: The key. The blanks and line breaks around it are not stored.
        origin: The place that the key is for, see :func:`origin_of`. It is
            handed out for requests to this place and to no other.

    Returns:
        Where the key was kept.

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
    keys = _read(path, whoever_can_read_it=True)
    said = ""
    taken: tuple[str, str | None] | None = None
    try:
        taken = _into_the_keyring(path, provider, key, origin)
    except _NoKeyring as refused:
        said = str(refused)
    if taken is None:
        keys[provider] = _Entry(key=key, sent_to=origin)
        _write(path, keys)
        return Kept(keyring=None, said=said)
    called, before = taken
    # The file says that there is a key and where it is sent, and holds no key.
    keys[provider] = _Entry(sent_to=origin)
    try:
        _write(path, keys)
    except CredentialsError:
        # Nothing is kept where the file could not be told of it.
        _put_back(path, provider, before)
        raise
    return Kept(keyring=called)


def forget(path: Path, provider: str) -> bool:
    """Delete the key that is stored for a service.

    Returns:
        Whether there was one.

    Raises:
        CredentialsError: The file that is there cannot be read or is not
            what ``columbina setup`` writes, or the file cannot be written.
            Or the key is kept in the keyring of the system, and could not
            be taken out of it.
    """
    keys = _read(path, whoever_can_read_it=True)
    entry = keys.pop(provider, None)
    # The keyring is asked whatever the file says. A key that the file lost sight of would
    # lie in the keyring for good, and so would one that was there before a later key went
    # into the file.
    in_the_keyring = False
    refused: str | None = None
    try:
        in_the_keyring = _out_of_the_keyring(path, provider)
    except _NoKeyring as error:
        refused = str(error)
    if refused is not None and entry is not None and entry.key is None:
        raise CredentialsError(
            f"the key of {provider} is kept in the keyring of this system, and it could not "
            f"be taken out of it: {refused}"
        )
    if entry is None:
        return in_the_keyring
    _write(path, keys)
    return True
