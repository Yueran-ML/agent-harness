"""The tools that edit video, as a model calls them: ``columbina.video``."""

import json
import shutil
from pathlib import Path

import pytest

import columbina.fs
import columbina.sandbox
import columbina.session
import columbina.tools
import columbina.video
from columbina.builtin_tools import edit, read
from columbina.fs import FS
from columbina.kernel import Context
from columbina.sandbox import SANDBOX
from columbina.session import SESSIONS
from columbina.tools import TOOLS
from columbina.video import VIDEOS, Config, Videos
from columbina.video.transcribe import Scripted, Whisper
from support.desk import Desk
from support.media import made, real

WORDS = [
    (200_000, 800_000, "one"),
    (900_000, 1_600_000, "two."),
    (2_400_000, 3_000_000, "three"),
    (3_100_000, 3_600_000, "four."),
]


async def desk_of(ctx: Context, tmp_path: Path, **config: object) -> Desk:
    work = tmp_path / "work"
    work.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(read)
    await ctx.plugin(edit)
    await ctx.plugin(columbina.video, {"enabled": True, **config})
    session = ctx.use(SESSIONS).create(ctx, work)
    return Desk(ctx, ctx.use(TOOLS), ctx.use(FS), ctx.use(SANDBOX), session, work)


@pytest.fixture
async def desk(ctx: Context, tmp_path: Path) -> Desk:
    """A working directory with a talk in it, and the tools that edit video."""
    real()
    made_desk = await desk_of(ctx, tmp_path)
    tone = "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in WORDS)
    made(
        made_desk.work / "raw", "talk.mp4",
        "-f", "lavfi", "-i", "color=c=red:s=160x120:r=25:d=4",
        "-f", "lavfi", "-i", f"aevalsrc='0.5*sin(440*2*PI*t)*({tone})':d=4:s=48000",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    written = [{"start_us": start, "end_us": end, "text": text} for start, end, text in WORDS]
    (tmp_path / "words.json").write_text(json.dumps(written), encoding="utf-8")
    ctx.use(VIDEOS).transcriber = Scripted(tmp_path / "words.json")
    return made_desk


async def test_a_model_cuts_a_talk_with_the_tools_and_the_files_of_the_project(desk: Desk) -> None:
    imported = await desk.call("video_import", project="film", paths=["raw/talk.mp4"])
    heard = await desk.call("video_transcribe", project="film", source="talk", language="en")
    await desk.have_read("film/story.md")
    cut = await desk.call("edit", path="film/story.md", old="three four.", new="~~three four.~~")
    dry = await desk.call("video_apply", project="film", dry=True)
    applied = await desk.call("video_apply", project="film", pause_most_ms=300)

    assert imported.error is None, imported.text
    assert "Taken in: talk (video, 0:04.0, 160x120)" in imported.text
    assert heard.text.startswith("talk was transcribed: 2 sentences and 4 words")
    assert cut.error is None, cut.text
    assert dry.text.startswith("Not committed.")
    assert applied.text.startswith("Committed.\nSentences cut: 1. Words cut: 2.")

    title = await desk.call(
        "video_overlay",
        project="film",
        action="add",
        kind="title",
        text="Hi",
        sentence="talk.0",
        words="two",
    )
    styled = await desk.call(
        "video_overlay", project="film", action="style", captions=False, duck=False
    )
    crossed = await desk.call(
        "video_overlay", project="film", action="style", sentence="talk.0", crossfade_frames=3
    )
    lonely = await desk.call("video_overlay", project="film", action="style", crossfade_frames=3)
    changed = await desk.call(
        "video_overlay", project="film", action="change", layer="title-1", position="top-left"
    )
    corrected = await desk.call(
        "video_correct", project="film", sentence="talk.0", wrong="two", right="2."
    )
    rendered = await desk.call("video_render", project="film")
    undone = await desk.call("video_undo", project="film")
    redone = await desk.call("video_undo", project="film", redo=True)
    removed = await desk.call("video_overlay", project="film", action="remove", layer="title-1")
    told = await desk.call("video_info", project="film")

    assert title.text.startswith("The layer title-1 (title) was added. It begins at 0:00.")
    assert styled.text == "Changed: captions off, ducking off."
    assert crossed.text == "Changed: talk.0 crosses 3 frames into the next sentence."
    assert lonely.error is not None
    assert lonely.error.code == "VIDEO_PROJECT"
    assert "a crossfade names the sentence" in lonely.text
    assert changed.text == "The layer title-1 (title) was changed. It begins at 0:00.8."
    assert corrected.text.startswith('In talk.0, "two" is shown as "2." from now on.')
    assert rendered.error is None, rendered.text
    assert (desk.work / "film" / "out" / "preview.mp4").exists()
    assert undone.text.startswith("Taken back.")
    assert redone.text.startswith("Done again.")
    assert removed.text == "The layer title-1 was taken away."
    assert "The main line has 1 sentences" in told.text


async def test_a_source_outside_the_working_directory_is_not_read(
    desk: Desk, tmp_path: Path
) -> None:
    shutil.copy(desk.work / "raw" / "talk.mp4", tmp_path / "elsewhere.mp4")

    outcome = await desk.call(
        "video_import", project="film", paths=[str(tmp_path / "elsewhere.mp4")]
    )

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"


async def test_what_cannot_be_done_is_said_with_a_code(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls = {
        "video_transcribe": {"source": "talk"},
        "video_apply": {},
        "video_correct": {"sentence": "talk.0", "wrong": "a", "right": "b"},
        "video_overlay": {"action": "remove", "layer": "x"},
        "video_render": {"quality": "final"},
        "video_undo": {},
        "video_import": {"paths": ["raw/nothing.mp4"]},
    }
    for name, arguments in calls.items():
        outcome = await desk.call(name, project="film", **arguments)
        assert outcome.error is not None, name
        assert outcome.error.code.startswith("VIDEO_"), (name, outcome.error.code)
    told = await desk.call("video_info", project="film")
    assert told.text.startswith("The project ")
    (desk.work / "film").mkdir(exist_ok=True)
    (desk.work / "film" / "project.json").write_text("{", encoding="utf-8")
    broken = await desk.call("video_info", project="film")
    assert broken.error is not None
    assert broken.error.code == "VIDEO_PROJECT"

    monkeypatch.setenv("PATH", "")
    fresh = Videos(Config(enabled=True), None)
    desk.ctx.use(VIDEOS)._tools = None  # pyright: ignore[reportPrivateUsage]
    outcome = await desk.call("video_info", project="other")
    assert outcome.error is not None
    assert outcome.error.code == "NO_FFMPEG"
    assert fresh.transcriber is None


async def test_the_tools_are_given_only_where_the_config_says_so(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.video, {"enabled": False})

    assert not [name for name in ctx.use(TOOLS).registry if name.startswith("video_")]
    assert ctx.get(VIDEOS) is None


def test_a_model_of_faster_whisper_is_what_transcribes() -> None:
    told = Config.model_validate({"enabled": "1", "whisper_model": "", "whisper_python": ""})
    given = Videos(Config(whisper_model=Path("/models/small"), device="cpu"), None)

    assert (told.enabled, told.whisper_model, told.whisper_python) == (True, None, None)
    assert isinstance(given.transcriber, Whisper)
    assert (given.transcriber.model, given.transcriber.device) == (Path("/models/small"), "cpu")
