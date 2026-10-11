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
    What was asked on the side goes no further once nobody waits for it: a
    keyring that answers late is not given a key that was put elsewhere
    meanwhile. Whoever wants no keyring to be asked sets the variable
    ``PYTHON_KEYRING_BACKEND`` to ``keyring.backends.null.Keyring``.

    A key is taken out of a keyring before another one is given to it, and
    is not written over. A keyring may keep what is written over under
    another name, and the one of Windows does. Where giving the new key
    fails, the keyring is left with what it held, as far as it lets one.

    A desktop may have several keyrings, which stand in a row. One of them
    is asked, the first, and its name is written down with the key. A key
    is looked for in the keyring that it was given to and in no other, so a
    key that was replaced is not found again in a keyring further back.

The file of keys
    One file says which services a key is stored for, and where each key is
    sent. It holds the key itself only where no keyring took it::

        {"version": 1,
         "keys": {"deepseek": {"sent_to": "https://api.deepseek.com",
                               "keyring": "Windows WinVaultKeyring"},
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

    The file and the directory that it lies in are its user's to write, and
    nobody else's. Whoever can write the file can put a key of their own
    into it, and the requests then go out under that key. That is as true of
    the file that says which service is in use.

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
import html
import json
import os
import secrets
import stat
import sys
from collections.abc import Callable, Sequence
from concurrent.futures import Future
from dataclasses import dataclass
from pathlib import Path
from threading import Event, Thread
from typing import Any, Literal, Protocol, TypeVar, cast
from urllib.parse import quote, urlsplit

from pydantic import BaseModel, ConfigDict, ValidationError

__all__ = [
    "CredentialsError",
    "Forgotten",
    "Kept",
    "forget",
    "keep",
    "key_for",
    "origin_of",
    "sendable",
    "spellings",
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
    """The keyring of the system did not do what it was asked. The text says why, without a key.

    Attributes:
        none_here: Whether the reason is that this system has no keyring at
            all.
    """

    def __init__(self, said: str, *, none_here: bool = False) -> None:
        super().__init__(said)
        self.none_here = none_here


class _GivenUp(Exception):  # noqa: N818 - how what was asked on the side is told to stop
    """Nobody waits any more for what a keyring was asked, so it is asked nothing further."""


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


@dataclass(frozen=True, slots=True)
class Forgotten:
    """What came of forgetting a key. It is true where there was one.

    Attributes:
        was_there: Whether a key was stored for the service.
        unasked: Why the keyring could not be asked whether it holds an
            earlier key of the service, or ``None`` if it was asked or this
            system has none. A key that lay in the file is gone all the same.
    """

    was_there: bool
    unasked: str | None = None

    def __bool__(self) -> bool:
        return self.was_there


class _Backend(Protocol):
    """What is asked of one keyring."""

    @property
    def name(self) -> str: ...

    def get_password(self, service: str, username: str, /) -> str | None: ...

    def set_password(self, service: str, username: str, password: str, /) -> None: ...

    def delete_password(self, service: str, username: str, /) -> None: ...


class _Entry(BaseModel):
    """Where a key is sent, and where the key is: in this entry, or in a keyring.

    Attributes:
        sent_to: The place that the key is sent to.
        key: The key, where no keyring took it.
        keyring: What the keyring that took the key calls itself. A file
            that an earlier version wrote does not say.
    """

    model_config = ConfigDict(extra="forbid")

    sent_to: str
    key: str | None = None
    keyring: str | None = None


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


def spellings(secret: str) -> list[str]:
    """Return the ways in which a text may repeat a secret, the longest first.

    A text that somebody else wrote does not always hold a key as it is. A
    body of JSON that is passed on unread writes a backslash and a quotation
    mark with a backslash before them, and some writers of JSON do the same
    for a slash. Python shows a text with its backslashes doubled. An
    address writes most signs as a percent sign and two digits, and a page
    of HTML writes a few of them as names.
    """
    as_json = json.dumps(secret)[1:-1]
    spelled = {
        secret,
        as_json,
        as_json.replace("/", chr(92) + "/"),
        repr(secret)[1:-1],
        quote(secret, safe=""),
        html.escape(secret),
    }
    return sorted(spelled, key=len, reverse=True)


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
    spelled = {spelling for secret in kept_out for spelling in spellings(secret)}
    # The longest first. A spelling that is part of a longer one would leave the rest of it.
    for spelling in sorted(spelled, key=len, reverse=True):
        said = said.replace(spelling, "[key]")
    said = " ".join(said.split())
    if len(said) > _SAID:
        return f"{said[:_SAID].rstrip()} [...]"
    return said.rstrip(".!") or type(error).__name__


class _Asked:
    """One keyring, asked about the keys of one file for as long as somebody waits for it.

    Args:
        vault: The keyring that is asked.
        behind: The keyrings that stand behind it in a row, if there is a
            row. They are asked for nothing but to give up a key of which
            they hold an earlier one.
        name: The name under which the keys of the file are filed.
        given_up: Set once nobody waits any more.
        nothing_there: What the keyring raises where it is asked to give up
            what it does not hold.
    """

    __slots__ = ("_behind", "_given_up", "_name", "_nothing_there", "_vault")

    def __init__(
        self,
        vault: _Backend,
        behind: Sequence[_Backend],
        name: str,
        given_up: Event,
        nothing_there: type[Exception],
    ) -> None:
        self._vault = vault
        self._behind = behind
        self._name = name
        self._given_up = given_up
        self._nothing_there = nothing_there

    @property
    def called(self) -> str:
        """What a person knows the keyring by."""
        known = _CALLED.get(type(self._vault).__module__)
        return f"the keyring of this system ({self.named})" if known is None else known

    @property
    def named(self) -> str:
        """What the keyring calls itself."""
        return str(self._vault.name)

    def _still_wanted(self) -> None:
        if self._given_up.is_set():
            raise _GivenUp

    def get(self, provider: str) -> str | None:
        """Return what the keyring holds for a service."""
        self._still_wanted()
        return self._vault.get_password(self._name, provider)

    def put(self, provider: str, value: str | None) -> None:
        """Have the keyring hold a value for a service, or nothing, in the place of what it holds.

        What it holds is taken out first, and is not written over. A keyring
        may keep what is written over under another name, and hand that out
        once the new value is gone.
        """
        there = self.get(provider) is not None
        self._still_wanted()
        try:
            self._vault.delete_password(self._name, provider)
        except self._nothing_there:
            # A keyring that showed nothing may hold something all the same, so it was asked.
            if there:
                raise
        if value is not None:
            self._still_wanted()
            self._vault.set_password(self._name, provider, value)

    def put_back(self, provider: str, before: str | None) -> None:
        """Leave the keyring with what it held before, as far as it lets one."""
        with contextlib.suppress(Exception):
            self.put(provider, before)

    def clear_behind(self, provider: str) -> None:
        """Take an earlier key of a service out of the keyrings that stand behind this one."""
        for other in self._behind:
            self._still_wanted()
            with contextlib.suppress(Exception):
                if other.get_password(self._name, provider) is not None:
                    other.delete_password(self._name, provider)


def _of_the_keyring(path: Path, ask: Callable[[_Asked], _T], *kept_out: str) -> _T:
    """Ask the keyring of the system for something, and return what it answers.

    Args:
        path: The file of keys whose keys are asked about.
        ask: Asks. It is given the keyring.
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
        from keyring.backends import fail
        from keyring.backends.chainer import ChainerBackend
        from keyring.errors import NoKeyringError, PasswordDeleteError

        name = f"{_FILED_UNDER} {path.resolve()}"
    except Exception as error:
        raise _NoKeyring(_said(error)) from None
    answer: Future[_T] = Future()
    given_up = Event()

    def run() -> None:
        try:
            vault = keyring.get_keyring()
            if isinstance(vault, fail.Keyring):
                # What stands in the place of a keyring where there is none. It is asked
                # nothing, and nothing is made of what it is called.
                raise NoKeyringError
            # Of several keyrings in a row, the first is the one that a key is given to.
            row = cast("Any", vault).backends if isinstance(vault, ChainerBackend) else [vault]
            first, *behind = cast("list[_Backend]", list(row))
            answer.set_result(ask(_Asked(first, behind, name, given_up, PasswordDeleteError)))
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
        # Whatever was asked on the side asks nothing further once it comes back.
        given_up.set()
        raise _NoKeyring(f"the keyring did not answer within {_WAIT:g} seconds") from None
    if error is None:
        return answer.result()
    if isinstance(error, _NoKeyring):
        raise error
    if isinstance(error, NoKeyringError):
        raise _NoKeyring("there is no keyring here", none_here=True)
    raise _NoKeyring(_said(error, *kept_out))


def _put_back(path: Path, provider: str, before: str | None) -> None:
    """Leave a keyring with what it held for a service before, as far as it lets one."""
    with contextlib.suppress(_NoKeyring):
        _of_the_keyring(path, lambda asked: asked.put_back(provider, before))


def _into_the_keyring(
    path: Path, provider: str, key: str, origin: str
) -> tuple[str, str, str | None]:
    """Give a key to the keyring of the system.

    Returns:
        What a person knows the keyring by that took the key, what that
        keyring calls itself, and what it held for the service until now.

    Raises:
        _NoKeyring: No keyring took the key. The text says why.
    """
    payload = _Entry(key=key, sent_to=origin).model_dump_json(exclude_none=True)

    def give(asked: _Asked) -> tuple[str, str, str | None]:
        before = asked.get(provider)
        try:
            asked.put(provider, payload)
            back = asked.get(provider)
        except BaseException:
            # What was begun is undone. The key that was there is not lost for a new one
            # that did not arrive.
            asked.put_back(provider, before)
            raise
        if back != payload:
            # A keyring that is set up to keep nothing takes what it is given without a
            # word. Whatever this one made of the key is not left in it.
            asked.put_back(provider, before)
            does = "keeps nothing" if back is None else "does not hand out what it was given"
            raise _NoKeyring(f"the keyring that is set up here {does}")
        asked.clear_behind(provider)
        return asked.called, asked.named, before

    return _of_the_keyring(path, give, payload, key)


def _another(recorded: str | None, asked: _Asked) -> None:
    """Refuse a keyring that is not the one that a key was given to."""
    if recorded is not None and asked.named != recorded:
        raise _NoKeyring(
            f"it was given to the keyring {recorded}, and the keyring that is reached now "
            f"is {asked.named}"
        )


def _from_the_keyring(path: Path, provider: str, recorded: str | None) -> tuple[str, str]:
    """Return a key that the keyring of the system keeps, and the place that it is sent to.

    Args:
        path: The file of keys.
        provider: The name of the service.
        recorded: What the keyring that took the key calls itself, if the
            file says.

    Raises:
        CredentialsError: The keyring cannot be asked, is another one than
            the key was given to, or does not hold what was put there.
    """

    def had(asked: _Asked) -> str | None:
        _another(recorded, asked)
        return asked.get(provider)

    back: str | None = None
    refused: str | None = None
    try:
        back = _of_the_keyring(path, had)
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


def _out_of_the_keyring(path: Path, provider: str, recorded: str | None) -> bool:
    """Take a key out of the keyring of the system, and return whether there was one.

    Raises:
        _NoKeyring: The keyring could not be asked, or is another one than
            the key was given to. The text says why.
    """

    def take(asked: _Asked) -> bool:
        _another(recorded, asked)
        there = asked.get(provider) is not None
        asked.put(provider, None)
        asked.clear_behind(provider)
        return there

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
        key, sent_to = _from_the_keyring(path, provider, entry.keyring)
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
    taken: tuple[str, str, str | None] | None = None
    try:
        taken = _into_the_keyring(path, provider, key, origin)
    except _NoKeyring as refused:
        said = str(refused)
    if taken is None:
        keys[provider] = _Entry(key=key, sent_to=origin)
        _write(path, keys)
        return Kept(keyring=None, said=said)
    called, named, before = taken
    # The file says that there is a key, where it is sent and which keyring has it, and
    # holds no key.
    keys[provider] = _Entry(sent_to=origin, keyring=named)
    try:
        _write(path, keys)
    except CredentialsError:
        # Nothing is kept where the file could not be told of it.
        _put_back(path, provider, before)
        raise
    return Kept(keyring=called)


def forget(path: Path, provider: str) -> Forgotten:
    """Delete the key that is stored for a service.

    The keyring is asked whatever the file says. A key that the file lost
    sight of would lie in the keyring for good, and so would one that was
    there before a later key went into the file.

    Returns:
        Whether there was a key, and why the keyring could not be asked if
        it could not.

    Raises:
        CredentialsError: The file that is there cannot be read or is not
            what ``columbina setup`` writes, or the file cannot be written.
            Or the key is kept in the keyring of the system, and could not
            be taken out of it.
    """
    keys = _read(path, whoever_can_read_it=True)
    entry = keys.pop(provider, None)
    in_the_keyring = False
    refused: _NoKeyring | None = None
    try:
        recorded = None if entry is None else entry.keyring
        in_the_keyring = _out_of_the_keyring(path, provider, recorded)
    except _NoKeyring as error:
        refused = error
    if refused is not None and entry is not None and entry.key is None:
        raise CredentialsError(
            f"the key of {provider} is kept in the keyring of this system, and it could not "
            f"be taken out of it: {refused}"
        )
    # A system without a keyring holds no earlier key in one.
    unasked = None if refused is None or refused.none_here else str(refused)
    if entry is None:
        return Forgotten(in_the_keyring, unasked)
    _write(path, keys)
    return Forgotten(True, unasked)
