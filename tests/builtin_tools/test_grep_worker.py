"""The process that matches lines for the grep tool: what it is asked and what it answers."""

import io
import json
import sys
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.builtin_tools import grep_worker

FFFD = chr(0xFFFD)


def asked(
    pattern: str,
    *files: bytes,
    ignore_case: bool = False,
    max_matches: int = 250,
    max_line: int = 200,
) -> bytes:
    """What the process is given: the request, and the files one by one."""
    request = {
        "pattern": pattern,
        "ignore_case": ignore_case,
        "max_matches": max_matches,
        "max_line": max_line,
    }
    parts = [json.dumps(request).encode() + b"\n"]
    for number, content in enumerate(files):
        parts.append(json.dumps({"id": number, "size": len(content)}).encode() + b"\n")
        parts.append(content)
    return b"".join(parts)


def answer(given: bytes) -> tuple[int, list[dict[str, Any]]]:
    """Run the process on what it is given, and return its exit code and its answers."""
    sink = io.BytesIO()
    code = grep_worker.main(io.BytesIO(given), sink)
    written = sink.getvalue()
    assert written.isascii()
    assert written == b"" or written.endswith(b"\n")
    return code, [json.loads(line) for line in written.split(b"\n")[:-1]]


def hit(file: int, line: int, text: str, more: int = 0) -> dict[str, Any]:
    return {"id": file, "line": line, "text": text, "more": more}


def done(files: int, *, cut: bool = False) -> dict[str, Any]:
    return {"done": True, "files": files, "cut": cut}


def test_the_lines_that_match_are_answered_with_their_file_and_number() -> None:
    first = b"import os\n\ndef main():\n    pass\n"

    code, answers = answer(asked(r"def \w+", first, b"x = 1\n", b"def other(): ...\n"))

    assert code == 0
    assert answers == [hit(0, 3, "def main():"), hit(2, 1, "def other(): ..."), done(3)]


def test_nothing_that_matches_is_answered_with_the_last_line_alone() -> None:
    assert answer(asked("needle", b"hay\nmore hay\n")) == (0, [done(1)])
    assert answer(asked("needle")) == (0, [done(0)])


@pytest.mark.parametrize(
    ("content", "lines"),
    [
        (b"", []),
        (b"a", [1]),
        (b"a\n", [1]),
        (b"a\nb", [1, 2]),
        (b"a\n\n", [1, 2]),
        (b"\n", [1]),
        (b"\n\n\n", [1, 2, 3]),
    ],
)
def test_what_follows_the_last_line_break_is_a_line_only_if_something_is_there(
    content: bytes, lines: list[int]
) -> None:
    _, answers = answer(asked("", content))

    assert [found["line"] for found in answers[:-1]] == lines


def test_a_carriage_return_before_a_line_feed_belongs_to_the_line_break() -> None:
    _, answers = answer(asked("two$", b"one\r\ntwo\r\nthree\r\n"))

    assert answers == [hit(0, 2, "two"), done(1)]


def test_a_carriage_return_within_a_line_stays_in_it() -> None:
    _, answers = answer(asked("b", b"a\rb\n"))

    assert answers == [hit(0, 1, "a\rb"), done(1)]


def test_upper_and_lower_case_are_told_apart_unless_the_request_says_otherwise() -> None:
    content = b"Needle\nneedle\nNEEDLE\n"

    assert answer(asked("needle", content))[1] == [hit(0, 2, "needle"), done(1)]
    assert answer(asked("needle", content, ignore_case=True))[1] == [
        hit(0, 1, "Needle"),
        hit(0, 2, "needle"),
        hit(0, 3, "NEEDLE"),
        done(1),
    ]


def test_of_a_long_line_the_beginning_is_answered_and_the_rest_is_counted() -> None:
    line = "needle " + "x" * 30

    assert answer(asked("needle", line.encode() + b"\n", max_line=10))[1] == [
        hit(0, 1, "needle xxx", 27),
        done(1),
    ]
    assert answer(asked("needle", line.encode() + b"\n", max_line=37))[1] == [
        hit(0, 1, line),
        done(1),
    ]


def test_no_more_lines_are_answered_than_were_asked_for() -> None:
    content = b"needle 1\nhay\nneedle 2\nneedle 3\n"

    assert answer(asked("needle", content, max_matches=2))[1] == [
        hit(0, 1, "needle 1"),
        hit(0, 3, "needle 2"),
        done(0, cut=True),
    ]


def test_as_many_lines_as_were_asked_for_are_all_there_are_to_say() -> None:
    content = b"needle 1\nneedle 2\nhay\n"

    assert answer(asked("needle", content, max_matches=2))[1] == [
        hit(0, 1, "needle 1"),
        hit(0, 2, "needle 2"),
        done(1),
    ]


def test_the_files_after_the_one_that_held_too_much_are_not_looked_at() -> None:
    given = asked("needle", b"needle\n", b"needle\nneedle\n", b"needle\n", max_matches=2)
    source = io.BytesIO(given)
    sink = io.BytesIO()

    assert grep_worker.main(source, sink) == 0

    last = json.loads(sink.getvalue().splitlines()[-1])
    assert last == done(1, cut=True)
    assert source.read() == json.dumps({"id": 2, "size": 7}).encode() + b"\nneedle\n"


def test_a_pattern_that_is_no_pattern_is_answered_with_what_is_wrong_with_it() -> None:
    code, answers = answer(asked("(unclosed", b"(unclosed\n"))

    assert code == 2
    assert len(answers) == 1
    assert "position 0" in answers[0]["error"]


def test_a_pattern_that_python_warns_about_is_used_without_a_word() -> None:
    # A warning would land among the answers, and nobody could read them.
    code, answers = answer(asked("[[a]", b"a\n[\nb\n"))

    assert code == 0
    assert answers == [hit(0, 1, "a"), hit(0, 2, "["), done(1)]


def test_input_that_ends_in_the_middle_of_a_file_is_not_answered_as_if_it_were_whole() -> None:
    whole = asked("needle", b"needle 1\n", b"needle 2\nneedle 3\n")

    code, answers = answer(whole[:-4])

    assert code == 1
    assert answers == [hit(0, 1, "needle 1")]


def test_input_that_ends_between_two_files_is_answered_with_how_many_were_searched() -> None:
    whole = asked("needle", b"needle 1\n", b"needle 2\n")
    first = asked("needle", b"needle 1\n")

    assert whole.startswith(first)
    assert answer(first) == (0, [hit(0, 1, "needle 1"), done(1)])


def test_bytes_that_are_no_text_are_answered_with_a_sign_in_their_place() -> None:
    _, answers = answer(asked("needle", b"caf\xe9 needle\n"))

    assert answers == [hit(0, 1, f"caf{FFFD} needle"), done(1)]


def test_text_of_any_script_is_matched_and_answered_whole() -> None:
    moon = chr(0x6708) + chr(0x795E)
    face = chr(0x1F600)
    content = f"# {moon} {face}\nplain\n".encode()

    _, answers = answer(asked(moon, content))

    assert answers == [hit(0, 1, f"# {moon} {face}"), done(1)]


def test_a_character_that_some_take_for_a_line_break_does_not_part_an_answer() -> None:
    separators = chr(0x1C) + chr(0x0B) + chr(0x0C) + chr(0x85) + chr(0x2028)
    content = f"a{separators}b\n".encode()

    _, answers = answer(asked("a", content))

    assert answers == [hit(0, 1, f"a{separators}b"), done(1)]


def test_a_line_is_cut_out_between_characters_and_not_within_one() -> None:
    # The second byte of this letter is 0x8a, and no byte of it is a line feed.
    letter = chr(0x10A)
    content = f"{letter}\n{letter}{letter}\n".encode()

    _, answers = answer(asked(letter + "$", content))

    assert answers == [hit(0, 1, letter), hit(0, 2, letter + letter), done(1)]


async def test_the_program_runs_as_text_that_is_handed_to_python(tmp_path: Path) -> None:
    # Byte 0x1a ends a text file for some programs on Windows. Here it is a byte like any other.
    content = b"one\x1a needle\r\ntwo\r\n"
    program = Path(grep_worker.__file__).read_text(encoding="utf-8")

    ran = await process.run(
        [sys.executable, "-I", "-S", "-c", program],
        cwd=tmp_path,
        stdin=asked("needle", content),
        timeout_seconds=20,
    )

    assert ran.exit_code == 0
    assert [json.loads(line) for line in ran.output.splitlines()] == [
        hit(0, 1, "one\x1a needle"),
        done(1),
    ]


async def test_the_exit_code_tells_a_pattern_that_cannot_be_used(tmp_path: Path) -> None:
    program = Path(grep_worker.__file__).read_text(encoding="utf-8")

    ran = await process.run(
        [sys.executable, "-I", "-S", "-c", program],
        cwd=tmp_path,
        stdin=asked("(", b"(\n"),
        timeout_seconds=20,
    )

    assert ran.exit_code == 2
    assert "error" in json.loads(ran.output)
