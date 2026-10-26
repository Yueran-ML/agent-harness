"""What the timeline is compiled into for ffmpeg: ``columbina.video.render``, without ffmpeg."""

from fractions import Fraction
from pathlib import Path

import pytest

from columbina.video.media import AudioFacts, Silence, VideoFacts
from columbina.video.project import Asset, Canvas, Identity, Manifest
from columbina.video.render import Cue, Plan, ass, captions, compiled, planned, srt, timed
from columbina.video.story import whole
from columbina.video.timeline import Cut, Entry, Layer, Timeline, Transcript, Word
from columbina.video.transcribe import transcript_of

CANVAS = Canvas(width=640, height=360, rate="25/1")
SAME = Identity(size=1, modified_ns=1, digest="d")


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


def asset(
    key: str, kind: str = "video", seconds: float = 6.0, *, sound: bool = True, picture: bool = True
) -> Asset:
    return Asset(
        id=key,
        path=f"/sources/{key}",
        kind=kind,  # type: ignore[arg-type]
        format="mov",
        duration_us=round(seconds * 1e6),
        video=VideoFacts(width=1280, height=720, rate="25/1") if picture else None,
        audio=AudioFacts(sample_rate=48000, channels=2) if sound else None,
        identity=SAME,
    )


MANIFEST = Manifest(
    canvas=CANVAS,
    assets=(
        asset("talk"),
        asset("screen", seconds=3.0, sound=False),
        asset("logo", "image", 0.0, sound=False),
        asset("music", "audio", 30.0, picture=False),
    ),
)


def plan_of(timeline: Timeline, canvas: Canvas | None = CANVAS) -> Plan:
    return planned(timeline, {"talk": talk()}, {"talk": QUIET}, {"talk": 6_000_000}, canvas)


def whole_line(**more: object) -> Timeline:
    return Timeline(main=whole(talk()), pause_most_us=500_000, **more)  # type: ignore[arg-type]


# The plan


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


# Captions


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


def test_captions_are_written_for_subrip_and_for_the_drawing_of_subtitles() -> None:
    cues = [
        Cue(1_234_000, 3_600_000_000 + 5_000, "Hi {there}\\you\nnext"),
        Cue(0, 500_000, "Title", "top-left"),
    ]

    assert srt(cues[:1]) == "1\n00:00:01,234 --> 01:00:00,005\nHi {there}\\you\nnext\n"
    styles = ass(cues, CANVAS)
    assert "PlayResX: 640" in styles
    assert "Style: caption," in styles
    assert "Style: top-left," in styles
    assert "Dialogue: 0,0:00:01.23,1:00:00.00,caption,Hi (there)/you\\Nnext" in styles
    assert "Dialogue: 1,0:00:00.00,0:00:00.50,top-left,Title" in styles


# The graph


def graph_of(
    timeline: Timeline, manifest: Manifest = MANIFEST, quality: str = "final"
) -> tuple[list[str], str, list[str], tuple[str, ...], str | None]:
    plan = plan_of(timeline, manifest.canvas)
    made = compiled(plan, timeline, manifest, {"talk": talk()}, quality)  # type: ignore[arg-type]
    return made.inputs, made.graph, made.maps, made.layers, made.styles


def test_stretches_that_go_forward_are_read_from_one_opening_of_their_source() -> None:
    inputs, graph, maps, floating, styles = graph_of(whole_line(captions=False))

    assert inputs.count("-i") == 1
    assert inputs[inputs.index("-i") + 1] == f"file:{Path('/sources/talk')}"
    assert "-protocol_whitelist" in inputs
    # The source is opened where the first stretch begins.
    assert inputs[:2] == ["-ss", "0.080000"]
    assert "[0:v]fps=25/1,trim=start=0.000000:end=1.760000" in graph
    assert "[v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[g0v][g0a]" in graph
    assert maps == ["-map", "[g0v]", "-map", "[g0a]"]
    assert (floating, styles) == ((), None)


def test_a_stretch_that_goes_back_opens_its_source_again_from_there() -> None:
    entries = whole(talk())
    inputs, graph, _, _, _ = graph_of(Timeline(main=(entries[2], entries[0]), captions=False))

    assert inputs.count("-i") == 2
    assert inputs[:2] == ["-ss", "4.280000"]
    assert "[0:a]atrim=start=0.000000:end=1.440000" in graph
    assert "[1:a]atrim=start=0.000000:end=1.640000" in graph


def test_a_film_without_a_frame_is_sound_alone() -> None:
    manifest = Manifest(canvas=None, assets=(asset("talk", "audio", picture=False),))
    _, graph, maps, _, styles = graph_of(whole_line(), manifest)

    assert "concat=n=3:v=0:a=1[g0a]" in graph
    assert "[0:v]" not in graph
    assert (maps, styles) == (["-map", "[g0a]"], None)


def test_a_talk_without_a_picture_is_black_in_a_film() -> None:
    manifest = Manifest(canvas=CANVAS, assets=(asset("talk", "audio", picture=False),))
    _, graph, _, _, _ = graph_of(whole_line(captions=False), manifest)

    assert "color=c=black:s=640x360:r=25/1:d=1.760000,format=yuv420p,setsar=1[v0]" in graph


def test_crossfades_join_the_stretches_around_them() -> None:
    _, graph, maps, _, _ = graph_of(whole_line(captions=False, crossfades={0: 10}))

    assert "[v1][a1][v2][a2]concat=n=2:v=1:a=1[g1v][g1a]" in graph
    assert "[v0][g1v]xfade=transition=fade:duration=0.400000:offset=1.360000[x1v]" in graph
    assert "[a0][g1a]acrossfade=d=0.400000[x1a]" in graph
    assert maps == ["-map", "[x1v]", "-map", "[x1a]"]


def test_pictures_are_laid_over_the_film_and_sounds_under_it() -> None:
    layers = (
        Layer(
            id="screen-over", kind="picture", asset="screen", anchor=("talk", 4), until=("talk", 5),
            position="top-right", scale=0.3, opacity=0.8, fade_us=200_000, speed=2.0,
            source_us=500_000,
        ),
        Layer(id="logo-over", kind="picture", asset="logo", at_frame=0, frames=25),
        Layer(
            id="music-under", kind="sound", asset="music", at_frame=0, volume=0.5,
            fade_us=1_000_000, speed=0.5,
        ),
        Layer(id="hello", kind="title", text="Hello", at_frame=0, frames=50, position="center"),
    )  # fmt: skip
    inputs, graph, maps, floating, styles = graph_of(whole_line(layers=layers))

    assert inputs.count("-i") == 4
    assert "scale=192:-2,setsar=1,format=yuva420p,colorchannelmixer=aa=0.8" in graph
    assert ",setpts=PTS/2," in graph
    assert "fade=t=in:st=" in graph
    assert "loop=loop=-1:size=1:start=0,fps=25/1,trim=start=0:duration=1.000000" in graph
    assert "scale=640:360:force_original_aspect_ratio=increase,crop=640:360" in graph
    assert "overlay=W-w-14:14:eof_action=pass:enable='between(t," in graph
    assert "atempo=0.5,volume=0.5,afade=t=in:st=0:d=1.000000" in graph
    assert "[g0a]asplit=2[voice][key]" in graph
    assert "sidechaincompress" in graph
    assert "[drawn]" in maps[1]
    assert floating == ()
    assert styles is not None
    assert ",center,Hello" in styles
    assert "subtitles=film.ass" in graph


def test_sounds_are_mixed_into_a_bed_and_need_not_duck() -> None:
    layers = (
        Layer(id="one", kind="sound", asset="music", at_frame=0),
        Layer(id="two", kind="sound", asset="talk", at_frame=10, frames=10),
    )
    _, graph, _, _, _ = graph_of(whole_line(layers=layers, duck=False, captions=False))

    assert "[s0][s1]amix=inputs=2:normalize=0[bed]" in graph
    assert "[g0a][bed]amix=inputs=2:duration=first:normalize=0[mixed]" in graph
    assert "sidechaincompress" not in graph


def test_layers_that_lie_nowhere_are_left_out() -> None:
    entries = whole(talk())
    layers = (
        Layer(id="cut-away", kind="picture", asset="screen", anchor=("talk", 2)),
        Layer(id="cut-end", kind="title", text="x", anchor=("talk", 0), until=("talk", 3)),
        Layer(id="gone", kind="picture", asset="nothing", at_frame=0),
        Layer(id="silent", kind="sound", asset="screen", at_frame=0),
        Layer(id="late", kind="title", text="late", at_frame=10_000),
        Layer(id="mute-picture", kind="picture", asset="music", at_frame=0),
    )
    timeline = Timeline(main=(entries[0], entries[2]), layers=layers, captions=False)

    _, graph, _, floating, styles = graph_of(timeline)

    assert set(floating) == {"cut-away", "cut-end", "gone", "silent", "late", "mute-picture"}
    assert styles is None
    assert "overlay" not in graph


def test_a_preview_is_small() -> None:
    _, graph, maps, _, _ = graph_of(whole_line(captions=False), quality="preview")
    manifest = Manifest(canvas=Canvas(width=320, height=180, rate="25/1"), assets=MANIFEST.assets)
    _, small_graph, _, _, _ = graph_of(whole_line(captions=False), manifest, quality="preview")

    assert "[g0v]scale=-2:360[small]" not in graph
    assert maps[1] == "[g0v]"
    assert "scale=-2" not in small_graph
    big = Manifest(canvas=Canvas(width=1920, height=1080, rate="25/1"), assets=MANIFEST.assets)
    assert (
        "[g0v]scale=-2:360[small]"
        in graph_of(whole_line(captions=False), big, quality="preview")[1]
    )


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
