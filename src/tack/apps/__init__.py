"""The programs that are built from the plugins: the launcher and the frontends."""

from typing import Protocol

from tack.kernel import ServiceKey


class Frontend(Protocol):
    """What the launcher runs once a profile is loaded."""

    async def run(self) -> int:
        """Do the work of the program and return its exit code."""
        ...


FRONTEND = ServiceKey[Frontend]("frontend")
"""The frontend of the program. A profile provides one, through its entry ``frontend``."""

__all__ = ["FRONTEND", "Frontend"]
