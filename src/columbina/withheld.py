"""Texts that are for no log.

Some of what a program is given lets somebody in: a token in an address, the
value of a header. Whoever holds such a text keeps it out of what it logs
itself. A library that it uses knows nothing of that, and repeats the text
when it logs why a request failed. What a library logs goes wherever the
logging of Python sends it, past everything that this program does with its
own log.

So such a text is named here for as long as it is in use, and every record
of the log is made without it, whoever writes the record and whoever writes
it out: the message, and the traceback that goes with it.

Records are made by one function for the whole process, see
:func:`logging.setLogRecordFactory`. While a text is withheld, that function
is one of this module. It is taken out again when the last text is let go.
"""

import logging
from collections import Counter
from collections.abc import Callable, Iterable

__all__ = ["withhold", "without"]

_BLOT = "[...]"
"""What stands in the place of a text that is withheld."""

_texts: Counter[str] = Counter()
"""The texts that are withheld, and how many withhold each."""

_longest_first: tuple[str, ...] = ()
"""The texts that are withheld, in the order in which they are taken out.

A record may be made in another thread while a text is added here or let go.
Whoever takes texts out goes through this, which is put in place whole and
never changed.
"""

_placed: list[tuple[Callable[..., logging.LogRecord], Callable[..., logging.LogRecord]]] = []
"""The function of this module that makes records now, and the one that made them before it."""

_written = logging.Formatter()
"""Writes a traceback the way the log does when nothing says otherwise."""


def without(text: str) -> str:
    """Return a text without what is withheld."""
    for secret in _longest_first:
        text = text.replace(secret, _BLOT)
    return text


def _ordered() -> None:
    """Put the texts in the order in which they are taken out, after one was added or let go."""
    global _longest_first
    # The longest first. A text that is part of a longer one would leave the rest of it.
    _longest_first = tuple(sorted(_texts, key=len, reverse=True))


def _cleared(record: logging.LogRecord) -> logging.LogRecord:
    """Take what is withheld out of a record: its message, and what is written under it."""
    try:
        said = record.getMessage()
    except Exception:
        # Its words and what fills them do not fit. Whoever writes it out says so, in the
        # way of the logging of Python, and it is left for that.
        return record
    clean = without(said)
    if clean != said:
        record.msg, record.args = clean, None
    if record.exc_info is not None and record.exc_info[0] is not None:
        # A traceback ends with what the error says, and an error of a request says where
        # the request went. Whoever writes the record out takes the text that is there.
        traced = _written.formatException(record.exc_info)
        if without(traced) != traced:
            record.exc_text = without(traced)
    if record.stack_info:
        record.stack_info = without(record.stack_info)
    return record


def _place() -> None:
    """Have every record made without what is withheld, from now on."""
    before = logging.getLogRecordFactory()

    def made(*arguments: object, **more: object) -> logging.LogRecord:
        return _cleared(before(*arguments, **more))

    _placed.append((made, before))
    logging.setLogRecordFactory(made)


def _displace() -> None:
    """Have records made as they were before, if nobody has built on what makes them now."""
    made, before = _placed.pop()
    if logging.getLogRecordFactory() is made:
        logging.setLogRecordFactory(before)


def withhold(texts: Iterable[str]) -> Callable[[], None]:
    """Keep texts out of every record of the log, and return what lets them go again.

    A text that several withhold stays out until each of them has let it go.
    A text of no length is in every text, and nothing is kept out by it.

    Args:
        texts: The texts. Each is taken out wherever it stands, also inside
            a longer word.

    Returns:
        What ends the withholding of these texts. Calling it again does
        nothing.
    """
    mine = [text for text in texts if text]
    if not mine:
        return lambda: None
    if not _texts:
        _place()
    _texts.update(mine)
    _ordered()
    over: list[None] = []

    def let_go() -> None:
        if over:
            return
        over.append(None)
        _texts.subtract(mine)
        for text in mine:
            if _texts[text] <= 0:
                del _texts[text]
        _ordered()
        if not _texts:
            _displace()

    return let_go
