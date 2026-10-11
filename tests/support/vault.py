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
from keyring.errors import KeyringLocked, PasswordDeleteError

__all__ = ["Dropping", "Locked", "Memory", "Refusing", "Rewriting", "Waiting", "settled"]

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
        asked: What was asked of it, in the order in which it was asked.
        asked_from: The name of every thread that asked, with whether the
            program waits for that thread when it ends.
    """

    priority = 1  # pyright: ignore[reportAssignmentType]

    def __init__(self) -> None:
        super().__init__()
        self.held: dict[tuple[str, str], str] = {}
        self.asked: list[tuple[str, str, str]] = []
        self.asked_from: set[tuple[str, bool]] = set()

    def _note(self, what: str, service: str, username: str) -> None:
        thread = threading.current_thread()
        self.asked.append((what, service, username))
        self.asked_from.add((thread.name, not thread.daemon))

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
    """A keyring that is there and does not open, in the words of one that a desktop has."""

    def get_password(self, service: str, username: str) -> str | None:
        self._note("get", service, username)
        raise KeyringLocked("Failed to unlock the collection!")

    def set_password(self, service: str, username: str, password: str) -> None:
        self._note("set", service, username)
        raise KeyringLocked("Failed to unlock the collection!")

    def delete_password(self, service: str, username: str) -> None:
        self._note("delete", service, username)
        raise KeyringLocked("Failed to unlock the item!")


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

    Attributes:
        reached: Set once the keyring was asked.
        released: Set by the test, to let the keyring answer.
    """

    def __init__(self) -> None:
        super().__init__()
        self.reached = threading.Event()
        self.released = threading.Event()

    def get_password(self, service: str, username: str) -> str | None:
        self.reached.set()
        self.released.wait(_LONGEST)
        return super().get_password(service, username)
