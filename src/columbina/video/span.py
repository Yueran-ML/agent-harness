"""Where a clip of a timeline reads its media: the counts of the engine, for the story.

The engine decides which place of its media each frame of a clip shows, and the story of a track
(:mod:`columbina.video.story`) and the cuts made by its words (:mod:`columbina.video.textcut`)
must agree with it to the frame. So :func:`ticks_of_scaled`, :func:`source_span` and
:func:`source_tick` count as ``engine/crates/model/src/span.rs`` of the repository counts, and
must be kept the same as those functions are:

- ``frames`` frames of a timeline of ``rate``, at ``speed``, last
  ``floor(frames * speed * rate.den * scale / rate.num)`` ticks of the ``scale`` of the media.
- A clip shows the ticks that its length lasts: its span. Its frame ``k`` shows the place that
  many ticks of ``k`` frames after where it begins in its media; a clip that plays backward
  shows in its frame ``k`` what its frame ``length - 1 - k`` shows forward.

The engine counts in 64 bits, and gives the largest count there is where one would be too large;
no project comes near that, and whole numbers of Python are not bounded, so that is not done
here. :func:`first_frame_at` and :func:`frames_showing` turn those counts round: which frames of a
clip show a stretch of its media.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from fractions import Fraction
from math import ceil
from typing import Any

from columbina.video.edits import Rate, Speed, Time

__all__ = [
    "Placed",
    "first_frame_at",
    "frames_showing",
    "placed",
    "source_span",
    "source_tick",
    "ticks_of_scaled",
]


_AS_RECORDED = Speed(num=1, den=1)


@dataclass(frozen=True)
class Placed:
    """A clip of media on a track, as far as where it reads its media goes.

    Attributes:
        id: The clip.
        media: Its media.
        start: The frame of the timeline that it begins at.
        length: How many frames it lasts.
        begins: Where it begins in its media, in ticks of the scale of the media.
        speed: How fast it plays.
        reverse: Whether it plays backward.
    """

    id: str
    media: str
    start: int
    length: int
    begins: Time
    speed: Speed = _AS_RECORDED
    reverse: bool = False

    @property
    def end(self) -> int:
        """The frame after its last."""
        return self.start + self.length


def placed(clip: Mapping[str, Any]) -> Placed | None:
    """Read a clip as the engine writes it; ``None`` for a clip of a title or a colour.

    Raises:
        ValueError: It is not a clip of the engine.
    """
    try:
        source = clip["source"]
        if source.get("kind") != "media":
            return None
        speed = clip.get("speed") or {"num": 1, "den": 1}
        return Placed(
            id=str(clip["id"]),
            media=str(source["media"]),
            start=int(clip["start"]),
            length=int(clip["length"]),
            begins=Time.model_validate(source["in"]),
            speed=Speed.model_validate(speed),
            reverse=bool(clip.get("reverse", False)),
        )
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError(f"it is no clip of the engine: {error!r}") from None


def ticks_of_scaled(frames: int, speed: Speed, rate: Rate, scale: int) -> int:
    """The ticks of ``scale`` that ``frames`` frames of ``rate`` last at ``speed``, rounded down.

    As ``ticks_of_scaled`` of ``span.rs``: ``frames * speed * rate.den * scale / rate.num``.
    """
    return (frames * rate.den * scale * speed.num) // (speed.den * rate.num)


def source_span(clip: Placed, rate: Rate) -> int:
    """The ticks of its media that a clip shows, in the scale of its media, as ``span.rs`` says."""
    return ticks_of_scaled(clip.length, clip.speed, rate, clip.begins.scale)


def source_tick(clip: Placed, k: int, rate: Rate) -> int:
    """The place in its media that frame ``k`` of a clip shows, as ``span.rs`` counts it.

    Forward, where it begins and the ticks of ``k`` frames; backward, what frame
    ``length - 1 - k`` shows forward.
    """
    frame = clip.length - 1 - k if clip.reverse else k
    return clip.begins.ticks + ticks_of_scaled(frame, clip.speed, rate, clip.begins.scale)


def first_frame_at(clip: Placed, ticks: Fraction | int, rate: Rate) -> int:
    """The first frame of a clip, counted forward, that shows ``ticks`` of its media or later.

    It is the length of the clip where none does, and 0 where all do.
    """
    after = Fraction(ticks) - clip.begins.ticks
    if after <= 0:
        return 0
    # floor(k * per) >= ceil(after) where k * per >= ceil(after).
    per = Fraction(clip.speed.num * rate.den * clip.begins.scale, clip.speed.den * rate.num)
    return min(clip.length, ceil(ceil(after) / per))


def frames_showing(
    clip: Placed, start_us: Fraction | int, end_us: Fraction | int, rate: Rate
) -> tuple[int, int]:
    """The frames of a clip that show its media from ``start_us`` to ``end_us``.

    Those are the frames whose place in the media is in the stretch, counted from the first
    frame of the clip, the first and the one after the last; none where the two are the same.
    A clip that plays backward shows them last first, so they are counted from its end.
    """
    scale = clip.begins.scale
    first = first_frame_at(clip, Fraction(start_us) * scale / 1_000_000, rate)
    after = first_frame_at(clip, Fraction(end_us) * scale / 1_000_000, rate)
    after = max(first, after)
    if clip.reverse:
        return clip.length - after, clip.length - first
    return first, after
