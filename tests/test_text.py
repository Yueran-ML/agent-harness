"""Text that can be written down: no half of a character is left in it."""

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.text import whole

HIGH, LOW = chr(0xD83D), chr(0xDE00)
"""The two halves of a face that grins."""

GONE = chr(0xFFFD)


@pytest.mark.parametrize("text", ["", "plain", "naïve 月亮", "\U0001f600", "a\x00b�"])
def test_a_text_without_a_half_comes_back_as_it_is(text: str) -> None:
    assert whole(text) is text


def test_two_halves_that_stand_together_become_the_character_that_they_spell() -> None:
    assert whole(f"a{HIGH}{LOW}b") == "a\U0001f600b"
    assert whole(f"{HIGH}{LOW}{HIGH}{LOW}") == "\U0001f600\U0001f600"


@pytest.mark.parametrize(
    ("text", "made"),
    [
        (HIGH, GONE),
        (LOW, GONE),
        (f"a{HIGH}", f"a{GONE}"),
        (f"{LOW}a", f"{GONE}a"),
        # The wrong way round, they are two halves and no character.
        (f"{LOW}{HIGH}", f"{GONE}{GONE}"),
        (f"{HIGH}{HIGH}{LOW}", f"{GONE}\U0001f600"),
        (f"{HIGH}a{LOW}", f"{GONE}a{GONE}"),
    ],
)
def test_a_half_that_stands_alone_gives_way_to_the_sign_for_what_could_not_be_read(
    text: str, made: str
) -> None:
    assert whole(text) == made


@given(st.text(alphabet=st.characters(codec=None)))
def test_whatever_a_text_holds_it_can_be_written_afterwards(text: str) -> None:
    made = whole(text)

    assert made.encode("utf-8").decode("utf-8") == made
    # Making it whole a second time changes nothing.
    assert whole(made) == made


@given(st.text())
def test_a_text_that_can_be_written_is_not_changed(text: str) -> None:
    assert whole(text) == text
