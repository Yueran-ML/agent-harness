"""What a file holds, from what ffprobe answered of it: ``columbina.video.media.facts``."""

import json
from pathlib import Path
from typing import Any, cast

import pytest

from columbina.video.media import (
    AudioFacts,
    Silence,
    Tools,
    UnreadableMediaError,
    VideoFacts,
    facts,
    probe_argv,
    silences_in,
    source,
)

DATA = Path(__file__).parent / "data"


def answer(name: str) -> dict[str, Any]:
    """What ffprobe answered of a real file of a kind."""
    return json.loads((DATA / f"{name}.json").read_text(encoding="utf-8"))


def made(*streams: dict[str, Any], **shape: Any) -> dict[str, Any]:
    """An answer of ffprobe, as a test writes it."""
    return {
        "streams": list(streams),
        "format": {"format_name": "mov,mp4", "duration": "2.0", **shape},
    }


PICTURE = {
    "codec_type": "video",
    "width": 640,
    "height": 360,
    "r_frame_rate": "25/1",
    "avg_frame_rate": "25/1",
}
SOUND = {"codec_type": "audio", "sample_rate": "48000", "channels": 2}


def test_a_film_has_its_picture_and_its_sound() -> None:
    told = facts(answer("landscape"))

    assert (told.kind, told.format, told.duration_us) == (
        "video",
        "mov,mp4,m4a,3gp,3g2,mj2",
        2_000_000,
    )
    assert told.video == VideoFacts(
        width=1920, height=1080, rate="30000/1001", pixel_format="yuv420p"
    )
    assert told.audio == AudioFacts(sample_rate=48000, channels=2)
    assert told.warnings == ()


def test_a_picture_that_is_turned_is_as_wide_as_it_is_shown() -> None:
    told = facts(answer("upright"))

    assert told.video is not None
    assert (told.video.width, told.video.height, told.video.rotation) == (1080, 1920, 90)


def test_frames_at_changing_times_are_taken_at_the_standard_rate_nearest_their_mean() -> None:
    told = facts(answer("changing"))

    assert told.video is not None
    assert (told.video.rate, told.video.variable) == ("24000/1001", True)
    assert told.warnings == ("its frames come at changing times, and are taken at 24000/1001",)


def test_a_picture_of_more_colours_than_are_converted_is_told_of() -> None:
    told = facts(answer("deep"))

    assert told.audio is None
    assert told.warnings == (
        "its picture has more than 8 bits a colour (yuv420p10le), which is not converted",
        "its picture is of a high dynamic range, which is not converted",
    )


def test_a_song_with_a_cover_is_sound_and_a_still_is_a_picture() -> None:
    song, still, voice = facts(answer("song")), facts(answer("still")), facts(answer("voice"))

    assert (song.kind, song.video, song.audio) == (
        "audio",
        None,
        AudioFacts(sample_rate=44100, channels=1),
    )
    assert (still.kind, still.duration_us, still.audio) == ("image", 0, None)
    assert (voice.kind, voice.duration_us) == ("audio", 3_000_000)


@pytest.mark.parametrize(
    ("rotation", "turned"), [(-90, 270), (180, 180), (-180, 180), (270, 270), (89.9, 90), (0, 0)]
)
def test_a_turn_is_one_of_four(rotation: float, turned: int) -> None:
    stream = {**PICTURE, "side_data_list": [{"side_data_type": "other"}, {"rotation": rotation}]}

    told = facts(made(stream))

    assert told.video is not None
    assert told.video.rotation == turned


def test_a_pixel_that_is_not_square_makes_the_picture_wider() -> None:
    told = facts(made({**PICTURE, "sample_aspect_ratio": "2:1"}))

    assert told.video is not None
    assert (told.video.width, told.video.height) == (1280, 360)


def test_a_rate_that_says_nothing_of_a_mean_is_the_rate() -> None:
    told = facts(made({**PICTURE, "avg_frame_rate": "0/0"}))

    assert told.video is not None
    assert (told.video.rate, told.video.variable) == ("25/1", False)


def test_a_mean_alone_is_taken_at_the_nearest_standard_rate() -> None:
    told = facts(made({**PICTURE, "r_frame_rate": "x", "avg_frame_rate": "2997/100"}))

    assert told.video is not None
    assert (told.video.rate, told.video.variable) == ("30000/1001", False)


def test_sound_that_says_too_little_of_itself_is_no_sound() -> None:
    told = facts(made(PICTURE, {**SOUND, "channels": 0}))

    assert told.audio is None


@pytest.mark.parametrize(
    ("answered", "said"),
    [
        (made(PICTURE, format_name="hls"), "its format hls is not taken"),
        ({"streams": [PICTURE]}, "its format that is not known is not taken"),
        (
            made({"codec_type": "subtitle"}, cast("dict[str, Any]", "nothing")),
            "neither picture nor sound",
        ),
        (made({**PICTURE, "width": 0}), "no size of its picture"),
        (
            made({**PICTURE, "r_frame_rate": "0/0", "avg_frame_rate": "1/0"}),
            "no rate of its frames",
        ),
        (made(PICTURE, duration="N/A"), "not how long it lasts"),
        (made(PICTURE, duration="-1"), "not how long it lasts"),
        (made(PICTURE, duration="inf"), "not how long it lasts"),
    ],
)
def test_what_is_no_source_is_refused(answered: dict[str, Any], said: str) -> None:
    with pytest.raises(UnreadableMediaError, match=said):
        facts(answered)


def test_ffprobe_opens_files_alone_of_the_formats_that_are_taken() -> None:
    tools = Tools(ffmpeg="ffmpeg", ffprobe="ffprobe", avformat=62, avfilter=11)

    argv = probe_argv(tools, Path("/work/-odd:name.mp4"))

    assert argv[:3] == ["ffprobe", "-hide_banner", "-loglevel"]
    assert argv[argv.index("-protocol_whitelist") + 1] == "file"
    assert "hls" not in argv[argv.index("-format_whitelist") + 1].split(",")
    assert argv[-1] == source(Path("/work/-odd:name.mp4")) == f"file:{Path('/work/-odd:name.mp4')}"


def test_silences_are_read_as_ffmpeg_writes_them() -> None:
    written = "\n".join(
        [
            "frame:12   pts:12000  pts_time:0.75",
            "lavfi.silence_start=0.75",
            "lavfi.silence_end=1.5",
            "lavfi.silence_duration=0.75",
            "lavfi.silence_end=1.9",
            "lavfi.silence_start=2.25",
            "lavfi.silence_start=oops",
        ]
    )

    assert silences_in(written, 3_000_000) == (
        Silence(750_000, 1_500_000),
        Silence(2_250_000, 3_000_000),
    )
    assert silences_in("lavfi.silence_start=3", 3_000_000) == ()
