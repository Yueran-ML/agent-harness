"""A film made from sources that ffmpeg makes here, checked by its colours and its length.

The talk is red for its first two seconds, green for the next two and blue for the last two,
with a tone wherever a word is said and silence between the sentences. A frame of the film says
by its colour which part of the talk it came from.
"""

import json
import subprocess
from pathlib import Path

import pytest

from columbina.video.editing import STORY, Editor, Placing, Shape
from columbina.video.media import Tools, located, probed
from columbina.video.project import ProjectError, StaleError
from columbina.video.render import RenderError
from columbina.video.story import StoryError
from columbina.video.transcribe import Scripted, TranscriptionError
from support.media import made, real

WORDS = [
    (200_000, 800_000, "one"), (900_000, 1_600_000, "two."),
    (2_400_000, 3_000_000, "three"), (3_100_000, 3_600_000, "four."),
    (4_400_000, 5_000_000, "five"), (5_100_000, 5_600_000, "six."),
]  # fmt: skip
TONE = "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in WORDS)


@pytest.fixture
async def tools(tmp_path: Path) -> Tools:
    found = real()
    return await located(str(Path(found).parent), [], cwd=tmp_path)


@pytest.fixture
def sources(tmp_path: Path) -> Path:
    """The talk, a yellow picture and a song, made by ffmpeg."""
    where = tmp_path / "raw"
    paint = (
        "color=black:s=160x120:r=25:d=6,format=rgb24,"
        "geq=r='255*lt(T,2)':g='255*between(T,2,4)':b='255*gte(T,4)'"
    )
    made(
        where, "talk.mp4",
        "-f", "lavfi", "-i", paint,
        "-f", "lavfi", "-i", f"aevalsrc='0.5*sin(440*2*PI*t)*({TONE})':d=6:s=48000",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    made(where, "yellow.png", "-f", "lavfi", "-i", "color=c=yellow:s=64x48:d=1", "-frames:v", "1")
    made(where, "song.wav", "-f", "lavfi", "-i", "sine=f=220:d=10")
    (where / "words.json").write_text(
        json.dumps(
            [{"start_us": start, "end_us": end, "text": text} for start, end, text in WORDS]
        ),
        encoding="utf-8",
    )
    return where


async def editor_of(tmp_path: Path, tools: Tools, sources: Path) -> Editor:
    editor = Editor(tmp_path / "film", tools, Scripted(sources / "words.json"))
    await editor.imported([sources / "talk.mp4", sources / "yellow.png", sources / "song.wav"])
    await editor.transcribed("talk")
    return editor


def colour_at(film: Path, seconds: float) -> tuple[int, int, int]:
    """The colour of a frame of a film, as one pixel."""
    done = subprocess.run(
        [real(), "-hide_banner", "-loglevel", "error", "-ss", f"{seconds:.3f}", "-i", str(film),
         "-frames:v", "1", "-vf", "scale=1:1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True, timeout=60, check=True,
    )  # fmt: skip
    red, green, blue = done.stdout[:3]
    return red, green, blue


def named(colour: tuple[int, int, int]) -> str:
    red, green, blue = colour
    if red > 150 and green > 150 and blue < 100:
        return "yellow"
    return max((("red", red), ("green", green), ("blue", blue)), key=lambda one: one[1])[0]


def edit(editor: Editor, old: str, new: str) -> None:
    story = editor.project.directory / STORY
    text = story.read_text(encoding="utf-8")
    assert old in text
    story.write_text(text.replace(old, new), encoding="utf-8")


async def test_a_sentence_that_is_cut_is_not_in_the_film(
    tmp_path: Path, tools: Tools, sources: Path
) -> None:
    editor = await editor_of(tmp_path, tools, sources)
    edit(editor, "[talk.1 00:02] three four.", "[talk.1 00:02] ~~three four.~~")

    receipt = await editor.applied()
    told = await editor.rendered("final")

    assert "Sentences cut: 1. Words cut: 2." in receipt
    film = editor.project.directory / "out" / "film.mp4"
    assert f"The final is {film}" in told
    facts = await probed(tools, film, place=tmp_path / "probe")
    # One second and 64 hundredths of the first sentence, and 44 of the last.
    assert abs(facts.duration_us - 3_080_000) <= 80_000
    assert [named(colour_at(film, at)) for at in (0.3, 1.5, 1.8, 2.9)] == [
        "red",
        "red",
        "blue",
        "blue",
    ]
    captions = (editor.project.directory / "out" / "film.srt").read_text(encoding="utf-8")
    assert "one two." in captions
    assert "five six." in captions
    assert "three" not in captions


async def test_a_picture_lies_over_the_words_that_it_is_laid_at(
    tmp_path: Path, tools: Tools, sources: Path
) -> None:
    editor = await editor_of(tmp_path, tools, sources)

    said = await editor.layered(
        "add", kind="picture", asset="yellow", placing=Placing("talk.1", "four")
    )
    await editor.layered("add", kind="sound", asset="song", shape=Shape(volume=0.3))
    await editor.styled(captions=False)
    await editor.rendered("preview")

    assert said == "The layer picture-1 (picture) was added. It begins at 0:03.0."
    film = editor.project.directory / "out" / "preview.mp4"
    # The pause between the first two sentences is short enough to be kept whole: "four" is said
    # in the film where it is said in the talk, less the 80 ms before the first word.
    begins = 3.1 - 0.08
    assert named(colour_at(film, begins + 0.2)) == "yellow"
    assert named(colour_at(film, begins - 0.5)) == "green"
    # Captions that are not drawn on the film are still written beside it.
    assert (editor.project.directory / "out" / "preview.srt").exists()


async def test_what_cannot_be_done_with_a_project_is_said(
    tmp_path: Path, tools: Tools, sources: Path
) -> None:
    editor = await editor_of(tmp_path, tools, sources)
    story = editor.project.directory / STORY

    edit(editor, "one two.", "one too.")
    with pytest.raises(StoryError, match=r"its words are not those of talk.0"):
        await editor.applied()
    assert await editor.applied(fresh=True) == (
        f"The story was written anew from the timeline as it is: {story}."
    )
    # A change of the story that is not committed yet stays through a change of the layers.
    edit(editor, "[talk.0 00:00] one two.\n", "")
    await editor.layered("add", kind="title", text="Welcome")
    assert "Sentences cut: 1." in await editor.applied()
    # It does not through a change of the main line: the story is of another timeline then.
    edit(editor, "[talk.2 00:04] five six.", "[talk.2 00:04] five ~~six.~~")
    await editor.undone()
    with pytest.raises(StaleError, match="written from another timeline"):
        await editor.applied()
    await editor.applied(fresh=True)
    assert "five six." in story.read_text(encoding="utf-8")
    with pytest.raises(ProjectError, match=r"there is no sentence talk.7"):
        await editor.corrected("talk.7", "a", "b")
    with pytest.raises(ProjectError, match=r'"seven" is not a word of talk.0'):
        await editor.corrected("talk.0", "seven", "7")
    assert (await editor.undone(redo=True)).startswith("Done again.")
    with pytest.raises(ProjectError, match="nothing to redo"):
        await editor.undone(redo=True)
    with pytest.raises(ProjectError, match="there is no layer nothing"):
        await editor.layered("remove", layer="nothing")
    with pytest.raises(ProjectError, match="a layer that is added needs a kind"):
        await editor.layered("add")
    with pytest.raises(ProjectError, match="a picture needs a source"):
        await editor.layered("add", kind="picture")
    with pytest.raises(ProjectError, match="a title needs its words"):
        await editor.layered("add", kind="title")
    with pytest.raises(ProjectError, match="has no source nothing"):
        await editor.layered("add", kind="sound", asset="nothing")
    with pytest.raises(ProjectError, match=r"talk\.0 is not on the main line"):
        await editor.layered("add", kind="title", text="x", placing=Placing("talk.0", "one"))
    with pytest.raises(ProjectError, match=r'"seven" is not among the kept words of talk\.1'):
        await editor.layered("add", kind="title", text="x", placing=Placing("talk.1", "seven"))
    with pytest.raises(ProjectError, match="the layer cannot be so"):
        await editor.layered("add", kind="title", text="x", shape=Shape(opacity=2.0))
    with pytest.raises(ProjectError, match=r"there is no sentence talk\.9"):
        await editor.styled(crossfade=("talk.9", 5))
    with pytest.raises(ProjectError, match=r"talk\.0 is not on the main line"):
        await editor.styled(crossfade=("talk.0", 5))
    assert await editor.styled() == "Nothing was changed."


async def test_a_project_without_words_has_nothing_to_make(
    tmp_path: Path, tools: Tools, sources: Path
) -> None:
    editor = Editor(tmp_path / "film", tools)
    await editor.imported([sources / "talk.mp4"])

    with pytest.raises(ProjectError, match="no transcriber is set up"):
        await editor.transcribed("talk")
    with pytest.raises(ProjectError, match="the main line is empty"):
        await editor.rendered()
    with pytest.raises(ProjectError, match="there is no story to apply"):
        await editor.applied()
    empty = sources / "none.json"
    empty.write_text("[]", encoding="utf-8")
    editor.transcriber = Scripted(empty)
    with pytest.raises(TranscriptionError, match="nothing was heard in talk"):
        await editor.transcribed("talk")
    assert "talk: video, 0:06.0" in await editor.info()
    assert RenderError
