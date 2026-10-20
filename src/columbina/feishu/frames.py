"""The frames of the long connection to Feishu: what one holds, and how it is written and read.

Feishu sends the events of an app over a WebSocket that the app opens itself, so that a
program on a machine with no address of its own on the internet hears of them. Every message
on that connection is one frame, written the way of Protocol Buffers, and of one kind only::

    message Header { required string key = 1; required string value = 2; }
    message Frame {
      required uint64 SeqID = 1;
      required uint64 LogID = 2;
      required int32 service = 3;
      required int32 method = 4;
      repeated Header headers = 5;
      optional string payload_encoding = 6;
      optional string payload_type = 7;
      optional bytes payload = 8;
      optional string LogIDNew = 9;
    }

So the few lines that write and read such a frame are here, and no library of Protocol
Buffers is needed for them. A field that a frame holds and this program does not know is
passed over when the frame is read, as Protocol Buffers want, and so is a known field that is
written in another way than its kind is.

A frame of the method :data:`CONTROL` keeps the connection going: the program sends a ping
now and then, and Feishu answers with a pong. A frame of the method :data:`DATA` carries an
event, as JSON in its payload. Its headers say of what kind it is, and an event that is too
large for one frame comes in several, which the headers number. How the connection goes is
in :mod:`columbina.feishu.link`.
"""

from collections.abc import Iterator
from dataclasses import dataclass, replace

__all__ = ["CONTROL", "DATA", "Frame", "FrameError", "decoded", "encoded"]

CONTROL = 0
"""The method of a frame that keeps the connection going."""

DATA = 1
"""The method of a frame that carries something for the app."""

_VARINT = 0
_FIXED64 = 1
_LENGTH = 2
_FIXED32 = 5
_FIXED = {_FIXED64: 8, _FIXED32: 4}
"""How many bytes a field of fixed size takes. No field of a frame is of that kind."""

_LONGEST = 10
"""How many bytes a number takes at most."""

_UINT64 = (1 << 64) - 1
_REQUIRED = {1: "SeqID", 2: "LogID", 3: "service", 4: "method"}


class FrameError(Exception):
    """What came over the connection is no frame."""


@dataclass(frozen=True, slots=True)
class Frame:
    """A frame of the long connection.

    A field that a frame may leave out is ``None`` where it was left out, so that a frame that
    is sent back is written with what it came with and nothing more.

    Attributes:
        seq_id: The number of the frame in a series of them. A ping has 0.
        log_id: A number that Feishu traces the frame by. A ping has 0.
        service: The service that the connection was opened for. Feishu names it in the
            address that it gives for the connection.
        method: :data:`CONTROL` or :data:`DATA`.
        headers: The headers, in their order.
        payload_encoding: How the payload is encoded, where the frame says so.
        payload_type: Of what type the payload is, where the frame says so.
        payload: What the frame carries.
        log_id_new: Another id that Feishu traces the frame by, where there is one.
    """

    seq_id: int = 0
    log_id: int = 0
    service: int = 0
    method: int = CONTROL
    headers: tuple[tuple[str, str], ...] = ()
    payload_encoding: str | None = None
    payload_type: str | None = None
    payload: bytes | None = None
    log_id_new: str | None = None

    def header(self, key: str) -> str | None:
        """Return the value of the first header of a key, or ``None`` if there is none."""
        return next((value for name, value in self.headers if name == key), None)

    def with_header(self, key: str, value: str) -> "Frame":
        """Return the frame with one more header, after the ones that it has."""
        return replace(self, headers=(*self.headers, (key, value)))


def _varint(value: int) -> bytes:
    """Write a number in as many bytes as it needs, seven bits to a byte, the lowest first.

    A number below zero is written as the 64 bits of its two's complement, in ten bytes.
    """
    value &= _UINT64
    written = bytearray()
    while value > 0x7F:
        written.append(value & 0x7F | 0x80)
        value >>= 7
    written.append(value)
    return bytes(written)


def _key(number: int, way: int) -> bytes:
    return _varint(number << 3 | way)


def _chunk(number: int, data: bytes) -> bytes:
    """Write a field that is a run of bytes: its key, its length, and the bytes."""
    return _key(number, _LENGTH) + _varint(len(data)) + data


def encoded(frame: Frame) -> bytes:
    """Write a frame as it goes over the connection, its fields in the order of their numbers."""
    parts = [
        _key(1, _VARINT) + _varint(frame.seq_id),
        _key(2, _VARINT) + _varint(frame.log_id),
        _key(3, _VARINT) + _varint(frame.service),
        _key(4, _VARINT) + _varint(frame.method),
    ]
    for key, value in frame.headers:
        parts.append(_chunk(5, _chunk(1, key.encode()) + _chunk(2, value.encode())))
    for number, text in ((6, frame.payload_encoding), (7, frame.payload_type)):
        if text is not None:
            parts.append(_chunk(number, text.encode()))
    if frame.payload is not None:
        parts.append(_chunk(8, frame.payload))
    if frame.log_id_new is not None:
        parts.append(_chunk(9, frame.log_id_new.encode()))
    return b"".join(parts)


def _number(data: bytes, at: int) -> tuple[int, int]:
    """Read a number that is written in as many bytes as it needs.

    Returns:
        The number, and where what follows it begins.

    Raises:
        FrameError: The number runs past the end, or takes more than ten bytes.
    """
    value = 0
    for shift in range(0, 7 * _LONGEST, 7):
        if at >= len(data):
            raise FrameError("a number runs past the end of the frame")
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value & _UINT64, at
    raise FrameError(f"a number of the frame takes more than {_LONGEST} bytes")


def _fields(data: bytes) -> Iterator[tuple[int, int | bytes]]:
    """Read the fields of a message, each as its number and its value.

    A number is the value of a field that is a number, and a run of bytes the value of one
    that is written as such. A field of fixed size is passed over.

    Raises:
        FrameError: The message is cut off, or is written in a way that frames never are.
    """
    at = 0
    while at < len(data):
        key, at = _number(data, at)
        number, way = key >> 3, key & 0x07
        if number == 0:
            raise FrameError("a field of the frame has the number 0")
        if way == _VARINT:
            value, at = _number(data, at)
            yield number, value
        elif way == _LENGTH:
            size, at = _number(data, at)
            if size > len(data) - at:
                raise FrameError("a field runs past the end of the frame")
            yield number, data[at : at + size]
            at += size
        elif way in _FIXED:
            at += _FIXED[way]
            if at > len(data):
                raise FrameError("a field runs past the end of the frame")
        else:
            raise FrameError(f"a field of the frame is written in a way, {way}, that is not used")


def _text(data: bytes) -> str:
    try:
        return data.decode()
    except UnicodeDecodeError:
        raise FrameError("a text of the frame is not UTF-8") from None


def _int32(value: int) -> int:
    """Return the number of 32 bits with a sign that a number read from a field stands for."""
    value &= 0xFFFFFFFF
    return value - (1 << 32) if value >= 1 << 31 else value


def _header(data: bytes) -> tuple[str, str]:
    """Read a header of a frame.

    Raises:
        FrameError: It is no header.
    """
    said: dict[int, str] = {}
    for number, value in _fields(data):
        if number in (1, 2) and isinstance(value, bytes):
            said[number] = _text(value)
    if 1 not in said or 2 not in said:
        raise FrameError("a header of the frame has no key or no value")
    return said[1], said[2]


def decoded(data: bytes) -> Frame:
    """Read a frame as it came over the connection.

    Of a field that may be given once and is given twice, the last one counts, as Protocol
    Buffers want.

    Raises:
        FrameError: It is no frame.
    """
    numbers: dict[int, int] = {}
    texts: dict[int, str] = {}
    headers: list[tuple[str, str]] = []
    payload: bytes | None = None
    for number, value in _fields(data):
        if isinstance(value, int):
            if number in _REQUIRED:
                numbers[number] = value
        elif number == 5:
            headers.append(_header(value))
        elif number in (6, 7, 9):
            texts[number] = _text(value)
        elif number == 8:
            payload = value
    missing = [name for number, name in _REQUIRED.items() if number not in numbers]
    if missing:
        raise FrameError(f"the frame has no {' and no '.join(missing)}")
    return Frame(
        seq_id=numbers[1],
        log_id=numbers[2],
        service=_int32(numbers[3]),
        method=_int32(numbers[4]),
        headers=tuple(headers),
        payload_encoding=texts.get(6),
        payload_type=texts.get(7),
        payload=payload,
        log_id_new=texts.get(9),
    )
