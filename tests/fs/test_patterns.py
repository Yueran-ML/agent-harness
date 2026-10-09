"""Laying the parts of a pattern along the names of a path."""

import sys
import time

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.fs.patterns import sequence_fits

ANY = None


def same(part: str, name: str) -> bool:
    return part == name


def fits(pattern: str, path: str) -> bool:
    """Whether a path fits a pattern. Both are written with slashes, and ``**`` is any number."""
    parts = [ANY if part == "**" else part for part in pattern.split("/") if part]
    return sequence_fits(parts, [name for name in path.split("/") if name], same)


@pytest.mark.parametrize(
    ("pattern", "path", "expected"),
    [
        ("", "", True),
        ("", "a", False),
        ("a", "", False),
        ("a", "a", True),
        ("a", "b", False),
        ("a/b", "a/b", True),
        ("a/b", "a", False),
        ("a/b", "a/b/c", False),
        ("**", "", True),
        ("**", "a", True),
        ("**", "a/b/c", True),
        ("**/c", "c", True),
        ("**/c", "a/b/c", True),
        ("**/c", "a/b", False),
        ("a/**", "a", True),
        ("a/**", "a/b/c", True),
        ("a/**", "b/a", False),
        ("a/**/c", "a/c", True),
        ("a/**/c", "a/b/c", True),
        ("a/**/c", "a/b/b/c", True),
        ("a/**/c", "a/b/c/d", False),
        ("a/**/**/c", "a/c", True),
        ("**/a/**/a/**", "a/a", True),
        ("**/a/**/a/**", "b/a/b/a/b", True),
        ("**/a/**/a/**", "b/a/b", False),
        ("**/**", "", True),
    ],
)
def test_a_path_fits_the_patterns_it_should(pattern: str, path: str, expected: bool) -> None:
    assert fits(pattern, path) is expected


def by_definition(parts: list[str | None], names: list[str]) -> bool:
    """The same question, answered by the book: try every way to lay the parts along the names."""
    if not parts:
        return not names
    if parts[0] is None:
        return any(by_definition(parts[1:], names[skip:]) for skip in range(len(names) + 1))
    return bool(names) and parts[0] == names[0] and by_definition(parts[1:], names[1:])


@given(
    st.lists(st.sampled_from([None, "a", "b"]), max_size=6),
    st.lists(st.sampled_from(["a", "b"]), max_size=7),
)
def test_the_answer_is_the_one_that_trying_every_way_gives(
    parts: list[str | None], names: list[str]
) -> None:
    assert sequence_fits(parts, names, same) is by_definition(parts, names)


def test_a_long_pattern_and_a_long_path_need_no_more_stack_than_a_short_one() -> None:
    depth = sys.getrecursionlimit() * 3
    names = ["a"] * depth

    assert sequence_fits([ANY] * depth + ["a"], ["a"], same)
    assert sequence_fits([ANY, "a"], names, same)
    assert sequence_fits(["a"] * depth, names, same)
    assert not sequence_fits(["a"] * depth, [*names, "a"], same)


def test_a_pattern_with_many_parts_for_any_number_does_not_take_long() -> None:
    parts: list[str | None] = [ANY, "a"] * 30 + ["b"]
    began = time.monotonic()

    assert not sequence_fits(parts, ["a"] * 400, same)
    assert time.monotonic() - began < 2
