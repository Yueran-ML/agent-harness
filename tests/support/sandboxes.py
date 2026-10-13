"""Providers of a sandbox for tests: they say how well they hold commands, and run them here."""

from collections.abc import Callable, Sequence
from dataclasses import replace
from pathlib import Path

from columbina import process
from columbina.process import Completed, Writable
from columbina.sandbox import Mode, Strength, held_less_well
from columbina.session import Session

__all__ = ["Holding"]


class Holding:
    """A provider that says how well it holds commands, and runs them on this machine.

    Attributes:
        holds: How well it says that it holds commands, whichever session asks.
        shell: The shell of the place where it says that its commands run, if
            that is not this machine.
        asked: What it was asked to run: the command, the mode and the directory of each.
        given: The input that each of them came with.
        expected: How well each of them was to be held.
        notes: What it says of every run besides.
    """

    def __init__(
        self, strength: Strength = "full", shell: str | None = None, notes: tuple[str, ...] = ()
    ) -> None:
        self.holds: Strength = strength
        self.shell = shell
        self.notes = notes
        self.asked: list[tuple[tuple[str, ...], Mode, Path]] = []
        self.given: list[bytes | None] = []
        self.expected: list[Strength] = []

    def strength(self, session: Session) -> Strength:
        return self.holds

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
        expected: Strength,
    ) -> Completed:
        if expected == "full" and self.holds != "full":
            raise held_less_well()
        self.asked.append((tuple(argv), mode, cwd))
        self.given.append(stdin)
        self.expected.append(expected)
        done = await process.run(
            self.held(argv, mode),
            cwd=cwd,
            stdin=stdin,
            timeout_seconds=timeout_seconds,
            overflow=overflow,
            max_overflow_bytes=max_overflow_bytes,
        )
        return replace(done, notes=self.notes)
