"""Keyrings for tests, which stand in for the keyring of a system.

No test asks the keyring of the system that it runs on: what a test stores there would lie
among the passwords of whoever runs the tests. The configuration of the tests puts a keyring
that keeps nothing in its place. A test of what happens with a keyring puts one of these in.
"""

import threading
from collections.abc import Callable
from typing import Self

import keyring
from keyring.backend import KeyringBackend
from keyring.backends.chainer import ChainerBackend
from keyring.errors import KeyringLocked, PasswordDeleteError, PasswordSetError

__all__ = [
    "Dropping",
    "LikeWindows",
    "Locked",
    "Losing",
    "Memory",
    "Refusing",
    "Rewriting",
    "Secrets",
    "Waiting",
    "Wallet",
    "row",
    "settled",
]

# How long a keyring of a test waits for its test at most. No test takes that long.
_LONGEST = 5.0


def settled() -> None:
    """Wait until nothing is asked of a keyring any more."""
    for thread in threading.enumerate():
        if thread.name == "keyring":
            thread.join(_LONGEST)


class Memory(KeyringBackend):
    """A keyring that keeps what it is given, for as long as the test lasts.

    Attributes:
        held: What it keeps, by the name that it is filed under and the name
            of its user.
        locked: Whether it does not open, whatever it is asked. A test sets
            this for a keyring that was open a moment ago.
        asked: What was asked of it, in the order in which it was asked.
        asked_from: The name of every thread that asked, with whether the
            program waits for that thread when it ends.
    """

    priority = 1  # pyright: ignore[reportAssignmentType]

    def __init__(self) -> None:
        super().__init__()
        self.held: dict[tuple[str, str], str] = {}
        self.locked = False
        self.asked: list[tuple[str, str, str]] = []
        self.asked_from: set[tuple[str, bool]] = set()

    def _note(self, what: str, service: str, username: str) -> None:
        thread = threading.current_thread()
        self.asked.append((what, service, username))
        self.asked_from.add((thread.name, not thread.daemon))
        if self.locked:
            # The words of the keyring that a desktop of Linux has.
            raise KeyringLocked("Failed to unlock the collection!")

    def get_password(self, service: str, username: str) -> str | None:
        self._note("get", service, username)
        return self.held.get((service, username))

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        self.held[service, username] = password

    def delete_password(self, service: str, username: str) -> None:
        self._note("delete", service, username)
        if (service, username) not in self.held:
            raise PasswordDeleteError("no such password")
        del self.held[service, username]

    def install(self) -> Self:
        """Put this keyring in the place of the one that is there."""
        keyring.set_keyring(self)
        return self


class Locked(Memory):
    """A keyring that is there and does not open."""

    def __init__(self) -> None:
        super().__init__()
        self.locked = True


class Dropping(Memory):
    """A keyring that takes what it is given without a word, and keeps none of it."""

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)


class Refusing(Memory):
    """A keyring that hands out what it holds and takes nothing new.

    Args:
        fails_with: Makes the error that it refuses with, from what it was
            given to keep.
    """

    def __init__(self, fails_with: Callable[[str], BaseException]) -> None:
        super().__init__()
        self.fails_with = fails_with

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        raise self.fails_with(password)


class Rewriting(Memory):
    """A keyring that takes what it is given, and hands out something else in its place.

    Args:
        hands_out: What it hands out for whatever it was given since it was
            made. ``None`` for a keyring that seems to have kept nothing.
    """

    def __init__(self, hands_out: str | None) -> None:
        super().__init__()
        self.hands_out = hands_out
        self.given: set[tuple[str, str]] = set()

    def get_password(self, service: str, username: str) -> str | None:
        held = super().get_password(service, username)
        return self.hands_out if (service, username) in self.given else held

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        self.held[service, username] = password
        self.given.add((service, username))

    def delete_password(self, service: str, username: str) -> None:
        self.given.discard((service, username))
        super().delete_password(service, username)


class Waiting(Memory):
    """A keyring that answers nothing until its test lets it.

    Args:
        first_only: Whether only whoever asks first is kept waiting, and
            whoever asks after that is answered at once.

    Attributes:
        reached: Set once the keyring was asked.
        released: Set by the test, to let the keyring answer.
    """

    def __init__(self, first_only: bool = False) -> None:
        super().__init__()
        self.first_only = first_only
        self.reached = threading.Event()
        self.released = threading.Event()

    def get_password(self, service: str, username: str) -> str | None:
        first = not self.reached.is_set()
        self.reached.set()
        if first or not self.first_only:
            self.released.wait(_LONGEST)
        return super().get_password(service, username)


class Losing(Memory):
    """A keyring that puts a new value in the place of an old one in two steps, and fails between.

    The one of macOS does it so: what is there is taken out, and then what is new is put in.

    Args:
        times: How often it fails. After that it takes what it is given.
    """

    def __init__(self, times: int = 1) -> None:
        super().__init__()
        self.times = times

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        self.held.pop((service, username), None)
        if self.times:
            self.times -= 1
            raise PasswordSetError("Can't store password on keychain")
        self.held[service, username] = password


class LikeWindows(Memory):
    """A keyring that keeps what is written over, as the package uses the one of Windows.

    That keyring has one entry for a name, with the user that it is of. The package wants
    several users under one name, so before it writes an entry it puts the one that is there
    under a name made of the user and the name, and it looks there when the entry itself is
    of another user or gone. What is asked to be deleted is deleted in both places.

    Attributes:
        targets: The entries, by the names that they lie under: each a user and what is
            kept for that user.
    """

    def __init__(self) -> None:
        super().__init__()
        self.targets: dict[str, tuple[str, str]] = {}

    def get_password(self, service: str, username: str) -> str | None:
        self._note("get", service, username)
        found = self.targets.get(service)
        if found is None or found[0] != username:
            found = self.targets.get(f"{username}@{service}")
        return None if found is None else found[1]

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        there = self.targets.get(service)
        if there is not None:
            self.targets[f"{there[0]}@{service}"] = there
        self.targets[service] = (username, password)

    def delete_password(self, service: str, username: str) -> None:
        self._note("delete", service, username)
        deleted = False
        for target in (service, f"{username}@{service}"):
            found = self.targets.get(target)
            if found is not None and found[0] == username:
                deleted = True
                del self.targets[target]
        if not deleted:
            raise PasswordDeleteError(service)


class Wallet(Memory):
    """One keyring of a desktop that has two."""


class Secrets(Memory):
    """The other keyring of a desktop that has two."""


def row(*backends: KeyringBackend) -> KeyringBackend:
    """Return several keyrings in a row, as the package puts the ones of a desktop in one.

    It is the row of the package itself, with these keyrings in the place of the ones that
    it would find: a key is given to the first, and looked for in each until one has it.
    """
    made = type("Row", (ChainerBackend,), {"backends": list(backends)})
    return made()
