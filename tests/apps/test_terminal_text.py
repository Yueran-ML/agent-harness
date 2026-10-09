"""Text from elsewhere is made fit for a terminal, and for the eyes of a person."""

import json

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.apps import terminal_text
from columbina.apps.terminal_text import escaped, harmless, spelled_out

ESC = chr(0x1B)
STAND_IN = chr(0xFFFD)
OVERRIDE = chr(0x202E)
POP = chr(0x202C)
BACKSLASH = chr(92)

ACTED_ON = [0x00, 0x01, 0x02, 0x07, 0x08, 0x0B, 0x0C, 0x0D, 0x1B, 0x7F, 0x85, 0x9B, 0x9D]
BREAKS = [0x2028, 0x2029]
TURNS = [0x061C, 0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066, 0x2067, 0x2069]


def test_the_stand_in_is_the_character_that_is_made_for_it() -> None:
    assert terminal_text.STAND_IN == STAND_IN


# Text that is shown


def test_ordinary_text_goes_to_the_terminal_as_it_is() -> None:
    joiner, accent, wide_space = chr(0x200D), chr(0x0301), chr(0x3000)
    accented, chinese = f"na{chr(0xEF)}ve caf{chr(0xE9)}", chr(0x5217) + chr(0x8868)
    text = f"{accented}, {chinese}{wide_space}e{accent} a{joiner}b {chr(0x1F600)}"

    assert harmless(text) == text
    assert harmless("") == ""
    assert harmless("one\ttwo\nthree") == "one\ttwo\nthree"


@pytest.mark.parametrize("code", [*ACTED_ON, *BREAKS])
def test_a_character_that_a_terminal_acts_on_is_replaced_by_one_that_it_shows(code: int) -> None:
    assert harmless(f"a{chr(code)}b") == f"a{STAND_IN}b"


@pytest.mark.parametrize("code", TURNS)
def test_a_character_that_turns_the_direction_of_text_is_replaced(code: int) -> None:
    assert harmless(f"a{chr(code)}b") == f"a{STAND_IN}b"


def test_half_of_a_pair_that_cannot_be_sent_is_replaced() -> None:
    assert harmless(f"a{chr(0xD800)}b{chr(0xDFFF)}") == f"a{STAND_IN}b{STAND_IN}"


def test_a_sequence_that_would_steer_the_terminal_is_shown_and_not_carried_out() -> None:
    clear, title = f"{ESC}[2J", f"{ESC}]2;hello{chr(7)}"

    assert harmless(f"before{clear}{title}after") == (
        f"before{STAND_IN}[2J{STAND_IN}]2;hello{STAND_IN}after"
    )


@given(st.text(alphabet=st.characters(), max_size=40))
def test_text_that_is_made_harmless_keeps_its_length_and_has_nothing_left_to_act_on(
    text: str,
) -> None:
    safe = harmless(text)

    assert len(safe) == len(text)
    assert harmless(safe) == safe
    for was, now in zip(text, safe, strict=True):
        assert now in (was, STAND_IN)
    assert not any(chr(code) in safe for code in (*ACTED_ON, *BREAKS, *TURNS))


# What a person is asked to say yes to


def test_what_cannot_be_seen_is_named_where_a_person_is_asked() -> None:
    zero_width, no_break, wide_space = chr(0x200B), chr(0xA0), chr(0x3000)

    assert spelled_out("python setup.py --dry-run") == "python setup.py --dry-run"
    assert spelled_out(f"python setup{zero_width}.py") == "python setup<U+200B>.py"
    assert spelled_out(f"rm{no_break}-rf{wide_space}x") == "rm<U+00A0>-rf<U+3000>x"
    assert spelled_out(f"git push {OVERRIDE}gro.live{POP}") == "git push <U+202E>gro.live<U+202C>"
    assert spelled_out(f"a{ESC}[2Jb\rc") == "a<U+001B>[2Jb<U+000D>c"
    assert spelled_out(f"a{chr(0xD800)}b") == "a<U+D800>b"
    assert spelled_out(f"a{chr(0x1F600)}b") == f"a{chr(0x1F600)}b"


def test_a_letter_or_a_mark_that_is_drawn_as_nothing_is_named_too() -> None:
    # Unicode files these with the letters and the marks, and nothing of them is seen: one
    # that joins what stands beside it, a filler, one that chooses a form, the Braille
    # pattern without dots. A file whose name holds one is another file.
    joiner, filler, selector, blank = chr(0x034F), chr(0x3164), chr(0xFE0F), chr(0x2800)

    assert spelled_out(f"python setup{joiner}.py") == "python setup<U+034F>.py"
    assert spelled_out(f"rm{filler}-rf x") == "rm<U+3164>-rf x"
    assert spelled_out(f"a{selector}b{blank}c") == "a<U+FE0F>b<U+2800>c"
    assert spelled_out(f"a{chr(0xE0100)}b") == "a<U+E0100>b"


def test_a_character_that_has_no_meaning_yet_or_a_private_one_is_named() -> None:
    assert spelled_out(f"a{chr(0xFFF0)}b{chr(0xE000)}c") == "a<U+FFF0>b<U+E000>c"


def test_an_accent_on_a_letter_stays_as_it_is() -> None:
    # A mark that is drawn on the letter before it is a part of what is seen.
    accented = f"cafe{chr(0x0301)} na{chr(0x0303)}o"

    assert spelled_out(accented) == accented


def test_spaces_tabs_and_line_feeds_are_not_named() -> None:
    assert spelled_out("a b\tc\nd") == "a b\tc\nd"


def test_whatever_a_command_is_refused_a_rule_for_is_named_in_the_question() -> None:
    """Where the splitter refuses a command for what cannot be seen in it, the question shows it."""
    from columbina.permissions.commands import split_bash, split_powershell

    for code in (
        0x00,
        0x1B,
        0x7F,
        0x85,
        0xA0,
        0xAD,
        0x034F,
        0x115F,
        0x200B,
        0x200D,
        0x202E,
        0x2028,
        0x2800,
        0x3000,
        0x3164,
        0xD800,
        0xE000,
        0xFE0F,
        0xFEFF,
        0xFFA0,
        0xFFF0,
        0xE0100,
    ):
        command = f"git{chr(code)}status"
        assert split_bash(command) is None, hex(code)
        assert split_powershell(command) is None, hex(code)
        assert spelled_out(command) == f"git<U+{code:04X}>status", hex(code)


# JSON that is printed


def test_json_in_plain_letters_and_in_other_languages_stays_as_it_is() -> None:
    line = json.dumps({"text": f"ok {chr(0x5217)}{chr(0x8868)} {chr(0x1F600)}"}, ensure_ascii=False)

    assert escaped(line) == line
    assert escaped('{"n": 1}') == '{"n": 1}'


@pytest.mark.parametrize("code", [0x7F, 0x85, 0x9B, 0x9D, *BREAKS, *TURNS])
def test_a_character_that_json_leaves_as_it_is_and_a_terminal_acts_on_is_written_by_its_number(
    code: int,
) -> None:
    line = json.dumps({"text": f"a{chr(code)}b"}, ensure_ascii=False)

    assert chr(code) in line
    assert escaped(line) == f'{{"text": "a{BACKSLASH}u{code:04x}b"}}'
    assert json.loads(escaped(line)) == {"text": f"a{chr(code)}b"}


def test_half_of_a_pair_is_written_by_its_number_too() -> None:
    line = json.dumps({"text": f"a{chr(0xD83D)}b"}, ensure_ascii=False)

    assert escaped(line) == f'{{"text": "a{BACKSLASH}ud83db"}}'
    assert json.loads(escaped(line)) == {"text": f"a{chr(0xD83D)}b"}


@given(st.dictionaries(st.text(max_size=8), st.text(alphabet=st.characters(), max_size=30)))
def test_json_says_the_same_after_it_was_made_fit_for_a_terminal(value: dict[str, str]) -> None:
    line = json.dumps(value, ensure_ascii=False)

    safe = escaped(line)

    assert json.loads(safe) == value
    assert harmless(safe) == safe
