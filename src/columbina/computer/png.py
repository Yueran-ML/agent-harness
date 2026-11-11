"""Pictures of the screen as PNG files, written with what Python brings.

A system hands over a picture as rows of pixels. :func:`encode` writes them
as a PNG, which every model takes, and :func:`shrunk` makes a picture
smaller first where the system did not: it takes the nearest pixel, which
is coarse and quick, and enough for a model to find a button by.
"""

import struct
import zlib
from operator import itemgetter

__all__ = ["encode", "shrunk"]

_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def _chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def encode(width: int, height: int, rgb: bytes) -> bytes:
    """Return a PNG of a picture.

    Args:
        width: How many pixels a row has.
        height: How many rows there are.
        rgb: The rows from the top, each pixel as its red, green and blue.

    Raises:
        ValueError: The pixels are not those of a picture of this size, or it has none.
    """
    stride = width * 3
    if width <= 0 or height <= 0 or len(rgb) != stride * height:
        raise ValueError(f"{len(rgb)} bytes are not a picture of {width} by {height} pixels")
    # Each row is written as it is: the byte before it says that nothing was done to it.
    rows = b"".join(b"\x00" + rgb[at : at + stride] for at in range(0, len(rgb), stride))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"".join(
        (
            _SIGNATURE,
            _chunk(b"IHDR", header),
            _chunk(b"IDAT", zlib.compress(rows, 6)),
            _chunk(b"IEND", b""),
        )
    )


def shrunk(width: int, height: int, rgb: bytes, size: tuple[int, int]) -> bytes:
    """Return the pixels of a picture at another size, each the nearest one of the picture.

    Args:
        width: How many pixels a row of the picture has.
        height: How many rows it has.
        rgb: Its rows from the top, each pixel as its red, green and blue.
        size: The width and the height to bring it to.

    Raises:
        ValueError: The pixels are not those of a picture of this size, or a size has none.
    """
    to_width, to_height = size
    stride = width * 3
    if min(width, height, to_width, to_height) <= 0 or len(rgb) != stride * height:
        raise ValueError(f"{len(rgb)} bytes are not a picture of {width} by {height} pixels")
    if (to_width, to_height) == (width, height):
        return rgb
    columns = [min(column * width // to_width, width - 1) * 3 for column in range(to_width)]
    # A pixel has three bytes, so there are three offsets at least, and the getter gives a row.
    pick = itemgetter(*(offset + channel for offset in columns for channel in range(3)))
    rows: list[bytes] = []
    for row in range(to_height):
        at = min(row * height // to_height, height - 1) * stride
        rows.append(bytes(pick(rgb[at : at + stride])))
    return b"".join(rows)
