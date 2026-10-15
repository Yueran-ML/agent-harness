"""Documents of JSON that another program writes to Columbina, and the one that it answers with.

``columbina setup --from-stdin`` and ``--check`` read a document from their standard input,
and answer with one document on one line.
"""

import io
import json
import sys
from typing import Any

import pytest

__all__ = ["one_document", "said", "written_by_a_program"]


def said(**document: object) -> bytes:
    """A document of JSON, as another program writes it."""
    return json.dumps(document, ensure_ascii=False).encode()


def written_by_a_program(monkeypatch: pytest.MonkeyPatch, document: bytes) -> io.BytesIO:
    """Have the standard input be a pipe into which another program wrote a document.

    Read as text it would be ASCII, so a test sees that it is read as the bytes that came.

    Returns:
        The pipe, which says how far it was read.
    """
    pipe = io.BytesIO(document)
    monkeypatch.setattr(sys, "stdin", io.TextIOWrapper(pipe, encoding="ascii"))
    return pipe


def one_document(out: str) -> Any:
    """Read what the command wrote to its standard output: one document of JSON, on one line."""
    assert out.endswith("\n")
    assert out.count("\n") == 1
    return json.loads(out)
