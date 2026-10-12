"""What the sandbox and its providers have in common: the modes, the strengths and the seam.

A provider that the sandbox brings with it is a module beside this one. It
needs these names, and the sandbox needs the provider, so they are kept
where both can reach them.
"""

from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Literal, Protocol

from columbina.process import Completed, Writable
from columbina.session import Session

__all__ = ["PROTECTED", "Mode", "SandboxProvider", "SandboxUnavailableError", "Strength"]

Mode = Literal["read-only", "workspace-write", "danger-full-access"]
"""How much a session may change."""

Strength = Literal["none", "partial", "full"]
"""How well a provider holds a command: not at all, in part, or fully."""

PROTECTED = (".git", ".columbina")
"""The names of the directories that are not written in ``workspace-write`` mode."""


class SandboxUnavailableError(Exception):
    """What is to hold commands is not there, or could not hold this one.

    The text says what is missing, in words for whoever can put it right.
    """


class SandboxProvider(Protocol):
    """Something that runs a command so that it is held.

    Attributes:
        strength: How well it holds a command. A provider that cannot do what
            it says here raises from :meth:`run`. It does not run a command
            that is held less well.
        shell: The shell of the place where it runs commands, such as
            ``bash``, if that place is not this machine. ``None`` if commands
            run on this machine, with the shell that it has. A provider says
            the same for as long as it is there.
    """

    @property
    def strength(self) -> Strength:
        """How well it holds a command."""
        ...

    @property
    def shell(self) -> str | None:
        """The shell of the place where it runs commands, if that is not this machine."""
        ...

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
        """Run a command to its end, held in the given mode, and say how it ended.

        When this returns or raises, everything that the command started has
        been stopped, as with :func:`columbina.process.run`, whose arguments
        the last four are.

        Raises:
            SandboxUnavailableError: The command could not be held, so it was
                not run.
            OSError: The command could not be started.
        """
        ...
