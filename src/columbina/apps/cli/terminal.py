"""The terminal that the interface talks to: where keys come from and where text goes.

The interface does not look for the console of the system by itself. It is
handed a :class:`Terminal`. A program that was started in a terminal gets the
real one from :func:`real`. A test provides one under :data:`TERMINAL`, with a
pipe to type into and a screen that keeps what was shown.

Keys and screen are those of ``prompt_toolkit``, which knows the consoles of
the systems apart.

What is shown on a terminal is not all written by this program. A reply
comes from a model, and the output of a command from whatever the command
read. A terminal carries out the control characters it is sent: they move
the cursor, write over what was shown before, name the window, fill the
clipboard. And some characters turn the direction of the text around, so
that it stands on the screen in another order than it has. Text from
elsewhere therefore goes through :func:`harmless` before it is shown, and
what a person is asked to say yes to goes through :func:`spelled_out`.
"""

import os
import sys
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass

from prompt_toolkit.input import Input, create_input
from prompt_toolkit.output import Output, create_output

from columbina.kernel import ServiceKey

__all__ = [
    "NO_CONSOLE",
    "STAND_IN",
    "TERMINAL",
    "Terminal",
    "harmless",
    "marks_encoding",
    "real",
    "spelled_out",
]

STAND_IN = chr(0xFFFD)
"""What is shown in the place of a character that must not reach a terminal."""

_TURNS = frozenset(
    chr(code) for code in (0x061C, 0x200E, 0x200F, *range(0x202A, 0x202F), *range(0x2066, 0x206A))
)
"""The characters that say in which direction text runs."""

_ACTED_ON = frozenset({"Cc", "Cs", "Zl", "Zp"})
"""Kinds of characters that a terminal acts on, or that cannot be sent to one."""

_UNSEEN = frozenset({"Cc", "Cf", "Cs", "Zl", "Zp", "Zs"})
"""Kinds of characters that cannot be seen, or that pass for a space."""


def harmless(text: str) -> str:
    """Return a text that a terminal shows and does not act on.

    A control character, a character that breaks the line without being a
    line feed, and a character that turns the direction of the text are each
    replaced by :data:`STAND_IN`. The text keeps its length. Tabs and line
    feeds stay.
    """
    if text.isprintable():
        return text
    return "".join(
        STAND_IN
        if character in _TURNS
        or (character not in "\t\n" and unicodedata.category(character) in _ACTED_ON)
        else character
        for character in text
    )


def spelled_out(text: str) -> str:
    """Return a text in which every character that cannot be seen is named.

    This is for what a person is asked to say yes to. A character that
    cannot be seen, that passes for a space, or that turns the direction of
    the text is written as its number, like ``<U+200B>``, so that what is
    read is what will be done. Spaces, tabs and line feeds stay.
    """
    if text.isprintable():
        return text
    return "".join(
        f"<U+{ord(character):04X}>"
        if character not in " \t\n" and unicodedata.category(character) in _UNSEEN
        else character
        for character in text
    )


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
