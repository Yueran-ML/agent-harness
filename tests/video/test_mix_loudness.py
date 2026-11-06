"""Bounded loudness proposals are measured without editing the live mix."""

import asyncio
import copy
from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock

import pytest

from columbina.video.audio import Loudness
from columbina.video.engine import Applied, Engine, EngineError


def mix() -> dict[str, Any]:
    return {
        "media": [],
        "timelines": [
            {
                "id": "t1",
                "name": "Mix",
                "rate": {"num": 25, "den": 1},
                "tracks": [
                    {
                        "id": "A1",
                        "kind": "audio",
                        "name": "Voice",
                        "clips": [],
                        "strip": {"fader_db": -6},
                    }
                ],
                "buses": [
                    {
                        "id": "MAIN",
                        "name": "Main",
                        "format": "stereo",
                        "strip": {"outputs": [], "fader_db": 2, "inserts": []},
                    }
                ],
            }
        ],
    }


def measured(target: dict[str, str], level: float | None, peak: float | None) -> dict[str, Any]:
    return {
        "standard": "youtube",
        "level": -14,
        "results": [
            {"target": target, "integrated": level, "true_peak": peak},
        ],
    }


def engine(project: dict[str, Any], answers: list[dict[str, Any]]) -> AsyncMock:
    stub = AsyncMock(spec=Engine)
    stub.session = 3
    stub.project.return_value = (7, project)
    stub.audio_standards.return_value = [
        {"id": "youtube", "target": -14, "true_peak": -1, "official": False, "verified": False}
    ]
    stub.analyze_sound.side_effect = answers
    stub.apply.return_value = Applied(8, [], 1)
    return stub


async def test_track_gain_preserves_lane_points_and_applies_only_the_verified_candidate() -> None:
    project = mix()
    lane: dict[str, Any] = {
        "kind": "continuous",
        "scale": "linear",
        "points": [
            {"at": {"ticks": 0, "scale": 48000}, "value": -8},
            {"at": {"ticks": 48000, "scale": 48000}, "value": -4},
        ],
    }
    project["timelines"][0]["tracks"][0]["lanes"] = {"fader_db": lane}
    original = copy.deepcopy(project)
    wire = engine(
        project, [measured({"track": "A1"}, -20, -12), measured({"track": "A1"}, -14, -6)]
    )
    result = await Loudness(wire, Path).run(action="apply", target="track_fader", ids=["A1"])
    assert project == original
    assert result["status"] == "applied"
    assert result["met"] is True
    proposal = wire.analyze_sound.call_args_list[1].args[1]["candidate"]
    assert proposal == [
        {
            "op": "set_lane",
            "timeline": "t1",
            "of": {"track": "A1"},
            "path": "fader_db",
            "lane": {
                **lane,
                "points": [
                    {**lane["points"][0], "value": -2},
                    {**lane["points"][1], "value": 2},
                ],
            },
        }
    ]
    wire.apply.assert_awaited_once_with(proposal, revision=7, session=3)


async def test_main_inserts_limiter_with_post_fader_compensation_and_measures_again() -> None:
    wire = engine(
        mix(),
        [
            measured({"bus": "MAIN"}, -20, -4),
            measured({"bus": "MAIN"}, -14, 2),
            measured({"bus": "MAIN"}, -14.1, -1.2),
        ],
    )
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["met"]
    assert "Unofficial" in result["notice"]
    proposal = wire.apply.call_args.args[0]
    assert proposal[0]["patch"]["strip"]["fader_db"] == 8
    limiter = proposal[-1]
    assert limiter["op"] == "add_insert"
    assert limiter["of"] == {"bus": "MAIN"}
    assert limiter["insert"]["effect"]["ceiling_db"] == -9  # -1 after the +8 dB fader
    assert len(wire.analyze_sound.call_args_list) == 3


async def test_main_reports_unmet_target_after_one_gain_correction() -> None:
    wire = engine(
        mix(),
        [
            measured({"bus": "MAIN"}, -20, -4),
            measured({"bus": "MAIN"}, -14, 2),
            measured({"bus": "MAIN"}, -16, -1.1),
            measured({"bus": "MAIN"}, -15.5, -1.1),
        ],
    )
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["status"] == "applied"
    assert result["met"] is False
    assert result["gain_corrections"] == 1
    assert len(wire.analyze_sound.call_args_list) == 4
    wire.apply.assert_awaited_once()


async def test_main_already_within_reported_tolerance_adds_no_history_or_limiter() -> None:
    project = mix()
    project["timelines"][0]["buses"][0]["strip"]["inserts"] = [
        {"id": "fx1", "on": True, "effect": {"kind": "limiter", "ceiling_db": -3}}
    ]
    wire = engine(project, [measured({"bus": "MAIN"}, -14.1, -0.9)])
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["status"] == "unchanged"
    assert result["met"] is True
    assert result["ops"] == []
    wire.apply.assert_not_awaited()
    wire.analyze_sound.assert_awaited_once()


async def test_main_reuses_the_last_active_limiter_in_a_full_chain() -> None:
    project = mix()
    inserts: list[dict[str, Any]] = [
        {"id": f"fx{i}", "on": True, "effect": {"kind": "gain", "gain_db": 0}} for i in range(5)
    ]
    inserts.append(
        {
            "id": "limit",
            "on": True,
            "effect": {"kind": "limiter", "ceiling_db": -1, "input_gain_db": 0, "release_ms": 250},
        }
    )
    project["timelines"][0]["buses"][0]["strip"]["inserts"] = inserts
    original = copy.deepcopy(project)
    wire = engine(
        project,
        [
            measured({"bus": "MAIN"}, -20, -4),
            measured({"bus": "MAIN"}, -14, 2),
            measured({"bus": "MAIN"}, -14.1, -1.1),
        ],
    )
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["met"] is True
    changed = result["ops"][-1]
    assert changed == {
        "op": "set_insert",
        "timeline": "t1",
        "of": {"bus": "MAIN"},
        "slot": 5,
        "insert": {**inserts[-1], "effect": {**inserts[-1]["effect"], "ceiling_db": -9}},
    }
    assert all(op["op"] != "add_insert" for op in result["ops"])
    assert project == original
    wire.apply.assert_awaited_once()


async def test_main_does_not_stack_a_limiter_to_override_its_automation() -> None:
    project = mix()
    bus = project["timelines"][0]["buses"][0]
    bus["strip"]["inserts"] = [
        {"id": "fx1", "on": True, "effect": {"kind": "limiter", "ceiling_db": -1}}
    ]
    bus["lanes"] = {"insert.fx1.ceiling_db": {"points": [{"value": -1}]}}
    wire = engine(project, [measured({"bus": "MAIN"}, -20, -4), measured({"bus": "MAIN"}, -14, 2)])
    with pytest.raises(EngineError, match="automation"):
        await Loudness(wire, Path).run(action="apply", target="main")
    wire.apply.assert_not_awaited()


async def test_main_reports_unmet_peak_without_sending_an_empty_candidate() -> None:
    project = mix()
    project["timelines"][0]["buses"][0]["strip"]["inserts"] = [
        {"id": "fx1", "on": True, "effect": {"kind": "limiter", "ceiling_db": -3}}
    ]
    wire = engine(project, [measured({"bus": "MAIN"}, -14, -0.5)])
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["status"] == "unchanged"
    assert result["met"] is False
    assert result["ops"] == []
    wire.analyze_sound.assert_awaited_once()
    wire.apply.assert_not_awaited()


async def test_silence_and_full_insert_chain_do_not_mutate_the_live_project() -> None:
    wire = engine(mix(), [measured({"bus": "MAIN"}, None, None)])
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["status"] == "unchanged"
    assert result["met"] is False
    wire.apply.assert_not_awaited()
    project = mix()
    project["timelines"][0]["buses"][0]["strip"]["inserts"] = [{"id": f"fx{i}"} for i in range(6)]
    wire = engine(project, [measured({"bus": "MAIN"}, -20, -4), measured({"bus": "MAIN"}, -14, 2)])
    with pytest.raises(EngineError, match="six"):
        await Loudness(wire, Path).run(action="apply", target="main")
    wire.apply.assert_not_awaited()


async def test_final_revision_refusal_is_not_retried_or_rebased() -> None:
    wire = engine(mix(), [measured({"bus": "MAIN"}, -20, -12), measured({"bus": "MAIN"}, -14, -6)])
    wire.apply.side_effect = EngineError("changed", code=-32001, data={"reason": "stale_revision"})
    with pytest.raises(EngineError, match="changed"):
        await Loudness(wire, Path).run(action="apply", target="main")
    wire.apply.assert_awaited_once()


async def test_off_automation_is_kept_and_static_fader_clamping_is_reported() -> None:
    project = mix()
    track = project["timelines"][0]["tracks"][0]
    track.update(
        automation_arm="off",
        lanes={"fader_db": {"points": [{"at": {"ticks": 0, "scale": 1}, "value": -9}]}},
    )
    track["strip"]["fader_db"] = 8
    wire = engine(
        project, [measured({"track": "A1"}, -20, -12), measured({"track": "A1"}, -18, -10)]
    )
    result = await Loudness(wire, Path).run(action="apply", target="track_fader", ids=["A1"])
    assert result["results"][0]["clamped"]
    assert result["met"] is False
    assert result["ops"] == [
        {
            "op": "set_track_audio",
            "timeline": "t1",
            "track": "A1",
            "patch": {"strip": {"fader_db": 10}},
        }
    ]


async def test_vca_post_gain_counts_shared_ancestors_once_when_placing_a_limiter() -> None:
    project = mix()
    timeline = project["timelines"][0]
    timeline["buses"][0]["strip"]["vcas"] = ["V1", "V2"]
    timeline["vcas"] = [
        {"id": "V1", "fader_db": 1, "vcas": ["V3"]},
        {"id": "V2", "fader_db": 2, "vcas": ["V3"]},
        {"id": "V3", "fader_db": 3},
    ]
    wire = engine(
        project,
        [
            measured({"bus": "MAIN"}, -20, -4),
            measured({"bus": "MAIN"}, -14, 2),
            measured({"bus": "MAIN"}, -14.1, -1.2),
        ],
    )
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["ops"][-1]["insert"]["effect"]["ceiling_db"] == -15  # -1 - fader 8 - VCAs 6


async def test_main_without_peak_limit_still_gets_one_measured_gain_correction() -> None:
    wire = engine(
        mix(),
        [
            measured({"bus": "MAIN"}, -20, -12),
            measured({"bus": "MAIN"}, -15, -7),
            measured({"bus": "MAIN"}, -14, -6),
        ],
    )
    wire.audio_standards.return_value[0]["true_peak"] = None
    result = await Loudness(wire, Path).run(action="apply", target="main")
    assert result["gain_corrections"] == 1
    assert result["met"]
    assert all(op["op"] != "add_insert" for op in result["ops"])


async def test_cancelling_candidate_analysis_never_applies_or_rolls_back_live_changes() -> None:
    wire = engine(mix(), [measured({"bus": "MAIN"}, -20, -12)])
    wire.analyze_sound.side_effect = [measured({"bus": "MAIN"}, -20, -12), asyncio.CancelledError()]
    with pytest.raises(asyncio.CancelledError):
        await Loudness(wire, Path).run(action="apply", target="main")
    wire.apply.assert_not_awaited()
