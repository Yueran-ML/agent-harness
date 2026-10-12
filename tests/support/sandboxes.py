"""Providers of a sandbox for tests: they say how well they hold commands, and run them here."""

from collections.abc import Callable, Sequence
from pathlib import Path

from columbina import process
from columbina.process import Completed, Writable
from columbina.sandbox import Mode, Strength
from columbina.session import Session

__all__ = ["Holding"]


class Holding:
    """A provider that says how well it holds commands, and runs them on this machine.

    Attributes:
        strength: What it says of itself.
        asked: What it was asked to run: the command, the mode and the directory of each.
    """

    def __init__(self, strength: Strength = "full") -> None:
        self.strength: Strength = strength
        self.asked: list[tuple[tuple[str, ...], Mode, Path]] = []

    def held(self, argv: Sequence[str], mode: Mode) -> Sequence[str]:
        """Return what is run in the place of a command. A test may put something else there."""
        return argv

    async def run(
        self,
        argv: Sequence[str],
        *,
        session: Session,
        mode: Mode,
        cwd: Path,
        stdin: bytes | None,
        timeout_seconds: float,
        overflow: Callable[[], Writable] | None,
        max_overflow_bytes: int,
    ) -> Completed:
        self.asked.append((tuple(argv), mode, cwd))
        return await process.run(
            self.held(argv, mode),
            cwd=cwd,
            stdin=stdin,
            timeout_seconds=timeout_seconds,
            overflow=overflow,
            max_overflow_bytes=max_overflow_bytes,
        )
