"""Where a clip of a timeline reads its media: the counts of the engine, for the story.

The engine decides which place of its media each frame of a clip shows, and the story of a track
(:mod:`columbina.video.story`) and the cuts made by its words (:mod:`columbina.video.textcut`)
must agree with it to the frame. So :func:`ticks_of_scaled`, :func:`source_span`,
:func:`source_tick` and :func:`media_frame` count as ``engine/crates/model/src/span.rs`` of the
repository counts, and must be kept the same as those functions are:

- ``frames`` frames of a timeline of ``rate``, at ``speed``, last
  ``floor(frames * speed * rate.den * scale / rate.num)`` ticks of the ``scale`` of the media.
- A clip shows the ticks that its length lasts: its span. Its frame ``k`` shows the place that
  many ticks of ``k`` frames after where it begins in its media; a clip that plays backward
  shows in its frame ``k`` what its frame ``length - 1 - k`` shows forward.
- A place in media is in the frame of its picture that is read for it: frame ``n`` of a picture
  of ``rate`` begins at ``floor(n * scale * rate.den / rate.num)`` ticks.

- A clip with speed points (``retime``) is read along them, and not at its speed, which is then
  1/1 forward: :class:`columbina.video.retime.Course` counts its path as the engine's
  ``retime.rs`` counts it. Its span is from the lowest place of the path, where the clip begins
  in its media, to the highest.

The engine counts in 64 bits, and gives the largest count there is where one would be too large;
no project comes near that, and whole numbers of Python are not bounded, so that is not done
here. :func:`first_frame_at`, :func:`frames_showing` and :func:`runs_showing` turn those counts
round: which frames of a clip show a stretch of its media.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from fractions import Fraction
from typing import Any

from columbina.video.edits import Rate, Speed, Time
from columbina.video.retime import Course, course_of

__all__ = [
    "Placed",
    "first_frame_at",
    "frames_showing",
    "media_frame",
    "placed",
    "runs_showing",
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
        speed: How fast it plays, where it has no speed points.
        reverse: Whether it plays backward, where it has no speed points.
        course: The course of a clip with speed points through its media, which it is read by,
            and not by its speed; ``None`` for a clip of one speed.
    """

    id: str
    media: str
    start: int
    length: int
    begins: Time
    speed: Speed = _AS_RECORDED
    reverse: bool = False
    course: Course | None = None

    @property
    def end(self) -> int:
        """The frame after its last."""
        return self.start + self.length


def placed(clip: Mapping[str, Any]) -> Placed | None:
    """Read a clip as the engine writes it; ``None`` for a clip of a title or a colour.

    Its speed points are read too (:func:`columbina.video.retime.course_of`); those that are not
    the speed points of the clip are left out, and the clip is read at its speed, as the engine
    reads it.

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
            course=course_of(clip),
        )
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError(f"it is no clip of the engine: {error!r}") from None


def ticks_of_scaled(frames: int, speed: Speed, rate: Rate, scale: int) -> int:
    """The ticks of ``scale`` that ``frames`` frames of ``rate`` last at ``speed``, rounded down.

    As ``ticks_of_scaled`` of ``span.rs``: ``frames * speed * rate.den * scale / rate.num``.
    """
    return (frames * rate.den * scale * speed.num) // (speed.den * rate.num)


def source_span(clip: Placed, rate: Rate) -> int:
    """The ticks of its media that a clip shows, in the scale of its media, as ``span.rs`` says.

    With speed points, from the lowest place of their path to the highest.
    """
    if clip.course is not None:
        return clip.course.span(rate, clip.begins.scale)
    return ticks_of_scaled(clip.length, clip.speed, rate, clip.begins.scale)


def source_tick(clip: Placed, k: int, rate: Rate) -> int:
    """The place in its media that frame ``k`` of a clip shows, as ``span.rs`` counts it.

    Forward, where it begins and the ticks of ``k`` frames; backward, what frame
    ``length - 1 - k`` shows forward. With speed points, the place of their path at the frame:
    one picture all through a freeze, and lower places frame by frame where a segment plays
    backward.
    """
    if clip.course is not None:
        return clip.course.tick(clip.begins.ticks, k, rate, clip.begins.scale)
    frame = clip.length - 1 - k if clip.reverse else k
    return clip.begins.ticks + ticks_of_scaled(frame, clip.speed, rate, clip.begins.scale)


def media_frame(tick: int, rate: Rate, duration: Time) -> int:
    """The frame of a picture of ``rate`` that the place ``tick`` is in, as ``span.rs`` counts it.

    ``tick`` is in ticks of the scale of ``duration``, how long the media lasts. Frame ``n``
    begins at the tick that ``n`` frames last, rounded down, so the frame of a tick is
    ``ceil((tick + 1) * rate.num / (scale * rate.den)) - 1``. A place before the start is in the
    first frame, and one at the end or after it in the last.
    """
    kept = min(max(tick, 0), max(duration.ticks - 1, 0))
    return ((kept + 1) * rate.num - 1) // (duration.scale * rate.den)


def first_frame_at(clip: Placed, ticks: Fraction | int, rate: Rate) -> int:
    """The first frame of a clip, counted forward, that shows ``ticks`` of its media or later.

    It is the length of the clip where none does, and 0 where all do. A clip with speed points
    may show such a place and an earlier one after it, where it plays backward.
    """
    ticks = Fraction(ticks)
    if clip.course is not None:
        after = ticks - clip.begins.ticks
        runs = clip.course.showing(after, None, rate, clip.begins.scale)
        return runs[0][0] if runs else clip.length
    return _first_frame(clip, ticks.numerator, ticks.denominator, rate)


def _first_frame(clip: Placed, num: int, den: int, rate: Rate) -> int:
    """:func:`first_frame_at` of ``num / den`` ticks, in whole numbers alone, which is quicker."""
    after = num - clip.begins.ticks * den
    if after <= 0:
        return 0
    # floor(k * per) >= ceil(after) where k * per >= ceil(after), and per is ticks a frame.
    per_num = clip.speed.num * rate.den * clip.begins.scale
    per_den = clip.speed.den * rate.num
    whole = -(-after // den)
    return min(clip.length, -(-whole * per_den // per_num))


def frames_showing(
    clip: Placed, start_us: Fraction | int, end_us: Fraction | int, rate: Rate
) -> tuple[int, int]:
    """The frames of a clip that show its media from ``start_us`` to ``end_us``.

    Those are the frames whose place in the media is in the stretch, counted from the first
    frame of the clip, the first and the one after the last; none where the two are the same.
    A clip that plays backward shows them last first, so they are counted from its end.

    A clip with speed points may come by the stretch more than once: these are the frames of
    the first time (:func:`runs_showing` gives them all), with those of a freeze that holds a
    picture of the stretch. Where no frame shows it, both are the first frame that shows
    ``start_us`` or later, or the length of the clip.
    """
    scale = clip.begins.scale
    start, end = Fraction(start_us), Fraction(end_us)
    if clip.course is not None:
        runs = runs_showing(clip, start, end, rate)
        if runs:
            return runs[0]
        nowhere = first_frame_at(clip, start * scale / 1_000_000, rate)
        return nowhere, nowhere
    first = _first_frame(clip, start.numerator * scale, start.denominator * 1_000_000, rate)
    after = _first_frame(clip, end.numerator * scale, end.denominator * 1_000_000, rate)
    after = max(first, after)
    if clip.reverse:
        return clip.length - after, clip.length - first
    return first, after


def runs_showing(
    clip: Placed,
    start_us: Fraction | int,
    end_us: Fraction | int,
    rate: Rate,
    *,
    heard: bool = False,
) -> list[tuple[int, int]]:
    """Every run of frames of a clip that shows its media from ``start_us`` to ``end_us``.

    Each is its first frame and the frame after its last, counted from the first frame of the
    clip, in the order of the frames. A clip of one speed has one run, that of
    :func:`frames_showing`, or none. A clip with speed points has one for each time that its
    path comes by the stretch; with ``heard``, the frames of a freeze are left out, since a
    freeze has no sound.
    """
    if clip.course is None:
        first, after = frames_showing(clip, start_us, end_us, rate)
        return [(first, after)] if first < after else []
    scale = clip.begins.scale
    start = Fraction(start_us) * scale / 1_000_000 - clip.begins.ticks
    end = Fraction(end_us) * scale / 1_000_000 - clip.begins.ticks
    return clip.course.showing(start, end, rate, scale, heard=heard)
