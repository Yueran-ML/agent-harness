"""Which characters cannot be seen.

Where a person is asked to say yes to a text, and where a rule is matched
against one, the text has to be what it looks like. A character that is not
drawn makes two texts look the same that are not: the name of another file,
another command. So the two places that deal with this go by one answer to
the question which characters those are.
"""

import unicodedata

__all__ = ["unseen"]

_DRAWN_AS_NOTHING = frozenset(
    chr(code)
    for code in (
        0x034F,  # joins the characters beside it
        0x115F,  # fillers of Hangul
        0x1160,
        0x3164,
        0xFFA0,
        0x17B4,  # vowels of Khmer that are not written
        0x17B5,
        *range(0x180B, 0x1810),  # choose the form of a letter of Mongolian
        *range(0xFE00, 0xFE10),  # choose the form of the character before them
        *range(0xE0100, 0xE01F0),
        0x2800,  # the pattern of Braille without dots
    )
)
"""Letters, marks and signs of which nothing is drawn.

Unicode files them with the letters, the marks and the symbols, so their
kind does not tell them from what is seen. All but the last are among those
that a program is told to pass over if it cannot show them.
"""


def unseen(character: str) -> bool:
    """Tell whether a character is not seen, or is seen as a space.

    That holds for every control character, for the characters that give a
    text its form or turn its direction, for every kind of space and of line
    break, for half of a pair, for a character that is for private use or
    has no meaning yet, and for the letters, marks and signs that are drawn
    as nothing.

    The plain space, the tab and the line feed are among them. Whoever asks
    knows which of those belong in the text at hand.

    A mark that is drawn on the letter before it, like an accent, is seen.
    """
    return unicodedata.category(character)[0] in "CZ" or character in _DRAWN_AS_NOTHING
