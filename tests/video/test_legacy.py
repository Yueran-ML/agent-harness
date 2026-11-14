"""The schema-1 timing plan retained for migration, without a renderer."""

from fractions import Fraction

import pytest

from columbina.video.legacy import Canvas, Cut, Entry, Plan, Timeline, captions, planned, timed
from columbina.video.media import Silence
from columbina.video.transcribe import transcript_of
from columbina.video.transcript import Transcript, Word
from support.legacy import whole

CANVAS = Canvas(width=640, height=360, rate="25/1")


def word(start: float, end: float, text: str) -> Word:
    return Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)


def talk() -> Transcript:
    words = [
        word(0.2, 0.8, "one"), word(0.9, 1.6, "two."),
        word(2.4, 3.0, "three"), word(3.1, 3.6, "four."),
        word(4.4, 5.0, "five"), word(5.1, 5.6, "six."),
    ]  # fmt: skip
    return transcript_of("talk", "d", words, [], 6_000_000)


QUIET = (
    Silence(1_600_000, 2_400_000),
    Silence(3_600_000, 4_400_000),
    Silence(5_600_000, 6_000_000),
)


def plan_of(timeline: Timeline, canvas: Canvas | None = CANVAS) -> Plan:
    return planned(timeline, {"talk": talk()}, {"talk": QUIET}, {"talk": 6_000_000}, canvas)


def whole_line(**more: object) -> Timeline:
    return Timeline(main=whole(talk()), pause_most_us=500_000, **more)  # type: ignore[arg-type]


def test_the_stretches_are_put_on_frames_one_after_the_other() -> None:
    plan = plan_of(whole_line())

    assert plan.rate == 25
    assert [(cut.start_us, cut.end_us) for cut in plan.cuts] == [
        (80_000, 1_850_000), (2_150_000, 3_850_000), (4_150_000, 5_720_000),
    ]  # fmt: skip
    assert plan.lengths == (44, 42, 39)
    assert plan.starts == (0, 44, 86)
    assert (plan.frames, plan.length_us) == (125, 5_000_000)
    assert plan.frame(plan.us(10)) == 10


def test_a_crossfade_takes_its_frames_from_the_film() -> None:
    plan = plan_of(whole_line(crossfades={0: 10, 2: 30}))

    assert plan.fades == (10, 0, 0)
    assert plan.starts == (0, 34, 76)
    assert plan.frames == 115


def test_a_crossfade_is_no_longer_than_either_stretch() -> None:
    plan = plan_of(whole_line(crossfades={0: 500}))

    # The next stretch lasts 42 frames, and is not crossed into whole.
    assert plan.fades[0] == 41


def test_a_word_has_its_time_in_the_film_where_it_is_kept() -> None:
    timeline = Timeline(main=(whole(talk())[0], whole(talk())[2]), pause_most_us=500_000)
    plan = plan_of(timeline)

    assert timed(plan, {"talk": talk()}, ("talk", 0)) == 120_000
    assert (
        timed(plan, {"talk": talk()}, ("talk", 4))
        == plan.us(plan.starts[1]) + 4_400_000 - 4_280_000
    )
    assert (
        timed(plan, {"talk": talk()}, ("talk", 5), end=True)
        == plan.us(plan.starts[1]) + 5_600_000 - 4_280_000
    )
    assert timed(plan, {"talk": talk()}, ("talk", 2)) is None
    assert timed(plan, {"talk": talk()}, ("other", 0)) is None
    assert timed(plan, {"talk": talk()}, ("talk", 99)) is None


def test_captions_are_the_kept_words_a_few_at_a_time() -> None:
    words = [word(index * 0.5, index * 0.5 + 0.4, f"word{index}") for index in range(12)]
    said = transcript_of(
        "talk",
        "d",
        [*words, word(6.0, 6.4, "这是一句很长很长很长很长很长的中文句子。")],
        [],
        8_000_000,
    )
    timeline = Timeline(main=whole(said))
    plan = planned(timeline, {"talk": said}, {"talk": ()}, {"talk": 8_000_000}, CANVAS)

    shown = captions(plan, timeline, {"talk": said})

    assert [cue.text for cue in shown] == [
        "word0 word1 word2 word3 word4 word5 word6",
        "word7 word8 word9 word10 word11",
        "这是一句很长很长很长很长很长的中文句子。",
    ]
    assert shown[0].start_us == timed(plan, {"talk": said}, ("talk", 0))
    assert all(cue.end_us > cue.start_us for cue in shown)


@pytest.mark.parametrize(("rate", "frames"), [("30000/1001", 151), ("24/1", 119)])
def test_a_plan_keeps_the_rate_of_the_film(rate: str, frames: int) -> None:
    plan = plan_of(whole_line(), Canvas(width=640, height=360, rate=rate))

    assert plan.rate == Fraction(*map(int, rate.split("/")))
    assert plan.frames == frames
    assert plan_of(whole_line(), None).rate == 1000


def test_a_cut_of_no_length_lasts_a_frame() -> None:
    plan = Plan(Fraction(25), (Cut("talk", 0, 0, 0),), (0,), (1,), (0,), 1)

    assert plan.length_us == 40_000
    assert (
        planned(
            Timeline(main=(Entry(asset="talk", sentence=0, kept=(0,)),)),
            {"talk": talk()},
            {"talk": ()},
            {"talk": 120_000},
            CANVAS,
        ).lengths[0]
        >= 1
    )
