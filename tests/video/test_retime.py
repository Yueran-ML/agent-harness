"""Speed points: a clip whose speed changes on the way is read as the engine reads it (T10e).

The numbers are those of the engine's own tests, each named where it is used: of
``engine/crates/model/src/retime.rs`` and ``span.rs``. Those of the readers of a story, of
ducking and of the frames that show a stretch are counted by hand, and say how.
"""

import math
from fractions import Fraction
from typing import Any

import pytest

from columbina.video.duck import speech
from columbina.video.duck_keys import sample_at, source_at
from columbina.video.edits import Rate, Speed, Time
from columbina.video.retime import MOST_POINTS, Course, course_of
from columbina.video.span import (
    Placed,
    first_frame_at,
    frames_showing,
    placed,
    runs_showing,
    source_span,
    source_tick,
)
from columbina.video.story import of_track, telling
from columbina.video.textcut import Range, cuts
from columbina.video.transcript import Transcript, Word
from support.studio import clip_of, timeline_of, track_of

PAL = Rate(num=25, den=1)
NTSC = Rate(num=30000, den=1001)
SCALE = 15360
"""The scale of the media of the engine's tests: 614.4 ticks a frame at 25 frames a second."""

Points = list[tuple[int, int, int]]
"""Speed points, each a frame and the ``num`` and ``den`` of its speed, as ``retime`` of the
tests of ``retime.rs`` takes them."""


def shows(points: Points, length: int) -> list[int]:
    """What each frame of a clip shows, in frames of its source after where it begins.

    On a timeline of 25 frames a second, in ticks of 1/25 of a second: ``shows`` of the tests
    of ``retime.rs``.
    """
    course = Course.of(points, length)
    return [course.offset(k, PAL, 25) for k in range(length)]


def pointed(clip: dict[str, Any], points: Points) -> dict[str, Any]:
    """A clip as the engine writes it, with speed points."""
    written = [{"frame": frame, "speed": {"num": num, "den": den}} for frame, num, den in points]
    return {**clip, "retime": {"points": written}}


# The course, as retime.rs counts it


def test_speed_points_of_one_speed_read_the_media_as_a_clip_of_that_speed_does() -> None:
    # a_mapping_of_one_speed_reads_its_media_as_a_clip_of_that_speed_does (retime.rs) and
    # speed_points_of_one_speed_read_as_that_speed_of_a_clip_does (span.rs).
    speeds = [(1, 1), (2, 1), (1, 2), (24000, 1001), (137, 250), (1, 100), (100, 1)]
    # Stretches of the media, in microseconds after where the clip begins in it.
    stretches = [(0, 100_000), (33_333, 1_234_567), (-50_000, 20_000), (0, 10**9)]
    length = 100
    for num, den in speeds:
        for reverse in (False, True):
            for rate, scale in [(PAL, SCALE), (PAL, 25), (NTSC, 1000), (NTSC, 90000)]:
                begins = Time(ticks=1000, scale=scale)
                plain = Placed("c1", "m1", 0, length, begins, Speed(num=num, den=den), reverse)
                course = Course.of([(0, -num if reverse else num, den)], length)
                along = Placed("c1", "m1", 0, length, begins, course=course)
                what = f"{num}/{den}, backward {reverse}, {rate} in {scale}"
                # The frames of the clip, and those that a transition shows about it.
                for k in range(-5, length + 5):
                    assert course.tick(1000, k, rate, scale) == source_tick(plain, k, rate), what
                    assert source_tick(along, k, rate) == source_tick(plain, k, rate), what
                assert course.span(rate, scale) == source_span(plain, rate), what
                assert source_span(along, rate) == source_span(plain, rate), what
                # The frames that show a stretch of the media are the same ones.
                begins_us = Fraction(1000 * 1_000_000, scale)
                for first, last in stretches:
                    low, high = begins_us + first, begins_us + last
                    assert runs_showing(along, low, high, rate) == runs_showing(
                        plain, low, high, rate
                    ), (what, first, last)


def test_a_frame_shows_the_place_of_the_path_that_the_engine_counts_for_it() -> None:
    # a_frame_on_a_speed_point_is_of_the_segment_that_begins_there
    mapping: Points = [(0, 1, 1), (4, 2, 1), (8, 1, 2)]
    course = Course.of(mapping, 12)
    assert [(course.segment_of(k).num, course.segment_of(k).den) for k in (3, 4, 7, 8, 11)] == [
        (1, 1), (2, 1), (2, 1), (1, 2), (1, 2),
    ]  # fmt: skip
    # Before the clip is of its first segment, and after it of its last.
    assert (course.segment_of(-1).num, course.segment_of(12).den) == (1, 2)
    assert (course.segment_of(4).start, course.segment_of(4).end) == (4, 8)
    assert (len(course.segments), course.length, course.unit) == (3, 12, 2)
    assert shows(mapping, 12) == [0, 1, 2, 3, 4, 6, 8, 10, 12, 12, 13, 13]
    # Places are counted in halves of a frame, as the slowest speed needs.
    assert (course.place(4), course.place(9)) == (8, 25)
    # A frame that a transition shows goes on as the segment at that end does.
    assert (course.offset(-2, PAL, 25), course.offset(13, PAL, 25)) == (-2, 14)
    assert course.shown(0, 0, PAL, 25) == (0, 13)
    assert course.shown(2, 3, PAL, 25) == (-2, 15)
    assert course.span(PAL, 25) == 14

    # a_freeze_holds_a_picture_and_the_clip_plays_on_from_it
    frozen: Points = [(0, 1, 1), (4, 0, 1), (7, 1, 1)]
    assert shows(frozen, 13) == [0, 1, 2, 3, 4, 4, 4, 4, 5, 6, 7, 8, 9]
    assert Course.of(frozen, 13).span(PAL, 25) == 10
    assert shows([(0, -1, 1)], 10) == [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
    assert shows([(0, -1, 1), (5, 0, 1), (8, -1, 1)], 13) == [9, 8, 7, 6, 5, 5, 5, 5, 4, 3, 2, 1, 0]
    assert shows([(0, 1, 1), (4, 0, 1), (6, 2, 1)], 10) == [0, 1, 2, 3, 4, 4, 4, 6, 8, 10]
    assert shows([(0, -1, 1), (5, 1, 1)], 10) == [4, 3, 2, 1, 0, 0, 1, 2, 3, 4]
    assert shows([(0, -1, 1), (5, 0, 1), (7, 1, 1)], 12) == [4, 3, 2, 1, 0, 0, 0, 0, 1, 2, 3, 4]
    # A clip that is a freeze frame from end to end shows where it begins.
    assert shows([(0, 0, 1)], 5) == [0] * 5
    assert Course.of([(0, 0, 1)], 5).span(PAL, SCALE) == 0

    # a_segment_turned_backward_goes_back_over_what_was_shown
    turned = Course.of([(0, 1, 1), (6, -1, 1)], 10)
    assert shows([(0, 1, 1), (6, -1, 1)], 10) == [0, 1, 2, 3, 4, 5, 5, 4, 3, 2]
    assert (turned.span(PAL, 25), turned.shown(0, 0, PAL, 25)) == (6, (0, 5))
    assert shows([(0, 1, 1), (4, -2, 1), (6, 1, 1)], 10) == [0, 1, 2, 3, 2, 0, 0, 1, 2, 3]
    # A clip that goes further back than it came begins in its media at the lowest place that
    # it shows, and its first frame is above it.
    deep = Course.of([(0, 1, 1), (2, -1, 1)], 10)
    assert shows([(0, 1, 1), (2, -1, 1)], 10) == [6, 7, 7, 6, 5, 4, 3, 2, 1, 0]
    assert (deep.span(PAL, 25), deep.shown(0, 0, PAL, 25)) == (8, (0, 7))

    # a_speed_point_that_is_cleared_leaves_a_straight_line_between_its_neighbours
    assert shows([(0, 3, 2), (8, 1, 2)], 12) == [0, 1, 3, 4, 6, 7, 9, 10, 12, 12, 13, 13]
    assert shows([(0, 1, 1), (4, 5, 4)], 12) == [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12]
    assert shows([(0, 1, 2), (8, 1, 1)], 12) == [0, 0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 7]

    # a_speed_point_added_where_the_speed_does_not_change_moves_no_frame: at 29.97 frames a
    # second in thousandths of a second no frame lasts a whole number of ticks.
    same = Course.of([(0, 1, 1), (1, 1, 1), (2, 1, 1), (3, 1, 1)], 10)
    assert [same.offset(k, NTSC, 1000) for k in range(5)] == [0, 33, 66, 100, 133]
    mapping = [(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)]
    whole = Course.of(mapping, 60)
    for at in range(1, 60):
        segment = whole.segment_of(at)
        if segment.start == at:
            continue
        more = sorted([*mapping, (at, segment.num, segment.den)])
        split = Course.of(more, 60)
        for k in range(-3, 63):
            assert split.offset(k, NTSC, 1000) == whole.offset(k, NTSC, 1000), (at, k)
        assert split.span(NTSC, 1000) == whole.span(NTSC, 1000)


def test_a_place_is_found_in_the_first_frame_that_shows_it() -> None:
    # a_place_is_found_in_the_first_frame_that_shows_it
    def find(points: Points, length: int, tick: int) -> int | None:
        return Course.of(points, length).frame_of(tick, PAL, 25)

    # Twice as fast: every other place is shown, and one between is of the frame before it.
    assert [find([(0, 2, 1)], 10, tick) for tick in range(20)] == [tick // 2 for tick in range(20)]
    assert (find([(0, 2, 1)], 10, 20), find([(0, 2, 1)], 10, -1)) == (None, None)
    # Half as fast: each place is shown twice, and found in the first of the two frames.
    assert [find([(0, 1, 2)], 10, tick) for tick in range(6)] == [0, 2, 4, 6, 8, None]
    # Backward: each place is shown once, and found.
    assert [find([(0, -1, 1)], 10, tick) for tick in range(11)] == [*range(9, -1, -1), None]
    # A freeze: the first frame that holds the picture.
    frozen: Points = [(0, 1, 1), (4, 0, 1), (7, 1, 1)]
    assert [find(frozen, 13, tick) for tick in (3, 4, 5, 9, 10)] == [3, 4, 8, 12, None]
    # A freeze at the highest place holds the place itself.
    held: Points = [(0, 1, 1), (5, 0, 1)]
    assert (find(held, 8, 5), find(held, 8, 6)) == (5, None)
    # Forward and back over it: the way up comes first.
    turned: Points = [(0, 1, 1), (6, -1, 1)]
    assert [find(turned, 10, tick) for tick in (3, 5, 6)] == [3, 5, None]
    # Back first and then forward: the way down comes first.
    returning: Points = [(0, -1, 1), (5, 1, 1)]
    assert (find(returning, 10, 4), find(returning, 10, 0)) == (0, 4)


def test_a_frame_and_the_place_that_it_shows_lead_back_to_each_other() -> None:
    # a_frame_and_the_place_that_it_shows_lead_back_to_each_other: each with its length, and
    # whether it goes one way from end to end.
    mappings: list[tuple[Points, int, bool]] = [
        ([(0, 1, 1)], 40, True),
        ([(0, 137, 250)], 40, True),
        ([(0, -3, 1)], 40, True),
        ([(0, 2, 1), (9, 0, 1), (15, 1, 3)], 40, True),
        ([(0, -1, 2), (11, -5, 3), (30, 0, 1)], 40, True),
        ([(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)], 60, False),
    ]
    for points, length, one_way in mappings:
        for rate, scale in [(PAL, 25), (PAL, SCALE), (NTSC, 1000)]:
            course = Course.of(points, length)
            ticks = [course.offset(k, rate, scale) for k in range(length)]
            # From a frame: its place is on show in it, and in no frame after it first.
            for k, tick in enumerate(ticks):
                found = course.frame_of(tick, rate, scale)
                assert found is not None, (points, k)
                assert found <= k
                assert ticks[found] <= tick
                if one_way:
                    # The place leads back to the first frame that shows it.
                    assert found == ticks.index(tick), (points, k)
            # From a place: every place below the highest is on show in some frame, which
            # shows the place or one below it.
            span = course.span(rate, scale)
            lowest, highest = course.shown(0, 0, rate, scale)
            assert lowest == 0
            assert highest <= span
            for tick in range(0, span, max(7, span // 300)):
                found = course.frame_of(tick, rate, scale)
                assert found is not None, (points, tick)
                assert 0 <= found < length
                assert ticks[found] <= tick
            assert course.frame_of(-1, rate, scale) is None
            assert course.frame_of(span + 1, rate, scale) is None


def test_speed_points_that_are_not_those_of_the_clip_are_refused() -> None:
    # a_mapping_that_is_not_one_of_the_clip_is_refused_and_told_why
    wrong: list[tuple[Points, int]] = [
        # No speed point, or none at the first frame.
        ([], 10),
        ([(1, 1, 1)], 10),
        ([(-1, 1, 1), (0, 1, 1)], 10),
        # Two at one frame, or one before the one before it.
        ([(0, 1, 1), (4, 2, 1), (4, 1, 1)], 10),
        ([(0, 1, 1), (6, 2, 1), (4, 1, 1)], 10),
        # One at the end of the clip, or after it; and a clip of no frame has none.
        ([(0, 1, 1), (10, 2, 1)], 10),
        ([(0, 1, 1), (11, 2, 1)], 10),
        ([(0, 1, 1)], 0),
        ([(0, 1, 1)], -5),
        # Too many of them.
        ([(frame, 1, 1) for frame in range(MOST_POINTS + 1)], 5000),
        # Speeds that no unit of 32 bits counts: 65537 and 65539 share no factor.
        ([(0, 65537, 65537), (4, 65539, 65539)], 10),
    ]
    # A speed that is none, or too slow or too fast either way.
    wrong += [
        ([(0, 1, 1), (4, num, den)], 10)
        for num, den in [(1, 0), (0, 0), (1, 101), (101, 1), (-1, 101), (-101, 1)]
    ]
    for points, length in wrong:
        with pytest.raises(ValueError, match="speed"):
            Course.of(points, length)

    Course.of([(0, 1, 1), (9, 2, 1)], 10)
    Course.of([(0, 1, 1)], 1)
    Course.of([(frame, 1, 1) for frame in range(MOST_POINTS)], 5000)
    for num, den in [(1, 100), (100, 1), (-1, 100), (-100, 1), (0, 1), (0, 7), (6, 4)]:
        Course.of([(0, 1, 1), (4, num, den)], 10)
    assert Course.of([(0, 65537, 65537), (4, 65535, 65535)], 10).unit == 65537 * 65535


def test_the_course_of_a_clip_is_read_from_the_clip_that_the_engine_writes() -> None:
    plain = clip_of("c1", 0, 10, ticks=100, scale=25)
    assert course_of(plain) is None
    course = course_of(pointed(plain, [(0, 1, 1), (4, 0, 1), (7, -1, 2)]))
    assert course == Course.of([(0, 1, 1), (4, 0, 1), (7, -1, 2)], 10)
    assert course is not None
    assert not course.one_way
    assert Course.of([(0, 1, 2), (4, 3, 1)], 10).one_way
    assert Course.of([(0, -1, 2), (4, -3, 1)], 10).one_way
    assert not Course.of([(0, 1, 1), (4, -1, 1)], 10).one_way
    # A speed that a speed point leaves out is that of the source.
    left_out = {**plain, "retime": {"points": [{"frame": 0}, {"frame": 5, "speed": None}]}}
    assert course_of(left_out) == Course.of([(0, 1, 1), (5, 1, 1)], 10)
    # speed_points_that_are_not_those_of_the_clip_show_nothing (span.rs): one at frame 20 of a
    # clip of 10 frames, which no operation leaves, and the clip is read at its speed.
    lost = placed(pointed(plain, [(0, 1, 1), (20, 2, 1)]))
    assert lost is not None
    assert lost.course is None
    assert source_tick(lost, 3, PAL) == 103
    assert course_of({**plain, "retime": {"points": "none"}}) is None


# The readers, on a clip with a forward segment, a freeze and a backward segment


def test_a_clip_with_speed_points_reads_its_media_along_their_path() -> None:
    # a_clip_with_speed_points_reads_its_media_along_their_path (span.rs): 4 frames forward, a
    # picture held for 3, and 3 frames back at half the speed, from tick 100 of 25 a second.
    points: Points = [(0, 1, 1), (4, 0, 1), (7, -1, 2)]
    clip = placed(pointed(clip_of("c1", 0, 10, ticks=100, scale=25), points))
    assert clip is not None
    assert (clip.speed, clip.reverse) == (Speed(num=1, den=1), False)

    ticks = [source_tick(clip, k, PAL) for k in range(10)]
    assert ticks == [100, 101, 102, 103, 104, 104, 104, 103, 103, 102]
    # The frames that a transition shows go on as the first segment and the last do.
    assert (source_tick(clip, -1, PAL), source_tick(clip, 10, PAL)) == (99, 102)
    # It shows 4 frames of its media, from where it begins in it.
    assert source_span(clip, PAL) == 4


def talking() -> dict[str, Any]:
    """Clip c3 of 100 frames, at 25 frames a second, from the start of m1 (512 ticks a frame).

    60 frames forward, to 2.4 s of the media; that picture held for 10 frames; and 30 frames
    back at the speed of the media, down to 1.2 s. So its frame ``k`` shows the place of
    ``k`` frames up to frame 59, the place of 60 frames from 60 to 69, and the place of
    ``129 - k`` frames from frame 70 on, where a frame shows the place at which it ends.
    """
    return pointed(clip_of("c3", 0, 100), [(0, 1, 1), (60, 0, 1), (70, -1, 1)])


def said() -> Transcript:
    """What m1 says: two sentences that the clip plays, and one after all that it shows."""
    words = [
        (0.2, 0.5, "Hello,"),
        (0.6, 0.8, "uh"),
        (0.9, 1.3, "today"),
        (1.35, 1.6, "we"),
        (1.65, 2.0, "cut."),
        (2.2, 2.6, "now."),
        (3.2, 3.5, "later."),
    ]
    return Transcript(
        asset="m1",
        digest="d" * 64,
        words=tuple(
            Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)
            for start, end, text in words
        ),
        sentences=((0, 5), (5, 6), (6, 7)),
    )


def test_the_frames_that_show_a_stretch_of_media_are_found_along_the_speed_points() -> None:
    clip = placed(talking())
    assert clip is not None
    assert source_span(clip, PAL) == 60 * 512

    # "today", from 0.9 s to 1.3 s: frames 23 to 32 on the way up (22.5 to 32.5 frames of the
    # media), and frames 97 to 99 on the way back (129 - k from 32 down to 30).
    assert runs_showing(clip, 900_000, 1_300_000, PAL) == [(23, 33), (97, 100)]
    assert frames_showing(clip, 900_000, 1_300_000, PAL) == (23, 33)
    # From 2.3 s to 2.5 s: frames 58 and 59 on the way up, the freeze, which holds 2.4 s, and
    # frames 70 and 71 on the way back (59 and 58 frames of the media); a freeze has no sound.
    assert runs_showing(clip, 2_300_000, 2_500_000, PAL) == [(58, 72)]
    assert runs_showing(clip, 2_300_000, 2_500_000, PAL, heard=True) == [(58, 60), (70, 72)]
    # What the clip does not come by is shown in no frame.
    assert runs_showing(clip, 3_200_000, 3_500_000, PAL) == []
    assert frames_showing(clip, 3_200_000, 3_500_000, PAL) == (100, 100)
    assert frames_showing(clip, 0, 100_000, PAL) == (0, 3)
    # The first frame that shows a place or a later one: on the way up, in the freeze, nowhere.
    assert first_frame_at(clip, 30 * 512, PAL) == 30
    assert first_frame_at(clip, Fraction(30 * 512 + 1), PAL) == 31
    assert first_frame_at(clip, 60 * 512, PAL) == 60
    assert first_frame_at(clip, 60 * 512 + 1, PAL) == 100
    # A clip of one speed has the one run of its frames, or none.
    plain = placed(clip_of("c1", 10, 50, ticks=512 * 5))
    assert plain is not None
    assert runs_showing(plain, 300_000, 500_000, PAL) == [(3, 8)]
    assert runs_showing(plain, 0, 100_000, PAL) == []


def test_a_story_tells_a_clip_with_speed_points_along_them_and_cuts_the_first_time() -> None:
    timeline = timeline_of(track_of("A1", [talking()]))
    words = {"m1": said()}

    told = telling(timeline, "A1", words)
    story = of_track(timeline, "A1", words, 3)

    # Each word is on the frames that first show it: those of the way up, and for "now.",
    # from 2.2 s on, the freeze and the first frames of the way back as well (55 to 74).
    assert [
        (part.sentence, [(one.index, one.first, one.end) for one in part.words])
        for part in told.parts
    ] == [
        (0, [(0, 5, 13), (1, 15, 20), (2, 23, 33), (3, 34, 40), (4, 42, 50)]),
        (1, [(5, 55, 75)]),
    ]
    assert story.splitlines()[2:] == ["[c3 00:00] Hello, uh today we cut.", "[c3 00:02] now."]
    # "today" is cut where the clip first plays it; the second time, backward, stays.
    assert cuts(story.replace("today", "~~today~~"), timeline, words) == [Range(23, 33)]


def test_speech_is_where_a_clip_with_speed_points_plays_its_words_and_not_in_a_freeze() -> None:
    track = track_of("A1", [talking()])

    score, spans = speech(track, PAL, {"m1": said()})

    # Six words are played, "later." never. On the way up they lie from frame 5 to 59; the
    # freeze, frames 60 to 69, is silent; on the way back "now." is on frames 70 to 74, "cut."
    # on 80 to 87, "we" on 90 to 95 and "today" on 97 to 99, with pauses of up to 7 frames
    # between them joined.
    assert score == 6 / 100
    assert spans == [(5, 60), (70, 100)]


def test_the_sound_goes_along_the_path_and_stands_still_in_a_freeze() -> None:
    # the_sound_goes_along_the_path_and_stands_still_in_a_freeze: 4 frames forward, a picture
    # held for 3, and 3 frames back at half the speed, at 25 frames a second and 48000
    # samples: 1920 samples to a frame. The clip begins 4 seconds into its media.
    points: Points = [(0, 1, 1), (4, 0, 1), (7, -1, 2)]
    clip = pointed(clip_of("c1", 0, 10, ticks=100, scale=25), points)
    rate = {"num": 25, "den": 1}
    course = Course.of(points, 10)
    assert course.sound_starts(0, PAL, 48000) == [7680, 13440]

    def read(sample: int, media_rate: int = 48000) -> int:
        return math.floor(source_at(clip, Fraction(sample), rate, 48000) * media_rate)

    # Forward, a sample of the media to each of the timeline, and before the clip too.
    assert [read(-3), read(0), read(7679)] == [191_997, 192_000, 199_679]
    # The freeze stands 4 frames into what the clip shows.
    assert [read(7680), read(13439)] == [199_680, 199_680]
    # Backward at half the speed a sample reads where it ends, half a sample below the one
    # before it.
    assert [read(13440), read(13441), read(13442)] == [199_679, 199_679, 199_678]
    # The last sample of the clip, and the last segment goes on after it.
    assert [read(19199), read(19200)] == [196_800, 196_799]
    # At another rate of the media the places are the same.
    assert [read(0, 44100), read(7680, 44100)] == [176_400, 183_456]
    # The time of the source rises, stands still and falls.
    assert [course.seconds_at(sample, 0, PAL, 48000) for sample in (0, 3840, 7680, 13000)] == [
        0, Fraction(8, 100), Fraction(16, 100), Fraction(16, 100),
    ]  # fmt: skip
    assert course.seconds_at(13440, 0, PAL, 48000) == Fraction(16, 100) - Fraction(1, 96000)
    assert course.seconds_at(19199, 0, PAL, 48000) == Fraction(1, 10)
    assert source_at(clip, Fraction(3840), rate, 48000) == 4 + Fraction(8, 100)


def test_the_sound_of_speed_points_of_one_speed_is_read_as_that_of_a_clip_of_that_speed() -> None:
    # the_sound_of_a_mapping_of_one_speed_is_read_as_that_of_a_clip_of_that_speed
    length = 40
    for num, den in [(1, 1), (2, 1), (1, 2), (137, 250), (24000, 1001)]:
        for reverse in (False, True):
            # At 29.97 frames a second no frame begins on a whole sample but every 1001st.
            for rate, scale, start in [(NTSC, 1000, 0), (NTSC, SCALE, 7), (PAL, SCALE, 1001)]:
                whole = clip_of("c1", start, length, ticks=1234, scale=scale)
                speed = {"num": num, "den": den}
                plain = {**whole, "speed": speed, "reverse": reverse}
                along = pointed(whole, [(0, -num if reverse else num, den)])
                told = rate.model_dump()
                for sample_rate in (48000, 44100):
                    first = sample_at(start, told, sample_rate)
                    end = sample_at(start + length, told, sample_rate)
                    # Samples all through the clip, its first and its last, and those before
                    # it and after it that a crossfade plays.
                    for sample in [*range(first - 5, end + 5, 97), first, end - 1, end, end + 4]:
                        at = Fraction(sample)
                        assert source_at(along, at, told, sample_rate) == source_at(
                            plain, at, told, sample_rate
                        ), (num, den, reverse, rate, sample_rate, sample)
