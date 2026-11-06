"""Real Ducker, stateful range verification and a single undo, with synthetic local audio."""

import asyncio
import math
import struct
import wave
from pathlib import Path
from typing import Any

from columbina.video import edits
from columbina.video.duck import Duck
from columbina.video.engine import Engine
from columbina.video.transcript import Transcript, Word
from columbina.video.words import Words, keep_words
from support.media import real_engine


async def test_duck_verifies_margin_recovery_and_undoes_once(tmp_path: Path) -> None:
    rate, seconds = 48000, 8
    sources: list[Path] = []
    for name in ("voice", "music"):
        path = tmp_path / f"{name}.wav"
        sources.append(path)
        with wave.open(str(path), "wb") as wav:
            wav.setnchannels(2)
            wav.setsampwidth(2)
            wav.setframerate(rate)
            data = bytearray()
            for n in range(rate * seconds):
                active = name == "music" or 2 * rate <= n < 4 * rate
                value = 0.15 * math.sin(2 * math.pi * 1000 * n / rate) if active else 0
                sample = round(value * 32767)
                data.extend(struct.pack("<hh", sample, sample))
            wav.writeframes(data)
    directory = tmp_path / "project"
    async with await Engine.attach(directory, program=real_engine(), idle_seconds=1) as wire:
        _, media = await wire.imported(sources)
        add: dict[str, Any] = {
            "op": "add_timeline",
            "timeline": {
                "id": "$mix",
                "name": "Ducking",
                "width": 160,
                "height": 120,
                "rate": {"num": 25, "den": 1},
                "sample_rate": rate,
                "tracks": [],
            },
        }
        created = await wire.apply(
            [
                add,
                edits.add_track("$mix", "audio"),
                edits.add_track("$mix", "audio"),
            ]
        )
        timeline = created.names["$mix"]
        _, initial = await wire.project()
        tracks = initial["timelines"][0]["tracks"]
        voice, music = tracks[0]["id"], tracks[1]["id"]
        await wire.apply(
            [
                edits.append(timeline, edits.of_media(media[0], length=200), audio_track=voice),
                edits.append(timeline, edits.of_media(media[1], length=200), audio_track=music),
            ]
        )
        before_revision, before = await wire.project()
        source = next(item for item in before["media"] if item["id"] == media[0])
        keep_words(
            directory,
            Words.of(
                Transcript(
                    asset=media[0],
                    digest=source["identity"]["digest"],
                    words=(
                        Word(start_us=2_000_000, end_us=3_000_000, text="voice"),
                        Word(start_us=3_000_000, end_us=4_000_000, text="continues"),
                    ),
                    sentences=((0, 2),),
                )
            ),
        )
        result = await Duck(wire, directory, Path).run(music=[music])
        print("verified duck", result)
        assert result["met"] is True
        assert result["verification_complete"] is True
        assert result["dialogue"] == voice
        revision, current = await wire.project()
        assert revision == before_revision + 1
        params: dict[str, Any] = {
            "timeline": timeline,
            "targets": [{"track": music}],
            "revision": revision,
            "range": {"from": 100, "to": 110},
        }
        recovery = await wire.analyze_sound("audio.analyze", params, session=wire.session)
        print("recovery", recovery["results"])
        # Graph.start already prerolls Ducker's settling interval. Do not add another
        # warmup path: the tail after speech must still be attenuated at this seek.
        assert recovery["results"][0]["sample_peak"] < -20
        assert recovery["results"][0]["seconds"] == 0.4
        assert current["timelines"] != before["timelines"]
        assert await wire.undo() is not None
        _, undone = await wire.project()
        assert undone["timelines"] == before["timelines"]
    for _ in range(100):
        if not (directory / "engine.lock").exists():
            break
        await asyncio.sleep(0.1)
    assert not (directory / "engine.lock").exists()
