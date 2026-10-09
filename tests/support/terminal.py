"""A terminal for tests: keys are typed into a pipe, and what is shown is kept.

The screen of a test is a terminal that is emulated, so a test can read what
stands on it the way a person at a real one would: :mod:`support.vt` carries
out what the library that draws the prompt sends.

Beside that, the screen keeps two things apart which a real one shows one
above the other. What was printed is there for good: the conversation. What
was drawn is the prompt, which is drawn anew whenever something changes. The
library hides the cursor while it draws and shows it again afterwards, and
that is how the two are told apart here.
"""

import asyncio
import contextlib
from collections.abc import Callable, Generator
from dataclasses import dataclass

from prompt_toolkit.input import PipeInput, create_pipe_input

from columbina.apps.cli.terminal import Terminal
from support.vt import Display, Emulated

__all__ = ["Desk", "Screen", "desk", "stands", "until"]

ENTER = "\r"
ESCAPE = "\x1b"
CTRL_C = "\x03"
CTRL_D = "\x04"


async def until(holds: Callable[[], bool], seconds: float = 5.0) -> None:
    """Wait for something that the terminal does in its own time."""
    async with asyncio.timeout(seconds):
        # What is waited for is drawn or typed by the terminal, which sets no event.
        while not holds():  # noqa: ASYNC110
            await asyncio.sleep(0.005)


async def stands(rows: Callable[[], list[str]], wanted: list[str], seconds: float = 5.0) -> None:
    """Wait until rows of a screen are what is wanted, and say what they are if they never are."""
    try:
        await until(lambda: rows() == wanted, seconds)
    except TimeoutError:
        raise AssertionError(f"wanted {wanted!r}, and the screen has {rows()!r}") from None


class Screen(Emulated):
    """A screen that shows what a terminal would, and keeps printed and drawn text apart.

    Attributes:
        printed: The pieces of text that were printed for good.
        drawn: The pieces of text that were drawn as a part of the prompt.
        everything: Both, in the order in which they came.
    """

    def __init__(self, rows: int = 24, columns: int = 80) -> None:
        super().__init__(Display(rows, columns))
        self.printed: list[str] = []
        self.drawn: list[str] = []
        self.everything: list[str] = []
        self._drawing = False

    def hide_cursor(self) -> None:
        self._drawing = True
        super().hide_cursor()

    def show_cursor(self) -> None:
        self._drawing = False
        super().show_cursor()

    def write(self, data: str) -> None:
        # Only text is kept. What moves the cursor or sets a colour is carried out by the
        # display and is nothing that a person reads.
        (self.drawn if self._drawing else self.printed).append(data)
        self.everything.append(data)
        super().write(data)


def _plain(pieces: list[str]) -> str:
    return "".join(pieces).replace("\r\n", "\n")


@dataclass(frozen=True)
class Desk:
    """A terminal with somebody at it who is a test.

    Attributes:
        terminal: What the interface is given.
    """

    terminal: Terminal
    _keys: PipeInput
    _screen: Screen

    def type(self, text: str) -> None:
        """Press the keys that write ``text``."""
        self._keys.send_text(text)

    def enter(self, line: str = "") -> None:
        """Type a line and press Enter."""
        self._keys.send_text(line + ENTER)

    def hang_up(self) -> None:
        """Close the input, as when the terminal goes away."""
        self._keys.close()

    def seen(self) -> str:
        """Return what was printed for good so far: the conversation above the prompt."""
        return _plain(self._screen.printed)

    def drawn(self) -> str:
        """Return everything that was ever drawn as the prompt, one drawing after the other."""
        return _plain(self._screen.drawn)

    def shown(self) -> str:
        """Return all text that the screen was sent, printed and drawn, in the order it came."""
        return _plain(self._screen.everything)

    def screen(self) -> list[str]:
        """Return the lines that stand on the screen now, with those that scrolled away."""
        self._screen.flush()
        return self._screen.display.lines()

    def window(self) -> list[str]:
        """Return the rows of the window as they are now."""
        self._screen.flush()
        return self._screen.display.window()

    def cursor(self) -> tuple[int, int]:
        """Return the row of the window and the column that the cursor is in."""
        self._screen.flush()
        return self._screen.display.cursor

    async def sees(self, text: str, seconds: float = 5.0) -> None:
        """Wait until a text was printed. What is shown gets to the screen in its own time."""
        try:
            await until(lambda: text in self.seen(), seconds)
        except TimeoutError:
            raise AssertionError(f"{text!r} was not printed. Printed was:\n{self.seen()}") from None

    async def draws(self, text: str, seconds: float = 5.0) -> None:
        """Wait until a text was drawn as a part of the prompt."""
        try:
            await until(lambda: text in self.drawn(), seconds)
        except TimeoutError:
            raise AssertionError(f"{text!r} was not drawn. Drawn was:\n{self.drawn()}") from None


@contextlib.contextmanager
def desk(encoding: str = "utf-8", rows: int = 24, columns: int = 80) -> Generator[Desk, None, None]:
    """Set up a terminal that takes its keys from a pipe and keeps what it shows."""
    with create_pipe_input() as keys:
        screen = Screen(rows, columns)
        terminal = Terminal(keys, screen, encoding=encoding, colours=False)
        yield Desk(terminal, keys, screen)
