"""Holding a write up in the middle, so that a test can do something meanwhile."""

import os
import threading
from typing import Any

import pytest

from columbina.fs import bound

__all__ = ["HeldWrite"]


class HeldWrite:
    """Holds the first write up when its new content is ready, before the content takes the name.

    Attributes:
        reached: Set when the write has come that far.
        go_on: Set by the test to let the write go on.
        ended: Set when the thread of the write has ended.
    """

    def __init__(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self.reached = threading.Event()
        self.go_on = threading.Event()
        self.ended = threading.Event()
        really_sync = os.fsync
        really_replace: Any = bound.replace

        def syncing(descriptor: int) -> None:
            really_sync(descriptor)
            if not self.reached.is_set():
                # Only the first write is held up.
                self.reached.set()
                self.go_on.wait(timeout=20)

        def replacing(*arguments: Any) -> Any:
            first = not self.reached.is_set()
            try:
                return really_replace(*arguments)
            finally:
                if first:
                    self.ended.set()

        monkeypatch.setattr(os, "fsync", syncing)
        monkeypatch.setattr(bound, "replace", replacing)
