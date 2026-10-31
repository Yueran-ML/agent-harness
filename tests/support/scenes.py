"""Frames of a scene made in numbers, for the tests of Shot Match: their pixels, their figures
as the engine counts them, and a PNG of them as the engine writes the thumbnail of a still."""

import random
import struct
import zlib

from columbina.video import shotmatch
from columbina.video.shotmatch import Figures, ShotMatch

__all__ = ["SCENE", "Pixel", "figures_of", "pixels", "png_of", "through"]

Pixel = tuple[float, float, float]


def pixels(seed: int = 1, count: int = 20000) -> list[Pixel]:
    """A frame of a scene: a spread of lightness, each channel a little apart, a warm cast."""
    rng = random.Random(seed)
    out: list[Pixel] = []
    for _ in range(count):
        y = rng.betavariate(2, 2.5)
        out.append(
            (
                min(max(y + rng.gauss(0, 0.06) + 0.02, 0.0), 1.0),
                min(max(y + rng.gauss(0, 0.04), 0.0), 1.0),
                min(max(y + rng.gauss(0, 0.07) - 0.03, 0.0), 1.0),
            )
        )
    return out


def figures_of(frame: list[Pixel], bins: int = 1024) -> Figures:
    """The figures of pixels as the engine counts them: bins about b / (bins - 1), clamped."""
    histogram = {name: [0] * bins for name in ("red", "green", "blue")}
    for pixel in frame:
        for name, value in zip(("red", "green", "blue"), pixel, strict=True):
            histogram[name][round(min(max(value, 0.0), 1.0) * (bins - 1))] += 1
    means = [sum(pixel[at] for pixel in frame) / len(frame) for at in range(3)]
    return shotmatch.figures(histogram, means)


def through(frame: list[Pixel], match: ShotMatch) -> list[Pixel]:
    return [(match.channel(p[0], 0), match.channel(p[1], 1), match.channel(p[2], 2)) for p in frame]


def png_of(rows: list[bytes], width: int, channels: int) -> bytes:
    """A PNG of 8 bits, each row written with the next of the five filters."""

    def chunk(kind: bytes, body: bytes) -> bytes:
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    stride = width * channels
    raw = bytearray()
    above = bytes(stride)
    for y, row in enumerate(rows):
        kind = y % 5
        out = bytearray()
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            up = above[x]
            corner = above[x - channels] if x >= channels else 0
            if kind == 0:
                guess = 0
            elif kind == 1:
                guess = left
            elif kind == 2:
                guess = up
            elif kind == 3:
                guess = (left + up) >> 1
            else:
                p = left + up - corner
                near = sorted(
                    ((abs(p - left), 0, left), (abs(p - up), 1, up), (abs(p - corner), 2, corner))
                )
                guess = near[0][2]
            out.append((row[x] - guess) & 0xFF)
        raw += bytes([kind]) + out
        above = row
    colour = 2 if channels == 3 else 6
    header = struct.pack(">IIBBBBB", width, len(rows), 8, colour, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw)))
        + chunk(b"IEND", b"")
    )


SCENE = pixels()
