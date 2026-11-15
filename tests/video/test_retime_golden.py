"""Speed points: the readers of a clip's source are held to the numbers of the engine itself.

The cases, and what the engine found for each, are written by the engine
(``engine/crates/model/tests/retime_golden.rs``, which says what each part of a case is). The
tests of ``test_retime.py`` hold the same functions to numbers copied by hand from the tests of
the engine; these are not copied.

The engine counts in 64 bits and gives the largest count there is where one would be too large;
the readers here do not (see :mod:`columbina.video.retime`), so what they count is brought within
64 bits before it is compared (:func:`within`).
"""

import json
import math
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from columbina.video.clip_keys import KeyClock
from columbina.video.duck_keys import sample_at, source_at
from columbina.video.edits import Rate
from columbina.video.retime import Course, course_of
from columbina.video.span import Placed, placed, source_span, source_tick

ENGINE_DATA = Path(__file__).resolve().parents[2] / "engine" / "crates" / "model" / "tests" / "data"
CASES: list[dict[str, Any]] = json.loads((ENGINE_DATA / "retime.json").read_text(encoding="utf-8"))
"""The engine's own cases of speed points: a clip, its media, the rate and the samples a second
of its timeline, and what the engine finds for it."""

each_case = pytest.mark.parametrize("one", CASES, ids=[one["name"] for one in CASES])

HELD = {
    "name", "clip", "media", "rate", "sample_rate", "unit", "segments", "frames", "ticks",
    "span", "shown", "frame_of", "keys", "sound",
}  # fmt: skip
"""The parts of a case that the readers are held to.

Of three of them a part is not held, since no reader counts it: the ``end`` of ``shown``
(``Course::reach`` of the engine, where what a clip shows ends), the ``shown`` of ``keys`` (the
first and the last frame of the media that the clip shows), and of the ``windows`` of ``sound``
the cutting of the sound into pieces (``pieces_along`` of the engine's ``resolve.rs``): of a
piece, the sample of the media that its first sample is read from is held, and of the pieces of
the whole clip the samples that the clip begins and ends at.
"""

NOT_HELD = {"speed_curve", "frame_curve", "from_frame", "with_length"}
"""The parts of a case that no reader counts, and so none is held to.

``speed_curve`` and ``frame_curve`` are the Retime Speed and Retime Frame curves, which an editor
draws and the agent does not. ``from_frame`` and ``with_length`` are what the engine makes of the
speed points of a clip whose head or tail is moved (``Retime::from_frame``,
``Retime::with_length``): the agent sends the trim, and reads the speed points that the engine
answers with.
"""

I64_MAX = 2**63 - 1
I64_MIN = -(2**63)


def within(count: int) -> int:
    """A count as the 64 bits of the engine hold it: the most or the least where it is beyond."""
    return min(max(count, I64_MIN), I64_MAX)


def whole(count: int | str) -> int:
    """A count that the engine wrote: a number, or its digits where a double does not hold it."""
    return int(count)


def rate_of(one: dict[str, Any]) -> Rate:
    return Rate.model_validate(one["rate"])


def scale_of(one: dict[str, Any]) -> int:
    return int(one["media"]["facts"]["duration"]["scale"])


def course(one: dict[str, Any]) -> Course:
    """The course of the clip of a case: along its speed points, or that of its one speed."""
    clip = one["clip"]
    along = course_of(clip)
    assert (along is None) == ("retime" not in clip), one["name"]
    if along is not None:
        return along
    speed = clip["speed"]
    num = -speed["num"] if clip["reverse"] else speed["num"]
    return Course.of([(0, num, speed["den"])], clip["length"])


def read(one: dict[str, Any]) -> Placed:
    """The clip of a case as the readers of a clip's source take it."""
    clip = placed(one["clip"])
    assert clip is not None, one["name"]
    return clip


def test_every_case_is_read_and_every_part_of_it_is_held_or_named_as_not_held() -> None:
    assert len(CASES) >= 20
    assert len({one["name"] for one in CASES}) == len(CASES)
    for one in CASES:
        assert set(one) == HELD | NOT_HELD, one["name"]
    # Clips of one speed, clips with speed points, and counts that a double does not hold.
    assert sum(1 for one in CASES if "retime" not in one["clip"]) >= 6
    assert any(len(one["clip"].get("retime", {}).get("points", [])) >= 12 for one in CASES)
    assert any(isinstance(tick, str) for one in CASES for tick in one["ticks"])


@each_case
def test_the_course_has_the_unit_and_the_segments_that_the_engine_counts(
    one: dict[str, Any],
) -> None:
    along = course(one)

    assert along.unit == one["unit"]
    assert along.length == one["clip"]["length"]
    assert [(part.start, part.end, part.num, part.den) for part in along.segments] == [
        (part["from"], part["to"], part["speed"]["num"], part["speed"]["den"])
        for part in one["segments"]
    ]
    # A clip of one speed has no speed points, and is read at its speed.
    assert (read(one).course is None) == ("retime" not in one["clip"])


@each_case
def test_a_frame_shows_the_tick_that_the_engine_shows(one: dict[str, Any]) -> None:
    along, clip, rate, scale = course(one), read(one), rate_of(one), scale_of(one)
    begins = one["clip"]["source"]["in"]["ticks"]

    # The frames of the clip, and those that a transition reads before it and after it.
    for k, tick in zip(one["frames"], one["ticks"], strict=True):
        assert within(along.tick(begins, k, rate, scale)) == whole(tick), k
        assert within(source_tick(clip, k, rate)) == whole(tick), k


@each_case
def test_a_clip_shows_the_span_and_the_lowest_and_the_highest_tick_that_the_engine_counts(
    one: dict[str, Any],
) -> None:
    along, clip, rate, scale = course(one), read(one), rate_of(one), scale_of(one)

    assert within(along.span(rate, scale)) == whole(one["span"])
    assert within(source_span(clip, rate)) == whole(one["span"])
    # With the frames that a transition shows of the clip before its first and after its last.
    for shown in one["shown"]:
        before, after = shown["before"], shown["after"]
        lowest, highest = along.shown(before, after, rate, scale)
        written = (whole(shown["first"]), whole(shown["last"]))
        assert (within(lowest), within(highest)) == written, (before, after)


@each_case
def test_a_place_is_found_in_the_frame_that_the_engine_finds_it_in(one: dict[str, Any]) -> None:
    along, rate, scale = course(one), rate_of(one), scale_of(one)

    for tick, frame in zip(one["frame_of"]["ticks"], one["frame_of"]["frames"], strict=True):
        # The most that 64 bits hold stands in the engine for every place beyond it, which is
        # shown here as it is: such a place is found in another frame, and is not asked.
        if not I64_MIN < whole(tick) < I64_MAX:
            continue
        assert along.frame_of(whole(tick), rate, scale) == frame, tick


@each_case
def test_the_keys_of_a_clip_are_at_the_frame_that_the_engine_reads_them_at(
    one: dict[str, Any],
) -> None:
    clip, keys = one["clip"], one["keys"]
    clock = KeyClock.of(clip, rate_of(one), one["media"])

    assert clock.counts == "the clip's media"
    assert [clock.at(frame) for frame in keys["frames"]] == keys["at"]
    # A key is shown at the first frame of the clip that is at it, by the engine's own count
    # of the frame of each: asked where the engine wrote every frame of the clip.
    start, length = clip["start"], clip["length"]
    if len(keys["frames"]) != length + 6:
        return
    first: dict[int, int] = {}
    for frame, key in zip(keys["frames"], keys["at"], strict=True):
        if start <= frame < start + length:
            first.setdefault(key, frame)
    for key in range(min(first) - 2, max(first) + 3):
        assert clock.shown(key) == first.get(key), key


@each_case
def test_the_sound_of_each_segment_begins_at_the_sample_that_the_engine_counts(
    one: dict[str, Any],
) -> None:
    clip, parts = one["clip"], one["sound"]["parts"]
    along, rate = course(one), rate_of(one)

    # The first part goes on before the clip and the last after it: neither has a number there.
    assert parts[0]["from"] is None
    assert parts[-1]["to"] is None
    starts = along.sound_starts(clip["start"], rate, one["sample_rate"])
    assert starts == [whole(part["from"]) for part in parts[1:]]
    assert starts == [whole(part["to"]) for part in parts[:-1]]


@each_case
def test_a_sample_of_the_timeline_reads_the_sample_of_the_media_that_the_engine_reads(
    one: dict[str, Any],
) -> None:
    clip, sound = one["clip"], one["sound"]
    along, rate, sample_rate = course(one), rate_of(one), one["sample_rate"]
    media_rate: int = one["media"]["facts"]["audio"]["rate"]
    begins = Fraction(clip["source"]["in"]["ticks"], clip["source"]["in"]["scale"])

    for sample, source in zip(sound["samples"], sound["source_samples"], strict=True):
        at = Fraction(whole(sample))
        # As ducking reads the time of the source: a clip of one speed at its speed.
        time = source_at(clip, at, one["rate"], sample_rate)
        assert within(math.floor(time * media_rate)) == whole(source), sample
        # Along the course, which for one speed reads what the speed reads.
        time = begins + along.seconds_at(at, clip["start"], rate, sample_rate)
        assert within(math.floor(time * media_rate)) == whole(source), sample


@each_case
def test_a_piece_of_sound_is_read_from_the_sample_of_the_media_that_the_engine_reads(
    one: dict[str, Any],
) -> None:
    clip, sound, sample_rate = one["clip"], one["sound"], one["sample_rate"]
    media_rate: int = one["media"]["facts"]["audio"]["rate"]

    for window in sound["windows"]:
        for piece in window["pieces"]:
            time = source_at(clip, Fraction(whole(piece["from"])), one["rate"], sample_rate)
            assert within(math.floor(time * media_rate)) == whole(piece["source_start"]), window
    # The first window is about the whole clip: a clip whose first segment plays is heard from
    # the sample that its first frame starts at, and one whose last plays to that of its end.
    pieces = sound["windows"][0]["pieces"]
    segments = one["segments"]
    if segments[0]["speed"]["num"] != 0:
        assert whole(pieces[0]["from"]) == sample_at(clip["start"], one["rate"], sample_rate)
    if segments[-1]["speed"]["num"] != 0:
        end = clip["start"] + clip["length"]
        assert whole(pieces[-1]["to"]) == sample_at(end, one["rate"], sample_rate)
