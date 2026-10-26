"""Whether a value of JSON is what a JSON Schema says."""

from typing import Any

import pytest

from columbina.json_schema import SchemaError, checked, problems


def check(value: object, schema: dict[str, Any]) -> list[str]:
    return problems(value, checked(schema))


@pytest.mark.parametrize(
    ("value", "kind", "fits"),
    [
        ({}, "object", True),
        ([], "array", True),
        ("a", "string", True),
        (1, "integer", True),
        (1.0, "integer", True),
        (1.5, "integer", False),
        (1.5, "number", True),
        (True, "number", False),
        (True, "boolean", True),
        (None, "null", True),
        ("1", "number", False),
    ],
)
def test_types(value: object, kind: str, fits: bool) -> None:
    assert (check(value, {"type": kind}) == []) is fits


def test_one_of_several_types() -> None:
    assert check(None, {"type": ["string", "null"]}) == []
    assert check(1, {"type": ["string", "null"]}) == [
        "the answer is 1, and has to be string or null"
    ]


def test_objects() -> None:
    schema = {
        "type": "object",
        "properties": {"name": {"type": "string"}, "age": {"type": "integer", "minimum": 0}},
        "required": ["name", "age"],
        "additionalProperties": False,
    }

    assert check({"name": "a", "age": 3}, schema) == []
    assert check({"age": -1, "x": 1}, schema) == [
        "the answer has no 'name'",
        "/age is -1, which is less than 0",
        "/x is not allowed there",
    ]
    assert (
        check(
            {"name": "a", "age": 1, "x": 1}, {**schema, "additionalProperties": {"type": "integer"}}
        )
        == []
    )


def test_arrays() -> None:
    schema = {
        "type": "array",
        "items": {"type": "string"},
        "minItems": 1,
        "maxItems": 2,
        "uniqueItems": True,
    }

    assert check(["a"], schema) == []
    assert check([], schema) == ["the answer has 0 items, and has to have 1 at least"]
    assert check(["a", "a", 3], schema) == [
        "the answer has 3 items, and may have 2 at most",
        "the answer has an item twice",
        "/2 is 3, and has to be string",
    ]


def test_strings() -> None:
    schema = {"type": "string", "minLength": 2, "maxLength": 3, "pattern": "^[a-z]+$"}

    assert check("ab", schema) == []
    assert check("A", schema) == [
        "the answer is shorter than 2 characters",
        "the answer is \"A\", which does not match '^[a-z]+$'",
    ]
    assert check("abcd", schema) == ["the answer is longer than 3 characters"]


def test_numbers() -> None:
    assert check(5, {"minimum": 1, "maximum": 5, "multipleOf": 0.5}) == []
    assert check(6, {"maximum": 5}) == ["the answer is 6, which is more than 5"]
    assert check(1, {"exclusiveMinimum": 1}) == ["the answer is 1, which is not more than 1"]
    assert check(5, {"exclusiveMaximum": 5}) == ["the answer is 5, which is not less than 5"]
    assert check(0.3, {"multipleOf": 0.25}) == ["the answer is 0.3, which is no multiple of 0.25"]


def test_enum_and_const() -> None:
    assert check("b", {"enum": ["a", "b"]}) == []
    assert check("c", {"enum": ["a", "b"]}) == [
        'the answer is "c", and has to be one of ["a", "b"]'
    ]
    assert check(2, {"const": 1}) == ["the answer is 2, and has to be 1"]


def test_combinations() -> None:
    assert check(1, {"anyOf": [{"type": "string"}, {"type": "integer"}]}) == []
    assert check(1.5, {"anyOf": [{"type": "string"}, {"type": "integer"}]}) == [
        "the answer fits none of the schemas that it may fit"
    ]
    assert check(1.5, {"oneOf": [{"type": "number"}, {"type": "integer"}]}) == []
    assert check(1, {"oneOf": [{"type": "number"}, {"type": "integer"}]}) == [
        "the answer fits 2 of the schemas of which it has to fit one"
    ]
    assert check(1, {"allOf": [{"type": "integer"}, {"minimum": 2}]}) == [
        "the answer is 1, which is less than 2"
    ]
    assert check("a", {"not": {"type": "string"}}) == [
        "the answer fits a schema that it may not fit"
    ]


def test_references() -> None:
    schema = {
        "$defs": {"item": {"type": "string"}},
        "definitions": {"list": {"type": "array", "items": {"$ref": "#/$defs/item"}}},
        "$ref": "#/definitions/list",
    }

    assert check(["a"], schema) == []
    assert check([1], schema) == ["/0 is 1, and has to be string"]
    assert (
        check({"next": {"next": {}}}, {"type": "object", "properties": {"next": {"$ref": "#"}}})
        == []
    )


def test_true_and_false_are_schemas() -> None:
    assert problems(1, True) == []
    assert problems(1, False) == ["the answer is not allowed there"]


def test_a_long_value_is_shown_short() -> None:
    (said,) = check("x" * 100, {"type": "integer"})

    assert said == 'the answer is "' + "x" * 56 + "..., and has to be integer"


def test_words_that_describe_check_nothing() -> None:
    schema = {"$schema": "x", "title": "T", "description": "d", "format": "email", "type": "string"}

    assert check("not an address", schema) == []


@pytest.mark.parametrize(
    ("schema", "said"),
    [
        ([], "a schema is an object of JSON"),
        (
            {"patternProperties": {}},
            "the schema has 'patternProperties', which is not checked here",
        ),
        ({"type": "text"}, 'the schema names "text" as a type'),
        ({"properties": []}, "the schema has 'properties' that is no object"),
        ({"properties": {"a": 1}}, "the schema/properties/a is no schema: it is 1"),
        ({"anyOf": []}, "the schema has 'anyOf' that is no list of schemas"),
        ({"items": {"if": {}}}, "the schema/items has 'if', which is not checked here"),
        ({"pattern": "("}, "the schema has a pattern that is no regular expression"),
        ({"pattern": 1}, "the schema has a pattern that is no regular expression"),
        (
            {"$ref": "#/$defs/none"},
            'the schema has a $ref that leads nowhere in it: "#/$defs/none"',
        ),
        ({"$ref": 1}, "the schema has a $ref that leads nowhere in it: 1"),
        ({"$defs": [], "$ref": "#/$defs/a"}, "the schema has '$defs' that is no object"),
        (
            {"$defs": {"a": {}}, "$ref": "#/$defs/b"},
            'the schema has a $ref that leads nowhere in it: "#/$defs/b"',
        ),
    ],
)
def test_a_schema_that_cannot_be_checked_is_not_taken(schema: object, said: str) -> None:
    with pytest.raises(SchemaError) as caught:
        checked(schema)

    assert str(caught.value) == said
