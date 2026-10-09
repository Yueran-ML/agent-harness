"""A terminal that is emulated: what its screen shows after everything it was sent.

The interface draws with ``prompt_toolkit``, which moves the cursor about,
writes over what it wrote before, and erases. What stands on the screen in
the end cannot be read off the characters that were sent. A :class:`Display`
carries them out the way a terminal does and keeps what results: the rows of
the window, and above them the lines that have scrolled out of it.

It knows what ``prompt_toolkit`` sends to a terminal that understands the
sequences of a VT100, and nothing more. Anything else is an error, so that
no test looks at a screen that was put together by guessing. A tab is such an
error too: the interface never sends one.

Two things that a real terminal does are left out. Colours and other
attributes of the text are not kept. And the window keeps its size.
"""

import re
from typing import TextIO, cast

from prompt_toolkit.data_structures import Size
from prompt_toolkit.output.vt100 import Vt100_Output
from prompt_toolkit.utils import get_cwidth

__all__ = ["Display", "Emulated"]

_ESCAPE = "\x1b"
_SEQUENCE = re.compile(r"\x1b\[(\??)([0-9;]*)( ?)([A-Za-z])")
"""A control sequence: its mark, its numbers, the space of some, and its last letter."""
_TITLE = re.compile(r"\x1b\][^\x07\x1b]*\x07")
"""The sequence that names the window."""
_BEGINNING = re.compile(r"\x1b(\[\??[0-9;]* ?|\][^\x07\x1b]*)?")
"""What either of them looks like while its end has not arrived."""

_NOTHING_TO_SEE = frozenset({1, 12, 1000, 1003, 1006, 1015, 2004})
"""Modes that say how keys and the mouse are reported, and whether the cursor blinks."""
_WRAPPING = 7
_CURSOR = 25

_HALF = ""
"""What the second cell of a character holds that takes two cells."""


class Display:
    """The screen of a terminal, with the lines that have scrolled out of its window.

    It is written to like a stream, which is how ``prompt_toolkit`` sends
    what it draws.

    Attributes:
        rows: The height of the window.
        columns: The width of the window.
        cursor_shown: Whether the cursor is shown.
        encoding: The encoding of the stream, which a terminal has.
    """

    encoding = "utf-8"

    def __init__(self, rows: int = 24, columns: int = 80) -> None:
        self.rows = rows
        self.columns = columns
        self.cursor_shown = True
        self._lines: list[list[str]] = [[]]
        # The line that the first row of the window shows, and where the cursor is.
        self._top: int = 0
        self._row: int = 0
        self._column: int = 0
        self._wraps = True
        # The beginning of a sequence whose end has not arrived yet.
        self._unfinished = ""

    @property
    def size(self) -> Size:
        """The size of the window."""
        return Size(rows=self.rows, columns=self.columns)

    @property
    def cursor(self) -> tuple[int, int]:
        """Where the cursor is: the row of the window and the column, both counted from 0."""
        return self._row - self._top, min(self._column, self.columns - 1)

    def rows_below(self) -> int:
        """Return how many rows the window has from the cursor down, the cursor's own among them."""
        return self._top + self.rows - self._row

    def lines(self) -> list[str]:
        """Return every line there is, from the first that scrolled away to the last with text."""
        texts = ["".join(line).rstrip() for line in self._lines]
        while texts and not texts[-1]:
            texts.pop()
        return texts

    def window(self) -> list[str]:
        """Return the rows of the window from the top, as many as the window has."""
        shown = ["".join(line).rstrip() for line in self._lines[self._top : self._top + self.rows]]
        return shown + [""] * (self.rows - len(shown))

    def write(self, data: str) -> int:
        """Carry out what a terminal is sent."""
        data, self._unfinished = self._unfinished + data, ""
        position = 0
        while position < len(data):
            if data[position] != _ESCAPE:
                self._character(data[position])
                position += 1
                continue
            sequence = _SEQUENCE.match(data, position)
            title = _TITLE.match(data, position)
            if sequence is not None:
                mark, numbers, space, final = sequence.groups()
                given = [int(number) for number in numbers.split(";") if number]
                self._sequence(
                    mark + numbers + space + final, bool(mark), given, bool(space), final
                )
                position = sequence.end()
            elif title is not None:
                position = title.end()
            elif _BEGINNING.fullmatch(data, position) is not None:
                self._unfinished = data[position:]
                break
            else:
                raise ValueError(f"the display does not know {data[position : position + 12]!r}")
        return len(data)

    def flush(self) -> None:
        """Do nothing: what was written is on the screen already."""

    def isatty(self) -> bool:
        """Say that this is a terminal."""
        return True

    def _line(self) -> list[str]:
        """Return the line that the cursor is in."""
        while len(self._lines) <= self._row:
            self._lines.append([])
        return self._lines[self._row]

    def _settle(self) -> None:
        # After a character in the last column the cursor waits there. It is not beyond it.
        self._column = min(self._column, self.columns - 1)

    def _down(self) -> None:
        """Go one line down, and let the window follow if the cursor was in its last row."""
        if self._row == self._top + self.rows - 1:
            self._top += 1
        self._row += 1

    def _character(self, character: str) -> None:
        if character == "\r":
            self._column = 0
        elif character == "\n":
            self._settle()
            self._down()
        elif character == "\b":
            self._settle()
            self._column = max(0, self._column - 1)
        elif character < " " or character == "\x7f":
            raise ValueError(f"the display does not know the character {character!r}")
        else:
            self._put(character)

    def _put(self, character: str) -> None:
        width = get_cwidth(character)
        if width == 0:
            # A mark that goes onto the character before it.
            line = self._line()
            if 0 < self._column <= len(line):
                line[self._column - 1] += character
            return
        if self._column + width > self.columns:
            if self._wraps:
                self._column = 0
                self._down()
            else:
                self._column = self.columns - width
        line = self._line()
        line.extend(" " * (self._column + width - len(line)))
        if line[self._column] == _HALF:
            # The first half of a wide character goes when its second half is written over.
            line[self._column - 1] = " "
        after = self._column + width
        if after < len(line) and line[after] == _HALF:
            # And the second half goes with the first.
            line[after] = " "
        line[self._column] = character
        if width == 2:
            line[self._column + 1] = _HALF
        self._column += width

    def _sequence(
        self, told: str, private: bool, given: list[int], space: bool, final: str
    ) -> None:
        unknown = ValueError(f"the display does not know the sequence {told!r}")
        if private:
            self._modes(given, final, unknown)
        elif space:
            # The shape of the cursor.
            if final != "q":
                raise unknown
        elif final in "ABCD":
            self._move(final, given[0] if given else 1)
        elif final == "H":
            row, column = [*given, 1, 1][:2]
            self._row = self._top + min(row, self.rows) - 1
            self._column = min(column, self.columns) - 1
        elif final == "K" and not given:
            del self._line()[self._column :]
        elif final == "J" and not given:
            del self._line()[self._column :]
            for line in self._lines[self._row + 1 :]:
                line.clear()
        elif final == "J" and given == [2]:
            for line in self._lines[self._top : self._top + self.rows]:
                line.clear()
        elif final not in "mn":
            # Colours are not kept, and nobody answers the question where the cursor is.
            raise unknown

    def _modes(self, given: list[int], final: str, unknown: ValueError) -> None:
        if final not in "hl":
            raise unknown
        for mode in given:
            if mode == _WRAPPING:
                self._wraps = final == "h"
            elif mode == _CURSOR:
                self.cursor_shown = final == "h"
            elif mode not in _NOTHING_TO_SEE:
                raise unknown

    def _move(self, final: str, amount: int) -> None:
        self._settle()
        if final == "A":
            self._row = max(self._top, self._row - amount)
        elif final == "B":
            self._row = min(self._top + self.rows - 1, self._row + amount)
        elif final == "C":
            self._column = min(self.columns - 1, self._column + amount)
        else:
            self._column = max(0, self._column - amount)


class Emulated(Vt100_Output):
    """What ``prompt_toolkit`` draws on in a test: a terminal that is emulated.

    Attributes:
        display: The screen, which a test looks at.
    """

    def __init__(self, display: Display) -> None:
        super().__init__(
            cast("TextIO", display),
            lambda: display.size,
            term="xterm",
            enable_bell=False,
            enable_cpr=False,
        )
        self.display = display

    def get_rows_below_cursor_position(self) -> int:
        """Say how much room there is below the cursor, as a console of Windows can."""
        self.flush()
        return self.display.rows_below()
