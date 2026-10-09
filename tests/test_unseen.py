"""Which characters cannot be seen."""

import unicodedata

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.unseen import unseen

SEEN = ["a", "Z", "0", "-", "~", chr(0xE9), chr(0x4E2D), chr(0x1F600), chr(0x0301), chr(0x3042)]
"""Letters of several scripts, a digit, signs, a face, and an accent that sits on a letter."""

UNSEEN = [
    0x00,  # a control character
    0x1B,
    0x7F,
    0x85,
    0x20,  # the space, and what passes for one
    0xA0,
    0x2003,
    0x3000,
    0x2028,  # what breaks a line or a paragraph
    0x2029,
    0xAD,  # what gives a text its form and is not drawn
    0x200B,
    0x200D,
    0x202E,
    0x2060,
    0xFEFF,
    0xD800,  # half of a pair
    0xE000,  # a character for private use
    0xFFF0,  # a character that has no meaning yet
    0x034F,  # letters and marks that are drawn as nothing
    0x115F,
    0x1160,
    0x17B4,
    0x17B5,
    0x180B,
    0x180F,
    0x2800,
    0x3164,
    0xFE00,
    0xFE0F,
    0xFFA0,
    0xE0100,
    0xE01EF,
]


@pytest.mark.parametrize("character", SEEN)
def test_a_character_that_is_drawn_can_be_seen(character: str) -> None:
    assert not unseen(character)


@pytest.mark.parametrize("code", UNSEEN)
def test_a_character_that_is_not_drawn_or_passes_for_a_space_cannot_be_seen(code: int) -> None:
    assert unseen(chr(code))


def test_the_neighbours_of_what_is_drawn_as_nothing_are_seen() -> None:
    # The table of those characters names ranges, and a range ends where it says.
    for code in (0x034E, 0x0350, 0x115E, 0x1161, 0x17B3, 0x17B6, 0x180A, 0x27FF, 0x2801):
        assert not unseen(chr(code)), hex(code)
    for code in (0x3163, 0x3165, 0xFE10, 0xFF9F, 0xFFA1):
        assert not unseen(chr(code)), hex(code)


@given(st.characters(categories=["Cc", "Cf", "Cs", "Co", "Cn", "Zs", "Zl", "Zp"]))
def test_every_control_format_and_separator_character_cannot_be_seen(character: str) -> None:
    assert unseen(character)


@given(st.characters(categories=["Lu", "Ll", "Nd", "Po", "Sm"]))
def test_cased_letters_digits_and_signs_are_seen(character: str) -> None:
    assert unicodedata.category(character) in ("Lu", "Ll", "Nd", "Po", "Sm")
    assert not unseen(character)
