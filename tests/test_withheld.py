"""Texts that are for no log stay out of every record, whoever writes it."""

import logging
from collections.abc import Iterator

import pytest

from columbina.withheld import withhold, without

TOKEN = "tok-9f2c7a11d4"
LIBRARY = "a_library.transport"


class Kept(logging.Handler):
    """Keeps what is logged, as a handler writes it out: the line, and the traceback under it."""

    def __init__(self) -> None:
        super().__init__()
        self.lines: list[str] = []
        self.setFormatter(logging.Formatter("%(name)s: %(message)s"))

    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(self.format(record))


@pytest.fixture
def kept() -> Iterator[Kept]:
    """What a library logs, as it would stand in a file."""
    handler = Kept()
    library = logging.getLogger(LIBRARY)
    library.addHandler(handler)
    library.propagate = False
    try:
        yield handler
    finally:
        library.removeHandler(handler)
        library.propagate = True


def test_a_text_that_is_withheld_is_in_no_record_of_a_library(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    let_go = withhold([TOKEN])
    try:
        library.warning("the request to https://example.com/mcp?key=%s failed", TOKEN)
        library.error(f"401 for the header 'Bearer {TOKEN}'")
    finally:
        let_go()

    assert kept.lines == [
        f"{LIBRARY}: the request to https://example.com/mcp?key=[...] failed",
        f"{LIBRARY}: 401 for the header 'Bearer [...]'",
    ]


def test_a_text_that_is_withheld_is_in_no_traceback_either(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    let_go = withhold([TOKEN])
    try:
        try:
            raise ConnectionError(f"no route to https://example.com/mcp?key={TOKEN}")
        except ConnectionError:
            library.exception("the stream broke")
    finally:
        let_go()

    (line,) = kept.lines
    assert line.startswith(f"{LIBRARY}: the stream broke\nTraceback (most recent call last):")
    assert line.endswith("ConnectionError: no route to https://example.com/mcp?key=[...]")
    assert TOKEN not in line


def test_a_text_that_is_withheld_is_not_in_the_stack_that_a_record_carries(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    let_go = withhold([TOKEN])
    try:
        # The stack is written with the line of the program that logged. That line names
        # the text here, as a line does that builds an address.
        library.warning("called", stack_info=(TOKEN == "tok-9f2c7a11d4"))
    finally:
        let_go()

    (line,) = kept.lines
    assert line.startswith(f"{LIBRARY}: called\nStack (most recent call last):")
    assert 'library.warning("called", stack_info=(TOKEN == "[...]"))' in line
    assert TOKEN not in line


def test_a_record_that_holds_nothing_withheld_is_as_it_was_made(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    let_go = withhold([TOKEN])
    try:
        library.warning("%d of %d requests failed", 2, 5)
        try:
            raise ValueError("nothing to hide")
        except ValueError:
            library.exception("it broke")
    finally:
        let_go()

    assert kept.lines[0] == f"{LIBRARY}: 2 of 5 requests failed"
    assert kept.lines[1].endswith("ValueError: nothing to hide")


def test_once_it_is_let_go_a_text_is_logged_as_any_other(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    before = logging.getLogRecordFactory()

    let_go = withhold([TOKEN])
    assert logging.getLogRecordFactory() is not before
    let_go()

    # Nothing is withheld any more, and records are made as they were before.
    assert logging.getLogRecordFactory() is before
    library.warning("the key is %s", TOKEN)
    assert kept.lines == [f"{LIBRARY}: the key is {TOKEN}"]
    # Letting go a second time lets nothing go that somebody else withholds.
    other = withhold([TOKEN])
    let_go()
    assert without(f"key={TOKEN}") == "key=[...]"
    other()
    assert without(f"key={TOKEN}") == f"key={TOKEN}"


def test_a_text_that_two_withhold_is_withheld_until_both_have_let_it_go(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    before = logging.getLogRecordFactory()
    first = withhold([TOKEN, "another-secret"])
    second = withhold([TOKEN])

    first()
    library.warning("%s and another-secret", TOKEN)
    second()
    library.warning("%s at last", TOKEN)

    assert kept.lines == [
        f"{LIBRARY}: [...] and another-secret",
        f"{LIBRARY}: {TOKEN} at last",
    ]
    assert logging.getLogRecordFactory() is before


def test_the_longest_text_is_taken_out_first() -> None:
    # One text is part of another. Taken out first, it would leave the rest of the longer one.
    let_go = withhold(["secret", "secret-and-more"])
    try:
        assert without("a secret-and-more, and a secret") == "a [...], and a [...]"
    finally:
        let_go()


def test_a_text_of_no_length_withholds_nothing() -> None:
    before = logging.getLogRecordFactory()

    let_go = withhold(["", ""])

    # It is in every text, and taking it out would take nothing out.
    assert without("plain words") == "plain words"
    assert logging.getLogRecordFactory() is before
    let_go()
    assert logging.getLogRecordFactory() is before


def test_a_record_that_cannot_be_put_together_is_left_to_whoever_writes_it() -> None:
    let_go = withhold([TOKEN])
    try:
        # More places than words: a mistake of whoever logs. Logging says so in its own way
        # when the record is written out, and gets the record as it was made for that.
        make = logging.getLogRecordFactory()
        record = make(LIBRARY, logging.WARNING, __file__, 1, "%s and %s", ("one word",), None)
    finally:
        let_go()

    assert (record.msg, record.args) == ("%s and %s", ("one word",))


def test_a_factory_that_somebody_else_put_in_meanwhile_is_left_in_place(kept: Kept) -> None:
    library = logging.getLogger(LIBRARY)
    before = logging.getLogRecordFactory()
    let_go = withhold([TOKEN])
    ours = logging.getLogRecordFactory()

    def theirs(*arguments: object, **more: object) -> logging.LogRecord:
        record = ours(*arguments, **more)
        record.msg = f"[theirs] {record.msg}"
        return record

    logging.setLogRecordFactory(theirs)
    try:
        let_go()
        # Theirs is built on what was there, and taking that away would break it. It stays,
        # and nothing is withheld by what it is built on any more.
        assert logging.getLogRecordFactory() is theirs
        library.warning("the key is %s", TOKEN)
        assert kept.lines == [f"{LIBRARY}: [theirs] the key is {TOKEN}"]
    finally:
        logging.setLogRecordFactory(before)
