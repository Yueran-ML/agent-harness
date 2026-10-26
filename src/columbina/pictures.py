"""Pictures that a model is shown: which kinds there are, and how large one is.

A model that takes pictures takes PNG, JPEG, GIF and WebP, each up to a few
megabytes. What kind a file is, is read from its first bytes and not from its
name: a file that says it is a PNG and is something else is no picture. How
large a picture is, in pixels, is read from its header, so that the model can
be told without the picture being decoded.
"""

import struct

__all__ = ["KINDS", "MOST_BYTES", "kind_of", "size_of"]

KINDS = ("image/png", "image/jpeg", "image/gif", "image/webp")
"""The kinds of picture that are shown to a model."""

MOST_BYTES = 3_750_000
"""How large a picture may be. In base64 it is then 5 MB, the most that Anthropic takes."""

_JPEG_FRAMES = frozenset(
    {0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF}
)
"""The markers of JPEG after which the size of the picture stands."""


def kind_of(data: bytes) -> str | None:
    """Return what kind of picture the bytes are, by their beginning, or ``None``."""
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if data.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if data.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    return None


def size_of(data: bytes) -> tuple[int, int] | None:
    """Return the width and the height of a picture, or ``None`` where its header does not say."""
    kind = kind_of(data)
    try:
        if kind == "image/png":
            width, height = struct.unpack(">II", data[16:24])
            return width, height
        if kind == "image/gif":
            width, height = struct.unpack("<HH", data[6:10])
            return width, height
        if kind == "image/webp":
            return _webp(data)
        if kind == "image/jpeg":
            return _jpeg(data)
    except struct.error:
        return None
    return None


def _webp(data: bytes) -> tuple[int, int] | None:
    chunk = data[12:16]
    if chunk == b"VP8X":
        width = int.from_bytes(data[24:27], "little") + 1
        height = int.from_bytes(data[27:30], "little") + 1
        return width, height
    if chunk == b"VP8L":
        bits = int.from_bytes(data[21:25], "little")
        return (bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1
    if chunk == b"VP8 ":
        width, height = struct.unpack("<HH", data[26:30])
        return width & 0x3FFF, height & 0x3FFF
    return None


def _jpeg(data: bytes) -> tuple[int, int] | None:
    at = 2
    while at + 4 <= len(data):
        if data[at] != 0xFF:
            return None
        marker = data[at + 1]
        if marker in _JPEG_FRAMES:
            height, width = struct.unpack(">HH", data[at + 5 : at + 9])
            return width, height
        (length,) = struct.unpack(">H", data[at + 2 : at + 4])
        at += 2 + length
    return None
