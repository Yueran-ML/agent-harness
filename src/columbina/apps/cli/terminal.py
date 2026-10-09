"""The terminal that the interface talks to: where keys come from and where text goes.

The interface does not look for the console of the system by itself. It is
handed a :class:`Terminal`. A program that was started in a terminal gets the
real one from :func:`real`. A test provides one under :data:`TERMINAL`, with a
pipe to type into and a screen that keeps what was shown.

Keys and screen are those of ``prompt_toolkit``, which knows the consoles of
the systems apart.
"""

import os
import sys
from collections.abc import Mapping
from dataclasses import dataclass

from prompt_toolkit.input import Input, create_input
from prompt_toolkit.output import Output, create_output

from columbina.kernel import ServiceKey

__all__ = ["NO_CONSOLE", "TERMINAL", "Terminal", "marks_encoding", "real"]

if sys.platform == "win32":  # pragma: windows only
    from prompt_toolkit.output.win32 import NoConsoleScreenBufferError

    NO_CONSOLE: tuple[type[Exception], ...] = (NoConsoleScreenBufferError,)
else:  # pragma: posix only
    NO_CONSOLE: tuple[type[Exception], ...] = ()
"""What is raised where a window says that it is a terminal and is no console of the system."""


@dataclass(frozen=True, slots=True)
class Terminal:
    """Where keys come from and where text goes.

    Attributes:
        input: The keys.
        output: The screen.
        encoding: The encoding of the screen. A symbol outside it cannot be
            shown there.
        colours: Whether colours are used.
    """

    input: Input
    output: Output
    encoding: str = "utf-8"
    colours: bool = True

    def symbol(self, wanted: str, plain: str) -> str:
        """Return a symbol if the screen can show it, and its plain spelling if not."""
        try:
            wanted.encode(self.encoding)
        except (UnicodeEncodeError, LookupError):
            return plain
        return wanted


TERMINAL: ServiceKey[Terminal] = ServiceKey("terminal")
"""The terminal of the program, if somebody provides another one than the real one."""


def marks_encoding(platform: str, environ: Mapping[str, str], encoding: str | None) -> str:
    """Return the encoding that says which marks a screen can show.

    The old console of Windows is sent every character there is and shows
    only those that its font holds, and nothing tells which those are. Plain
    letters are always among them, so marks are plain there. The newer
    terminal of Windows says in the environment that it is there.

    Args:
        platform: The platform, as ``sys.platform`` names it.
        environ: The environment of the program.
        encoding: The encoding of the standard output, if it names one.
    """
    if platform == "win32" and "WT_SESSION" not in environ:
        return "ascii"
    return encoding or "utf-8"


def real() -> Terminal | None:
    """Return the terminal that the program was started in, or ``None`` if there is none.

    There is none when the input or the output of the program is a file or a
    pipe, and on Windows when the window is not a console of the system,
    which is so in some terminals that are made for other systems.
    """
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        return None
    try:
        screen = create_output()
    except NO_CONSOLE:
        return None
    # The convention of NO_COLOR: present and not empty.
    colours = not os.environ.get("NO_COLOR")
    encoding = marks_encoding(sys.platform, os.environ, sys.stdout.encoding)
    return Terminal(create_input(), screen, encoding, colours)
