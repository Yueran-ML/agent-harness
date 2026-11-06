"""The loudness tool uses engine measurements and one curve-preserving Normalize job."""

import asyncio
import copy
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.tools import TOOLS, ToolRun
from columbina.video import VIDEOS
from columbina.video.engine import (
    Changes,
    Engine,
    EngineError,
    SoundAnalysis,
    _Wire,  # pyright: ignore[reportPrivateUsage]
)
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")


async def test_duck_tool_registration_and_no_transcript_hint(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    clip = fake.project["timelines"][0]["tracks"][0]["clips"][0]
    clip.update(start=0, length=100)
    clip["source"]["in"] = {"ticks": 0, "scale": 48000}
    tool = ctx.use(TOOLS).registry["video_duck"]
    assert tool.timeout_seconds == 6 * 60 * 60
    assert tool.effect == "write"
    said = result(await desk.call("video_duck", music=["A2"]))
    assert said["status"] == "needs_dialogue"
    assert "video_transcribe" in said["notice"]


# Deliberately not the real table: numeric defaults belong to the engine alone.
STANDARD = {
    "id": "bilibili",
    "name": "Bilibili",
    "target": -17.5,
    "tolerance": None,
    "true_peak": None,
    "measure": "gated",
    "dialogue": False,
    "resolve": False,
    "official": False,
    "verified": False,
    "source": "No published target",
}


@pytest.fixture
async def studio(
    ctx: Context,
    tmp_path: Path,
) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        fake.project["timelines"] = [
            {
                "id": "t1",
                "name": "Sound",
                "rate": {"num": 25, "den": 1},
                "width": 1920,
                "height": 1080,
                "sample_rate": 48000,
                "tracks": [
                    {
                        "id": "A1",
                        "kind": "audio",
                        "clips": [
                            {
                                "id": "c1",
                                "source": {"kind": "media", "media": "m1"},
                                "gain_db": {
                                    "keys": [
                                        {"at": {"ticks": 0, "scale": 25}, "value": -6},
                                        {"at": {"ticks": 25, "scale": 25}, "value": -3},
                                    ]
                                },
                            }
                        ],
                    }
                ],
                "buses": [{"id": "MAIN7", "name": "Main", "strip": {"outputs": []}}],
            }
        ]
        fake.project["media"] = [{"id": "m1", "path": str(desk.work / "sound.wav")}]

        fake.standards = [STANDARD]
        yield desk, fake


@pytest.mark.parametrize(
    ("target", "ids", "expected"),
    [
        ("clip", ["c1"], {"clip": "c1"}),
        ("track", ["A1"], {"track": "A1"}),
        ("bus", ["MAIN7"], {"bus": "MAIN7"}),
        ("main", [], {"bus": "MAIN7"}),
    ],
)
async def test_measure_resolves_targets_and_preserves_readings(
    studio: tuple[Desk, Fake],
    target: str,
    ids: list[str],
    expected: dict[str, str],
) -> None:
    desk, fake = studio
    readings = [
        {
            "target": expected,
            "integrated": None,
            "short_term_max": None,
            "momentary_max": -30,
            "range": None,
            "true_peak": -8,
            "sample_peak": -9,
        }
    ]
    fake.sound_found = {"standard": "bilibili", "level": -17.5, "results": readings}
    kwargs: dict[str, Any] = {} if target == "clip" else {"range": {"from": 10, "to": 40}}
    told = result(
        await desk.call("video_loudness", action="measure", target=target, ids=ids, **kwargs)
    )
    assert told["results"] == readings
    assert told["standard_info"] == STANDARD
    assert "unofficial" in told["notice"].lower()
    assert fake.sounds == [
        (
            "audio.analyze",
            {
                "timeline": "t1",
                "targets": [expected],
                "revision": 0,
                **kwargs,
            },
        )
    ]
    assert not fake.steps


async def test_main_requires_selection_when_there_are_multiple_outputs(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    fake.project["timelines"][0]["buses"].extend(
        [
            {"id": "ALT9", "name": "Alt", "strip": {"outputs": []}},
            {"id": "GROUP2", "name": "Group", "strip": {"outputs": [{"bus": "MAIN7"}]}},
        ]
    )
    failed = await desk.call("video_loudness", action="measure", target="main")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_AMBIGUOUS"
    assert "MAIN7" in failed.text
    assert "ALT9" in failed.text
    assert not fake.sounds
    result(await desk.call("video_loudness", action="measure", target="main", ids=["ALT9"]))
    assert fake.sounds[-1][1]["targets"] == [{"bus": "ALT9"}]
    failed = await desk.call("video_loudness", action="measure", target="main", ids=["GROUP2"])
    assert failed.error is not None
    assert failed.error.code == "VIDEO_MISSING"


async def test_clip_range_is_forwarded_in_timeline_frames(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    measured = await desk.call(
        "video_loudness", action="measure", target="clip", ids=["c1"], range={"from": 0, "to": 25}
    )
    assert measured.error is None
    assert fake.sounds[-1][1]["range"] == {"from": 0, "to": 25}


@pytest.mark.parametrize("mode", ["relative", "independent"])
async def test_normalize_binds_revision_and_returns_null_clamp_and_the_real_receipt(
    studio: tuple[Desk, Fake],
    mode: str,
) -> None:
    desk, fake = studio
    original = copy.deepcopy(fake.project)
    fake.sound_found = {
        "level": "bilibili",
        "target": -17.5,
        "mode": mode,
        "measured": None,
        "clips": [
            {"clip": "c1", "gain_db": 40, "measured": -57.5, "clamped": True},
            {"clip": "silent", "gain_db": None, "measured": None, "clamped": False},
        ],
        "applied": {"revision": 1, "done": [{"op": "set_clip", "gain_db": {"keys": []}}]},
    }
    told = result(
        await desk.call(
            "video_loudness",
            action="apply",
            target="clip_gain",
            ids=["c1"],
            mode=mode,
            standard="bilibili",
        )
    )
    assert fake.sounds == [
        (
            "audio.normalize",
            {
                "timeline": "t1",
                "clips": ["c1"],
                "level": "bilibili",
                "mode": mode,
                "apply": True,
                "revision": 0,
            },
        )
    ]
    assert told["clips"] == fake.sound_found["clips"]
    assert told["applied"] == fake.sound_found["applied"]
    assert told["status"] == "applied"
    # The tool delegates the curve operation entirely to Normalize; it never sets fixed gain.
    assert fake.project == original
    assert "transaction.apply" not in fake.asked


@pytest.mark.parametrize(
    "refused", [None, {"code": -32001, "message": "changed", "data": {"reason": "stale_revision"}}]
)
async def test_done_does_not_mean_any_gain_was_applied(
    studio: tuple[Desk, Fake], refused: Any
) -> None:
    desk, fake = studio
    fake.sound_found = {
        "clips": [{"clip": "c1", "gain_db": None, "clamped": False}],
        "applied": None,
    }
    if refused is not None:
        fake.sound_found["refused"] = refused
    told = result(await desk.call("video_loudness", action="apply", target="clip_gain", ids=["c1"]))
    assert told["status"] == ("unchanged" if refused is None else "refused")
    assert told.get("refused") == refused
    assert not fake.steps


@pytest.mark.parametrize("state", ["failed", "cancelled"])
async def test_terminal_failure_is_not_reported_as_measurement(
    studio: tuple[Desk, Fake], state: str
) -> None:
    desk, fake = studio
    fake.sound_end = state
    failed = await desk.call("video_loudness", action="measure", target="main")
    assert failed.error is not None
    assert state in failed.text


async def test_loudness_checks_media_read_permission(
    studio: tuple[Desk, Fake], tmp_path: Path
) -> None:
    desk, fake = studio
    fake.project["media"][0]["path"] = str(tmp_path / "outside.wav")
    failed = await desk.call("video_loudness", action="measure", target="main")
    assert failed.error is not None
    assert failed.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert not fake.sounds


async def test_loudness_is_long_and_uses_studio_permissions(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    tool = ctx.use(TOOLS).registry["video_loudness"]
    assert tool.timeout_seconds == 6 * 60 * 60
    assert tool.effect == "write"
    result(await desk.call("video_loudness", action="measure", target="main"))
    assert "audio.analyze" in fake.asked
    failed = await desk.call(
        "video_loudness", action="measure", target="main", project=".columbina/elsewhere"
    )
    assert failed.error is not None


async def test_analysis_session_binding_prevents_sending_to_restarted_engine(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    editor = await ctx.use(VIDEOS).editor(
        ToolRun("inspect", desk.work, desk.session), "video_loudness"
    )
    revision, _ = await editor.engine.project()
    session = editor.engine.session
    editor.engine._drop()  # pyright: ignore[reportPrivateUsage]
    with pytest.raises(EngineError) as refused:
        await editor.engine.analyze_sound(
            "audio.normalize",
            {"timeline": "t1", "clips": ["c1"], "apply": True, "revision": revision},
            session=session,
        )
    assert refused.value.data == {"reason": "stale_revision", "session": session + 1}
    assert not fake.sounds


async def test_tool_cancellation_stops_the_job(
    studio: tuple[Desk, Fake], ctx: Context, monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio
    fake.sound_hold = True
    editor = await ctx.use(VIDEOS).editor(
        ToolRun("inspect", desk.work, desk.session), "video_loudness"
    )
    from columbina.video.audio import Loudness

    started = asyncio.Event()
    sound = fake._sound  # pyright: ignore[reportPrivateUsage]

    def start(method: str, params: dict[str, Any]) -> dict[str, Any]:
        answer = sound(method, params)
        started.set()
        return answer

    monkeypatch.setattr(fake, "_sound", start)
    task = asyncio.create_task(
        Loudness(editor.engine, lambda path: Path(path)).run(action="measure", target="main")
    )
    await asyncio.wait_for(started.wait(), 5)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    assert fake.jobs["audio-1"] == "cancelled"


@pytest.mark.parametrize(
    "params",
    [
        {"action": "apply", "target": "main", "mode": "relative"},
        {"action": "apply", "target": "track_fader"},
        {"action": "apply", "target": "clip_gain", "ids": ["c1"], "mode": "optimize"},
        {"action": "measure", "target": "main", "mode": "relative"},
        {"action": "measure", "target": "main", "standard": "unknown"},
        {"action": "measure", "target": "main", "range": {"from": 40, "to": 10}},
    ],
)
async def test_only_supported_modes_are_accepted(
    studio: tuple[Desk, Fake], params: dict[str, Any]
) -> None:
    desk, fake = studio
    assert (await desk.call("video_loudness", **params)).error is not None
    assert not fake.sounds


async def test_early_completion_is_not_lost(
    studio: tuple[Desk, Fake], monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio

    def early(method: str, params: dict[str, Any]) -> dict[str, Any]:
        fake.tell("audio.analysis", {"job": "elsewhere", "state": "failed"})
        fake.tell(
            "audio.analysis", {"job": "early", "state": "done", "results": [], "method": method}
        )
        return {"job": "early"}

    monkeypatch.setattr(fake, "_sound", early)
    told = result(
        await asyncio.wait_for(desk.call("video_loudness", action="measure", target="main"), 5)
    )
    assert told["job"] == "early"
    assert told["status"] == "measured"


async def test_an_official_measure_without_target_uses_the_timeline_target(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    standard = {**STANDARD, "id": "bs1770-1", "target": None, "official": True}

    fake.standards = [standard]
    fake.sound_found = {"standard": "bs1770-1", "level": -23, "results": []}
    told = result(await desk.call("video_loudness", action="measure", target="main"))
    assert "timeline" in told["notice"].lower()
    assert "official published target" not in told["notice"].lower()


async def test_standards_and_project_are_bound_to_the_same_session(
    studio: tuple[Desk, Fake],
    ctx: Context,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    desk, fake = studio
    editor = await ctx.use(VIDEOS).editor(
        ToolRun("inspect", desk.work, desk.session), "video_loudness"
    )
    read_standards = editor.engine.audio_standards

    async def from_old_engine() -> list[dict[str, Any]]:
        table = await read_standards()
        editor.engine._drop()  # pyright: ignore[reportPrivateUsage]
        return table

    monkeypatch.setattr(editor.engine, "audio_standards", from_old_engine)
    failed = await desk.call("video_loudness", action="measure", target="main")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_STALE_REVISION"
    assert not fake.sounds


async def test_track_fader_tool_registration_uses_candidates_then_one_real_request(
    studio: tuple[Desk, Fake], monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio
    sound = fake._sound  # pyright: ignore[reportPrivateUsage]

    def measured(method: str, params: dict[str, Any]) -> dict[str, Any]:
        fake.sound_found = {
            "standard": "bilibili",
            "level": -17.5,
            "results": [
                {
                    "target": {"track": "A1"},
                    "integrated": -17.5 if "candidate" in params else -23.5,
                    "true_peak": -8,
                }
            ],
        }
        return sound(method, params)

    monkeypatch.setattr(fake, "_sound", measured)
    told = result(
        await desk.call("video_loudness", action="apply", target="track_fader", ids=["A1"])
    )
    assert told["met"]
    assert told["status"] == "applied"
    assert len(fake.steps) == 1
    assert fake.project["timelines"][0]["tracks"][0]["strip"]["fader_db"] == 6
    assert fake.sounds[0][1].get("candidate") is None
    assert fake.sounds[1][1]["candidate"] == told["ops"]


async def test_audio_info_is_a_compact_snapshot_without_analysis_or_asset_io(
    studio: tuple[Desk, Fake], ctx: Context, monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio
    timeline = fake.project["timelines"][0]
    track = timeline["tracks"][0]
    track.update(name="Voice", format="mono", enabled=False, mute=True, solo_safe=True)
    track["strip"] = {
        "fader_db": -4,
        "pan": {"x": 0.3},
        "inserts_on": False,
        "inserts": [{"id": "fx9", "effect": {"kind": "limiter"}}],
        "track_fx": {"on": True, "sources": [{"track": "A2"}]},
        "outputs": [{"bus": "MAIN7", "on": False}],
        "sends": [{"bus": "RETURN2", "level_db": -8, "pre": True}],
        "vcas": ["VCA2"],
    }
    track["lanes"] = {"fader_db": {"points": [{"at": {"ticks": 0, "scale": 25}, "value": 2}]}}
    timeline["tracks"].append({"id": "V1", "kind": "video", "clips": []})
    timeline["vcas"] = [{"id": "VCA2", "name": "Voices"}]
    timeline["track_groups"] = [{"id": "TG2", "name": "Speakers", "members": ["A1"]}]
    from columbina.video import grading, words

    def unwanted(*args: Any, **kwargs: Any) -> Any:
        pytest.fail("audio info must not inspect LUTs or transcripts")

    monkeypatch.setattr(grading, "missing_luts", unwanted)
    monkeypatch.setattr(words, "transcripts_of", unwanted)
    info = result(await desk.call("video_info", scope="audio"))
    assert info["scope"] == "audio"
    assert info["analysis_cache_source"] == "engine_client_calls"
    audio = info["timelines"][0]
    assert len(audio["tracks"]) == 1
    voice = audio["tracks"][0]
    assert (voice["id"], voice["name"], voice["format"]) == ("A1", "Voice", "mono")
    assert (voice["fader_db"], voice["pan"]["x"]) == (-4, 0.3)
    assert (voice["enabled"], voice["mute"], voice["solo"], voice["solo_safe"]) == (
        False,
        True,
        False,
        True,
    )
    assert voice["outputs"] == [{"bus": "MAIN7", "on": False}]
    assert voice["sends"][0]["level_db"] == -8
    assert voice["sends"][0]["on"] is True
    assert voice["inserts"] == [{"id": "fx9", "name": "limiter", "on": True}]
    assert voice["inserts_on"] is False
    assert voice["track_fx"] == {"name": "ducker", "on": True}
    assert voice["lanes"] == ["fader_db"]
    assert "clips" not in voice
    assert audio["buses"][0]["outputs"] == []
    assert audio["vcas"][0]["fader_db"] == 0
    assert audio["groups"][0]["members"] == ["A1"]
    assert audio["latest_analysis"] is None
    assert not fake.sounds
    assert ctx.use(TOOLS).registry["video_info"].effect == "read"


@pytest.mark.parametrize(
    ("buses", "strip", "outputs"),
    [
        ([{"id": "MAIN7", "name": "Main"}], None, []),
        ([{"id": "B1", "name": "Bus 1"}], None, [{"bus": "B1", "on": True}]),
        ([{"id": "B1", "name": "Bus 1"}], {}, []),
        ([], None, []),
    ],
)
async def test_audio_info_uses_model_defaults_for_omitted_strip(
    studio: tuple[Desk, Fake],
    buses: list[dict[str, Any]],
    strip: dict[str, Any] | None,
    outputs: list[dict[str, Any]],
) -> None:
    desk, fake = studio
    timeline = fake.project["timelines"][0]
    timeline["buses"] = buses
    if strip is not None:
        timeline["tracks"][0]["strip"] = strip
    audio = result(await desk.call("video_info", scope="audio"))["timelines"][0]
    track = audio["tracks"][0]
    assert track["outputs"] == outputs
    assert (track["format"], track["fader_db"], track["pan"]["x"]) == ("stereo", 0, 0)
    assert track["pan"]["spread"] == 99
    assert track["inserts"] == []
    assert track["lanes"] == []


async def test_audio_cache_is_success_only_revision_bound_and_isolated(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    other = copy.deepcopy(fake.project["timelines"][0])
    other["id"] = "t2"
    fake.project["timelines"].append(other)
    readings = [{"target": {"bus": "MAIN7"}, "integrated": None, "true_peak": -4}]
    fake.sound_found = {"results": readings}
    result(
        await desk.call(
            "video_loudness",
            action="measure",
            target="main",
            timeline="t1",
            range={"from": 10, "to": 40},
        )
    )
    info = result(await desk.call("video_info", scope="audio"))
    cached = info["timelines"][0]["latest_analysis"]
    assert cached["result"]["results"] == readings
    assert cached["request"]["range"] == {"from": 10, "to": 40}
    assert cached["range"] == {"from": 10, "to": 40}
    assert cached["request"]["targets"] == [{"bus": "MAIN7"}]
    assert cached["measured_revision"] == 0
    assert cached["session"] == info["session"]
    assert cached["completed_at"]
    assert cached["stale"] is False
    assert info["timelines"][1]["latest_analysis"] is None
    for state in ("failed", "cancelled"):
        fake.sound_end = state
        await desk.call("video_loudness", action="measure", target="main", timeline="t1")
        again = result(await desk.call("video_info", scope="audio", timeline="t1"))
        assert again["timelines"][0]["latest_analysis"] == cached
    fake.revision += 1
    stale = result(await desk.call("video_info", scope="audio", timeline="t1"))
    assert stale["timelines"][0]["latest_analysis"]["stale"] is True
    editor = await ctx.use(VIDEOS).editor(ToolRun("inspect", desk.work, desk.session), "video_info")
    editor.engine._drop()  # pyright: ignore[reportPrivateUsage]
    reconnected = result(await desk.call("video_info", scope="audio", timeline="t1"))
    assert reconnected["timelines"][0]["latest_analysis"] is None
    fake.sound_end = "done"
    result(await desk.call("video_loudness", action="measure", target="main", timeline="t1"))
    current = result(await desk.call("video_info", scope="audio", timeline="t1"))
    fresh = current["timelines"][0]["latest_analysis"]
    assert fresh["session"] == cached["session"] + 1
    assert fresh["measured_revision"] == fake.revision
    assert fresh["stale"] is False


async def test_normalization_cache_identifies_before_apply_measurements(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    fake.sound_found = {
        "clips": [{"clip": "c1", "measured": -30, "gain_db": 7, "clamped": False}],
        "applied": {"revision": 1},
    }
    result(await desk.call("video_loudness", action="apply", target="clip_gain", ids=["c1"]))
    fake.revision = 1
    cached = result(await desk.call("video_info", scope="audio"))["timelines"][0]["latest_analysis"]
    assert cached["measurement_phase"] == "before_apply"
    assert cached["measured_revision"] == 0
    assert cached["result"]["applied"]["revision"] == 1
    assert cached["stale"] is True
    assert cached["result"]["clips"][0]["measured"] == -30
    assert "after" not in cached["result"]


async def test_cache_does_not_claim_external_jobs_or_unbound_revision_are_current(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    editor = await ctx.use(VIDEOS).editor(ToolRun("inspect", desk.work, desk.session), "video_info")
    fake.tell("audio.analysis", {"job": "desktop-1", "state": "done", "results": []})
    assert editor.engine.latest_sound("t1", revision=0) is None
    params: dict[str, Any] = {"timeline": "t1", "targets": [{"bus": "MAIN7"}]}
    done = await editor.engine.analyze_sound("audio.analyze", params)
    params["targets"].clear()
    done["state"] = "tampered"
    cached = editor.engine.latest_sound("t1", revision=0)
    assert cached is not None
    assert cached["measured_revision"] is None
    assert cached["stale"] is True
    assert cached["range"] is None
    assert cached["request"]["targets"] == [{"bus": "MAIN7"}]
    assert cached["result"]["state"] == "done"
    cached["result"]["state"] = "mutated"
    reread = editor.engine.latest_sound("t1", revision=0)
    assert reread is not None
    assert reread["result"]["state"] == "done"


async def test_candidate_analysis_does_not_replace_actual_project_measurements(
    studio: tuple[Desk, Fake], ctx: Context
) -> None:
    desk, fake = studio
    editor = await ctx.use(VIDEOS).editor(ToolRun("inspect", desk.work, desk.session), "video_info")
    params: dict[str, Any] = {"timeline": "t1", "targets": [{"bus": "MAIN7"}], "revision": 0}
    await editor.engine.analyze_sound("audio.analyze", params)
    before = editor.engine.latest_sound("t1", revision=0)
    params["candidate"] = [
        {"edit": "set_bus", "timeline": "t1", "id": "MAIN7", "patch": {"strip": {"fader_db": 3}}}
    ]
    fake.sound_found = {"results": [{"target": {"bus": "MAIN7"}, "integrated": -14}]}
    result = await editor.engine.analyze_sound("audio.analyze", params)
    assert result["results"][0]["integrated"] == -14
    assert fake.sounds[-1][1]["candidate"] == params["candidate"]
    assert editor.engine.latest_sound("t1", revision=0) == before
    assert fake.revision == 0


async def test_cancelled_call_keeps_the_previous_success(
    studio: tuple[Desk, Fake], ctx: Context, monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio
    result(await desk.call("video_loudness", action="measure", target="main"))
    editor = await ctx.use(VIDEOS).editor(ToolRun("inspect", desk.work, desk.session), "video_info")
    cached = editor.engine.latest_sound("t1", revision=0)
    fake.sound_hold = True
    started = asyncio.Event()
    sound = fake._sound  # pyright: ignore[reportPrivateUsage]

    def start(method: str, params: dict[str, Any]) -> dict[str, Any]:
        answer = sound(method, params)
        started.set()
        return answer

    monkeypatch.setattr(fake, "_sound", start)
    task = asyncio.create_task(
        editor.engine.analyze_sound(
            "audio.analyze", {"timeline": "t1", "targets": [{"bus": "MAIN7"}], "revision": 0}
        )
    )
    await asyncio.wait_for(started.wait(), 5)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    assert editor.engine.latest_sound("t1", revision=0) == cached


async def test_audio_scope_validates_timeline_and_scope(studio: tuple[Desk, Fake]) -> None:
    desk, _ = studio
    missing = await desk.call("video_info", scope="audio", timeline="missing")
    assert missing.error is not None
    assert missing.error.code == "VIDEO_MISSING"
    invalid = await desk.call("video_info", scope="other")
    assert invalid.error is not None


@pytest.mark.parametrize("method", ["audio.normalize", "audio.level_match"])
@pytest.mark.parametrize("apply", [False, None])
async def test_dry_run_revision_is_not_a_bound_measurement_snapshot(
    studio: tuple[Desk, Fake], ctx: Context, method: SoundAnalysis, apply: bool | None
) -> None:
    desk, fake = studio
    editor = await ctx.use(VIDEOS).editor(ToolRun("inspect", desk.work, desk.session), "video_info")
    params: dict[str, Any] = {"timeline": "t1", "clips": ["c1"], "revision": 7}
    if method == "audio.level_match":
        params["reference"] = "c2"
    if apply is not None:
        params["apply"] = apply
    await editor.engine.analyze_sound(method, params)
    cached = result(await desk.call("video_info", scope="audio"))["timelines"][0]["latest_analysis"]
    assert cached["request"]["revision"] == 7
    assert cached["stale"] is True
    # Reaching the requested revision later does not make the old dry-run a measurement of it.
    fake.revision = 7
    later = result(await desk.call("video_info", scope="audio"))["timelines"][0]["latest_analysis"]
    assert later["stale"] is True
    assert cached["measured_revision"] is None
    assert later["measured_revision"] is None


@pytest.mark.parametrize("reverse_notifications", [False, True])
async def test_latest_analysis_follows_wire_completion_order_not_reader_wake_order(
    tmp_path: Path, reverse_notifications: bool
) -> None:
    engine = Engine(tmp_path, "unused", "cache-order-test", None)
    wire = _Wire(engine._heard, engine._lost)  # pyright: ignore[reportPrivateUsage]
    engine._wire = wire  # pyright: ignore[reportPrivateUsage]

    class MemoryTransport(asyncio.Transport):
        def write(self, data: Any) -> None:
            request = json.loads(data)
            response = {
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"job": f"job-{request['id']}"},
            }
            wire.data_received((json.dumps(response) + "\n").encode())

        def close(self) -> None:
            pass

    wire.connection_made(MemoryTransport())
    readers: dict[Changes, str] = {}
    tasks: list[asyncio.Task[dict[str, Any]]] = []
    consumed: list[str] = []

    async def measure(number: int) -> dict[str, Any]:
        result = await engine.analyze_sound(
            "audio.analyze", {"timeline": "t1", "revision": 0, "targets": [{"track": f"A{number}"}]}
        )
        consumed.append(result["job"])
        return result

    for number in (1, 2):
        previous = set(engine._readers)  # pyright: ignore[reportPrivateUsage]
        tasks.append(asyncio.create_task(measure(number)))
        await asyncio.sleep(0)
        reader = (set(engine._readers) - previous).pop()  # pyright: ignore[reportPrivateUsage]
        readers[reader] = f"job-{number}"
    wake_order = [readers[reader] for reader in engine._readers]  # pyright: ignore[reportPrivateUsage]
    notification_order = list(reversed(wake_order)) if reverse_notifications else wake_order
    lines = [
        json.dumps(
            {
                "jsonrpc": "2.0",
                "method": "audio.analysis",
                "params": {
                    "job": job,
                    "state": "done",
                    "results": [{"integrated": -20 - index}],
                },
            }
        )
        for index, job in enumerate(notification_order)
    ]
    # One actual Wire batch: both Changed messages arrive before either consumer resumes.
    wire.data_received(("\n".join(lines) + "\n").encode())
    found = await asyncio.wait_for(asyncio.gather(*tasks), 5)
    assert [done["job"] for done in found] == ["job-1", "job-2"]
    assert consumed == wake_order
    if reverse_notifications:
        assert consumed == list(reversed(notification_order))
    cached = engine.latest_sound("t1", revision=0)
    assert cached is not None
    assert cached["result"]["job"] == notification_order[-1]
    assert cached["result"]["results"] == [{"integrated": -21}]
    assert cached["stale"] is False
    await engine.aclose()
