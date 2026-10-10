"""What the documents of the protocol are checked against: the events as the code has them.

``docs/protocol`` tells whoever builds a frontend what a session log holds.
Three things there are made from the code, so that they cannot say something
else than the code does: the JSON Schema of the events, the table of the
events in the text, and a sample of a session. This module finds the events
and writes the schema.
"""

import importlib
import pkgutil
from pathlib import Path
from typing import Any, get_args

from pydantic import BaseModel, TypeAdapter
from pydantic.json_schema import models_json_schema

import columbina
from columbina.llm.types import Chunk
from columbina.session import EventData, SessionHeader, UnknownEvent

__all__ = ["DOCS", "chunk_kinds", "event_kinds", "schema"]

DOCS = Path(__file__).parent.parent.parent / "docs" / "protocol"
"""Where the documents of the protocol are."""

_REF = "#/$defs/{model}"
_NOT_A_MODULE_TO_LOAD = "__main__"


def event_kinds() -> dict[str, type[EventData]]:
    """Return every type of event that a module of the package declares, by its name.

    The package is gone through for them, so a type that a new plugin brings
    is found without being named here. Types that tests declare for
    themselves are left out, and so is what stands for a type that nothing
    knows.
    """
    for module in pkgutil.walk_packages(columbina.__path__, "columbina."):
        if not module.name.endswith(_NOT_A_MODULE_TO_LOAD):
            importlib.import_module(module.name)
    found: dict[str, type[EventData]] = {}
    pending = list(EventData.__subclasses__())
    while pending:
        kind = pending.pop()
        pending.extend(kind.__subclasses__())
        if kind.__module__.startswith("columbina.") and not issubclass(kind, UnknownEvent):
            found[kind.event_type] = kind
    return dict(sorted(found.items()))


def chunk_kinds() -> dict[str, type[BaseModel]]:
    """Return the kinds of chunks of a reply that is being written, by their names."""
    union = get_args(Chunk)[0]
    kinds: tuple[type[BaseModel], ...] = get_args(union)
    return {get_args(kind.model_fields["type"].annotation)[0]: kind for kind in kinds}


def _carries_a_message(kind: type[EventData]) -> bool:
    return kind.for_model is not EventData.for_model


def _line(name: str, kind: type[EventData]) -> dict[str, Any]:
    """Return the schema of the line that stores an event of one type."""
    properties: dict[str, Any] = {
        "type": {"const": name},
        "seq": {"type": "integer", "minimum": 0},
        "time": {"type": "integer"},
        "data": {"$ref": _REF.format(model=kind.__name__)},
    }
    required = ["type", "seq", "time", "data"]
    if kind.ignorable:
        properties["ignorable"] = {"const": True}
        required.append("ignorable")
    if _carries_a_message(kind):
        properties["surface_op"] = {"$ref": _REF.format(model="SurfaceOp")}
        properties["source_event_seqs"] = {"type": "array", "items": {"type": "integer"}}
    return {
        "title": name,
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": False,
    }


_SURFACE_OP: dict[str, Any] = {
    "description": (
        "Present if the event carries a message that the model sees. The message is "
        "appended to the conversation, or it takes the place of the messages of the events "
        "that source_event_seqs lists."
    ),
    "oneOf": [
        {"const": "append"},
        {
            "type": "object",
            "properties": {
                "op": {"const": "replace"},
                "start_seq": {"type": "integer"},
                "end_seq": {"type": "integer"},
            },
            "required": ["op", "start_seq", "end_seq"],
            "additionalProperties": False,
        },
    ],
}


def _untitled(node: Any) -> Any:
    """Return a part of a schema without the titles that every field was given.

    A title on a field says its name once more. A program that makes types
    from a schema takes such a title for the name of a type of its own.
    """
    if isinstance(node, list):
        return [_untitled(item) for item in node]  # pyright: ignore[reportUnknownVariableType]
    if not isinstance(node, dict):
        return node
    return {key: _untitled(value) for key, value in node.items() if key != "title"}  # pyright: ignore[reportUnknownVariableType]


def _as_written(definition: dict[str, Any]) -> dict[str, Any]:
    """Return the schema of a model as its lines are written: every field is there."""
    properties = definition.get("properties")
    if definition.get("type") != "object" or not isinstance(properties, dict):
        return definition
    fields: dict[str, Any] = properties  # pyright: ignore[reportUnknownVariableType]
    return {**definition, "properties": _untitled(fields), "required": list(fields)}


def schema() -> dict[str, Any]:
    """Return the JSON Schema of what a session file and a stream of chunks hold."""
    kinds = event_kinds()
    models: list[tuple[type[BaseModel], Any]] = [
        (kind, "serialization") for kind in (*kinds.values(), SessionHeader)
    ]
    _, document = models_json_schema(models, ref_template=_REF)
    definitions: dict[str, Any] = document["$defs"]
    chunk = TypeAdapter[Any](Chunk).json_schema(mode="serialization", ref_template=_REF)
    definitions.update(chunk.pop("$defs"))
    definitions = {name: _as_written(found) for name, found in definitions.items()}
    definitions["Chunk"] = chunk
    definitions["SurfaceOp"] = _SURFACE_OP
    definitions["Event"] = {"oneOf": [_line(name, kind) for name, kind in kinds.items()]}
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Columbina: the lines of a session file and the chunks of a reply",
        "description": (
            "Made from the code by tests/support/protocol.py. Do not change it by hand: "
            "run the tests with COLUMBINA_UPDATE_SNAPSHOTS=1."
        ),
        "anyOf": [
            {"$ref": _REF.format(model="Event")},
            {"$ref": _REF.format(model="Chunk")},
            {"$ref": _REF.format(model="SessionHeader")},
        ],
        "$defs": dict(sorted(definitions.items())),
    }
