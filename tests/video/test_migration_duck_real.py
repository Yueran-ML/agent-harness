"""The desktop command path reaches a real local engine; consent writes once and is retained."""

import asyncio
import json
import math
import struct
import wave
from pathlib import Path

import pytest

from columbina.apps.launcher import main
from columbina.video import edits
from columbina.video.engine import Engine
from columbina.video.migration_duck import pending
from support.media import real_engine


async def test_real_cli_duck_consent_keeps_receipts_and_one_undo(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    program = real_engine()
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("COLUMBINA_ENGINE", program)
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(tmp_path / "agents"))
    path = tmp_path / "tone.wav"
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(48000)
        wav.writeframes(
            b"".join(
                struct.pack("<hh", value, value)
                for value in (
                    round(3000 * math.sin(2 * math.pi * 1000 * n / 48000)) for n in range(96000)
                )
            )
        )
    directory = tmp_path / ".columbina/studio"
    async with await Engine.attach(directory, program=program, idle_seconds=1) as engine:
        _, media = await engine.imported([path])
        created = await engine.apply(
            [
                {
                    "op": "add_timeline",
                    "timeline": {
                        "id": "$mix",
                        "name": "Legacy",
                        "width": 160,
                        "height": 120,
                        "rate": {"num": 25, "den": 1},
                        "sample_rate": 48000,
                        "tracks": [],
                    },
                },
                edits.add_track("$mix", "audio", name="A1"),
                edits.add_track("$mix", "audio", name="A2"),
            ]
        )
        timeline = created.names["$mix"]
        _, project = await engine.project()
        tracks = project["timelines"][0]["tracks"]
        await engine.apply(
            [
                edits.append(timeline, edits.of_media(media[0], length=50), audio_track=track["id"])
                for track in tracks
            ]
        )
        receipt = directory / "analysis/migrations" / f"{timeline}.json"
        receipt.parent.mkdir(parents=True, exist_ok=True)
        receipt.write_text(
            json.dumps(
                {
                    "timeline": timeline,
                    "source": str(tmp_path / "Old Film"),
                    "dropped": ["duck: omitted"],
                    "at": "old",
                    "media": {},
                }
            )
        )
        assert len(await pending(tmp_path, engine)) == 1
        assert (
            await asyncio.to_thread(
                main, ["video", "migrate", "--workspace", str(tmp_path), "--json"]
            )
            == 0
        )
        listed = json.loads(capsys.readouterr().out)["migrations"]
        assert len(listed) == 1
        assert listed[0]["duck_pending"] is True
        before_revision, before = await engine.project()
        args = [
            "video",
            "migration-duck",
            "--workspace",
            str(tmp_path),
            "--timeline",
            timeline,
            "--answer",
            "yes",
            "--json",
        ]
        assert await asyncio.to_thread(main, args) == 0
        result = json.loads(capsys.readouterr().out)
        assert result["mode"] == "ducker"
        assert result["duck_prompted"] is True
        assert result["dialogue"] == tracks[0]["id"]
        assert result["music"] == [tracks[1]["id"]]
        revision, after = await engine.project()
        assert revision == before_revision + 1
        assert after["timelines"][0]["tracks"][1]["strip"]["track_fx"]["on"] is True
        assert await pending(tmp_path, engine) == []
        assert await asyncio.to_thread(main, args) == 0
        assert json.loads(capsys.readouterr().out)["status"] == "already_answered"
        assert (await engine.project())[0] == revision
        await engine.undo()
        assert (await engine.project())[1]["timelines"] == before["timelines"]
        # Explicit Undo does not reset the user's once-only answer.
        assert await pending(tmp_path, engine) == []
        print("Real CLI consent: one transaction, A1/A2, durable receipts, repeat no-op, one Undo.")

    for _ in range(100):
        if not (directory / "engine.lock").exists():
            break
        await asyncio.sleep(0.1)
    assert not (directory / "engine.lock").exists()
