"""What goes wrong, or seldom happens, in a project of editing."""

import json
import os
import sys
from fractions import Fraction
from pathlib import Path

import pytest

import columbina.video
from columbina.tools import ToolRun
from columbina.video import Config, Videos, media, render, whisper_worker
from columbina.video.editing import STORY, Editor, Placing
from columbina.video.media import AudioFacts, Facts, Silence, Tools, VideoFacts, located
from columbina.video.project import Asset, Canvas, Identity, Manifest, ProjectError
from columbina.video.render import Cue, Plan, RenderError, captions, rendered
from columbina.video.story import whole
from columbina.video.timeline import Cut, Entry, Layer, Timeline, Word
from columbina.video.transcribe import Scripted, transcript_of
from support.media import made, real

WORDS = [(200_000, 800_000, "one"), (900_000, 1_600_000, "two."), (2_400_000, 3_000_000, "three.")]


@pytest.fixture
async def tools(tmp_path: Path) -> Tools:
    return await located(str(Path(real()).parent), [], cwd=tmp_path)


def tone() -> str:
    return "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in WORDS)


@pytest.fixture
def raw(tmp_path: Path) -> Path:
    where = tmp_path / "raw"
    made(
        where, "talk.mp4",
        "-f", "lavfi", "-i", "color=c=red:s=160x120:r=25:d=3.2",
        "-f", "lavfi", "-i", f"aevalsrc='0.5*sin(440*2*PI*t)*({tone()})':d=3.2:s=48000",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    made(
        where,
        "voice.wav",
        "-f",
        "lavfi",
        "-i",
        f"aevalsrc='0.5*sin(440*2*PI*t)*({tone()})':d=3.2:s=48000",
    )
    written = [{"start_us": start, "end_us": end, "text": text} for start, end, text in WORDS]
    (where / "words.json").write_text(json.dumps(written), encoding="utf-8")
    return where


async def transcribed(tmp_path: Path, tools: Tools, raw: Path, source: str = "talk.mp4") -> Editor:
    editor = Editor(tmp_path / "film", tools, Scripted(raw / "words.json"))
    await editor.imported([raw / source])
    await editor.transcribed(Path(source).stem)
    return editor


async def test_a_talk_without_a_picture_makes_a_film_of_sound(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw, "voice.wav")

    told = await editor.rendered("final")

    assert (editor.project.directory / "out" / "film.m4a").exists()
    assert "The final is" in told
    imported = await editor.imported([raw / "voice.wav"])
    assert "The film is" not in imported


async def test_a_source_that_changed_is_said_to_be_transcribed_again(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)

    again = await editor.transcribed("talk")
    made(
        raw, "talk.mp4",
        "-f", "lavfi", "-i", "color=c=blue:s=160x120:r=25:d=3.2", "-f", "lavfi", "-i", "sine=d=3.2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    changed = await editor.imported([raw / "talk.mp4"])
    stale = await editor.info()
    anew = await editor.transcribed("talk", again=True)

    assert again == "talk was transcribed before: 2 sentences. The story is story.md."
    assert changed.endswith("A source that changed is to be transcribed again before it is cut.")
    assert "(changed since, to be transcribed again)" in stale
    assert anew.startswith("talk was transcribed: 2 sentences")


async def test_a_receipt_names_what_lies_nowhere_now(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    await editor.layered("add", kind="title", text="On three", placing=Placing("talk.1"))
    await editor.layered("add", kind="title", text="Whole", placing=Placing(at_frame=5))
    story = editor.project.directory / STORY
    story.write_text(
        story.read_text(encoding="utf-8").replace("three.", "~~three.~~"), encoding="utf-8"
    )

    receipt = await editor.applied()
    film = await editor.rendered()

    assert "These layers lie nowhere now, their words were cut: title-1." in receipt
    assert "may be heard" not in receipt
    assert "Left out, they lie nowhere: title-1." in film


async def test_a_word_that_was_corrected_in_a_story_that_was_changed_stays_as_it_was(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    story = editor.project.directory / STORY
    story.write_text(
        story.read_text(encoding="utf-8").replace("three.", "~~three.~~"), encoding="utf-8"
    )

    said = await editor.corrected("talk.0", "one", "1")

    assert said.endswith("The story was changed, and is as it was.")
    assert "one two." in story.read_text(encoding="utf-8")


async def test_a_story_without_its_first_line_is_left_alone(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    story = editor.project.directory / STORY
    first, _, rest = story.read_text(encoding="utf-8").partition("\n")
    story.write_text(f"# My story\n{rest}", encoding="utf-8")

    await editor.styled(captions=False)
    await editor.styled(crossfade=("talk.0", 3))
    await editor.styled(crossfade=("talk.0", 0))
    assert story.read_text(encoding="utf-8").startswith("# My story\n")
    story.unlink()
    await editor.styled(duck=False)

    assert editor.timeline()[0].crossfades == {}
    assert story.read_text(encoding="utf-8").startswith("<!-- columbina story: ")
    assert first.startswith("<!-- columbina story: ")


async def test_a_receipt_says_where_a_cut_may_be_heard_and_when_nothing_is_left(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    story = editor.project.directory / STORY
    text = story.read_text(encoding="utf-8")

    story.write_text(text.replace("one two.", "~~one~~ two."), encoding="utf-8")
    tight = await editor.applied()
    story.write_text(
        "\n".join(story.read_text(encoding="utf-8").splitlines()[:2]), encoding="utf-8"
    )
    empty = await editor.applied()

    assert "Cuts in talk.0 fall where no pause is, and may be heard." in tight
    assert "The film lasts 0:00.0;" in empty


async def test_a_layer_at_a_sentence_lasts_as_long_as_it(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)

    await editor.layered("add", kind="title", text="Second", placing=Placing("talk.1"))

    (layer,) = editor.timeline()[0].layers
    assert (layer.anchor, layer.until) == (("talk", 2), ("talk", 2))


async def test_files_of_a_project_that_cannot_be_read_are_said(
    tmp_path: Path, tools: Tools, raw: Path
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    transcript = editor.project.directory / "transcripts" / "talk.json"
    transcript.write_text("{", encoding="utf-8")

    with pytest.raises(ProjectError, match=r"the transcript .* cannot be read"):
        editor.transcripts()
    (editor.project.directory / "timeline.json").write_text('{"main": 1}', encoding="utf-8")
    with pytest.raises(ProjectError, match="the timeline is not what this program writes"):
        editor.timeline()


async def test_a_film_that_is_not_the_plan_is_not_kept(
    tmp_path: Path, tools: Tools, raw: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    editor = await transcribed(tmp_path, tools, raw)
    timeline = editor.timeline()[0]
    transcripts = editor.transcripts()
    quiet = {"talk": await editor.project.cached_silences(editor.project.asset("talk"), tools)}

    async def short(*args: object, **more: object) -> Facts:
        return Facts(
            kind="video",
            format="mov",
            duration_us=1,
            video=VideoFacts(width=1, height=1, rate="25/1"),
        )

    monkeypatch.setattr(media, "probed", short)
    with pytest.raises(RenderError, match="It was not kept"):
        await rendered(editor.project, tools, timeline, transcripts, quiet, "preview", "x")

    async def silent(*args: object, **more: object) -> Facts:
        plan = render.planned(
            timeline, transcripts, quiet, {"talk": 3_200_000}, editor.project.manifest().canvas
        )
        return Facts(
            kind="video",
            format="mov",
            duration_us=plan.length_us,
            video=VideoFacts(width=1, height=1, rate="25/1"),
        )

    monkeypatch.setattr(media, "probed", silent)
    with pytest.raises(RenderError, match="lacks its picture or its sound"):
        await rendered(editor.project, tools, timeline, transcripts, quiet, "preview", "x")
    with pytest.raises(RenderError, match="the main line is empty"):
        await rendered(editor.project, tools, Timeline(), transcripts, quiet, "preview", "x")
    assert not list((editor.project.directory / "out").glob("x.*"))


async def test_a_film_without_captions_has_no_file_of_them(
    tmp_path: Path, tools: Tools, raw: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    editor = await transcribed(tmp_path, tools, raw)

    def nothing(*args: object) -> list[Cue]:
        return []

    monkeypatch.setattr(render, "captions", nothing)

    told = await editor.rendered()

    assert "The captions are" not in told
    assert not (editor.project.directory / "out" / "preview.srt").exists()


def test_captions_leave_out_what_has_no_time_in_the_film() -> None:
    said = transcript_of(
        "talk",
        "d",
        [
            Word(start_us=0, end_us=100_000, text="a"),
            Word(start_us=200_000, end_us=300_000, text="b"),
        ],
        [],
        400_000,
    )
    timeline = Timeline(
        main=(
            Entry(asset="talk", sentence=0, kept=()),
            Entry(asset="talk", sentence=0, kept=(0, 1)),
        )
    )
    plan = Plan(Fraction(25), (Cut("talk", 0, 100_000, 1),), (0,), (3,), (0,), 3)

    assert captions(plan, timeline, {"talk": said}) == []


def test_titles_last_to_the_end_where_nothing_else_is_said(monkeypatch: pytest.MonkeyPatch) -> None:
    said = transcript_of("talk", "d", [Word(start_us=0, end_us=900_000, text="a.")], [], 1_000_000)
    timeline = Timeline(
        main=whole(said),
        layers=(Layer(id="t", kind="title", text="Hi", at_frame=2),),
        crossfades={},
    )
    canvas = Canvas(width=64, height=48, rate="25/1")
    plan = render.planned(timeline, {"talk": said}, {"talk": ()}, {"talk": 1_000_000}, canvas)
    cues = [Cue(plan.us(2), plan.length_us, "Hi", "full")]

    styles = render.ass(cues, canvas)

    for system, font in (("darwin", "PingFang SC"), ("linux", "Noto Sans CJK SC")):
        monkeypatch.setattr(sys, "platform", system)
        assert font in render.ass(cues, canvas)
    assert "Dialogue: 1,0:00:00.08,0:00:01.00,full,Hi" in styles


def test_sound_alone_is_crossed_without_a_picture() -> None:
    words = [
        Word(start_us=0, end_us=400_000, text="a."),
        Word(start_us=1_500_000, end_us=1_900_000, text="b."),
    ]
    said = transcript_of("talk", "d", words, [], 2_000_000)
    timeline = Timeline(main=whole(said), crossfades={0: 2})
    quiet = {"talk": (Silence(400_000, 1_500_000),)}
    plan = render.planned(timeline, {"talk": said}, quiet, {"talk": 2_000_000}, None)
    manifest = Manifest(
        assets=(
            Asset(
                id="talk",
                path="/t.wav",
                kind="audio",
                format="wav",
                duration_us=2_000_000,
                audio=AudioFacts(sample_rate=16000, channels=1),
                identity=Identity(size=1, modified_ns=1, digest="d"),
            ),
        )
    )
    made_graph = render.compiled(plan, timeline, manifest, {"talk": said}, "final")

    assert "acrossfade" in made_graph.graph
    assert "xfade" not in made_graph.graph.replace("acrossfade", "")


def test_the_worker_imports_by_name() -> None:
    assert whisper_worker._module("json") is json  # pyright: ignore[reportPrivateUsage]


def test_without_the_file_seam_a_path_is_taken_from_the_working_directory(tmp_path: Path) -> None:
    videos = Videos(Config(enabled=True), None)
    run = ToolRun("c1", tmp_path, object())  # type: ignore[arg-type]

    assert videos.checked(run, "video_info", "film", "read") == (tmp_path / "film").resolve()
    assert columbina.video.name == "video"
    assert os.sep
