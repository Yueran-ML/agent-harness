"""Whether a value of JSON is what a JSON Schema says, in the part of JSON Schema used most.

A program that runs a task and reads its answer says what the answer is to
look like with a schema: an object with these fields, a list of such
objects, a number in a range. :func:`problems` says how a value is not what
its schema says, and :func:`checked` says whether a schema can be checked
here at all.

What is checked: ``type`` (one or a list, where an integer is a number too),
``enum``, ``const``, ``properties``, ``required``, ``additionalProperties``,
``items``, ``minItems``, ``maxItems``, ``uniqueItems``, ``minLength``,
``maxLength``, ``pattern``, ``minimum``, ``maximum``, ``exclusiveMinimum``,
``exclusiveMaximum``, ``multipleOf``, ``anyOf``, ``oneOf``, ``allOf``,
``not``, and ``$ref`` to a schema in ``$defs`` or ``definitions`` of the same
document. Words that only describe, such as ``title``, ``description``,
``default``, ``examples`` and ``format``, check nothing. A schema with any
other word is not taken: a word that is not checked would let through what
its author meant to keep out.

A value is what comes of reading JSON: a dict, a list, a text, a number, a
boolean or ``None``.
"""

import json
import math
import re
from collections.abc import Callable, Mapping, Sequence
from typing import Any, cast

__all__ = ["SchemaError", "checked", "problems"]

_DESCRIBING = frozenset(
    {
        "$schema", "$id", "$comment", "title", "description", "default", "examples",
        "format", "readOnly", "writeOnly", "deprecated", "$defs", "definitions",
    }
)  # fmt: skip
"""Words of a schema that describe and check nothing."""

_CHECKED = frozenset(
    {
        "type", "enum", "const", "properties", "required", "additionalProperties", "items",
        "minItems", "maxItems", "uniqueItems", "minLength", "maxLength", "pattern", "minimum",
        "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "anyOf", "oneOf",
        "allOf", "not", "$ref",
    }
)  # fmt: skip
"""Words of a schema that are checked here."""

_TYPES = frozenset({"object", "array", "string", "number", "integer", "boolean", "null"})

_SHOWN = 60
"""How many characters of a value a problem shows."""


class SchemaError(ValueError):
    """A schema that cannot be checked here, and why."""


def _shown(value: object) -> str:
    text = json.dumps(value, ensure_ascii=False)
    return text if len(text) <= _SHOWN else text[: _SHOWN - 3] + "..."


def _where(path: str) -> str:
    return path or "the answer"


def _is(value: object, kind: str) -> bool:
    if kind == "object":
        return isinstance(value, dict)
    if kind == "array":
        return isinstance(value, list)
    if kind == "string":
        return isinstance(value, str)
    if kind == "boolean":
        return isinstance(value, bool)
    if kind == "null":
        return value is None
    if isinstance(value, bool) or not isinstance(value, int | float):
        return False
    return kind == "number" or float(value).is_integer()


def checked(schema: object) -> Mapping[str, Any]:
    """Return a schema if every word of it can be checked here.

    Raises:
        SchemaError: It is no schema, or it holds a word that is not
            checked here, or a ``$ref`` that leads nowhere in it.
    """
    if not isinstance(schema, dict):
        raise SchemaError("a schema is an object of JSON")
    root = cast("dict[str, Any]", schema)
    _look(root, root, "the schema")
    return root


def _look(schema: object, root: Mapping[str, Any], where: str) -> None:
    """Make sure that every word of a part of a schema is known, and look into its parts."""
    if isinstance(schema, bool):
        return
    if not isinstance(schema, dict):
        raise SchemaError(f"{where} is no schema: it is {_shown(schema)}")
    part = cast("dict[str, Any]", schema)
    for word, value in part.items():
        if word not in _CHECKED and word not in _DESCRIBING:
            raise SchemaError(f"{where} has {word!r}, which is not checked here")
        if word == "type":
            kinds = cast("list[object]", value) if isinstance(value, list) else [value]
            for kind in kinds:
                if kind not in _TYPES:
                    raise SchemaError(f"{where} names {_shown(kind)} as a type")
        elif word == "$ref":
            _referred(root, value, where)
        elif word in ("properties", "$defs", "definitions"):
            if not isinstance(value, dict):
                raise SchemaError(f"{where} has {word!r} that is no object")
            for key, sub in cast("dict[str, object]", value).items():
                _look(sub, root, f"{where}/{word}/{key}")
        elif word in ("anyOf", "oneOf", "allOf"):
            if not isinstance(value, list) or not value:
                raise SchemaError(f"{where} has {word!r} that is no list of schemas")
            for number, sub in enumerate(cast("list[object]", value)):
                _look(sub, root, f"{where}/{word}/{number}")
        elif word in ("items", "additionalProperties", "not"):
            _look(value, root, f"{where}/{word}")
        elif word == "pattern":
            try:
                re.compile(value)
            except (re.error, TypeError):
                raise SchemaError(f"{where} has a pattern that is no regular expression") from None


def _referred(root: Mapping[str, Any], ref: object, where: str) -> object:
    """Return the schema that a ``$ref`` leads to."""
    if isinstance(ref, str):
        for place in ("$defs", "definitions"):
            prefix = f"#/{place}/"
            defined = root.get(place)
            if ref.startswith(prefix) and isinstance(defined, dict):
                found = cast("dict[str, object]", defined).get(ref.removeprefix(prefix))
                if found is not None:
                    return found
        if ref == "#":
            return root
    raise SchemaError(f"{where} has a $ref that leads nowhere in it: {_shown(ref)}")


def problems(value: object, schema: Mapping[str, Any] | bool, path: str = "") -> list[str]:
    """Say how a value is not what its schema says. Nothing said means that it is.

    The schema has to have been :func:`checked`.

    Args:
        value: The value.
        schema: Its schema.
        path: Where the value stands in the answer, as a pointer of JSON.
    """
    return _problems(value, schema, schema if isinstance(schema, Mapping) else {}, path)


def _problems(value: object, schema: object, root: Mapping[str, Any], path: str) -> list[str]:
    if schema is True:
        return []
    if schema is False:
        return [f"{_where(path)} is not allowed there"]
    part = cast("Mapping[str, Any]", schema)
    found: list[str] = []
    here = _where(path)
    if "$ref" in part:
        found += _problems(value, _referred(root, part["$ref"], "the schema"), root, path)
    if "type" in part:
        named: object = part["type"]
        kinds = cast("list[str]", named) if isinstance(named, list) else [cast("str", named)]
        if not any(_is(value, kind) for kind in kinds):
            return [*found, f"{here} is {_shown(value)}, and has to be {' or '.join(kinds)}"]
    if "enum" in part and value not in part["enum"]:
        found.append(f"{here} is {_shown(value)}, and has to be one of {_shown(part['enum'])}")
    if "const" in part and value != part["const"]:
        found.append(f"{here} is {_shown(value)}, and has to be {_shown(part['const'])}")
    if isinstance(value, dict):
        found += _object(cast("dict[str, object]", value), part, root, path)
    elif isinstance(value, list):
        found += _array(cast("list[object]", value), part, root, path)
    elif isinstance(value, str):
        found += _string(value, part, here)
    elif isinstance(value, int | float) and not isinstance(value, bool):
        found += _number(value, part, here)
    found += _combined(cast("object", value), part, root, path)
    return found


def _object(
    value: dict[str, object], part: Mapping[str, Any], root: Mapping[str, Any], path: str
) -> list[str]:
    properties: Mapping[str, object] = part.get("properties", {})
    required: list[str] = part.get("required", [])
    found = [f"{_where(path)} has no {key!r}" for key in required if key not in value]
    extra = part.get("additionalProperties", True)
    for key, sub in value.items():
        if key in properties:
            found += _problems(sub, properties[key], root, f"{path}/{key}")
        else:
            found += _problems(sub, extra, root, f"{path}/{key}")
    return found


def _array(
    value: list[object], part: Mapping[str, Any], root: Mapping[str, Any], path: str
) -> list[str]:
    found: list[str] = []
    here = _where(path)
    if "minItems" in part and len(value) < part["minItems"]:
        found.append(f"{here} has {len(value)} items, and has to have {part['minItems']} at least")
    if "maxItems" in part and len(value) > part["maxItems"]:
        found.append(f"{here} has {len(value)} items, and may have {part['maxItems']} at most")
    if part.get("uniqueItems"):
        seen = [json.dumps(item, sort_keys=True) for item in value]
        if len(set(seen)) != len(seen):
            found.append(f"{here} has an item twice")
    if "items" in part:
        for number, item in enumerate(value):
            found += _problems(item, part["items"], root, f"{path}/{number}")
    return found


def _string(value: str, part: Mapping[str, Any], here: str) -> list[str]:
    found: list[str] = []
    if "minLength" in part and len(value) < part["minLength"]:
        found.append(f"{here} is shorter than {part['minLength']} characters")
    if "maxLength" in part and len(value) > part["maxLength"]:
        found.append(f"{here} is longer than {part['maxLength']} characters")
    if "pattern" in part and re.search(part["pattern"], value) is None:
        found.append(f"{here} is {_shown(value)}, which does not match {part['pattern']!r}")
    return found


def _number(value: float, part: Mapping[str, Any], here: str) -> list[str]:
    found: list[str] = []
    bounds: tuple[tuple[str, Callable[[float], bool], str], ...] = (
        ("minimum", lambda limit: value < limit, "less than"),
        ("maximum", lambda limit: value > limit, "more than"),
        ("exclusiveMinimum", lambda limit: value <= limit, "not more than"),
        ("exclusiveMaximum", lambda limit: value >= limit, "not less than"),
    )
    for word, beyond, said in bounds:
        if word in part and beyond(part[word]):
            found.append(f"{here} is {_shown(value)}, which is {said} {part[word]}")
    step = part.get("multipleOf")
    if step and not math.isclose(value / step, round(value / step)):
        found.append(f"{here} is {_shown(value)}, which is no multiple of {step}")
    return found


def _combined(
    value: object, part: Mapping[str, Any], root: Mapping[str, Any], path: str
) -> list[str]:
    found: list[str] = []
    here = _where(path)
    for sub in cast("Sequence[object]", part.get("allOf", [])):
        found += _problems(value, sub, root, path)
    if "anyOf" in part and not any(not _problems(value, sub, root, path) for sub in part["anyOf"]):
        found.append(f"{here} fits none of the schemas that it may fit")
    if "oneOf" in part:
        fitting = sum(not _problems(value, sub, root, path) for sub in part["oneOf"])
        if fitting != 1:
            found.append(f"{here} fits {fitting} of the schemas of which it has to fit one")
    if "not" in part and not _problems(value, part["not"], root, path):
        found.append(f"{here} fits a schema that it may not fit")
    return found
