"""The loudness tool uses engine measurements and one curve-preserving Normalize job."""

import asyncio
import copy
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.tools import TOOLS, ToolRun
from columbina.video import VIDEOS
from columbina.video.engine import EngineError
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")

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


async def test_clip_range_is_refused_instead_of_silently_ignored(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    failed = await desk.call(
        "video_loudness", action="measure", target="clip", ids=["c1"], range={"from": 0, "to": 25}
    )
    assert failed.error is not None
    assert failed.error.code == "VIDEO_INVALID"
    assert "range" in failed.text
    assert not fake.sounds


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
        {"action": "apply", "target": "main"},
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
