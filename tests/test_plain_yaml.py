"""YAML that somebody else wrote: values come back, and what builds values of values does not."""

import time

import pytest

from columbina.plain_yaml import PlainYamlError, load


def refused(text: str) -> PlainYamlError:
    with pytest.raises(PlainYamlError) as error:
        load(text)
    return error.value


def test_values_come_back_as_the_safe_reader_gives_them() -> None:
    assert load("name: notes\ntools: [read, grep]\nsteps: 3\n") == {
        "name": "notes",
        "tools": ["read", "grep"],
        "steps": 3,
    }
    assert load("- one\n- two\n") == ["one", "two"]
    assert load("just text") == "just text"
    assert load("") is None


def test_a_name_that_is_given_to_a_value_and_not_used_does_no_harm() -> None:
    assert load("name: &given notes\nother: plain\n") == {"name": "notes", "other": "plain"}


def test_a_mapping_may_take_in_keys_that_are_written_out_where_it_stands() -> None:
    assert load("a: {<<: {x: 1}, y: 2}\n") == {"a": {"x": 1, "y": 2}}


@pytest.mark.parametrize(
    "text",
    [
        "a: &x 1\nb: *x\n",
        "a: &x [1, 2]\nb: [*x, *x]\n",
        "base: &base {x: 1}\nmore: {<<: *base, y: 2}\n",
        "- &x one\n- *x\n",
    ],
)
def test_a_value_that_is_used_again_by_its_name_is_refused(text: str) -> None:
    error = refused(text)

    assert (error.kind, error.said, error.line) == ("alias", "", None)


def test_a_small_file_that_would_build_a_value_without_end_is_refused_at_once() -> None:
    # Each line takes in the one before it twice. Built, the last one has 2**40 entries.
    lines = ["a0: &a0 {x: 1}"]
    lines.extend(f"a{n}: &a{n} {{<<: [*a{n - 1}, *a{n - 1}]}}" for n in range(1, 41))
    # Were a value that is used again built at all, the file would be the end of this
    # run. So a small one is tried first, and it stops the test if it is taken.
    refused("a: &x 1\nb: *x\n")
    began = time.monotonic()

    error = refused("\n".join(lines) + "\n")

    assert error.kind == "alias"
    assert time.monotonic() - began < 5


def test_what_is_no_yaml_is_told_in_one_line_with_the_line_that_it_stopped_at() -> None:
    error = refused("deny: [a\nmore: b\n")

    assert error.kind == "syntax"
    assert error.line == 2
    assert error.said.startswith("while parsing a flow sequence")
    assert "\n" not in error.said
    assert str(error) == error.said


def test_what_the_reader_says_is_not_passed_on_at_length() -> None:
    long = "x" * 2000

    # A tag that nothing is known of is repeated by the reader, however long it is.
    unknown = refused(f"key: !{long} value" + chr(10))
    # And so is what was to be made a yes or a no.
    unmade = refused(f"key: !!bool {long}" + chr(10))

    assert unknown.kind == "syntax"
    assert unmade.kind == "value"
    for error in (unknown, unmade):
        assert len(error.said) == 303
        assert error.said.endswith("xxx...")


def test_what_the_reader_says_in_just_as_many_characters_as_are_passed_on_is_whole() -> None:
    # What could not be made a yes or a no is repeated between two marks.
    just = refused(f"key: !!bool {'x' * 298}" + chr(10))
    more = refused(f"key: !!bool {'x' * 299}" + chr(10))

    assert just.said == "'" + "x" * 298 + "'"
    assert len(just.said) == 300
    assert more.said == "'" + "x" * 299 + "..."


def test_what_is_nested_too_deeply_is_told_as_that() -> None:
    error = refused("[" * 100_000)

    assert (error.kind, error.said, error.line) == ("depth", "", None)
    assert str(error) == "depth"


@pytest.mark.parametrize(
    ("text", "said"),
    [
        ("when: 2024-13-45\n", "month must be in 1..12"),
        ("when: 2024-01-01 25:00:00\n", "hour must be in 0..23"),
    ],
)
def test_a_value_that_cannot_be_made_is_told_as_that(text: str, said: str) -> None:
    error = refused(text)

    assert (error.kind, error.said, error.line) == ("value", said, None)


def test_a_value_of_python_itself_is_not_made() -> None:
    error = refused("a: !!python/object/apply:os.system ['echo no']\n")

    assert error.kind == "syntax"
    assert "could not determine a constructor" in error.said
