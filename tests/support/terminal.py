"""A terminal for tests: keys are typed into a pipe, and what is shown is kept as text.

The screen of a test keeps two things apart, which a real screen shows one
above the other. What was printed is there for good: the conversation. What
was drawn is the prompt, which is drawn anew whenever something changes. The
library that draws the prompt hides the cursor while it draws and shows it
again afterwards, and that is how the two are told apart here.
"""

import asyncio
import contextlib
import io
from collections.abc import Callable, Generator
from dataclasses import dataclass

from prompt_toolkit.input import PipeInput, create_pipe_input
from prompt_toolkit.output.plain_text import PlainTextOutput

from columbina.apps.cli.terminal import Terminal

__all__ = ["Desk", "Screen", "desk", "until"]

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


class Screen(PlainTextOutput):
    """A screen that keeps what was printed for good apart from what was drawn as the prompt."""

    def __init__(self) -> None:
        self.everything = io.StringIO()
        super().__init__(self.everything)
        self.printed: list[str] = []
        self.drawn: list[str] = []
        self._drawing = False

    def hide_cursor(self) -> None:
        self._drawing = True

    def show_cursor(self) -> None:
        self._drawing = False

    def write(self, data: str) -> None:
        (self.drawn if self._drawing else self.printed).append(data)
        super().write(data)

    def write_raw(self, data: str) -> None:
        (self.drawn if self._drawing else self.printed).append(data)
        super().write_raw(data)


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
        """Return everything that the screen was sent, printed and drawn, in the order it came."""
        return self._screen.everything.getvalue()

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
def desk(encoding: str = "utf-8") -> Generator[Desk, None, None]:
    """Set up a terminal that takes its keys from a pipe and keeps what it shows."""
    with create_pipe_input() as keys:
        screen = Screen()
        terminal = Terminal(keys, screen, encoding=encoding, colours=False)
        yield Desk(terminal, keys, screen)
