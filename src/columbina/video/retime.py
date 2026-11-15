"""Where a clip with speed points reads its source: the course of the engine, counted exactly.

A clip whose speed changes on the way has speed points (``retime`` of ``engine/PROTOCOL.md``,
the Retime Controls of DaVinci Resolve). A speed point is at a frame of the clip, counted from
its first, and gives the speed of the segment that begins there and lasts to the next speed
point or to the end of the clip: above 0 forward, below 0 backward, 0 a freeze. :class:`Course`
counts where such a clip reads its media as ``Course`` of ``engine/crates/model/src/retime.rs``
counts it, and must be kept the same:

- The speeds are counted in one unit, the least number that every ``den`` divides, and a place
  is a whole number of that unit of a frame of the timeline. Put one after another, the
  segments are one path through the source without a jump; places are counted above the lowest
  place of the path within the clip, which is where the clip begins in its media
  (``source.in``).
- A frame of a segment that goes forward, or of a freeze, shows the place where the frame
  begins; a frame of a segment that goes backward shows the place where it ends. A frame before
  the clip is counted as the first segment goes, and one after it as the last.
- A place is brought to ticks of the media once, rounded down:
  ``floor(place * rate.den * scale / (unit * rate.num))``. So speed points of one speed give
  the ticks that :mod:`columbina.video.span` gives for a clip of that speed, to the tick.
- The sound goes along the same path. It is laid on the samples of the timeline from the frame
  at which the path is lowest (the first, where there are several): that frame begins at the
  sample that it starts at, rounded down, and every other sample is so many samples from
  there, exactly. A sample is of the segment that it begins in; one of a segment that goes
  backward reads the place where the sample ends.

The engine counts in 128 bits and gives the largest count there is where one would be too
large; whole numbers of Python are not bounded, so that is not done here.
"""

from bisect import bisect_right
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, replace
from fractions import Fraction
from itertools import pairwise
from math import lcm
from typing import Any

from columbina.video.edits import Rate

__all__ = ["MOST_POINTS", "Course", "Segment", "course_of"]

MOST_POINTS = 1024
"""The most speed points that a clip may have, as the engine has it."""

_LARGEST_UNIT = 0xFFFF_FFFF
"""The largest unit of the speeds of a clip: the engine keeps it in 32 bits."""


@dataclass(frozen=True)
class Segment:
    """A segment of a clip: the frames from one speed point to the next, at one speed.

    Attributes:
        start: Its first frame, where its speed point is.
        end: The frame after its last: where the next speed point is, or the end of the clip.
        num: The numerator of its speed: below 0 backward, and 0 a freeze.
        den: The denominator of its speed.
        at: The place of the path where it begins, in units of a frame, above the lowest place
            of the path within the clip.
        step: How far the path goes with each frame of it, in units of a frame: below 0, down
            the source.
    """

    start: int
    end: int
    num: int
    den: int
    at: int
    step: int

    def place(self, frame: int) -> int:
        """The place of the path at ``frame``, as the segment goes: before it and after it too."""
        return self.at + self.step * (frame - self.start)

    def shown(self, k: int) -> int:
        """The place that frame ``k`` shows: where the frame begins, or, backward, where it ends."""
        return self.place(k + 1 if self.step < 0 else k)


def _first(low: int, high: int, holds: Callable[[int], bool]) -> int:
    """The first frame from ``low`` to the one before ``high`` that ``holds`` for.

    ``holds`` holds for every frame after one that it holds for; ``high`` where it holds for
    none of them.
    """
    while low < high:
        middle = (low + high) // 2
        if holds(middle):
            high = middle
        else:
            low = middle + 1
    return low


@dataclass(frozen=True)
class Course:
    """The course of a clip with speed points through its source, counted out for reading.

    A place of the media is given in ticks of its scale, counted from where the clip begins in
    its media. The rate is that of the timeline.

    Attributes:
        segments: The segments, in the order of their frames: one at least.
        unit: How many units of a place make a frame: so many make a speed of 1.
        highest: The highest place of the path within the clip, above the lowest.
        length: How many frames the clip lasts.
    """

    segments: tuple[Segment, ...]
    unit: int
    highest: int
    length: int

    @classmethod
    def of(cls, points: Sequence[tuple[int, int, int]], length: int) -> "Course":
        """The course of a clip of ``length`` frames with ``points``.

        Each point is the frame of a speed point and the ``num`` and the ``den`` of its speed.

        Raises:
            ValueError: They are not the speed points of a clip of that length, as
                ``Retime::check`` of the engine says: there is none, or more than
                :data:`MOST_POINTS`; the first is not at frame 0; two are not in the order of
                their frames, each after the one before; one is at the end of the clip or after
                it; a speed has a ``den`` below 1, or plays and is not from 1/100 to 100 either
                way; or the speeds have no unit of 32 bits.
        """
        if (
            not points
            or len(points) > MOST_POINTS
            or points[0][0] != 0
            or points[-1][0] >= length
            or any(one[0] >= other[0] for one, other in pairwise(points))
        ):
            raise ValueError("the speed points are not those of a clip of that length")
        for _, num, den in points:
            plays = abs(num) * 100 >= den and abs(num) <= den * 100
            if den < 1 or not (num == 0 or plays):
                raise ValueError(f"{num}/{den} is no speed of a segment")
        unit = lcm(*(den for _, _, den in points))
        if unit > _LARGEST_UNIT:
            raise ValueError("the speeds have no unit of 32 bits")
        made: list[Segment] = []
        at = lowest = highest = 0
        for index, (frame, num, den) in enumerate(points):
            end = points[index + 1][0] if index + 1 < len(points) else length
            step = num * (unit // den)
            made.append(Segment(frame, end, num, den, at, step))
            at += step * (end - frame)
            lowest, highest = min(lowest, at), max(highest, at)
        segments = tuple(replace(one, at=one.at - lowest) for one in made)
        return cls(segments, unit, highest - lowest, length)

    @property
    def one_way(self) -> bool:
        """Whether every segment plays, and all of them the same way: no freeze and no turn."""
        steps = [segment.step for segment in self.segments]
        return all(step > 0 for step in steps) or all(step < 0 for step in steps)

    def segment_of(self, k: int) -> Segment:
        """The segment that frame ``k`` of the clip is in.

        It is the one whose speed point is at the frame, or the last before it. A frame before
        the clip is of the first segment, and one after it of the last.
        """
        after = bisect_right(self.segments, k, key=lambda segment: segment.start)
        return self.segments[max(after - 1, 0)]

    def place(self, k: int) -> int:
        """The place that frame ``k`` shows, in units of a frame, above the lowest of the clip."""
        return self.segment_of(k).shown(k)

    def ticks(self, place: int, rate: Rate, scale: int) -> int:
        """A place of the path in ticks of ``scale``, rounded down, on a timeline of ``rate``."""
        return place * rate.den * scale // (self.unit * rate.num)

    def offset(self, k: int, rate: Rate, scale: int) -> int:
        """The place in its media that frame ``k`` shows, in ticks after where the clip begins.

        For speed points of one speed it is what ``ticks_of_scaled`` of
        :mod:`columbina.video.span` counts for the frame, or, backward, for frame
        ``length - 1 - k``.
        """
        return self.ticks(self.place(k), rate, scale)

    def tick(self, begins: int, k: int, rate: Rate, scale: int) -> int:
        """The place in its media that frame ``k`` of a clip that begins at ``begins`` shows."""
        return begins + self.offset(k, rate, scale)

    def span(self, rate: Rate, scale: int) -> int:
        """The ticks of its media that the clip shows, from the lowest place to the highest."""
        return self.ticks(self.highest, rate, scale)

    def shown(self, before: int, after: int, rate: Rate, scale: int) -> tuple[int, int]:
        """The lowest and the highest place that the frames of the clip show.

        With ``before`` frames more before its first and ``after`` more after its last, as a
        transition shows them: in ticks of ``scale`` after where the clip begins in its media.
        """
        earliest, latest = -max(before, 0), self.length - 1 + max(after, 0)
        last = len(self.segments) - 1
        ticks: list[int] = []
        for index, segment in enumerate(self.segments):
            # A segment goes one way: its first frame and its last show its ends.
            first = earliest if index == 0 else segment.start
            final = latest if index == last else segment.end - 1
            ticks.append(self.ticks(segment.shown(first), rate, scale))
            ticks.append(self.ticks(segment.shown(final), rate, scale))
        return min(ticks), max(ticks)

    def _frame_in(self, segment: Segment, tick: int, rate: Rate, scale: int) -> int | None:
        """The first frame of ``segment`` in which the place ``tick`` is on show, if any."""

        def shows(k: int) -> int:
            return self.ticks(segment.shown(k), rate, scale)

        start, end = segment.start, segment.end
        if segment.step == 0:
            return start if shows(start) == tick else None
        if segment.step > 0:
            # Its frames show higher and higher places, up to where it stops.
            stops = self.ticks(segment.place(end), rate, scale)
            above = _first(start, end, lambda k: shows(k) > tick)
            if above == start:
                return None
            found = shows(above - 1)
            # Where it stops, and above, only a place that a frame shows itself is on show.
            if tick >= stops and found != tick:
                return None
            return _first(start, above, lambda k: shows(k) >= found)
        # Its frames show lower and lower places, down from where it begins: there, and above,
        # only a place that a frame shows itself is on show.
        begins = self.ticks(segment.place(start), rate, scale)
        found = _first(start, end, lambda k: shows(k) <= tick)
        return found if found < end and (tick < begins or shows(found) == tick) else None

    def frame_of(self, tick: int, rate: Rate, scale: int) -> int | None:
        """The first frame of the clip in which the place ``tick`` of its media is on show.

        ``tick`` is in ticks of ``scale`` after where the clip begins in its media. A place is
        on show in the frame that shows it, and in the frame that shows the nearest place below
        it that its segment shows, as a picture is on show until the next one is. ``None`` where
        the clip does not come by the place. Where it comes by a second time, it is the frame of
        the first time.
        """
        for segment in self.segments:
            found = self._frame_in(segment, tick, rate, scale)
            if found is not None:
                return found
        return None

    def _run(
        self, segment: Segment, start: Fraction, end: Fraction | None, rate: Rate, scale: int
    ) -> tuple[int, int]:
        """The frames of ``segment`` that show a place from ``start`` to before ``end``."""

        def shows(k: int) -> int:
            return self.ticks(segment.shown(k), rate, scale)

        if segment.step >= 0:
            # Its frames show higher and higher places, or one place all through a freeze.
            first = _first(segment.start, segment.end, lambda k: shows(k) >= start)
            if end is None:
                return first, segment.end
            return first, _first(first, segment.end, lambda k: shows(k) >= end)
        first = segment.start
        if end is not None:
            first = _first(segment.start, segment.end, lambda k: shows(k) < end)
        return first, _first(first, segment.end, lambda k: shows(k) < start)

    def showing(
        self,
        start: Fraction | int,
        end: Fraction | int | None,
        rate: Rate,
        scale: int,
        *,
        heard: bool = False,
    ) -> list[tuple[int, int]]:
        """The runs of frames of the clip that show its media from ``start`` to ``end``.

        ``start`` and ``end`` are ticks of ``scale`` after where the clip begins in its media,
        and need not be whole; ``end`` may be ``None`` for no end. A frame is of a run where the
        place that it shows is at ``start`` or after it, and before ``end``. A run is its first
        frame and the frame after its last; the runs are in the order of their frames, and two
        that meet are one. A clip that turns and comes by the stretch again has a run for each
        time. With ``heard``, the frames of a freeze are left out, since a freeze has no sound.
        """
        low = Fraction(start)
        high = None if end is None else Fraction(end)
        runs: list[tuple[int, int]] = []
        for segment in self.segments:
            if heard and segment.step == 0:
                continue
            first, after = self._run(segment, low, high, rate, scale)
            if after <= first:
                continue
            if runs and runs[-1][1] == first:
                runs[-1] = (runs[-1][0], after)
            else:
                runs.append((first, after))
        return runs

    def _lowest_frame(self) -> int:
        """The frame at which the path is lowest: where a segment begins, or the clip ends."""
        return next((one.start for one in self.segments if one.at == 0), self.length)

    def sound_starts(self, start: int, rate: Rate, sample_rate: int) -> list[int]:
        """The first sample of the timeline of each segment but the first.

        For a clip that begins at frame ``start`` of a timeline of ``rate`` frames and
        ``sample_rate`` samples a second, as ``Course::sounding`` of the engine counts them: the
        first sample that begins at the speed point or after it.
        """
        frame = self._lowest_frame()
        per = sample_rate * rate.den
        origin = (start + frame) * per // rate.num
        return [origin - (frame - one.start) * per // rate.num for one in self.segments[1:]]

    def seconds_at(
        self, sample: Fraction | int, start: int, rate: Rate, sample_rate: int
    ) -> Fraction:
        """The time of the source that ``sample`` of the timeline reads, exactly.

        In seconds after where the clip begins in its media, for a clip that begins at frame
        ``start`` of a timeline of ``rate`` frames and ``sample_rate`` samples a second, as
        ``Sounding::seconds`` of the engine counts it: it rises through a segment that goes
        forward, stands still through a freeze, and falls through one that goes backward. The
        keys of the gain and of the pan of a clip are read there. A sample before the clip is
        of its first segment, and one after it of its last.
        """
        frame = self._lowest_frame()
        origin = (start + frame) * sample_rate * rate.den // rate.num
        starts = self.sound_starts(start, rate, sample_rate)
        segment = self.segments[sum(1 for one in starts if one <= sample)]
        # A sample of a segment that goes backward reads the place where the sample ends.
        along = Fraction(sample) + (1 if segment.step < 0 else 0) - origin
        reads = segment.place(frame) * sample_rate * rate.den + segment.step * along * rate.num
        return reads / (sample_rate * self.unit * rate.num)


def course_of(clip: Mapping[str, Any]) -> Course | None:
    """The course of a clip as the engine writes it; ``None`` for a clip of one speed.

    ``None`` too for speed points that are not those of the clip, which no operation of the
    engine leaves so: such a clip is read at its speed, as ``Reading::of`` of the engine's
    ``span.rs`` reads it. A speed that a speed point leaves out is that of the source.
    """
    retime: Any = clip.get("retime")
    if retime is None:
        return None
    try:
        points: list[tuple[int, int, int]] = []
        for point in retime["points"]:
            speed: Any = point.get("speed") or {"num": 1, "den": 1}
            points.append((int(point["frame"]), int(speed["num"]), int(speed["den"])))
        return Course.of(points, int(clip["length"]))
    except (KeyError, TypeError, ValueError, AttributeError):
        return None
