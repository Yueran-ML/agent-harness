"""Cutting by the words: what is struck out of the story of a track, as ranges of frames."""

import time
from dataclasses import dataclass
from fractions import Fraction
from itertools import pairwise
from pathlib import Path
from typing import Any

import pytest
from hypothesis import given, reject, settings
from hypothesis import strategies as st

from columbina.video.edits import Rate, Speed, Time
from columbina.video.engine import Engine
from columbina.video.span import (
    Placed,
    first_frame_at,
    frames_showing,
    placed,
    source_span,
    source_tick,
    ticks_of_scaled,
)
from columbina.video.story import (
    Part,
    StaleStoryError,
    StoryError,
    line_of,
    normal,
    of_track,
    telling,
)
from columbina.video.textcut import Range, cuts, edits_of, merged
from columbina.video.transcribe import transcript_of
from columbina.video.transcript import Transcript, Word
from support import fake_engine
from support.fake_engine import served
from support.media import made, real_engine
from support.studio import SCALE, clip_of, timeline_of, track_of

PAL = Rate(num=25, den=1)
DIGEST = "d" * 64


def word(start: float, end: float, text: str) -> Word:
    return Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)


def talk() -> Transcript:
    return transcript_of(
        "m1",
        DIGEST,
        [
            word(0.2, 0.5, "Hello,"),
            word(0.6, 0.8, "uh"),
            word(0.9, 1.3, "today"),
            word(1.35, 1.6, "we"),
            word(1.65, 2.0, "cut."),
            word(3.2, 3.5, "This"),
            word(3.55, 3.8, "tool"),
            word(3.85, 4.6, "works."),
        ],
        [],
        5_000_000,
    )


def spoken(*clips: dict[str, Any], locked: bool = False) -> dict[str, Any]:
    return timeline_of(track_of("A1", list(clips), locked=locked))


def struck(story: str, *changes: tuple[str, str]) -> str:
    for before, after in changes:
        assert before in story
        story = story.replace(before, after)
    return story


async def test_a_blade_split_word_is_removed_and_restored_by_the_real_engine(
    tmp_path: Path,
) -> None:
    source = made(
        tmp_path / "raw",
        "talk.wav",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=2",
        "-c:a",
        "pcm_s16le",
    )
    async with await Engine.attach(
        tmp_path / "film", program=real_engine(), idle_seconds=1
    ) as engine:
        _, ids = await engine.imported([source])
        _, project = await engine.project()
        scale = project["media"][0]["facts"]["duration"]["scale"]
        timeline = spoken(
            clip_of("$first", 0, 30, media=ids[0], scale=scale),
            clip_of("$second", 30, 20, media=ids[0], ticks=scale * 6 // 5, scale=scale),
        )
        timeline["id"] = "$timeline"
        timeline["tracks"][0]["id"] = "$audio"
        names = (await engine.apply([{"op": "add_timeline", "timeline": timeline}])).names
        track = names["$audio"]
        timeline_id = names["$timeline"]
        revision, before = await engine.project()
        transcript = transcript_of(
            ids[0],
            DIGEST,
            [word(0.2, 0.5, "Before"), word(0.6, 1.4, "filler"), word(1.5, 1.8, "after.")],
            [],
            2_000_000,
        )
        words = {ids[0]: transcript}
        story = of_track(before["timelines"][0], track, words, revision)
        cut = cuts(story.replace("filler", "~~filler~~"), before["timelines"][0], words)
        assert cut == [Range(15, 35)]
        await engine.apply(edits_of(timeline_id, cut))
        assert await engine.duration(timeline_id) == 30
        revision, after = await engine.project()
        rewritten = of_track(after["timelines"][0], track, words, revision)
        assert "filler" not in rewritten
        assert "Before" in rewritten
        assert "after." in rewritten
        await engine.undo()
        _, undone = await engine.project()
        assert undone["timelines"] == before["timelines"]


# Where a clip reads its media, as span.rs counts it


def test_where_a_clip_reads_its_media_is_counted_as_the_engine_counts_it() -> None:
    def clip(ticks: int, length: int, speed: Speed, reverse: bool, scale: int = 15360) -> Placed:
        return Placed("c1", "m1", 0, length, Time(ticks=ticks, scale=scale), speed, reverse)

    fast = clip(0, 100, Speed(num=2, den=1), False)
    slow = clip(0, 4, Speed(num=1, den=2), False, scale=25)
    backward = clip(1000, 100, Speed(num=1, den=1), True)

    # The numbers of the tests of span.rs.
    assert source_span(fast, PAL) == 8 * 15360
    assert (source_tick(fast, 1, PAL), source_tick(fast, 99, PAL)) == (1228, 121_651)
    assert [source_tick(slow, k, PAL) for k in range(4)] == [0, 0, 1, 1]
    assert source_tick(backward, 0, PAL) == 1000 + 60825
    assert source_tick(backward, 99, PAL) == 1000
    assert source_tick(backward, 100, PAL) == 1000 - 615
    assert ticks_of_scaled(-1, Speed(num=1, den=1), PAL, 25) == -1


def test_the_frames_that_show_a_stretch_of_media_are_found_either_way_round() -> None:
    forward = placed(clip_of("c1", 10, 50, ticks=512 * 5))
    backward = placed(clip_of("c1", 10, 50, ticks=512 * 5, reverse=True))
    halved = placed(clip_of("c1", 10, 50, speed=Fraction(1, 2)))
    assert forward is not None
    assert backward is not None
    assert halved is not None

    # From 0.3 s to 0.5 s of the media; the clip begins 0.2 s in.
    assert frames_showing(forward, 300_000, 500_000, PAL) == (3, 8)
    assert frames_showing(backward, 300_000, 500_000, PAL) == (42, 47)
    assert frames_showing(halved, 300_000, 500_000, PAL) == (15, 25)
    assert frames_showing(forward, 0, 100_000, PAL) == (0, 0)
    assert frames_showing(forward, 9_000_000, 9_500_000, PAL) == (50, 50)
    assert first_frame_at(forward, Fraction(512 * 5 + 1), PAL) == 1
    assert (
        placed({**clip_of("c9", 0, 1), "source": {"kind": "color", "color": "#000000ff"}}) is None
    )
    with pytest.raises(ValueError, match="no clip of the engine"):
        placed({"id": "c1"})


# The story of a track


def test_the_story_of_a_track_says_the_words_of_its_clips_and_their_pauses() -> None:
    timeline = spoken(clip_of("c2", 0, 125))

    story = of_track(timeline, "A1", {"m1": talk()}, 7)

    head, blank, *lines = story.splitlines()
    assert (
        head == f"<!-- columbina story: t1 7 {telling(timeline, 'A1', {'m1': talk()}).digest} -->"
    )
    assert blank == ""
    assert lines == [
        "[c2 00:00] Hello, uh today we cut.",
        "[pause 1.2s]",
        "[c2 00:03] This tool works.",
    ]


def test_words_struck_out_in_the_middle_of_a_sentence_cut_the_frames_that_show_them() -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    story = of_track(timeline, "A1", {"m1": talk()}, 7)

    middle = cuts(struck(story, ("uh today", "~~uh today~~")), timeline, {"m1": talk()})
    taken = cuts(struck(story, ("Hello, uh today", "Hello,")), timeline, {"m1": talk()})
    first = cuts(
        struck(story, ("This tool works", "~~This tool~~ works")), timeline, {"m1": talk()}
    )

    # "uh" begins at 0.60 s, and "today" ends at 1.30 s, closer to the words about them than
    # 120 ms: frames 15 to 32 show 0.60 s to 1.28 s.
    assert middle == taken == [Range(15, 33)]
    # 120 ms of silence are left after "cut.", which ends at 2.00 s, and "tool" ends at 3.80 s.
    assert first == [Range(53, 95)]


def test_a_clip_twice_as_fast_cuts_half_as_many_frames() -> None:
    timeline = spoken(clip_of("c2", 0, 63, speed=2))
    story = of_track(timeline, "A1", {"m1": talk()}, 7)

    found = cuts(struck(story, ("uh today", "~~uh today~~")), timeline, {"m1": talk()})

    assert found == [Range(8, 17)]
    assert found[0].frames * 2 == Range(15, 33).frames


def test_a_line_taken_away_cuts_its_sentence_to_the_end_of_its_clip() -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    story = of_track(timeline, "A1", {"m1": talk()}, 7)

    found = cuts(struck(story, ("[c2 00:03] This tool works.\n", "")), timeline, {"m1": talk()})
    first = cuts(
        struck(story, ("[c2 00:00] Hello, uh today we cut.\n[pause 1.2s]\n", "")),
        timeline,
        {"m1": talk()},
    )

    assert found == [Range(53, 125)]
    # From the start of the clip to 120 ms before "This".
    assert first == [Range(0, 77)]


def test_pauses_are_shortened_in_their_middle() -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    story = of_track(timeline, "A1", {"m1": talk()}, 7)
    words = {"m1": talk()}

    named = cuts(struck(story, ("[pause 1.2s]", "[pause 0.4s]")), timeline, words)
    longer = cuts(struck(story, ("[pause 1.2s]", "[pause 3s]")), timeline, words)
    taken = cuts(struck(story, ("[pause 1.2s]\n", "")), timeline, words)
    most = cuts(story, timeline, words, pause_most_ms=500)
    as_written = cuts(story, timeline, words)

    # "cut." ends with frame 49 and "This" begins with frame 80: 30 frames, of which 10 stay.
    assert named == [Range(55, 75)]
    assert longer == []
    # 300 ms are 7.5 frames, 8 once rounded.
    assert taken == [Range(54, 76)]
    assert most == [Range(56, 74)]
    assert as_written == []


def test_the_cuts_of_a_story_are_one_transaction_from_the_last_to_the_first() -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    story = struck(
        of_track(timeline, "A1", {"m1": talk()}, 7),
        ("uh today", "~~uh today~~"),
        ("This tool", "~~This tool~~"),
    )

    found = cuts(story, timeline, {"m1": talk()})

    assert edits_of("t1", found) == [
        {"edit": "ripple_delete_range", "timeline": "t1", "from": 53, "to": 95},
        {"edit": "ripple_delete_range", "timeline": "t1", "from": 15, "to": 33},
    ]
    assert merged([Range(5, 9), Range(1, 3), Range(3, 4), Range(8, 12), Range(7, 7)]) == [
        Range(1, 4),
        Range(5, 12),
    ]


def test_a_story_of_a_timeline_that_changed_since_is_refused() -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    story = of_track(timeline, "A1", {"m1": talk()}, 7)
    corrected = talk().model_copy(update={"shown": {1: "um"}})

    with pytest.raises(StaleStoryError, match="revision 7 of timeline t1"):
        cuts(story, spoken(clip_of("c2", 0, 120)), {"m1": talk()})
    with pytest.raises(StaleStoryError):
        cuts(story, timeline, {"m1": corrected})
    with pytest.raises(StaleStoryError):
        cuts(story.replace(telling(timeline, "A1", {"m1": talk()}).digest, "0" * 12), timeline,
             {"m1": talk()})  # fmt: skip
    # Another revision of the same timeline, as the media was given a proxy meanwhile, is not.
    assert cuts(story.replace(" 7 ", " 9 ", 1), timeline, {"m1": talk()}) == []


def test_a_story_that_is_not_understood_or_of_a_locked_track_cuts_nothing() -> None:
    timeline = spoken(clip_of("c2", 0, 60), clip_of("c3", 60, 65, ticks=60 * 512))
    words = {"m1": talk()}
    story = of_track(timeline, "A1", words, 7)
    first, second = "[c2 00:00] Hello, uh today we cut.", "[c3 00:03] This tool works."
    assert story.splitlines()[2:] == [first, "[pause 1.2s]", second]

    for before, after, said in (
        (first, f"{first} Now.", "its words are not those of a sentence of clip c2 in its place"),
        (first, "[c9 00:00] Hello", "line 3: there is no clip c9 on track A1"),
        (first, f"Hello there\n{first}", "it is neither a line of a clip, a pause nor a note"),
        (second, f"{second}\n{first}", "line 6: clip c2 comes before clip c3 on the track"),
    ):
        with pytest.raises(StoryError, match=said):
            cuts(struck(story, (before, after)), timeline, words)
    with pytest.raises(StoryError, match="the line that names its timeline"):
        cuts(story.split("\n", 1)[1], timeline, words)
    with pytest.raises(StoryError, match="the story is of timeline t2"):
        cuts(struck(story, ("story: t1", "story: t2")), timeline, words)
    locked = spoken(clip_of("c2", 0, 60), clip_of("c3", 60, 65, ticks=60 * 512), locked=True)
    with pytest.raises(StoryError, match="track A1 is locked"):
        cuts(of_track(locked, "A1", words, 7), locked, words)
    # Each clip cuts what it plays: "cut." to the end of c2, and "This" from the start of c3,
    # which meet, and are one range.
    found = cuts(struck(story, ("cut.", "~~cut.~~"), ("This", "~~This~~")), timeline, words)
    assert found == [Range(42, 88)]


def test_a_sentence_said_twice_keeps_the_one_said_later() -> None:
    twice = transcript_of(
        "m1",
        DIGEST,
        [word(0.2, 0.5, "Okay."), word(1.6, 1.9, "Okay."), word(3.0, 3.4, "Done.")],
        [],
        4_000_000,
    )
    timeline = spoken(clip_of("c2", 0, 100))
    story = of_track(timeline, "A1", {"m1": twice}, 1)

    assert story.splitlines()[2:4] == ["[c2 00:00] Okay.", "[pause 1.1s]"]

    found = cuts(struck(story, ("[c2 00:00] Okay.\n", "")), timeline, {"m1": twice})
    paused = cuts(
        struck(story, ("[c2 00:00] Okay.\n[pause 1.1s]", "[pause 0.5s]")), timeline, {"m1": twice}
    )

    # The first "Okay." is cut, from the start of the clip to 120 ms before the second; a pause
    # left before every line is passed over.
    assert found == paused == [Range(0, 37)]


@pytest.mark.parametrize("change", ["times", "text", "sentences"])
def test_transcription_changes_invalidate_a_story_even_when_media_and_word_count_match(
    change: str,
) -> None:
    timeline = spoken(clip_of("c2", 0, 125))
    transcript = talk()
    story = of_track(timeline, "A1", {"m1": transcript}, 7)
    if change == "times":
        transcript = transcript.model_copy(
            update={
                "words": tuple(
                    item.model_copy(
                        update={
                            "start_us": item.start_us + 200_000,
                            "end_us": item.end_us + 200_000,
                        }
                    )
                    for item in transcript.words
                )
            }
        )
    elif change == "text":
        transcript = transcript.model_copy(
            update={
                "words": (
                    transcript.words[0].model_copy(update={"text": "Welcome,"}),
                    *transcript.words[1:],
                )
            }
        )
    else:
        transcript = transcript.model_copy(update={"sentences": ((0, 2), (2, 5), (5, 8))})
    with pytest.raises(StaleStoryError):
        cuts(story, timeline, {"m1": transcript})


@pytest.mark.parametrize("same_second", [False, True])
def test_repeated_sentences_keep_the_occurrence_whose_time_marker_remains(
    same_second: bool,
) -> None:
    second_start, second_end = (0.6, 0.9) if same_second else (1.6, 1.9)
    transcript = transcript_of(
        "m1",
        DIGEST,
        [word(0.2, 0.5, "Okay."), word(second_start, second_end, "Okay."), word(3.0, 3.4, "Done.")],
        [],
        4_000_000,
    )
    timeline = spoken(clip_of("c2", 0, 100))
    story = of_track(timeline, "A1", {"m1": transcript}, 1)
    repeats = [line for line in story.splitlines() if line.endswith("Okay.")]
    assert repeats[0] != repeats[1], "the two occurrences need distinct time markers"
    edited = story.replace(repeats[1] + "\n", "")
    assert cuts(edited, timeline, {"m1": transcript}) == [Range(15 if same_second else 16, 72)]


def test_an_ambiguous_repeated_sentence_without_its_time_marker_is_refused() -> None:
    transcript = transcript_of(
        "m1", DIGEST, [word(0.2, 0.5, "Okay."), word(1.6, 1.9, "Okay.")], [], 2_000_000
    )
    timeline = spoken(clip_of("c2", 0, 50))
    story = of_track(timeline, "A1", {"m1": transcript}, 1).splitlines()[0] + "\n[c2] Okay.\n"
    with pytest.raises(StoryError, match="ambiguous"):
        cuts(story, timeline, {"m1": transcript})


@pytest.mark.parametrize(
    ("speed", "forward", "backward"),
    [(1, (15, 35), (15, 35)), (2, (8, 18), (7, 17)), (Fraction(1, 2), (30, 70), (30, 70))],
)
@pytest.mark.parametrize("reverse", [False, True])
def test_striking_a_word_split_by_a_blade_removes_both_continuous_fragments(
    speed: Fraction | int,
    forward: tuple[int, int],
    backward: tuple[int, int],
    reverse: bool,
) -> None:
    transcript = transcript_of(
        "m1",
        DIGEST,
        [word(0.2, 0.5, "Before"), word(0.6, 1.4, "filler"), word(1.5, 1.8, "after.")],
        [],
        2_000_000,
    )
    left, right = int(30 / speed), int(20 / speed)
    clips = (
        [
            clip_of("c2", 0, right, ticks=30 * 512, speed=speed, reverse=True),
            clip_of("c3", right, left, speed=speed, reverse=True),
        ]
        if reverse
        else [
            clip_of("c2", 0, left, speed=speed),
            clip_of("c3", left, right, ticks=30 * 512, speed=speed),
        ]
    )
    timeline = spoken(*clips)
    story = of_track(timeline, "A1", {"m1": transcript}, 1)
    assert story.count("filler") == 1
    assert cuts(story.replace("filler", "~~filler~~"), timeline, {"m1": transcript}) == [
        Range(*(backward if reverse else forward))
    ]


def test_a_word_cut_through_its_middle_is_told_by_the_clip_that_plays_its_middle() -> None:
    transcript = transcript_of(
        "m1",
        DIGEST,
        [word(0.2, 0.5, "Before"), word(0.6, 1.4, "filler"), word(1.5, 1.8, "after.")],
        [],
        2_000_000,
    )
    # The blade is at 1.0 s, the middle of "filler": each clip plays 10 frames of it.
    timeline = spoken(clip_of("c2", 0, 25), clip_of("c3", 25, 25, ticks=25 * 512))
    story = of_track(timeline, "A1", {"m1": transcript}, 1)

    assert story.splitlines()[2:] == ["[c2 00:00] Before", "[c3 00:00] filler after."]
    assert cuts(story.replace("filler", "~~filler~~"), timeline, {"m1": transcript}) == [
        Range(15, 35)
    ]


def test_a_trimmed_word_fragment_is_still_visible_and_cuttable() -> None:
    transcript = transcript_of(
        "m1", DIGEST, [word(0.6, 1.4, "filler"), word(1.5, 1.8, "after.")], [], 2_000_000
    )
    timeline = spoken(clip_of("c2", 0, 20, ticks=30 * 512))
    story = of_track(timeline, "A1", {"m1": transcript}, 1)
    assert "filler" in story
    assert cuts(story.replace("filler", "~~filler~~"), timeline, {"m1": transcript}) == [
        Range(0, 5)
    ]


def test_words_without_letters_follow_the_nearest_word_of_their_line() -> None:
    transcript = transcript_of(
        "m1",
        DIGEST,
        [
            word(0.2, 0.4, "—"),
            word(0.5, 0.9, "Hello"),
            word(1.0, 1.5, "world."),
            word(3.0, 3.5, "Next."),
            word(3.6, 4.0, "..."),
        ],
        [],
        5_000_000,
    )
    timeline = spoken(clip_of("c1", 0, 125))
    words = {"m1": transcript}
    story = of_track(timeline, "A1", words, 1)
    alone = "[c1 00:03.600] ..."
    assert story.splitlines()[2:] == [
        "[c1 00:00] — Hello world.",
        "[pause 1.5s]",
        "[c1 00:03.000] Next.",
        alone,
    ]

    # A story that is not changed cuts nothing, the dash before the first word and the line
    # of dots alone included.
    assert cuts(story, timeline, words) == []
    # The dash begins its line, and goes with the word after it.
    assert cuts(struck(story, ("Hello", "~~Hello~~")), timeline, words) == [Range(0, 23)]
    assert cuts(struck(story, ("world.", "~~world.~~")), timeline, words) == [Range(25, 72)]
    # A line of dots alone goes when it is struck through or taken away, not otherwise.
    assert cuts(struck(story, (alone, "[c1 00:03.600] ~~...~~")), timeline, words) == [
        Range(90, 125)
    ]
    assert cuts(struck(story, (alone + "\n", "")), timeline, words) == [Range(90, 125)]
    assert cuts(struck(story, (alone, "[c1 00:03.600]")), timeline, words) == [Range(90, 125)]
    assert cuts(struck(story, (alone, "[c1 00:03.600] ..")), timeline, words) == []


def test_cutting_one_repeated_clip_does_not_cut_the_same_word_in_another_take() -> None:
    transcript = transcript_of(
        "m1",
        DIGEST,
        [word(0.2, 0.5, "Before"), word(0.6, 1.4, "filler"), word(1.5, 1.8, "after.")],
        [],
        2_000_000,
    )
    timeline = spoken(clip_of("c2", 0, 50), clip_of("c3", 50, 50))
    story = of_track(timeline, "A1", {"m1": transcript}, 1)
    assert story.count("filler") == 2
    assert cuts(story.replace("filler", "~~filler~~", 1), timeline, {"m1": transcript}) == [
        Range(15, 35)
    ]


async def test_the_cuts_applied_by_an_engine_leave_the_words_that_were_kept(
    tmp_path: Path,
) -> None:
    words = {"m1": talk()}
    project_dir = tmp_path / "film"
    async with (
        served(project_dir),
        await Engine.attach(project_dir, program=fake_engine.__file__) as one,
    ):
        built = spoken(clip_of("c2", 0, 125))
        await one.apply([{"op": "add_timeline", "timeline": built}])
        revision, project = await one.project()
        timeline = project["timelines"][0]
        story = struck(
            of_track(timeline, "A1", words, revision),
            ("uh today", "~~uh today~~"),
            ("This tool", "~~This tool~~"),
        )
        applied = await one.apply(edits_of("t1", cuts(story, timeline, words)))
        revision, project = await one.project()

    clip_ids = [clip["id"] for clip in project["timelines"][0]["tracks"][0]["clips"]]
    assert len(clip_ids) == len(set(clip_ids)), clip_ids
    after = of_track(project["timelines"][0], "A1", words, revision)
    said = [line.split("] ", 1)[1] for line in after.splitlines() if line.startswith("[c")]
    assert said == ["Hello,", "we cut.", "works."]
    assert len([one for one in applied.done if one["op"] == "insert_clip"]) == 2
    with pytest.raises(StaleStoryError):
        cuts(story, project["timelines"][0], words)


# Any way of striking words out


@dataclass(frozen=True)
class Drawn:
    """A track of clips of one media, and a way of changing its story."""

    timeline: dict[str, Any]
    transcript: Transcript
    out: list[bool]
    gone: set[int]
    pauses: list[str]
    most: int | None


@st.composite
def stories(draw: st.DrawFn) -> Drawn:
    """Clips of one media one after the other, each at a speed and in a direction of its own,
    with words of the media inside them; which words are struck out, which lines of words that
    are all struck out are taken away, and what becomes of each pause."""
    clips: list[dict[str, Any]] = []
    words: list[Word] = []
    frame, ticks = 0, 0
    for number in range(draw(st.integers(1, 3))):
        length = draw(st.integers(20, 80))
        speed = draw(st.sampled_from([Fraction(1), Fraction(2), Fraction(1, 2)]))
        backward = draw(st.booleans())
        clips.append(
            clip_of(f"c{number + 2}", frame, length, ticks=ticks, speed=speed, reverse=backward)
        )
        span = int(length * speed * 512)
        begins, ends = ticks * 1_000_000 // SCALE, (ticks + span) * 1_000_000 // SCALE
        times = draw(
            st.lists(
                st.integers(begins + 1000, ends - 1000), min_size=0, max_size=10, unique=True
            ).map(sorted)
        )
        for at in range(0, len(times) - 1, 2):
            # Some words have no letters, as a dash, a note of music or dots, and some no length.
            text = draw(st.sampled_from(["w", "w", "w", "w", "\u2014", "\u266a", "...", "\u2026"]))
            if text == "w":
                text = f"w{len(words)}{'.' if draw(st.booleans()) else ''}"
            end = times[at] if draw(st.integers(0, 5)) == 0 else times[at + 1]
            words.append(Word(start_us=times[at], end_us=end, text=text))
        frame, ticks = frame + length, ticks + span
    transcript = transcript_of("m1", DIGEST, words, [], ticks * 1_000_000 // SCALE)
    return Drawn(
        timeline=timeline_of(track_of("A1", clips)),
        transcript=transcript,
        out=draw(st.lists(st.booleans(), min_size=len(words), max_size=len(words))),
        gone=draw(st.sets(st.integers(0, 40))),
        pauses=draw(
            st.lists(st.sampled_from(["", "drop", "0.1", "0.5", "9"]), min_size=40, max_size=40)
        ),
        most=draw(st.one_of(st.none(), st.integers(40, 1500))),
    )


def _following(part: Part, transcript: Transcript, out: list[bool]) -> dict[int, bool]:
    """Whether each word of a line is cut: a word without letters as the nearest word with them
    in its line, the one before it or else the one after it, and as its line where none is."""
    counted = [one.index for one in part.words if normal(transcript.text(one.index))]
    if not counted:
        gone = all(out[one.index] for one in part.words)
        return {one.index: gone for one in part.words}
    found: dict[int, bool] = {}
    for one in part.words:
        before = [index for index in counted if index <= one.index]
        found[one.index] = out[before[-1] if before else counted[0]]
    return found


@given(stories())
@settings(max_examples=300, deadline=None)
def test_any_striking_cuts_the_frames_of_the_words_struck_out_and_no_other(drawn: Drawn) -> None:
    timeline, transcript = drawn.timeline, drawn.transcript
    words = {"m1": transcript}
    told = telling(timeline, "A1", words)
    story = of_track(timeline, "A1", words, 3)
    lines = iter(told.parts)
    changed: list[str] = []
    for place, line in enumerate(story.splitlines()):
        if line.startswith("[pause"):
            pause = drawn.pauses[place]
            if pause != "drop":
                changed.append(f"[pause {pause}s]" if pause else line)
            continue
        if not line.startswith("[c"):
            changed.append(line)
            continue
        part = next(lines)
        marks = [(transcript.text(one.index), drawn.out[one.index]) for one in part.words]
        if all(cut for _, cut in marks) and place in drawn.gone:
            continue
        changed.append(f"{line[: line.index(']') + 1]} {line_of(marks)}")

    # A story that is not changed cuts nothing.
    assert cuts(story, timeline, words) == []
    try:
        found = cuts("\n".join(changed) + "\n", timeline, words, pause_most_ms=drawn.most)
    except StoryError as error:
        # Two lines of dots alone, at one frame of one clip, are told apart by nothing.
        if "ambiguous" not in str(error):
            raise
        reject()

    clips = timeline["tracks"][0]["clips"]
    first, last = clips[0]["start"], clips[-1]["start"] + clips[-1]["length"]
    assert all(one.start < one.end for one in found)
    assert all(one.end < after.start for one, after in pairwise(found))
    assert all(first <= one.start and one.end <= last for one in found)
    cut = {frame for one in found for frame in range(one.start, one.end)}
    for part in told.parts:
        out = _following(part, transcript, drawn.out)
        for one in part.words:
            shown = set(range(one.first, one.end))
            if out[one.index]:
                assert shown <= cut, (one, found)
            else:
                assert not shown & cut, (one, found)


def test_a_long_talk_in_many_clips_is_told_and_cut_quickly() -> None:
    # 8000 words, a sentence of every twelve, in 200 clips: once a minute or more of work.
    said = [
        Word(
            start_us=at * 300_000 + 10_000,
            end_us=at * 300_000 + 250_000,
            text=f"w{at}{'.' if at % 12 == 11 else ''}",
        )
        for at in range(8000)
    ]
    transcript = transcript_of("m1", DIGEST, said, [], 8000 * 300_000 + 1_000_000)
    each = 8000 * 300_000 * 25 // 1_000_000 // 200
    timeline = spoken(
        *(clip_of(f"c{at + 1}", at * each, each - 1, ticks=at * each * 512) for at in range(200))
    )
    words = {"m1": transcript}

    began = time.perf_counter()
    story = of_track(timeline, "A1", words, 1)
    struck_out = story
    for at in range(5, 8000, 400):
        struck_out = struck_out.replace(f" w{at} ", f" ~~w{at}~~ ", 1)
    found = cuts(struck_out, timeline, words)
    took = time.perf_counter() - began

    assert len(found) == 20
    # About a third of a second on a laptop; the bound is for a slow machine under load.
    assert took < 8, took
