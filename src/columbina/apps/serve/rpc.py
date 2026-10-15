"""JSON-RPC 2.0, a line at a time: what a line that came means, and the lines that go out.

The service process and the interface that drives it send each other messages
of JSON-RPC 2.0, one to a line. Nothing here knows what
the messages are about. A line that came is turned into one of three things:
a request, which wants an answer, a notification, which wants none, or the
answer to something that this side asked. A line that is none of them is
:class:`MalformedLineError`, and says with which error it is to be answered.
That includes a line that is JSON and more than is read here: one that is
nested too deeply, or holds a whole number of thousands of digits.

Batches are not taken: a line holds one message.

What goes out is put together as text. The largest part of it, an event of a
session, is JSON already, as the log of the session holds it, and is not read
and written again on its way.
"""

import json
from collections.abc import Mapping
from dataclasses import dataclass
from typing import cast

__all__ = [
    "CANNOT_OPEN",
    "INTERNAL_ERROR",
    "INVALID_PARAMS",
    "INVALID_REQUEST",
    "LEAVING",
    "METHOD_NOT_FOUND",
    "NOT_IDLE",
    "NOT_INITIALIZED",
    "NO_SESSION",
    "PARSE_ERROR",
    "STOPPED",
    "Id",
    "MalformedLineError",
    "Notification",
    "Request",
    "Response",
    "RpcError",
    "answered",
    "asked",
    "on_the_wire",
    "parsed",
    "refused",
    "text",
    "told",
]

PARSE_ERROR = -32700
"""The line is no JSON."""
INVALID_REQUEST = -32600
"""The line is JSON, and no message."""
METHOD_NOT_FOUND = -32601
INVALID_PARAMS = -32602
INTERNAL_ERROR = -32603
NOT_INITIALIZED = 1000
"""Nothing was asked for yet with ``initialize``, or it was asked for twice."""
NO_SESSION = 1001
"""No session is open."""
CANNOT_OPEN = 1002
"""The session cannot be opened: it cannot be read, or another process has it open."""
NOT_IDLE = 1003
"""The agent is not at rest, and this has to wait until it is."""
STOPPED = 1004
"""The agent has stopped for good."""
LEAVING = 1005
"""The program is leaving, and what was asked for was broken off."""

Id = int | str
"""What a request is known by. Each side numbers its own."""

_VERSION = "2.0"


@dataclass(frozen=True, slots=True)
class Request:
    """Something that the other side asks for, and wants an answer to.

    Attributes:
        id: What the answer is to name.
        method: What is asked for.
        params: What it is asked with. Empty where the message gave none.
    """

    id: Id
    method: str
    params: Mapping[str, object]


@dataclass(frozen=True, slots=True)
class Notification:
    """Something that the other side says, and wants no answer to."""

    method: str
    params: Mapping[str, object]


@dataclass(frozen=True, slots=True)
class Response:
    """The answer of the other side to something that this side asked.

    Attributes:
        id: The id of what was asked, or ``None`` where the answer names
            nothing that can be one. Nobody waits for such an answer.
        result: What was answered, or ``None`` if the answer is a failure.
        failed: Whether the answer is an error, or no proper answer at all.
            Whoever waited for it has to know that it came, whatever it holds.
    """

    id: Id | None
    result: object
    failed: bool


class MalformedLineError(Exception):
    """A line that is no message.

    Attributes:
        code: The code of the error that answers it.
        message: A sentence for a person.
        id: The id of the request, where the line let one be read.
    """

    def __init__(self, code: int, message: str, id: Id | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.id = id


class RpcError(Exception):
    """A request cannot be done. It is answered with this error.

    Attributes:
        code: The code of the error.
        message: A sentence for a person.
    """

    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class _NotReadError(ValueError):
    """A line that is JSON, and more than this program reads."""


def _no_such_number(written: str) -> object:
    raise ValueError(f"{written} is no number of JSON")


def _whole(written: str) -> int:
    try:
        return int(written)
    except ValueError:
        # It is written as JSON writes a whole number, so only its length is in the way:
        # Python reads no more than some thousand digits.
        raise _NotReadError("holds a whole number that is too long to be read") from None


def _id_of(message: Mapping[str, object]) -> Id | None:
    """Return the id of a message, if it has one that can be an id."""
    given = message.get("id")
    # True and False are whole numbers to Python, and are none here.
    if isinstance(given, str) or (isinstance(given, int) and not isinstance(given, bool)):
        return given
    return None


def parsed(line: bytes) -> Request | Notification | Response:
    """Read the message that a line holds.

    Args:
        line: The line, without the line break.

    Raises:
        MalformedLineError: The line holds no message.
    """
    try:
        written = line.decode("utf-8")
    except UnicodeDecodeError:
        raise MalformedLineError(PARSE_ERROR, "The line is not UTF-8.") from None
    try:
        read: object = json.loads(written, parse_constant=_no_such_number, parse_int=_whole)
    except _NotReadError as error:
        raise MalformedLineError(PARSE_ERROR, f"The line {error}.") from None
    except RecursionError:
        raise MalformedLineError(
            PARSE_ERROR, "The line is nested more deeply than this program reads."
        ) from None
    except ValueError as error:
        raise MalformedLineError(PARSE_ERROR, f"The line is not JSON: {error}.") from None
    if isinstance(read, list):
        raise MalformedLineError(
            INVALID_REQUEST, "A line holds one message. Batches are not taken."
        )
    if not isinstance(read, dict):
        raise MalformedLineError(INVALID_REQUEST, "A message is an object of JSON.")
    message = cast("dict[str, object]", read)
    id = _id_of(message)
    versioned = message.get("jsonrpc") == _VERSION
    if "method" not in message:
        return _response(message, id, versioned)
    if not versioned:
        raise MalformedLineError(INVALID_REQUEST, 'A message says "jsonrpc": "2.0".', id)
    method = message["method"]
    if not isinstance(method, str):
        raise MalformedLineError(INVALID_REQUEST, "The method of a message is a string.", id)
    params = message.get("params", {})
    if "id" not in message:
        # Nothing answers a notification, so nothing can say what is wrong with one.
        shaped = cast("dict[str, object]", params) if isinstance(params, dict) else {}
        return Notification(method, shaped)
    if id is None:
        raise MalformedLineError(
            INVALID_REQUEST, "The id of a request is a string or a whole number."
        )
    if not isinstance(params, dict):
        raise MalformedLineError(INVALID_PARAMS, "The params of a request are an object.", id)
    return Request(id, method, cast("dict[str, object]", params))


def _response(message: Mapping[str, object], id: Id | None, versioned: bool) -> Response:
    if id is None and "result" not in message and "error" not in message:
        raise MalformedLineError(
            INVALID_REQUEST, "A message has a method, or it answers a request and names its id."
        )
    # An answer that is none, such as one with a result and an error, is a failure to
    # whoever waited for it: it came, and it cannot be gone by. The same goes for one
    # that does not say which protocol it speaks. Nothing is sent back for either: an
    # error would name the id, and the id of an answer is a number of this side.
    #
    # An answer that names no request is an answer too: the error of a side that could
    # not read a line has no id. It is not refused either. Another program like this one
    # would answer the refusal with a refusal, and so on without end.
    proper = versioned and id is not None and "result" in message and "error" not in message
    return Response(id, message.get("result") if proper else None, failed=not proper)


def text(value: object) -> str:
    """Write a value as JSON on one line, with every character as it is."""
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def answered(id: Id, result: str = "{}") -> str:
    """Return the line that answers a request.

    Args:
        id: The id of the request.
        result: The result, as JSON.
    """
    return f'{{"jsonrpc":"2.0","id":{text(id)},"result":{result}}}'


def refused(id: Id | None, code: int, message: str) -> str:
    """Return the line that answers a request with an error.

    Args:
        id: The id of the request, or ``None`` where the line let none be read.
        code: The code of the error.
        message: A sentence for a person.
    """
    error = text({"code": code, "message": message})
    return f'{{"jsonrpc":"2.0","id":{text(id)},"error":{error}}}'


def told(method: str, params: str) -> str:
    """Return the line of a notification. The params are JSON."""
    return f'{{"jsonrpc":"2.0","method":{text(method)},"params":{params}}}'


def asked(id: Id, method: str, params: str) -> str:
    """Return the line of a request that this side makes. The params are JSON."""
    return f'{{"jsonrpc":"2.0","id":{text(id)},"method":{text(method)},"params":{params}}}'


def on_the_wire(line: str) -> bytes:
    """Return the bytes that send a line, with the line break.

    A text may hold half a character: a name of a file that could not be
    read as text does. Such a line cannot be written as UTF-8. It is written
    with every character beyond ASCII spelled out instead, which is JSON all
    the same, so no message is lost over it.
    """
    try:
        return line.encode("utf-8") + b"\n"
    except UnicodeEncodeError:
        return json.dumps(json.loads(line), separators=(",", ":")).encode("ascii") + b"\n"
