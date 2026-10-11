"""API keys: what one looks like."""

import string

import pytest

from columbina.llm.credentials import sendable, trimmed

KEY = "sk-test-0123456789"


@pytest.mark.parametrize(
    "found",
    [KEY, f"{KEY}\n", f"{KEY}\r\n", f" \t{KEY}", f"\n {KEY} \n", KEY + chr(0xA0)],
    ids=["as it is", "a line break", "a line break of Windows", "blanks", "both", "a hard blank"],
)
def test_what_is_around_a_key_is_no_part_of_it(found: str) -> None:
    assert trimmed(found) == KEY


def test_what_is_inside_a_key_stays() -> None:
    assert trimmed(" sk test\n0123 ") == "sk test\n0123"
    assert trimmed(" \n\t") == ""


def test_a_key_of_visible_characters_can_be_sent() -> None:
    every = string.ascii_letters + string.digits + string.punctuation

    assert sendable(KEY)
    assert sendable(every)
    # The first and the last of the characters that can be seen.
    assert sendable("!")
    assert sendable("~")


@pytest.mark.parametrize(
    "key",
    [
        "",
        " ",
        "sk test",
        "sk\ntest",
        "sk\ttest",
        "sk" + chr(0x1F),
        "sk" + chr(0x7F),
        "sk" + chr(0xE9),
        chr(0x201C) + "sk" + chr(0x201D),
        "sk" + chr(0x200B),
        "sk" + chr(0xFEFF),
    ],
    ids=[
        "nothing",
        "a blank alone",
        "a blank",
        "a line break",
        "a tab",
        "the last control character before the blank",
        "the control character after the tilde",
        "a letter with an accent",
        "the quotation marks of a word processor",
        "a blank of no width",
        "the mark at the head of a file",
    ],
)
def test_a_key_with_anything_else_in_it_cannot_be_sent(key: str) -> None:
    assert not sendable(key)
