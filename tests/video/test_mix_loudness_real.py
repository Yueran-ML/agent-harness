"""Actual candidate DSP, final readback and one-step undo for main loudness optimization."""

import asyncio
import math
import struct
import wave
from pathlib import Path
from typing import Any

from columbina.video import edits
from columbina.video.audio import Loudness
from columbina.video.engine import Engine
from support.media import real_engine


async def test_main_optimization_meets_loudness_and_peak_and_undoes_once(tmp_path: Path) -> None:
    rate, seconds = 48000, 4
    sound = tmp_path / "crest.wav"
    with wave.open(str(sound), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        data = bytearray()
        for n in range(rate * seconds):
            # A measurable bed and sparse high peaks require real limiting after the gain.
            value = 0.8 if n % rate == rate // 2 else 0.1 * math.sin(2 * math.pi * 1000 * n / rate)
            sample = round(value * 32767)
            data.extend(struct.pack("<hh", sample, sample))
        wav.writeframes(data)
    project_dir = tmp_path / "project"
    async with await Engine.attach(project_dir, program=real_engine(), idle_seconds=1) as wire:
        _, media = await wire.imported([sound])
        add: dict[str, Any] = {
            "op": "add_timeline",
            "timeline": {
                "id": "$mix",
                "name": "Mix",
                "width": 160,
                "height": 120,
                "rate": {"num": 25, "den": 1},
                "sample_rate": rate,
                "tracks": [],
            },
        }
        created = await wire.apply([add, edits.add_track("$mix", "audio")])
        timeline = created.names["$mix"]
        await wire.apply([edits.append(timeline, edits.of_media(media[0], length=25 * seconds))])
        before_revision, before = await wire.project()
        result = await Loudness(wire, Path).run(action="apply", target="main", standard="youtube")
        print("mix optimization", result)
        assert result["met"] is True
        assert any(op["op"] == "add_insert" for op in result["ops"])
        actual = await Loudness(wire, Path).run(action="measure", target="main", standard="youtube")
        row = actual["results"][0]
        assert abs(row["integrated"] + 14) <= 0.3
        assert row["true_peak"] <= -0.8
        revision, _ = await wire.project()
        assert revision == before_revision + 1
        repeated = await Loudness(wire, Path).run(action="apply", target="main", standard="youtube")
        assert repeated["status"] == "unchanged"
        assert repeated["met"] is True
        assert repeated["ops"] == []
        assert (await wire.project())[0] == revision
        assert await wire.undo() is not None
        _, undone = await wire.project()
        assert undone["timelines"] == before["timelines"]
    for _ in range(100):
        if not (project_dir / "engine.lock").exists():
            break
        await asyncio.sleep(0.1)
    assert not (project_dir / "engine.lock").exists()
