"""Ducker proposals reuse word mapping and commit only the best measured candidate."""

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
                {"target": {"sum": ["A1"]}, "integrated": -18, "sample_peak": -12},
                {"target": {"sum": ["A2"]}, "integrated": -18 - margin, "sample_peak": -20},
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


@pytest.mark.parametrize(
    ("kind", "first", "second", "metric", "met"),
    [
        ("quiet", {"k_weighted_mean": -30}, {"k_weighted_mean": -30.5}, "k_weighted_mean", True),
        (
            "quiet",
            {"integrated": -28, "k_weighted_mean": -30},
            {"k_weighted_mean": -30.5},
            "k_weighted_mean",
            True,
        ),
        ("quiet", {"silence": True}, {"silence": True}, "silence", True),
        ("quiet", {"silence": True}, {"k_weighted_mean": -30}, "silence_changed", False),
        (
            "quiet",
            {"sample_peak": -12, "seconds": 0},
            {"sample_peak": -12, "seconds": 0},
            "unavailable",
            None,
        ),
        (
            "speech",
            {"integrated": -18, "k_weighted_mean": -20},
            {"k_weighted_mean": -32},
            "k_weighted_mean",
            True,
        ),
        ("speech", {"k_weighted_mean": -80}, {"k_weighted_mean": -92}, "k_weighted_mean", True),
        ("speech", {"silence": True}, {"k_weighted_mean": -32}, "unavailable", None),
        ("speech", {"integrated": -18}, {"silence": True}, "silent_background", True),
    ],
)
async def test_short_and_silent_intervals_use_matching_loudness_metrics(
    tmp_path: Path,
    kind: str,
    first: dict[str, Any],
    second: dict[str, Any],
    metric: str,
    met: bool | None,
) -> None:
    wire = setup(tmp_path)

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        speaking = params.get("range") == {"from": 50, "to": 100}
        voice, background = {"integrated": -18}, {"integrated": -30}
        if speaking and kind == "speech":
            voice, background = first, second
        elif not speaking and kind == "quiet":
            background = second if "candidate" in params else first
        return {
            "results": [
                {"target": {"sum": ["A1"]}, **voice},
                {"target": {"sum": ["A2"]}, **background},
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    checks = [row for row in result["checks"] if row["kind"] == kind]
    assert checks
    assert all(row["metric"] == metric and row["met"] is met for row in checks)
    assert result["met"] is met
    assert result["verification_complete"] is (met is not None)
    wire.apply.assert_awaited_once()


def amount(params: dict[str, Any]) -> float:
    """The candidate's Ducker amount, or 0 for a measurement without a candidate."""
    ops: list[Any] = params.get("candidate") or []
    return ops[0]["patch"]["strip"]["track_fx"]["amount_db"] if ops else 0


async def test_unmet_margin_stops_after_exactly_two_three_db_adjustments(tmp_path: Path) -> None:
    wire = setup(tmp_path)

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        # During speech each 3 dB of duck gains 1 LU of margin, never reaching 10 LU.
        speaking = params.get("range") == {"from": 50, "to": 100}
        margin = 4 + (amount(params) - 12) / 3 if speaking else 4
        return {
            "results": [
                {"target": {"sum": ["A1"]}, "integrated": -18},
                {"target": {"sum": ["A2"]}, "integrated": -18 - margin},
            ]
        }

    wire.analyze_sound.side_effect = measure
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


async def test_two_backgrounds_each_twelve_lu_lower_are_judged_by_real_sum(tmp_path: Path) -> None:
    wire = setup(tmp_path)
    tracks = wire.project.return_value[1]["timelines"][0]["tracks"]
    tracks.append({**copy.deepcopy(tracks[1]), "id": "A3"})

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        # Each 3 dB of duck lowers the sum 1 LU; it stays above the 10 LU margin.
        speaking = params.get("range") == {"from": 50, "to": 100}
        gained = (amount(params) - 12) / 3 if speaking else 0
        return {
            "results": [
                {
                    "target": target,
                    "integrated": -18
                    if target == {"sum": ["A1"]}
                    else -24 - gained
                    if len(target["sum"]) > 1
                    else -30,
                    "sample_peak": -12,
                }
                for target in params["targets"]
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(music=["A2", "A3"])
    assert result["per_track_met"] is True
    assert result["met"] is False
    assert result["verification_complete"] is True
    assert result["adjustments"] == 2
    assert result["amount_db"] == {"A2": 18, "A3": 18}
    assert (
        next(row for row in result["checks"] if row["kind"] == "speech" and "sum" in row)[
            "margin_lu"
        ]
        == 8
    )
    wire.apply.assert_awaited_once()


async def test_keyframes_keep_existing_keys_and_track_fx_with_one_transaction(
    tmp_path: Path,
) -> None:
    wire = setup(tmp_path)
    track = wire.project.return_value[1]["timelines"][0]["tracks"][1]
    track["lanes"] = {"track_fx.amount_db": {"points": [{"value": 3}]}}
    track["clips"][0]["gain_db"] = {
        "keys": [
            {"at": {"ticks": 0, "scale": 25}, "value": -2, "interp": "linear"},
            {"at": {"ticks": 200, "scale": 25}, "value": 2, "interp": "linear"},
        ]
    }
    original = copy.deepcopy(wire.project.return_value)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"], mode="keyframes")
    assert result["met"] is True
    assert result["mode"] == "keyframes"
    ops = result["ops"]
    assert len(ops) == 1
    assert ops[0]["op"] == "set_clip"
    assert set(ops[0]) == {"op", "timeline", "id", "gain_db"}
    keys = ops[0]["gain_db"]["keys"]
    assert keys[0] == track["clips"][0]["gain_db"]["keys"][0]
    assert keys[-1] == track["clips"][0]["gain_db"]["keys"][-1]
    wire.apply.assert_awaited_once_with(ops, revision=7, session=3)
    assert wire.project.return_value == original


async def test_keyframes_need_words_and_refuse_affected_easing_before_analysis(
    tmp_path: Path,
) -> None:
    wire = setup(tmp_path, words=False)
    result = await Duck(wire, tmp_path, Path).run(music=["A2"], dialogue="A1", mode="keyframes")
    assert result["status"] == "needs_transcript"
    keep_words(tmp_path, Words.of(transcript()))
    clip = wire.project.return_value[1]["timelines"][0]["tracks"][1]["clips"][0]
    clip["gain_db"] = {
        "keys": [
            {"at": {"ticks": 0, "scale": 1}, "value": 0, "interp": "ease_out"},
            {"at": {"ticks": 8, "scale": 1}, "value": 3, "interp": "linear"},
        ]
    }
    with pytest.raises(EngineError, match="mode='ducker'"):
        await Duck(wire, tmp_path, Path).run(music=["A2"], mode="keyframes")
    wire.analyze_sound.assert_not_awaited()
    wire.apply.assert_not_awaited()


async def test_maximum_backgrounds_split_analysis_jobs_but_apply_once(tmp_path: Path) -> None:
    wire = setup(tmp_path)
    tracks = wire.project.return_value[1]["timelines"][0]["tracks"]
    original = tracks[1]
    for number in range(3, 257):
        tracks.append({**copy.deepcopy(original), "id": f"A{number}"})
    ids = [track["id"] for track in tracks[1:]]

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        assert len(params["targets"]) <= 256
        return {
            "results": [
                {"target": target, "integrated": -18 if target == {"sum": ["A1"]} else -30}
                for target in params["targets"]
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(music=ids)
    assert result["met"] is True
    assert any(
        call.args[1]["targets"] == [{"sum": ids}] for call in wire.analyze_sound.call_args_list
    )
    assert len(result["ops"]) == 255
    wire.apply.assert_awaited_once()


@pytest.mark.parametrize("mode", ["ducker", "keyframes"])
@pytest.mark.parametrize("background_count", [1, 2])
async def test_post_pan_silent_dialogue_never_passes_even_with_loud_pre_pan_signal(
    tmp_path: Path, mode: str, background_count: int
) -> None:
    wire = setup(tmp_path)
    tracks = wire.project.return_value[1]["timelines"][0]["tracks"]
    if background_count == 2:
        tracks.append({**copy.deepcopy(tracks[1]), "id": "A3"})
        tracks[-1]["clips"][0]["id"] = "cA3"

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        return {
            "results": [
                {
                    "target": target,
                    "integrated": None
                    if target == {"sum": ["A1"]}
                    else -18
                    if target == {"track": "A1"}
                    else -36,
                    "sample_peak": None if target == {"sum": ["A1"]} else -12,
                }
                for target in params["targets"]
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(
        music=[track["id"] for track in tracks[1:]], mode=mode
    )
    assert result["met"] is None
    assert result["verification_complete"] is False
    assert result["measurement_point"] == "stereo_post_pan"
    assert all(row["margin_lu"] is None for row in result["checks"] if row["kind"] == "speech")
    assert all(
        "sum" in target
        for call in wire.analyze_sound.call_args_list
        for target in call.args[1]["targets"]
    )
    wire.apply.assert_awaited_once()


async def test_repeat_with_same_ducker_adds_no_history(tmp_path: Path) -> None:
    wire = setup(tmp_path)
    strip = wire.project.return_value[1]["timelines"][0]["tracks"][1]["strip"]
    strip["track_fx"] = {
        "on": True,
        "sources": [{"track": "A1"}],
        "amount_db": 12.0,
        "recovery_ms": 600.0,
        "lookahead_ms": 15.0,
        "rise_ms": 10.0,
        "hold_ms": 150.0,
    }
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["status"] == "unchanged"
    assert result["changed"] is False
    assert result["applied"] is None
    assert result["met"] is True
    wire.apply.assert_not_awaited()
    strip["track_fx"]["amount_db"] = 9.0
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["status"] == "applied"
    assert result["changed"] is True
    wire.apply.assert_awaited_once()


async def test_oscillating_adjustment_applies_the_best_measured_candidate(tmp_path: Path) -> None:
    """12 dB misses speech by 2 LU; 15 dB meets speech but moves one quiet range by 2 dB."""
    wire = setup(tmp_path)

    async def measure(_method: str, params: dict[str, Any], **_kwargs: Any) -> dict[str, Any]:
        duck = amount(params)
        frames = params.get("range")
        background = (
            -18 - (8 + duck - 12)
            if frames == {"from": 50, "to": 100}
            else -32
            if frames == {"from": 119, "to": 200} and duck > 12
            else -30
        )
        return {
            "results": [
                {"target": {"sum": ["A1"]}, "integrated": -18},
                {"target": {"sum": ["A2"]}, "integrated": background},
            ]
        }

    wire.analyze_sound.side_effect = measure
    result = await Duck(wire, tmp_path, Path).run(music=["A2"])
    assert result["amount_db"] == {"A2": 15}
    assert result["adjustments"] == 1
    assert [row["amount_db"] for row in result["candidates"]] == [{"A2": 12}, {"A2": 15}]
    assert [(row["failed"], row["shortfall_db"]) for row in result["candidates"]] == [
        (1, 2),
        (1, 1),
    ]
    assert result["met"] is False
    candidates = [
        call.args[1]["candidate"]
        for call in wire.analyze_sound.call_args_list
        if "candidate" in call.args[1]
    ]
    # The return to 12 dB is not measured twice.
    assert {ops[0]["patch"]["strip"]["track_fx"]["amount_db"] for ops in candidates} == {12, 15}
    wire.apply.assert_awaited_once_with(result["ops"], revision=7, session=3)
    assert result["ops"][0]["patch"]["strip"]["track_fx"]["amount_db"] == 15
