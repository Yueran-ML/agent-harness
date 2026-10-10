r"""Text that can be written down.

A text in Python can hold half of a character. UTF-16 writes a character
outside its first plane as two units, a pair, and Python lets either unit
stand alone in a text. JSON can spell such a half, as ``"\ud83d"``, and a
reader of JSON hands it on as it stands. So a half can arrive in anything
that came as JSON from elsewhere: the answer of a hook, a file of a project,
the reply of a model. It can arrive as two halves side by side, too, when
the one who wrote them cut a text between them.

A text with a half in it cannot be written as UTF-8. It cannot go into the
log of a session, into a request, or to a program that is told something.
:func:`whole` makes a text that can.
"""

import re

__all__ = ["whole"]

_HALF = re.compile("[\ud800-\udfff]")


def whole(text: str) -> str:
    """Return a text without half characters.

    Two halves that stand together and make a character become that
    character. A half that stands alone becomes U+FFFD, the sign for
    something that could not be read. A text that holds no half comes back
    as it is.
    """
    if _HALF.search(text) is None:
        return text
    return text.encode("utf-16-le", "surrogatepass").decode("utf-16-le", "replace")
