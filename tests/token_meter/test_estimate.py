"""How many tokens a text comes to, by the kinds of its characters."""

import json
import math
import time

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.llm import (
    AssistantMessage,
    ReasoningBlock,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    UserMessage,
)
from columbina.token_meter import (
    PER_MESSAGE,
    RATES,
    TOOLS_DECLARED,
    estimate,
    estimate_message,
    estimate_tools,
)

HAN = chr(0x4E2D)
KANA = chr(0x3042)
HANGUL = chr(0xAC00)
FULL_WIDTH_COMMA = chr(0xFF0C)
ACCENTED = chr(0xE9)
CYRILLIC = chr(0x0434)


def of(**counts: int) -> int:
    """What so many characters of each kind come to: the sum, and a part of a token as one."""
    hundredths = sum(round(RATES[kind] * 100) * count for kind, count in counts.items())
    return math.ceil(hundredths / 100)


def test_the_rates_are_those_that_were_measured() -> None:
    # They were chosen on 28 texts that the service of DeepSeek counted. See the plan of
    # stage 5 for the texts and for how far off the table is on them.
    assert RATES == {
        "space": 0.15,
        "digit": 0.50,
        "letter": 0.24,
        "dense": 0.85,
        "sign": 0.70,
        "wide": 0.62,
        "other": 0.50,
    }


def test_nothing_comes_to_nothing() -> None:
    assert estimate("") == 0


def test_a_text_comes_to_a_whole_number_of_tokens_and_a_part_counts_as_one() -> None:
    assert estimate("a") == 1
    assert estimate("abcd") == 1
    assert estimate("abcde") == 2
    # Twenty words of four letters, each with a space after it.
    assert estimate("word " * 20) == 23


def test_a_sign_costs_more_than_a_letter() -> None:
    assert estimate("return") == of(letter=6)
    assert estimate("{};[]()") == of(sign=7)
    assert estimate("return{};[]()") == of(letter=6, sign=7)


def test_spaces_tabs_and_line_breaks_cost_little() -> None:
    assert estimate(" " * 100) == 15
    assert estimate("\t\r\n " * 25) == 15
    assert estimate("a b") == of(letter=2, space=1)


def test_digits_cost_more_than_letters() -> None:
    assert estimate("0123456789") == of(digit=10)
    assert estimate("v1.2") == of(letter=1, sign=1, digit=2)


def test_letters_in_a_long_run_of_letters_and_digits_are_dense() -> None:
    # A hash, a key, a piece of base64: no words, so nearly every letter is a token of its own.
    run = "abcdefghijklmnopqrstuvwx"
    assert len(run) == 24

    assert estimate(run) == of(dense=24)
    assert estimate(run[:-1]) == of(letter=23)
    assert estimate(run + "yz" * 20) == of(dense=64)


def test_the_digits_of_a_long_run_stay_digits() -> None:
    sha = "8d8ebb2a1a960f27f76499eea4ea5d57c9710d9e"
    digits = sum(character.isdigit() for character in sha)

    assert estimate(sha) == of(digit=digits, dense=len(sha) - digits)


def test_a_run_ends_at_whatever_is_no_letter_and_no_digit() -> None:
    twelve = "abcdefghijkl"

    assert estimate(f"{twelve}_{twelve}") == of(letter=24, sign=1)
    assert estimate(f"{twelve} {twelve}") == of(letter=24, space=1)
    assert estimate(f"{twelve}{HAN}{twelve}") == of(letter=24, wide=1)


@pytest.mark.parametrize("character", [HAN, KANA, HANGUL, FULL_WIDTH_COMMA, chr(0x20000)])
def test_a_character_that_east_asian_scripts_write_into_a_square_is_wide(character: str) -> None:
    assert estimate(character * 10) == 7


@pytest.mark.parametrize("character", [ACCENTED, CYRILLIC, chr(0x1F600), chr(0x2014)])
def test_any_other_character_beyond_ascii_counts_as_half_a_token(character: str) -> None:
    assert estimate(character * 10) == 5


def test_the_kinds_of_a_mixed_text_are_added_up() -> None:
    text = f"{HAN * 4} v2 {ACCENTED}"

    assert estimate(text) == of(wide=4, space=2, letter=1, digit=1, other=1)


@given(st.text())
def test_a_text_never_comes_to_more_tokens_than_it_has_characters(text: str) -> None:
    assert 0 <= estimate(text) <= len(text)


@given(st.text(), st.text())
def test_two_texts_that_a_space_holds_apart_come_to_no_more_than_each_by_itself(
    one: str, other: str
) -> None:
    # A space ends a run, so no letter of either text gets dearer by the other being there.
    assert estimate(f"{one} {other}") <= estimate(one) + estimate(other) + 1


def test_a_long_text_is_counted_in_a_time_that_grows_with_its_length() -> None:
    # A million characters, half of them beyond ASCII. Whatever looked at the characters one
    # by one in Python would take a good part of a second for it.
    text = (f"def run(value_{HAN}): return 0x1f " * 30_000)[:1_000_000]

    began = time.perf_counter()
    estimate(text)

    assert time.perf_counter() - began < 0.5


# Messages


def test_a_message_costs_three_tokens_beside_its_text() -> None:
    said = UserMessage(content=(TextBlock(text="word " * 20),))

    assert PER_MESSAGE == 3
    assert estimate_message(said) == 23 + 3
    assert estimate_message(SystemMessage(content=(TextBlock(text="word " * 20),))) == 23 + 3


def test_the_blocks_of_a_message_are_counted_one_by_one() -> None:
    reply = AssistantMessage(
        content=(
            ReasoningBlock(text="word " * 20),
            TextBlock(text="abcd"),
            ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
        )
    )
    call = estimate("read") + estimate('{"path": "a.txt"}') + estimate("call-1")

    assert estimate_message(reply) == 23 + 1 + call + 3


def test_the_result_of_a_call_is_counted_with_the_name_of_its_call() -> None:
    result = ToolMessage(call_id="call-1", content=(TextBlock(text="word " * 20),))

    assert estimate_message(result) == 23 + estimate("call-1") + 3


# Tools


def test_no_tools_cost_nothing() -> None:
    assert estimate_tools(()) == 0


def test_declaring_tools_costs_a_fixed_part_and_what_is_written_about_each() -> None:
    read = ToolSchema(
        name="read",
        description="Read a text file.",
        parameters={"type": "object", "properties": {"path": {"type": "string"}}},
    )
    written = json.dumps(
        {"name": read.name, "description": read.description, "parameters": read.parameters}
    )

    assert TOOLS_DECLARED == 210
    assert estimate_tools((read,)) == 210 + estimate(written)
    assert estimate_tools((read, read)) == 210 + 2 * estimate(written)
