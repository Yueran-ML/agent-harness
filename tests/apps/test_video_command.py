"""``columbina video``: a film cut by hand, with the same operations as the tools."""

import asyncio
import json
import sys
import time
from pathlib import Path

import pytest

from columbina.apps.launcher import main
from columbina.video import engine
from columbina.video.engine import Engine
from support import fake_engine
from support.media import made, real, real_engine

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


def gone(lock: Path) -> None:
    deadline = time.monotonic() + 15
    while lock.exists():
        assert time.monotonic() < deadline, "the engine did not end"
        time.sleep(0.05)


def test_the_engine_says_that_it_answers(
    home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("COLUMBINA_ENGINE", fake_engine.__file__)
    monkeypatch.setattr(engine, "_argv", fake_engine.argv_of(idle=1))

    assert main(["video", "engine", "--project", "film"]) == 0
    told = said(capsys)
    assert main(["video", "engine", "--project", "film", "--json"]) == 0
    answered = json.loads(said(capsys))
    gone(tmp_path / "film" / "engine.lock")

    assert told.startswith("The engine 0.1.0-fake answers in protocol 1, as process ")
    assert told.endswith(f"for {tmp_path / 'film'}: revision 0, 0 media, 0 timelines.\n")
    assert answered["revision"] == answered["media"] == answered["timelines"] == 0
    assert answered["project"] == str(tmp_path / "film")


def test_an_engine_that_is_missing_or_fails_is_said(
    home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("COLUMBINA_ENGINE", raising=False)
    # One in the working directory is not taken, though the search path leads there.
    lying = tmp_path / ("columbina-engine.exe" if sys.platform == "win32" else "columbina-engine")
    lying.write_bytes(b"")
    lying.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path))

    with pytest.raises(SystemExit):
        main(["video", "engine"])
    assert "--project" in capsys.readouterr().err
    assert main(["video", "engine", "--project", "film"]) == 2
    missing = said(capsys)
    monkeypatch.setenv("COLUMBINA_ENGINE", "engine")
    assert main(["video", "engine", "--project", "film"]) == 2
    refused = said(capsys)
    monkeypatch.setenv("COLUMBINA_ENGINE", fake_engine.__file__)
    monkeypatch.setattr(engine, "_argv", fake_engine.argv_of("error"))
    assert main(["video", "engine", "--project", "film"]) == 2

    assert missing.startswith("columbina: no columbina-engine program was found: build it with")
    assert refused.startswith("columbina: COLUMBINA_ENGINE names engine as the columbina-engine")
    assert said(capsys) == "columbina: project.json cannot be read\n"


def test_the_engine_itself_says_how_a_project_stands(
    home: Path, talk: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    program = real_engine()
    monkeypatch.setenv("COLUMBINA_ENGINE", program)

    async def edited() -> int:
        async with await Engine.attach(talk / "film", program=program, idle_seconds=1) as one:
            await one.imported([talk / "raw" / "talk.mp4"])
            rate = {"num": 25, "den": 1}
            timeline: dict[str, object] = {"id": "", "name": "Cut", "rate": rate, "tracks": []}
            await one.apply([{"op": "add_timeline", "timeline": timeline}])
            return int(one.hello["pid"])

    pid = asyncio.run(edited())
    # The engine has ended before the command asks, which starts another: it reads the project
    # that the first one saved, and counts its revisions from 0 again.
    gone(talk / "film" / "engine.lock")
    started = engine._argv  # pyright: ignore[reportPrivateUsage]

    def briefly(program: str, directory: Path, idle_seconds: int | None) -> list[str]:
        return started(program, directory, 1)

    monkeypatch.setattr(engine, "_argv", briefly)
    assert main(["video", "engine", "--project", "film", "--json"]) == 0
    answered = json.loads(said(capsys))
    gone(talk / "film" / "engine.lock")

    assert answered["engine"] == "0.1.0"
    assert answered["pid"] != pid
    assert (answered["revision"], answered["media"], answered["timelines"]) == (0, 1, 1)
