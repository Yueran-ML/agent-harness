"""``columbina video``: a film cut by hand, with the same operations as the tools."""

import json
from pathlib import Path

import pytest

from columbina.apps.launcher import main
from support.media import made, real

WORDS = [(200_000, 800_000, "one"), (900_000, 1_600_000, "two."), (2_400_000, 3_000_000, "three.")]


@pytest.fixture
def talk(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    real()
    monkeypatch.chdir(tmp_path)
    tone = "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in WORDS)
    made(
        tmp_path / "raw", "talk.mp4",
        "-f", "lavfi", "-i", "color=c=red:s=160x120:r=25:d=3.2",
        "-f", "lavfi", "-i", f"aevalsrc='0.5*sin(440*2*PI*t)*({tone})':d=3.2:s=48000",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip
    written = [{"start_us": start, "end_us": end, "text": text} for start, end, text in WORDS]
    (tmp_path / "words.json").write_text(json.dumps(written), encoding="utf-8")
    return tmp_path


def said(capsys: pytest.CaptureFixture[str]) -> str:
    return capsys.readouterr().out


def test_a_film_is_cut_by_hand(home: Path, talk: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["video", "import", "film", "raw/talk.mp4"]) == 0
    assert "Taken in: talk" in said(capsys)
    assert (
        main(["video", "transcribe", "film", "talk", "--words", "words.json", "--language", "en"])
        == 0
    )
    assert said(capsys).startswith("talk was transcribed: 2 sentences and 3 words")
    story = talk / "film" / "story.md"
    story.write_text(
        story.read_text(encoding="utf-8").replace("three.", "~~three.~~"), encoding="utf-8"
    )
    assert main(["video", "apply", "film", "--dry"]) == 0
    assert said(capsys).startswith("Not committed.")
    assert main(["video", "apply", "film", "--pause-most-ms", "400"]) == 0
    assert "Sentences cut: 1." in said(capsys)
    assert main(["video", "correct", "film", "talk.0", "two", "2"]) == 0
    assert 'is shown as "2"' in said(capsys)
    overlay = ["video", "overlay", "film"]
    assert (
        main(
            [
                *overlay,
                "add",
                "--kind",
                "title",
                "--text",
                "Hi",
                "--at-frame",
                "0",
                "--frames",
                "10",
            ]
        )
        == 0
    )
    assert (
        main(
            [
                *overlay,
                "change",
                "--layer",
                "title-1",
                "--position",
                "top-right",
                "--opacity",
                "0.5",
            ]
        )
        == 0
    )
    assert main([*overlay, "style", "--captions", "off", "--duck", "on"]) == 0
    assert main([*overlay, "style", "--sentence", "talk.0", "--crossfade", "2"]) == 0
    assert "Changed: talk.0 crosses 2 frames" in said(capsys).splitlines()[-1]
    assert main(["video", "render", "film"]) == 0
    assert "The preview is" in said(capsys)
    assert main(["video", "undo", "film"]) == 0
    assert main(["video", "undo", "film", "--redo"]) == 0
    assert said(capsys).splitlines()[-1].startswith("Done again.")
    assert main(["video", "info", "film"]) == 0
    assert "layer title-1: title Hi" in said(capsys)


def test_what_cannot_be_done_by_hand_is_said(
    home: Path, talk: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    assert main(["video", "import", "film", "raw/nothing.mp4"]) == 2
    assert "columbina: " in said(capsys)
    assert main(["video", "import", "film", "raw/talk.mp4"]) == 0
    said(capsys)
    monkeypatch.delenv("COLUMBINA_WHISPER_MODEL", raising=False)
    assert main(["video", "transcribe", "film", "talk"]) == 2
    assert "no transcriber is set up" in said(capsys)
    monkeypatch.setenv("COLUMBINA_WHISPER_MODEL", str(talk / "model"))
    assert main(["video", "transcribe", "film", "talk"]) == 2
    assert "there is no model of faster-whisper" in said(capsys)
    assert (
        main(["video", "overlay", "film", "add", "--kind", "picture", "--sentence", "talk.0"]) == 2
    )
    assert main(["video", "render", "film", "--final"]) == 2
    assert "the main line is empty" in said(capsys).splitlines()[-1]
