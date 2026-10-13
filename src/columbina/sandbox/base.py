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

__all__ = [
    "PROTECTED",
    "Mode",
    "SandboxProvider",
    "SandboxUnavailableError",
    "Strength",
    "held_less_well",
]

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


def held_less_well(why: str | None = None) -> SandboxUnavailableError:
    """Return what is raised for a command that was let through for a hold that is not there.

    A command that is held fully is not asked about. Whoever let it through
    went by what was known then, and by the time the command is to run more
    may be known, or something else may run it.

    Args:
        why: What stands against a full hold now, if that is known.
    """
    now = "they are held less well now" if why is None else f"now {why}"
    return SandboxUnavailableError(
        "it was let through without a question because commands were held fully, and "
        f"{now}. Call it again, and it is asked about"
    )


class SandboxProvider(Protocol):
    """Something that runs a command so that it is held.

    Attributes:
        shell: The shell of the place where it runs commands, such as
            ``bash``, if that place is not this machine. ``None`` if commands
            run on this machine, with the shell that it has. A provider says
            the same for as long as it is there.
    """

    @property
    def shell(self) -> str | None:
        """The shell of the place where it runs commands, if that is not this machine."""
        ...

    def strength(self, session: Session) -> Strength:
        """Say how well it holds the commands of a session.

        A provider may know more of a session once it has run a command for
        it, and then say less than it did before.
        """
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
        expected: Strength,
    ) -> Completed:
        """Run a command to its end, held in the given mode, and say how it ended.

        When this returns or raises, everything that the command started has
        been stopped, as with :func:`columbina.process.run`, whose arguments
        ``stdin`` to ``max_overflow_bytes`` are.

        ``expected`` says how well the command has to be held. It is
        ``full`` for a command that was let through without a question for
        no other reason than that it is held fully, and ``none`` otherwise.
        A provider that does not hold the command that well when it comes to
        run it does not run it, and raises :func:`held_less_well`.

        Raises:
            SandboxUnavailableError: The command could not be held, or not as
                well as was expected, so it was not run.
            OSError: The command could not be started.
        """
        ...
