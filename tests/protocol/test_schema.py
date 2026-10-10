"""The documents of the protocol say what the code does.

``docs/protocol`` is what somebody goes by who builds a frontend without
reading this code. So the schema there is the one that the code gives, and
the tables in the text name the types and the fields that there are. A type
of event that a plugin brings fails these tests until it is written down.
"""

import json
import re

from support.protocol import DOCS, chunk_kinds, event_kinds, schema
from support.snapshot import stored

README = DOCS / "README.md"
SCHEMA = DOCS / "events.schema.json"

_EVENT = re.compile(r"^\| `([a-z]+(?:/[a-z-]+)+)` \| ([^|]*) \|", re.MULTILINE)
_CHUNK = re.compile(r"^\| `([a-z-]+)` \| ([^|]*) \|", re.MULTILINE)
_NAME = re.compile(r"`([a-z_]+)`")


def section(number: int) -> str:
    """The text of one of the numbered sections of the document."""
    text = README.read_text(encoding="utf-8")
    start = text.index(f"\n## {number}. ")
    end = text.find("\n## ", start + 1)
    return text[start:] if end == -1 else text[start:end]


def rows(text: str, row: re.Pattern[str]) -> dict[str, set[str]]:
    """The rows of a table that names types: for each type, the names in the next column."""
    return {found[1]: set(_NAME.findall(found[2])) for found in row.finditer(text)}


def test_the_schema_in_the_documents_is_the_one_that_the_code_gives() -> None:
    written = json.dumps(schema(), indent=2, ensure_ascii=False) + "\n"

    assert written == stored(SCHEMA, written)


def test_the_schema_tells_the_types_of_events_apart_by_their_names() -> None:
    lines = schema()["$defs"]["Event"]["oneOf"]

    assert [line["properties"]["type"]["const"] for line in lines] == list(event_kinds())
    # Only an event that carries a message can stand in the conversation.
    carrying = {line["title"] for line in lines if "surface_op" in line["properties"]}
    assert carrying == {"system/message", "user/message", "assistant/message", "tool/result"}


def test_the_document_lists_every_type_of_event_with_its_fields() -> None:
    listed = rows(section(7), _EVENT)

    assert listed == {name: set(kind.model_fields) for name, kind in event_kinds().items()}


def test_the_document_lists_every_kind_of_chunk_with_its_fields() -> None:
    listed = rows(section(8), _CHUNK)

    assert listed == {
        name: set(kind.model_fields) - {"type"} for name, kind in chunk_kinds().items()
    }
