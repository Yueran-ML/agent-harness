"""Timelines of the engine for the tests, as the engine writes them in a project of schema 2."""

from fractions import Fraction
from typing import Any

__all__ = ["SCALE", "clip_of", "timeline_of", "track_of"]

SCALE = 12800
"""The scale of the media of the tests: 512 ticks a frame at 25 frames a second."""


def clip_of(
    id: str,
    start: int,
    length: int,
    *,
    media: str = "m1",
    ticks: int = 0,
    speed: Fraction | int = 1,
    reverse: bool = False,
    scale: int = SCALE,
) -> dict[str, Any]:
    """A clip of media that begins ``ticks`` into it."""
    speed = Fraction(speed)
    return {
        "id": id,
        "start": start,
        "length": length,
        "source": {"kind": "media", "media": media, "in": {"ticks": ticks, "scale": scale}},
        "enabled": True,
        "speed": {"num": speed.numerator, "den": speed.denominator},
        "reverse": reverse,
        "link": None,
        "transform": {"x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "opacity": 1.0},
        "gain_db": 0.0,
        "pan": 0.0,
        "fade_in": 0,
        "fade_out": 0,
    }


def track_of(
    id: str, clips: list[dict[str, Any]], *, kind: str = "audio", locked: bool = False
) -> dict[str, Any]:
    """A track of picture or sound with clips on it."""
    return {
        "id": id,
        "kind": kind,
        "name": id,
        "enabled": True,
        "locked": locked,
        "clips": clips,
        "transitions": [],
    }


def timeline_of(*tracks: dict[str, Any], rate: tuple[int, int] = (25, 1)) -> dict[str, Any]:
    """Timeline ``t1`` with tracks."""
    return {
        "id": "t1",
        "name": "Cut",
        "rate": {"num": rate[0], "den": rate[1]},
        "width": 1920,
        "height": 1080,
        "sample_rate": 48000,
        "tracks": list(tracks),
    }
