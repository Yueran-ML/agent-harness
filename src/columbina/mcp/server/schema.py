"""What a tool of a server is for, and what its arguments look like.

Both are written by the server. The schema of the arguments goes to a model
as the server wrote it: a model is told the truth about what the tool takes,
and nothing is read into the schema here. Two things are put right, because
a service refuses a tool without them, and two limits are kept, because
both texts are sent again with every request.
"""

import json
from collections.abc import Mapping
from typing import cast

from pydantic import JsonValue

__all__ = [
    "MAX_DESCRIPTION",
    "MAX_SCHEMA_BYTES",
    "briefly",
    "described",
    "refused",
    "repaired",
]

MAX_DESCRIPTION = 2048
"""How many characters of what a tool is for a model is told at most."""

MAX_SCHEMA_BYTES = 16 * 1024
"""How large the schema of the arguments of a tool may be, as the JSON that is sent."""

_SHOWN = 80
"""How many characters of something that a server wrote are repeated in a reason."""


def briefly(value: object) -> str:
    """Write something that a server sent in a few characters.

    A reason why a tool is not offered goes to a log, and it names what was
    wrong. A server can make that as long as it likes, so only its beginning
    is repeated.
    """
    try:
        said = repr(value)
    except RecursionError:
        return "something that is nested too deeply to be written out"
    return said if len(said) <= _SHOWN else f"{said[:_SHOWN]}…"


def described(description: str | None) -> str:
    """Return what a model is told that a tool is for.

    A text that is too long is cut, and its last character says so.

    Args:
        description: What the server says of the tool, if it says anything.
    """
    text = description or ""
    if len(text) <= MAX_DESCRIPTION:
        return text
    return f"{text[: MAX_DESCRIPTION - 1]}…"


def repaired(schema: Mapping[str, object]) -> dict[str, JsonValue]:
    """Return the schema of the arguments of a tool, as a model is given it.

    It is the schema of the server. A schema that does not say that the
    arguments are an object is made to say so, and one that names no
    properties is given none: a service takes the parameters of a function
    only in this form.

    Args:
        schema: The schema as the server sent it.
    """
    mended = cast("dict[str, JsonValue]", dict(schema))
    mended.setdefault("type", "object")
    mended.setdefault("properties", {})
    return mended


def refused(schema: Mapping[str, object]) -> str | None:
    """Say why a tool with this schema is not offered, or return ``None`` if it is.

    Args:
        schema: The schema of the arguments, already repaired.

    Returns:
        The reason, in words for a log, or ``None``.
    """
    kind = schema.get("type")
    if kind != "object":
        return (
            f"its arguments are said to be of the type {briefly(kind)}, and only an object is taken"
        )
    if not isinstance(schema.get("properties", {}), Mapping):
        # A service looks at the parameters of every function that a request offers, and
        # refuses the whole request for one that is written wrongly.
        return "the properties of its arguments are not written as an object"
    try:
        sent = json.dumps(schema, ensure_ascii=False, separators=(",", ":"))
    except (TypeError, ValueError):
        return "the schema of its arguments cannot be written as JSON"
    size = len(sent.encode("utf-8", "surrogatepass"))
    if size > MAX_SCHEMA_BYTES:
        return (
            f"the schema of its arguments has {size} bytes, and {MAX_SCHEMA_BYTES} is the "
            "most that is sent with every request"
        )
    return None
