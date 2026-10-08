"""The line format of a session file: a header line, then one event on each line."""

import functools
import json
from collections.abc import Mapping
from typing import ClassVar, Literal, cast

from pydantic import ConfigDict, JsonValue, PrivateAttr, ValidationError

from tack.data import Data
from tack.session.errors import UnreadableSessionError
from tack.session.events import EventData, SessionEvent

FORMAT_VERSION = 1
"""The version of the file format that this program reads and writes."""


class SessionHeader(Data):
    """The first line of a session file.

    Attributes:
        type: Always ``session``.
        version: The version of the file format.
        id: The id of the session.
        created_at: When the session was created, in milliseconds since the epoch.
        cwd: The working directory that the session was created in.
        parent_session: The id of the session that this one was forked from.
    """

    type: Literal["session"] = "session"
    version: int = FORMAT_VERSION
    id: str
    created_at: int
    cwd: str
    parent_session: str | None = None


class UnknownEvent(EventData):
    """An event of a type that nothing has registered, kept exactly as it was read.

    Only an event that was written as ignorable is kept this way. It takes no
    part in anything. It remembers the line it was read from, and that line is
    what is written when a log is copied, byte for byte.
    Each unknown type gets a subclass of its own, which carries the name.
    """

    model_config = ConfigDict(frozen=True, extra="allow")

    event_type: ClassVar[str] = "unknown"
    ignorable: ClassVar[bool] = True

    _line: str = PrivateAttr(default="")


@functools.cache
def _unknown(name: str) -> type[UnknownEvent]:
    return cast("type[UnknownEvent]", type("UnknownEvent", (UnknownEvent,), {"event_type": name}))


class _Replace(Data):
    op: Literal["replace"]
    start_seq: int
    end_seq: int


class _Line(Data):
    type: str
    seq: int
    time: int
    data: dict[str, JsonValue]
    ignorable: bool = False
    surface_op: Literal["append"] | _Replace | None = None
    source_event_seqs: tuple[int, ...] = ()


def _summary(error: ValidationError) -> str:
    return "; ".join(
        f"{'.'.join(str(part) for part in problem['loc'])}: {problem['msg']}"
        for problem in error.errors()
    )


def encode_header(header: SessionHeader) -> str:
    """Return the line that opens a session file, without the line break."""
    return header.model_dump_json()


def decode_header(line: str | bytes) -> SessionHeader:
    """Read the line that opens a session file.

    Raises:
        UnreadableSessionError: The line is not a header, or the file was
            written in another version of the format.
    """
    try:
        raw: object = json.loads(line)
    except ValueError as error:
        raise UnreadableSessionError("the first line is not a session header") from error
    if not isinstance(raw, dict) or cast("dict[str, object]", raw).get("type") != "session":
        raise UnreadableSessionError("the first line is not a session header")
    version = cast("dict[str, object]", raw).get("version")
    if isinstance(version, int) and version != FORMAT_VERSION:
        raise UnreadableSessionError(
            f"the session was written in format version {version}, "
            f"and this program reads version {FORMAT_VERSION}"
        )
    try:
        return SessionHeader.model_validate(raw)
    except ValidationError as error:
        raise UnreadableSessionError(
            f"the first line is not a session header: {_summary(error)}"
        ) from error


def encode_event(event: SessionEvent[EventData]) -> str:
    """Return the line that stores ``event``, without the line break."""
    data = event.data
    if isinstance(data, UnknownEvent):
        return data._line  # pyright: ignore[reportPrivateUsage]
    parts = [
        f'"type":{json.dumps(event.type)}',
        f'"seq":{event.seq}',
        f'"time":{event.time}',
        f'"data":{event.data.model_dump_json()}',
    ]
    if event.data.ignorable:
        parts.append('"ignorable":true')
    if event.replaces:
        first, last = event.replaces[0], event.replaces[-1]
        listed = ",".join(str(seq) for seq in event.replaces)
        parts.append(f'"surface_op":{{"op":"replace","start_seq":{first},"end_seq":{last}}}')
        parts.append(f'"source_event_seqs":[{listed}]')
    elif event.on_surface:
        parts.append('"surface_op":"append"')
    return "{" + ",".join(parts) + "}"


def decode_event(
    line: str | bytes, kinds: Mapping[str, type[EventData]]
) -> SessionEvent[EventData]:
    """Read the line that stores one event.

    An event of a type that ``kinds`` does not hold is kept as an
    :class:`UnknownEvent` if its line marks it as ignorable.

    Raises:
        UnreadableSessionError: The line is not a well-formed event, or it
            holds an event of an unknown type that may not be skipped.
    """
    try:
        raw: object = json.loads(line)
    except ValueError as error:
        raise UnreadableSessionError("the line is not valid JSON") from error
    if not isinstance(raw, dict):
        raise UnreadableSessionError("the line is not an event")
    try:
        parsed = _Line.model_validate(raw)
    except ValidationError as error:
        raise UnreadableSessionError(
            f"the line is not a well-formed event: {_summary(error)}"
        ) from error

    replaces = parsed.source_event_seqs
    if isinstance(parsed.surface_op, _Replace):
        if not replaces:
            raise UnreadableSessionError("the event does not list the events it replaces")
        if (replaces[0], replaces[-1]) != (parsed.surface_op.start_seq, parsed.surface_op.end_seq):
            raise UnreadableSessionError(
                "the range the event replaces does not match the events it lists"
            )
    elif replaces:
        raise UnreadableSessionError("the event lists the events it replaces without replacing any")
    on_surface = parsed.surface_op is not None

    kind = kinds.get(parsed.type)
    data: EventData
    if kind is None:
        if not parsed.ignorable:
            raise UnreadableSessionError(f"the event type {parsed.type!r} is not known")
        if on_surface:
            raise UnreadableSessionError(
                f"an event of the unknown type {parsed.type!r} carries a message "
                "and cannot be skipped"
            )
        placeholder = _unknown(parsed.type).model_validate(parsed.data)
        placeholder._line = line if isinstance(line, str) else line.decode()  # pyright: ignore[reportPrivateUsage]
        data = placeholder
    else:
        try:
            # Read as JSON, the way append checked it: a strict model takes a JSON
            # array for a tuple only this way.
            data = kind.model_validate_json(json.dumps(parsed.data))
        except ValidationError as error:
            raise UnreadableSessionError(
                f"the data of the {parsed.type} event is not well formed: {_summary(error)}"
            ) from error
        if on_surface and data.for_model() is None:
            raise UnreadableSessionError(
                f"the {parsed.type} event is marked as a message but holds none"
            )
    return SessionEvent(
        seq=parsed.seq, time=parsed.time, data=data, on_surface=on_surface, replaces=replaces
    )
