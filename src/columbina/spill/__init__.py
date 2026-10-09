"""Where text goes that is too large for a conversation.

A tool can return more than a model should be made to read: the output of a
build, a file of logs, a search with thousands of hits. The whole of it is
saved somewhere, the model is shown its beginning and its end, and it is told
where the rest is and how to read it. This module is the seam: what such a
place can be asked to do. :mod:`columbina.spill.local` provides one, which
keeps files under the home of the program.

A text is saved in one go. What a command writes is saved as it comes, since
it may be more than fits into memory, and nobody knows how much it will be
until the command has ended.
"""

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from columbina.kernel import ServiceKey
from columbina.session import Session

__all__ = ["SPILL", "Saved", "SpillFile", "SpillStore"]


@dataclass(frozen=True, slots=True)
class Saved:
    """What was saved, as a model is told of it.

    Attributes:
        locator: Where it is. A model is given this as it stands.
        bytes: How much was saved.
        hint: How to read it, in a sentence for the model.
    """

    locator: str
    bytes: int
    hint: str


class SpillFile(Protocol):
    """Something that is being saved as it comes."""

    def write(self, data: bytes) -> None:
        """Save the next piece. This may block for as long as a write to a disk takes.

        Raises:
            OSError: The piece could not be saved.
            ValueError: The file is closed.
        """
        ...

    def close(self) -> Saved:
        """End the saving, and say what was saved. Closing again says the same."""
        ...


class SpillStore(Protocol):
    """A place that keeps what is too large for a conversation."""

    def save_text(self, session: Session, cwd: Path, name: str, text: str) -> Saved:
        """Save a text. This blocks, so it belongs in a worker thread.

        Args:
            session: The session that the text belongs to.
            cwd: The working directory of the agent.
            name: What the text is, in a word or two that may end up in the
                name of a file.
            text: The text.

        Raises:
            OSError: The text could not be saved.
        """
        ...

    def open(self, session: Session, cwd: Path, name: str) -> SpillFile:
        """Begin to save something that comes in pieces.

        Raises:
            OSError: Nothing can be saved.
        """
        ...


SPILL = ServiceKey[SpillStore]("spill")
"""The place that keeps what is too large for a conversation."""
