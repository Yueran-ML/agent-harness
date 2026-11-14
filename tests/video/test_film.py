"""Four real tool-to-engine films checked after the Python renderer was retired.

The source is red, green and blue for two seconds each. Colours, audio and subtitle times
are checked in the exported files, independently of the engine's project receipts.
"""

import asyncio
import json
import math
import struct
import subprocess
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.video import VIDEOS, edits
from columbina.video.media import Tools, located, probed
from columbina.video.transcribe import Scripted
from support.desk import Desk
from support.media import made, real, real_engine
from support.studio_tools import desk_of, result

# The studio's own platforms are tested in tests/video/test_platform.py.
pytestmark = pytest.mark.usefixtures("studio_here")

WORDS = [
    (220_000, 820_000, "one"), (920_000, 1_620_000, "two."),
    (2_420_000, 3_020_000, "three"), (3_120_000, 3_620_000, "four."),
    (4_420_000, 5_020_000, "five"), (5_120_000, 5_620_000, "six."),
]  # fmt: skip


def sources(where: Path) -> None:
    voiced = "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in WORDS)
    made(
        where, "talk.mp4",
        "-f", "lavfi", "-i", "color=black:s=160x120:r=25:d=6,format=rgb24,"
        "geq=r='255*lt(T,2)':g='255*between(T,2,4)':b='255*gte(T,4)'",
        "-f", "lavfi", "-i", f"aevalsrc='0.5*sin(440*2*PI*t)*({voiced})':d=6:s=48000",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    made(where, "yellow.png", "-f", "lavfi", "-i", "color=c=yellow:s=64x48:d=1", "-frames:v", "1")
    made(where, "song.wav", "-f", "lavfi", "-i", "sine=f=220:d=10")
    (where / "words.json").write_text(
        json.dumps([{"start_us": a, "end_us": b, "text": t} for a, b, t in WORDS]),
        encoding="utf-8",
    )


def pixels(path: Path, at: float, *, average: bool = True) -> bytes:
    scale = ["-vf", "scale=1:1"] if average else []
    return subprocess.run(
        [real(), "-v", "error", "-ss", str(at), "-i", str(path), "-frames:v", "1", *scale,
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True, timeout=60, check=True,
    ).stdout  # fmt: skip


async def colours(path: Path, *times: float) -> list[str]:
    found: list[str] = []
    for at in times:
        red, green, blue = await asyncio.to_thread(pixels, path, at)
        if red > 150 and green > 150 and blue < 100:
            found.append("yellow")
        else:
            found.append(
                max((("red", red), ("green", green), ("blue", blue)), key=lambda p: p[1])[0]
            )
    return found


def loudness(path: Path, at: float) -> float:
    data = subprocess.run(
        [real(), "-v", "error", "-ss", str(at), "-i", str(path), "-t", "0.08", "-vn",
         "-ac", "1", "-ar", "48000", "-f", "f32le", "-"],
        capture_output=True, timeout=60, check=True,
    ).stdout  # fmt: skip
    samples = [one[0] for one in struct.iter_unpack("<f", data)]
    assert len(samples) >= 3_000
    return math.sqrt(sum(one * one for one in samples) / len(samples))


@pytest.fixture
async def film(ctx: Context, tmp_path: Path) -> tuple[Desk, dict[str, str], list[str], Tools]:
    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    await asyncio.to_thread(sources, desk.work)
    desk.ctx.use(VIDEOS).transcriber = Scripted(desk.work / "words.json")
    ids = result(
        await desk.call(
            "video_import",
            paths=[str(desk.work / name) for name in ("talk.mp4", "yellow.png", "song.wav")],
        )
    )["added"]
    batch = [
        edits.add_timeline(
            "Film",
            edits.Rate(num=25, den=1),
            id="$film",
            width=160,
            height=120,
            tracks=[
                edits.track("video", id="$v"),
                edits.track("video", id="$overlay"),
                edits.track("audio", id="$a"),
                edits.track("audio", id="$music"),
                edits.track("subtitle", id="$sub"),
            ],
        ),
        edits.insert("$film", edits.of_media(ids[0], length=150), 0),
    ]
    names = result(await desk.call("video_edit", edits=batch))["names"]
    result(
        await desk.call(
            "video_transcribe", media=ids[0], timeline=names["$film"], subtitles=names["$sub"]
        )
    )
    tools = await located(str(Path(real()).parent), [], cwd=tmp_path)
    return desk, names, ids, tools


async def render(desk: Desk, name: str, tools: Tools, *, seconds: float) -> Path:
    path = desk.work / f"out/{name}.mov"
    result(await desk.call("video_render", path=str(path), codec="prores"))
    facts = await probed(tools, path, place=desk.work / "probe")
    assert abs(facts.duration_us / 1e6 - seconds) <= 0.045
    assert facts.video is not None
    assert (facts.video.width, facts.video.height) == (160, 120)
    assert facts.audio is not None
    return path


async def tracks(desk: Desk) -> dict[str, dict[str, Any]]:
    info = result(await desk.call("video_info"))
    return {one["id"]: one for one in info["timelines"][0]["tracks"]}


async def test_a_sentence_cut_has_no_picture_or_subtitle_in_the_film(
    film: tuple[Desk, dict[str, str], list[str], Tools],
) -> None:
    desk, _, _, tools = film
    path = Path(result(await desk.call("video_story"))["path"])
    text = path.read_text(encoding="utf-8")
    assert "three four." in text
    path.write_text(text.replace("three four.", "~~three four.~~"), encoding="utf-8")
    changed = result(await desk.call("video_apply"))
    # Keep 120 ms after 'two' and before 'five': [1.74, 4.30) rounds up to [44, 108).
    # Word times lie halfway between frames so a sample of silence alignment cannot flip one.
    assert changed["ranges"] == [{"start": 44, "end": 108}]
    output = await render(desk, "cut", tools, seconds=3.44)
    assert await colours(output, 0.3, 1.5, 1.84, 3.2) == ["red", "red", "blue", "blue"]
    subtitles = output.with_suffix(".srt").read_text(encoding="utf-8")
    assert "one two." in subtitles
    assert "five six." in subtitles
    assert "three" not in subtitles
    assert "00:00:01,880 --> 00:00:03,080" in subtitles


async def test_picture_and_music_over_words_are_exported_on_their_tracks(
    film: tuple[Desk, dict[str, str], list[str], Tools],
) -> None:
    desk, names, ids, tools = film
    timeline = names["$film"]
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.overwrite(
                    timeline,
                    edits.of_media(ids[1], length=25),
                    78,
                    video_track=names["$overlay"],
                    audio_track=None,
                ),
                edits.overwrite(
                    timeline,
                    edits.of_media(ids[2], length=150),
                    0,
                    video_track=None,
                    audio_track=names["$music"],
                ),
            ],
        )
    )
    music = (await tracks(desk))[names["$music"]]["clips"][0]["id"]
    result(
        await desk.call("video_edit", edits=[edits.set_clip(timeline, music, gain_db=-10.457575)])
    )
    output = await render(desk, "overlay", tools, seconds=6)
    assert await colours(output, 2.8, 3.4, 4.4) == ["green", "yellow", "blue"]
    # In the quiet before 'one' only the music remains; during 'one' both tracks are audible.
    assert 0.015 < await asyncio.to_thread(loudness, output, 0.05) < 0.05
    assert await asyncio.to_thread(loudness, output, 0.5) > 0.2
    assert "three four." in output.with_suffix(".srt").read_text(encoding="utf-8")


async def test_reverse_speed_and_undo_redo_change_the_exported_film(
    film: tuple[Desk, dict[str, str], list[str], Tools],
) -> None:
    desk, names, _, tools = film
    video = (await tracks(desk))[names["$v"]]["clips"][0]["id"]
    # Subtitle timing is independent of source speed. Disable that track in the same undo step.
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.set_track(names["$film"], names["$sub"], enabled=False),
                edits.set_speed(
                    names["$film"], video, edits.Speed(num=2, den=1), reverse=True, ripple=True
                ),
            ],
        )
    )
    fast = await render(desk, "reverse", tools, seconds=3)
    assert await colours(fast, 0.3, 1.4, 2.7) == ["blue", "green", "red"]
    result(await desk.call("video_undo"))
    original = await render(desk, "undo", tools, seconds=6)
    assert await colours(original, 0.6, 2.8, 5.4) == ["red", "green", "blue"]
    result(await desk.call("video_undo", redo=True))
    again = await render(desk, "redo", tools, seconds=3)
    for at in (0.3, 1.4, 2.7):
        assert await asyncio.to_thread(pixels, again, at) == await asyncio.to_thread(
            pixels, fast, at
        )


async def test_title_transition_and_separate_subtitles_survive_export(
    film: tuple[Desk, dict[str, str], list[str], Tools],
) -> None:
    desk, names, _, tools = film
    timeline = names["$film"]
    result(await desk.call("video_edit", edits=[edits.ripple_delete_range(timeline, 25, 125)]))
    video = (await tracks(desk))[names["$v"]]["clips"]
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.add_transition(timeline, video[0]["id"], video[1]["id"], 10),
                edits.overwrite(
                    timeline,
                    edits.of_title("TEST", 15, edits.TitleStyle(size=32)),
                    0,
                    video_track=names["$overlay"],
                    audio_track=None,
                ),
            ],
        )
    )
    output = await render(desk, "title", tools, seconds=2)
    red, green, blue = await asyncio.to_thread(pixels, output, 1.0)
    assert red > 60
    assert blue > 60
    assert green < 40
    frame = await asyncio.to_thread(pixels, output, 0.3, average=False)
    assert (
        sum(
            r > 200 and g > 200 and b > 200
            for r, g, b in zip(frame[::3], frame[1::3], frame[2::3], strict=False)
        )
        > 50
    )
    # The title is burnt in; SRT subtitles remain separately editable and are never burnt in.
    clean = await asyncio.to_thread(pixels, output, 1.6, average=False)
    assert (
        sum(
            r > 200 and g > 200 and b > 200
            for r, g, b in zip(clean[::3], clean[1::3], clean[2::3], strict=False)
        )
        == 0
    )
    assert output.with_suffix(".srt").is_file()


async def test_one_subtitle_track_is_shown_and_any_is_exported(
    film: tuple[Desk, dict[str, str], list[str], Tools],
) -> None:
    desk, names, _, _ = film
    timeline, shown = names["$film"], names["$sub"]
    # Another subtitle track is added hidden, since the timeline shows one at a time.
    added = result(
        await desk.call(
            "video_edit",
            edits=[edits.add_track(timeline, "subtitle", name="French", language="fr")],
        )
    )
    french = added["done"][0]["track"]
    assert french["enabled"] is False
    refused = await desk.call(
        "video_edit", edits=[edits.set_track(timeline, french["id"], enabled=True)]
    )
    assert refused.error is not None
    assert refused.error.code == "VIDEO_SUBTITLE_SHOWN"
    # Shown in its place, the shown one hidden first in the same batch.
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.set_track(timeline, shown, enabled=False),
                edits.set_track(timeline, french["id"], enabled=True),
            ],
        )
    )
    assert [
        one["enabled"] for one in (await tracks(desk)).values() if one["kind"] == "subtitle"
    ] == [
        False,
        True,
    ]
    # The visible track by default: French, which has no subtitle.
    path = desk.work / "out/shown.mov"
    result(await desk.call("video_render", path=str(path), codec="prores"))
    assert path.with_suffix(".srt").read_text(encoding="utf-8") == ""
    # Every track, the hidden one too, each named by its language or its name.
    path = desk.work / "out/all.mov"
    result(await desk.call("video_render", path=str(path), codec="prores", subtitles="all"))
    written = sorted(one.name for one in path.parent.glob("all.*.srt"))
    assert written == ["all.fr.srt", "all.subtitles.srt"]
    assert "one two." in (path.parent / "all.subtitles.srt").read_text(encoding="utf-8")
    assert (path.parent / "all.fr.srt").read_text(encoding="utf-8") == ""
