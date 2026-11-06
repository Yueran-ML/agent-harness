"""Real Ducker, stateful range verification and a single undo, with synthetic local audio."""

import asyncio
import math
import struct
import wave
from pathlib import Path
from typing import Any

import pytest

from columbina.video import edits
from columbina.video.duck import Duck
from columbina.video.engine import Engine
from columbina.video.transcript import Transcript, Word
from columbina.video.words import Words, keep_words
from support.media import real_engine


@pytest.mark.parametrize(
    ("mode", "background_count", "pan_silences_dialogue", "short_tail"),
    [
        ("ducker", 1, False, False),
        ("ducker", 2, False, False),
        ("keyframes", 2, False, False),
        ("ducker", 2, True, False),
        ("keyframes", 2, True, False),
        ("ducker", 1, False, True),
        ("keyframes", 2, False, True),
    ],
)
async def test_duck_verifies_margin_recovery_and_undoes_once(
    tmp_path: Path, mode: str, background_count: int, pan_silences_dialogue: bool, short_tail: bool
) -> None:
    rate, seconds = 48000, 8
    length = (125 if mode == "ducker" else 120) if short_tail else 200
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
                left = 0 if name == "voice" and pan_silences_dialogue else sample
                data.extend(struct.pack("<hh", left, sample))
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
                *[edits.add_track("$mix", "audio") for _ in range(background_count)],
                edits.add_track("$mix", "audio"),
            ]
        )
        timeline = created.names["$mix"]
        _, initial = await wire.project()
        tracks = initial["timelines"][0]["tracks"]
        voice, music = tracks[0]["id"], tracks[1]["id"]
        backgrounds = [track["id"] for track in tracks[1:]]
        await wire.apply(
            [
                edits.append(timeline, edits.of_media(media[0], length=length), audio_track=voice),
                *[
                    edits.append(
                        timeline, edits.of_media(media[1], length=length), audio_track=track
                    )
                    for track in backgrounds
                ],
            ]
        )
        if mode == "keyframes":
            _, snapshot = await wire.project()
            await wire.apply(
                [
                    {
                        "op": "set_clip",
                        "timeline": timeline,
                        "id": track["clips"][0]["id"],
                        "reverse": True,
                        "gain_db": {
                            "keys": [
                                {
                                    "at": {"ticks": 0, "scale": rate},
                                    "value": -1,
                                    "interp": "linear",
                                },
                                {
                                    "at": {"ticks": 8 * rate, "scale": rate},
                                    "value": 1,
                                    "interp": "linear",
                                },
                            ]
                        },
                    }
                    for track in snapshot["timelines"][0]["tracks"][1:]
                ]
            )
        if pan_silences_dialogue:
            await wire.apply(
                [
                    {
                        "op": "set_track_audio",
                        "timeline": timeline,
                        "track": voice,
                        "patch": {"strip": {"pan": {"on": True, "x": -1}}},
                    }
                ]
            )
        before_revision, before = await wire.project()
        if pan_silences_dialogue:
            voice_levels = await wire.analyze_sound(
                "audio.analyze",
                {
                    "timeline": timeline,
                    "revision": before_revision,
                    "targets": [{"track": voice}, {"sum": [voice]}],
                    "range": {"from": 50, "to": 100},
                },
                session=wire.session,
            )
            pre, post = voice_levels["results"]
            print("dialogue pan regression", pre["integrated"], post["integrated"])
            assert pre["integrated"] is not None
            assert post["integrated"] is None
        if background_count > 1:
            correlated = await wire.analyze_sound(
                "audio.analyze",
                {
                    "timeline": timeline,
                    "revision": before_revision,
                    "targets": [{"track": music}, {"sum": backgrounds}],
                    "range": {"from": 50, "to": 100},
                },
                session=wire.session,
            )
            each, combined = correlated["results"]
            print("correlated actual sum", each["integrated"], combined["integrated"])
            assert combined["integrated"] - each["integrated"] == pytest.approx(
                20 * math.log10(2), abs=0.05
            )
            assert await wire.project() == (before_revision, before)
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
        result = await Duck(wire, directory, Path).run(music=backgrounds, mode=mode)
        print("verified duck", result)
        assert result["met"] is (None if pan_silences_dialogue else True)
        assert result["verification_complete"] is (not pan_silences_dialogue)
        assert result["measurement_point"] == "stereo_post_pan"
        if short_tail:
            assert any(
                row["kind"] == "quiet"
                and row["metric"] == "k_weighted_mean"
                and row["range"][1] - row["range"][0] < 10
                for row in result["checks"]
            )
        if pan_silences_dialogue:
            assert all(
                row["margin_lu"] is None for row in result["checks"] if row["kind"] == "speech"
            )
        assert result["dialogue"] == voice
        if background_count > 1:
            assert result["adjustments"] == (0 if pan_silences_dialogue else 2)
            assert all(
                amount == (12 if pan_silences_dialogue else 18)
                for amount in result["amount_db"].values()
            )
            assert any("sum" in row and row["kind"] == "speech" for row in result["checks"])
        assert "analysis-sum" not in str(result["ops"])
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
        assert current["timelines"][0]["buses"] == before["timelines"][0]["buses"]
        assert await wire.undo() is not None
        _, undone = await wire.project()
        assert undone["timelines"] == before["timelines"]
    for _ in range(100):
        if not (directory / "engine.lock").exists():
            break
        await asyncio.sleep(0.1)
    assert not (directory / "engine.lock").exists()
