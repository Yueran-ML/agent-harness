"""What a call of a tool of a server gave back, as the text that a model reads.

A result is a list of blocks, and it may have content of another kind
beside them, which the protocol calls structured. A model reads text. So
each block becomes text, or a line that says what was left out.

===========================  ==============================================
What the server gave back    What the model reads
===========================  ==============================================
A text                       The text
A picture, a sound           A line that says what it is and how large
A link to a resource         ``Resource link: name (address)``
A resource that is text      Its address on a line, and the text after it
A resource that is not       A line that says what it is and how large
Structured content           Its JSON, if nothing else that was given back
                             is text
Nothing                      A line that says so
===========================  ==============================================

The blocks stand in the order in which they came, each on lines of its own.

Structured content is shown only where there is no text. A server that
gives both says the same thing twice, once for a program and once for a
reader, and a model is the reader.

Whether the result is one that the server calls an error is not said here.
Whoever makes the call turns such a result into a failure, with this text.
"""

import json

from mcp.types import (
    AudioContent,
    CallToolResult,
    ContentBlock,
    ImageContent,
    ResourceLink,
    TextContent,
    TextResourceContents,
)

__all__ = ["NOTHING", "shown"]

NOTHING = "[the tool gave back nothing that can be shown]"
"""What a model reads where a result holds nothing."""


def _size(encoded: str) -> int:
    """Return how many bytes a text in base64 stands for."""
    return len(encoded) * 3 // 4 - encoded[-2:].count("=")


def _left_out(what: str, kind: str | None, encoded: str) -> str:
    """Say that something was left out that is no text."""
    size = _size(encoded)
    large = "1 byte" if size == 1 else f"{size} bytes"
    return f"[{what} is left out here: {kind or 'of no named type'}, {large}]"


def _block(block: ContentBlock) -> tuple[str, bool]:
    """Return what a model reads of one block, and whether that is text of the server."""
    if isinstance(block, TextContent):
        return block.text, True
    if isinstance(block, ImageContent):
        return _left_out("a picture", block.mime_type, block.data), False
    if isinstance(block, AudioContent):
        return _left_out("a sound", block.mime_type, block.data), False
    if isinstance(block, ResourceLink):
        return f"Resource link: {block.name} ({block.uri})", False
    resource = block.resource
    if isinstance(resource, TextResourceContents):
        return f"{resource.uri}\n{resource.text}", True
    return _left_out(f"the resource {resource.uri}", resource.mime_type, resource.blob), False


def shown(result: CallToolResult) -> str:
    """Return what a model reads of the result of a call.

    Args:
        result: The result, as the SDK of the protocol hands it over.
    """
    parts: list[str] = []
    has_text = False
    for block in result.content:
        part, is_text = _block(block)
        parts.append(part)
        has_text = has_text or is_text
    if not has_text and result.structured_content is not None:
        parts.append(json.dumps(result.structured_content, ensure_ascii=False, default=str))
    return "\n".join(parts) if parts else NOTHING
