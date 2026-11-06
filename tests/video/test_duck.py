"""Ducker proposals reuse word mapping and commit only the last measured candidate."""

import asyncio
import copy
from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock

import pytest

from columbina.video.duck import Duck, quiet_ranges, speech
from columbina.video.edits import Rate
from columbina.video.engine import Applied, Engine, EngineError
from columbina.video.transcript import Transcript, Word
from columbina.video.words import Words, keep_words


def project() -> dict[str, Any]:
    def track(id: str, media: str) -> dict[str, Any]:
        return {
            "id": id,
            "kind": "audio",
            "enabled": True,
            "clips": [
                {
                    "id": "c" + id,
                    "start": 0,
                    "length": 200,
                    "source": {"kind": "media", "media": media, "in": {"ticks": 0, "scale": 48000}},
                }
            ],
            "strip": {"fader_db": -3, "inserts": [{"id": "kept"}]},
        }

    return {
        "media": [
            {"id": "voice", "path": "voice.wav", "identity": {"digest": "a" * 64}},
            {"id": "music", "path": "music.wav", "identity": {"digest": "b" * 64}},
        ],
        "timelines": [
            {
                "id": "t1",
                "rate": {"num": 25, "den": 1},
                "tracks": [track("A1", "voice"), track("A2", "music")],
            }
        ],
    }


def transcript() -> Transcript:
    return Transcript(
        asset="voice",
        digest="a" * 64,
        words=(
            Word(start_us=2_000_000, end_us=3_000_000, text="speech"),
            Word(start_us=3_000_000, end_us=4_000_000, text="continues"),
        ),
        sentences=((0, 2),),
    )


def setup(tmp_path: Path, *, words: bool = True, margin: float = 12) -> AsyncMock:
    wire = AsyncMock(spec=Engine)
    wire.session = 3
    wire.project.return_value = (7, project())
    wire.apply.return_value = Applied(8, [], 1)
    if words:
        keep_words(tmp_path, Words.of(transcript()))

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        return {
            "results": [
                {"target": {"track": "A1"}, "integrated": -18, "sample_peak": -12},
                {"target": {"track": "A2"}, "integrated": -18 - margin, "sample_peak": -20},
            ]
        }

    wire.analyze_sound.side_effect = measure
    return wire


async def test_no_words_does_not_guess_or_change_any_track(tmp_path: Path) -> None:
    wire = setup(tmp_path, words=False)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["status"] == "needs_dialogue"
    assert "video_transcribe" in result["notice"]
    wire.apply.assert_not_awaited()
    wire.analyze_sound.assert_not_awaited()


async def test_explicit_source_without_words_is_applied_but_never_claimed_verified(
    tmp_path: Path,
) -> None:
    wire = setup(tmp_path, words=False)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"], dialogue="A1")
    assert result["met"] is None
    assert result["verification_complete"] is False
    assert result["checks"] == []
    wire.analyze_sound.assert_awaited_once()
    wire.apply.assert_awaited_once()


async def test_candidate_verifies_speech_and_quiet_with_history_before_one_apply(
    tmp_path: Path,
) -> None:
    wire = setup(tmp_path)
    original = copy.deepcopy(wire.project.return_value)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["met"] is True
    assert result["dialogue"] == "A1"
    assert result["adjustments"] == 0
    ops = result["ops"]
    assert ops[0]["patch"] == {
        "strip": {
            "track_fx": {
                "on": True,
                "sources": [{"track": "A1"}],
                "amount_db": 12,
                "recovery_ms": 600,
                "lookahead_ms": 15,
                "rise_ms": 10,
                "hold_ms": 150,
            }
        }
    }
    wire.apply.assert_awaited_once_with(ops, revision=7, session=3)
    assert original == wire.project.return_value
    calls = wire.analyze_sound.call_args_list
    assert all(call.args[1]["revision"] == 7 and call.kwargs["session"] == 3 for call in calls)
    assert calls[-1].args[1]["candidate"] == ops


async def test_unmet_margin_stops_after_exactly_two_three_db_adjustments(tmp_path: Path) -> None:
    wire = setup(tmp_path, margin=4)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["met"] is False
    assert result["adjustments"] == 2
    assert result["amount_db"] == {"A2": 18}
    candidates = [
        call.args[1]["candidate"]
        for call in wire.analyze_sound.call_args_list
        if "candidate" in call.args[1]
    ]
    assert {ops[0]["patch"]["strip"]["track_fx"]["amount_db"] for ops in candidates} == {12, 15, 18}
    wire.apply.assert_awaited_once_with(candidates[-1], revision=7, session=3)


@pytest.mark.parametrize("failure", [asyncio.CancelledError(), EngineError("stale", code=-32001)])
async def test_cancel_or_failed_candidate_never_applies(
    tmp_path: Path, failure: BaseException
) -> None:
    wire = setup(tmp_path)
    wire.analyze_sound.side_effect = failure
    with pytest.raises(type(failure)):
        await Duck(wire, tmp_path, Path).run(music=["A2"])
    wire.apply.assert_not_awaited()


async def test_locked_background_and_existing_fx_automation_give_actionable_errors(
    tmp_path: Path,
) -> None:
    wire = setup(tmp_path)
    music = wire.project.return_value[1]["timelines"][0]["tracks"][1]
    music["locked"] = True
    with pytest.raises(EngineError, match="Unlock"):
        await Duck(wire, tmp_path, Path).run(music=["A2"])
    music["locked"] = False
    music["lanes"] = {"track_fx.amount_db": {"points": [{"value": 3}]}}
    with pytest.raises(EngineError, match="automation Off"):
        await Duck(wire, tmp_path, Path).run(music=["A2"])
    wire.analyze_sound.assert_not_awaited()
    wire.apply.assert_not_awaited()


def test_word_mapping_uses_source_trim_speed_and_reverse() -> None:
    raw = project()["timelines"][0]["tracks"][0]
    clip = raw["clips"][0]
    clip.update(start=100, length=50, speed={"num": 2, "den": 1}, reverse=True)
    score, spans = speech(raw, Rate(num=25, den=1), {"voice": transcript()})
    assert score == 2 / 50
    assert spans == [(100, 125)]
    assert quiet_ranges([(50, 100)], 200, Rate(num=25, den=1)) == [(0, 49), (119, 200)]


async def test_two_backgrounds_each_twelve_lu_lower_do_not_prove_their_sum(tmp_path: Path) -> None:
    wire = setup(tmp_path)
    tracks = wire.project.return_value[1]["timelines"][0]["tracks"]
    tracks.append({**copy.deepcopy(tracks[1]), "id": "A3"})

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        return {
            "results": [
                {
                    "target": target,
                    "integrated": -18 if target == {"track": "A1"} else -30,
                    "sample_peak": -12,
                }
                for target in params["targets"]
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(music=["A2", "A3"])
    assert result["per_track_met"] is True
    assert result["met"] is None
    assert result["verification_complete"] is False
    assert "combined mix" in result["notice"]
    wire.apply.assert_awaited_once()
