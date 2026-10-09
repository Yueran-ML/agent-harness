"""Text that arrives in pieces: the lines that are complete, and the one that is not yet."""

from itertools import pairwise

from hypothesis import given
from hypothesis import strategies as st

from columbina.apps.cli.lines import Lines


def test_a_piece_completes_as_many_lines_as_it_has_line_breaks() -> None:
    lines = Lines()

    assert lines.feed("one\ntwo\nthr") == ["one", "two"]
    assert lines.partial == "thr"


def test_text_without_a_line_break_stays_unfinished_until_one_comes() -> None:
    lines = Lines()

    assert lines.feed("Hel") == []
    assert lines.feed("lo wor") == []
    assert lines.partial == "Hello wor"
    assert lines.feed("ld\n") == ["Hello world"]
    assert lines.partial == ""


def test_an_empty_line_is_a_line() -> None:
    lines = Lines()

    assert lines.feed("one\n\ntwo\n") == ["one", "", "two"]


def test_a_line_break_of_two_characters_counts_once() -> None:
    lines = Lines()

    assert lines.feed("one\r\ntwo\r\n") == ["one", "two"]


def test_a_line_break_of_two_characters_that_comes_in_two_pieces_counts_once() -> None:
    lines = Lines()

    assert lines.feed("one\r") == []
    assert lines.feed("\ntwo") == ["one"]
    assert lines.partial == "two"


def test_a_carriage_return_inside_a_line_stays_where_it_is() -> None:
    lines = Lines()

    assert lines.feed("10%\r20%\n") == ["10%\r20%"]


def test_the_unfinished_line_is_handed_over_when_nothing_more_comes() -> None:
    lines = Lines()
    lines.feed("one\ntwo")

    assert lines.flush() == "two"
    assert lines.partial == ""
    assert lines.flush() is None


def test_without_an_unfinished_line_nothing_is_handed_over() -> None:
    lines = Lines()
    lines.feed("one\n")

    assert lines.flush() is None


def test_a_carriage_return_that_waited_for_its_line_feed_is_dropped_at_the_end() -> None:
    lines = Lines()
    lines.feed("one\r")

    assert lines.flush() == "one"


TEXT = st.text(alphabet=st.sampled_from(["a", "b", " ", "\n", "\r", chr(0x6708)]), max_size=40)


@given(text=TEXT, cuts=st.lists(st.integers(min_value=0, max_value=40), max_size=8))
def test_however_a_text_is_cut_into_pieces_the_lines_are_the_same(
    text: str, cuts: list[int]
) -> None:
    lines = Lines()
    edges = sorted({0, len(text), *(cut for cut in cuts if cut < len(text))})

    complete = [line for start, end in pairwise(edges) for line in lines.feed(text[start:end])]
    last = lines.flush()

    whole = complete if last is None else [*complete, last]
    expected = text.replace("\r\n", "\n").removesuffix("\r").split("\n")
    if expected[-1] == "":
        # A text that ends with a line break has no unfinished line after it.
        expected.pop()
    assert whole == expected
