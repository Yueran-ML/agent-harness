"""Text from elsewhere, made fit for a terminal and for the eyes of a person.

What a frontend shows is not all written by this program. A reply comes from
a model, the output of a command from whatever the command read, an error
message from a service. A terminal carries out the control characters it is
sent: they move the cursor, write over what was shown before, name the
window, fill the clipboard. And some characters turn the direction of the
text around, so that it stands on the screen in another order than it has.

Three functions deal with that, each for one use.

- :func:`harmless` is for text that is shown. What a terminal would act on
  becomes a character that it shows.
- :func:`spelled_out` is for what a person is asked to say yes to. Whatever
  cannot be seen is named, so that what is read is what will be done.
- :func:`escaped` is for JSON that is printed. The same characters are
  written the way JSON writes any character by its number, which changes
  nothing for whoever reads the JSON.
"""

import re
import unicodedata

from columbina.unseen import unseen

__all__ = ["STAND_IN", "escaped", "harmless", "spelled_out"]

STAND_IN = chr(0xFFFD)
"""What is shown in the place of a character that must not reach a terminal."""

_TURNS = frozenset(
    chr(code) for code in (0x061C, 0x200E, 0x200F, *range(0x202A, 0x202F), *range(0x2066, 0x206A))
)
"""The characters that say in which direction text runs."""

_ACTED_ON = frozenset({"Cc", "Cs", "Zl", "Zp"})
"""Kinds of characters that a terminal acts on, or that cannot be sent to one."""


def _acted_on(character: str) -> bool:
    return character in _TURNS or unicodedata.category(character) in _ACTED_ON


def harmless(text: str) -> str:
    """Return a text that a terminal shows and does not act on.

    A control character, a character that breaks the line without being a
    line feed, half of a pair that cannot be sent, and a character that
    turns the direction of the text are each replaced by :data:`STAND_IN`.
    The text keeps its length. Tabs and line feeds stay.
    """
    if text.isprintable():
        return text
    return "".join(
        STAND_IN if character not in "\t\n" and _acted_on(character) else character
        for character in text
    )


def spelled_out(text: str) -> str:
    """Return a text in which every character that cannot be seen is named.

    A character that cannot be seen, that passes for a space, or that turns
    the direction of the text is written as its number, like ``<U+200B>``.
    Which characters those are is told in :mod:`columbina.unseen`. Spaces,
    tabs and line feeds stay.
    """
    if text.isascii() and text.isprintable():
        # Nothing but the letters and signs of a keyboard, which is most of what is asked.
        return text
    return "".join(
        f"<U+{ord(character):04X}>" if character not in " \t\n" and unseen(character) else character
        for character in text
    )


_IN_JSON = re.compile("[^\t\n -~]")
"""Whatever is no plain letter, which is where the characters in question are looked for."""


def _numbered(found: re.Match[str]) -> str:
    character = found.group()
    return f"\\u{ord(character):04x}" if _acted_on(character) else character


def escaped(written: str) -> str:
    """Return JSON in which the characters that a terminal acts on are written by their numbers.

    JSON itself writes the control characters below the space that way. The
    ones above the letters of ASCII it leaves as they are, when it is asked
    to leave other languages readable. Here those are written by their
    numbers too, and so are the characters that turn the direction of text.
    Such a character stands inside a string of the JSON and nowhere else, so
    what the JSON says stays the same.
    """
    return written if written.isprintable() else _IN_JSON.sub(_numbered, written)
