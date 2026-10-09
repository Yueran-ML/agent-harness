"""The terminal that the tests of the interface look at shows what a real one would.

The tests of the interface read what stands on a screen that is emulated. That
is only worth something if the emulation is right, so here it is held against
what a terminal does with each thing it is sent, and against the library that
draws the prompt.
"""

import asyncio

import pytest
from prompt_toolkit import PromptSession
from prompt_toolkit.input import create_pipe_input

from support.terminal import until
from support.vt import Display, Emulated

ESC = "\x1b"
WIDE = chr(0x4E2D)
ACCENT = chr(0x0301)


def after(*sent: str, rows: int = 5, columns: int = 10) -> Display:
    """A display that was sent these pieces, one after the other."""
    display = Display(rows, columns)
    for piece in sent:
        display.write(piece)
    return display


# Text


def test_a_new_display_is_empty_with_the_cursor_at_the_top() -> None:
    display = Display(rows=3, columns=10)

    assert display.lines() == []
    assert display.window() == ["", "", ""]
    assert display.cursor == (0, 0)
    assert display.size == (3, 10)
    assert display.isatty()


def test_text_is_written_from_the_left_and_the_cursor_follows() -> None:
    display = after("one")

    assert display.lines() == ["one"]
    assert display.cursor == (0, 3)


def test_a_carriage_return_and_a_line_feed_begin_the_next_line() -> None:
    display = after("one\r\ntwo\r\n")

    assert display.lines() == ["one", "two"]
    assert display.cursor == (2, 0)


def test_a_line_feed_alone_keeps_the_column() -> None:
    assert after("ab\ncd").lines() == ["ab", "  cd"]


def test_after_a_carriage_return_text_is_written_over_what_was_there() -> None:
    assert after("abcdef\rxy").lines() == ["xycdef"]


def test_a_backspace_goes_one_column_back_and_not_beyond_the_first() -> None:
    assert after("abc\b\bX").lines() == ["aXc"]
    assert after("a\b\b\bX").lines() == ["X"]


def test_spaces_at_the_end_of_a_line_are_not_part_of_what_is_read() -> None:
    assert after("a   \r\nb  ").lines() == ["a", "b"]


def test_a_mark_that_goes_onto_a_letter_takes_no_cell() -> None:
    display = after(f"e{ACCENT}x")

    assert display.lines() == [f"e{ACCENT}x"]
    assert display.cursor == (0, 2)
    assert after(ACCENT).lines() == []


# Wide characters


def test_a_wide_character_takes_two_cells() -> None:
    display = after(f"{WIDE}a")

    assert display.lines() == [f"{WIDE}a"]
    assert display.cursor == (0, 3)


def test_a_wide_character_that_does_not_fit_into_the_line_goes_to_the_next() -> None:
    assert after(WIDE * 3, columns=5).lines() == [WIDE * 2, WIDE]


def test_half_of_a_wide_character_does_not_stay_when_the_other_half_is_written_over() -> None:
    assert after(f"{WIDE}{WIDE}\rx").lines() == [f"x {WIDE}"]
    assert after(f"{WIDE}{WIDE}\r{ESC}[Cx").lines() == [f" x{WIDE}"]
    assert after(f"a{WIDE}b\r{WIDE}").lines() == [f"{WIDE} b"]


# The end of a line


def test_text_that_is_longer_than_the_window_is_wide_goes_on_in_the_next_line() -> None:
    assert after("abcdefg", columns=5).lines() == ["abcde", "fg"]


def test_without_wrapping_the_last_column_is_written_over() -> None:
    display = after(f"{ESC}[?7labcdefg", columns=5)

    assert display.lines() == ["abcdg"]
    assert after(f"{ESC}[?7labc{WIDE}{WIDE}", columns=5).lines() == [f"abc{WIDE}"]
    assert after(f"{ESC}[?7l{ESC}[?7habcdefg", columns=5).lines() == ["abcde", "fg"]


def test_after_the_last_column_the_cursor_waits_there_until_more_text_comes() -> None:
    full = after("abcde", columns=5)
    then_a_new_line = after("abcde\r\nf", columns=5)
    then_back = after(f"abcde{ESC}[2DX", columns=5)

    assert full.cursor == (0, 4)
    assert full.lines() == ["abcde"]
    assert then_a_new_line.lines() == ["abcde", "f"]
    assert then_back.lines() == ["abXde"]
    assert after("abcde\bX", columns=5).lines() == ["abcXe"]
    assert after("abcde\nf", columns=5).lines() == ["abcde", "    f"]


# The window


def test_the_window_follows_the_cursor_and_keeps_what_scrolled_away() -> None:
    display = after("1\r\n2\r\n3\r\n4", rows=3)

    assert display.lines() == ["1", "2", "3", "4"]
    assert display.window() == ["2", "3", "4"]
    assert display.cursor == (2, 1)


def test_text_that_wraps_in_the_last_row_scrolls_the_window_too() -> None:
    display = after("1\r\n2\r\nabcdefg", rows=3, columns=5)

    assert display.window() == ["2", "abcde", "fg"]


def test_the_room_below_the_cursor_is_counted_with_the_row_of_the_cursor() -> None:
    assert Display(rows=4, columns=10).rows_below() == 4
    assert after("a\r\n", rows=4).rows_below() == 3
    assert after("1\r\n2\r\n3\r\n4\r\n5", rows=4).rows_below() == 1
    assert after(f"1\r\n2\r\n3\r\n4\r\n5{ESC}[2A", rows=4).rows_below() == 3


# Moving the cursor


def test_the_cursor_is_moved_by_as_many_cells_as_a_sequence_says() -> None:
    display = after(f"one\r\ntwo\r\nthree{ESC}[2A{ESC}[3DX{ESC}[BY{ESC}[2CZ")

    assert display.lines() == ["onX", "twoY  Z", "three"]


def test_a_sequence_without_a_number_moves_by_one() -> None:
    assert after(f"abc{ESC}[D{ESC}[DX").lines() == ["aXc"]
    assert after(f"a\r\nb{ESC}[A{ESC}[CX").lines() == ["a X", "b"]


def test_the_cursor_does_not_leave_the_window() -> None:
    display = after(f"1\r\n2\r\n3\r\n4{ESC}[9A{ESC}[9DX{ESC}[99CY{ESC}[9BZ", rows=3, columns=6)

    # Line 1 has scrolled away. Nothing reaches it any more.
    assert display.lines() == ["1", "X    Y", "3", "4    Z"]


def test_the_cursor_is_put_at_a_place_that_is_counted_from_the_top_of_the_window() -> None:
    display = after(f"1\r\n2\r\n3\r\n4{ESC}[2;3HX{ESC}[HY{ESC}[99;99HZ", rows=3, columns=6)

    assert display.window() == ["Y", "3 X", "4    Z"]


# Erasing


def test_the_rest_of_a_line_is_erased() -> None:
    assert after(f"abcdef\r{ESC}[3C{ESC}[K").lines() == ["abc"]


def test_everything_from_the_cursor_on_is_erased() -> None:
    display = after(f"one\r\ntwo\r\nthree{ESC}[2A\r{ESC}[2C{ESC}[J")

    assert display.lines() == ["on"]
    assert display.cursor == (0, 2)


def test_the_window_is_erased_and_what_scrolled_away_is_kept() -> None:
    display = after(f"1\r\n2\r\n3\r\n4{ESC}[2J", rows=3)

    assert display.lines() == ["1"]
    assert display.cursor == (2, 1)


# What shows nothing


def test_colours_and_the_modes_of_the_keys_show_nothing() -> None:
    display = after(
        f"{ESC}[0m{ESC}[0;38;5;196;1ma{ESC}[?2004h{ESC}[?1l{ESC}[?1000l{ESC}[?12l{ESC}[2 qb{ESC}[6n"
    )

    assert display.lines() == ["ab"]


def test_the_cursor_can_be_hidden_and_shown() -> None:
    display = Display()

    display.write(f"{ESC}[?25l")
    hidden = display.cursor_shown
    display.write(f"{ESC}[?12l{ESC}[?25h")

    assert (hidden, display.cursor_shown) == (False, True)


def test_the_name_of_the_window_shows_nothing() -> None:
    assert after(f"a{ESC}]2;the name\x07b").lines() == ["ab"]


def test_a_sequence_that_arrives_in_pieces_is_carried_out_when_it_is_whole() -> None:
    assert after("abc\r", ESC, "[", "2", "CX").lines() == ["abX"]
    assert after(f"a{ESC}]2;the", " name\x07b").lines() == ["ab"]
    assert after(f"abc\r{ESC}[?", "25lX").lines() == ["Xbc"]


# What it does not know


@pytest.mark.parametrize(
    "sent",
    [
        f"{ESC}c",
        f"{ESC}[5i",
        f"{ESC}[?25x",
        f"{ESC}[?1049h",
        f"{ESC}[2 x",
        f"{ESC}[1K",
        f"{ESC}[1J",
        "a\tb",
        "a\x00b",
        "a\x07b",
        "a\x7fb",
    ],
)
def test_what_the_display_does_not_know_is_an_error_and_not_a_guess(sent: str) -> None:
    with pytest.raises(ValueError, match="the display does not know"):
        Display().write(sent)


# With the library that draws


def test_the_output_of_the_library_writes_onto_the_display() -> None:
    display = Display(rows=4, columns=20)
    screen = Emulated(display)

    screen.write("one\r\ntwo")
    screen.cursor_up(1)
    screen.cursor_backward(3)
    screen.write("ONE")
    screen.flush()

    assert display.lines() == ["ONE", "two"]
    assert screen.get_size() == (4, 20)
    assert screen.responds_to_cpr is False


def test_the_library_is_told_how_much_room_there_is_before_it_asks_the_terminal() -> None:
    display = Display(rows=4, columns=20)
    screen = Emulated(display)

    screen.write("one\r\ntwo\r\n")

    # What was written is on its way to the display. Asking sends it there first.
    assert screen.get_rows_below_cursor_position() == 2
    assert display.lines() == ["one", "two"]


def test_text_with_an_escape_in_it_is_not_carried_out_by_the_output_of_the_library() -> None:
    display = Display()
    screen = Emulated(display)

    screen.write(f"a{ESC}[2Jb")
    screen.flush()

    assert display.lines() == ["a?[2Jb"]


async def test_a_prompt_of_the_library_looks_on_the_display_as_it_does_in_a_terminal() -> None:
    with create_pipe_input() as keys:
        display = Display(rows=6, columns=30)
        session: PromptSession[str] = PromptSession(
            "> ", input=keys, output=Emulated(display), bottom_toolbar="the status"
        )
        waiting = asyncio.ensure_future(session.prompt_async())

        keys.send_text("helo")
        await until(lambda: display.window()[0] == "> helo")
        typed = display.window()
        # The key that goes one to the left, and the letter that was left out.
        keys.send_text("\x1b[Dl")
        await until(lambda: display.window()[0] == "> hello")
        corrected = display.cursor
        keys.send_text("\r")

        assert await asyncio.wait_for(waiting, 5) == "hello"
    # The line below the prompt stands in the last row of the window while the prompt is open.
    assert typed == ["> helo", "", "", "", "", "the status"]
    assert corrected == (0, 6)
    # When the line is submitted, it stays, and the line below the prompt is gone.
    assert display.lines() == ["> hello"]
    assert display.cursor == (1, 0)
