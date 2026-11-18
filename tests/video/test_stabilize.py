"""The agent's stabilizer of a clip: read, set a control at a time, measured and taken away."""

import asyncio
import copy
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any, cast, get_args

import pytest

from columbina.fs import Access
from columbina.kernel import Context
from columbina.video import Config, Videos, stabilize, tools_of
from columbina.video.engine import Engine
from support import fake_engine
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")

ENGINE_DATA = Path(__file__).resolve().parents[2] / "engine" / "crates" / "model" / "tests" / "data"
CASES: list[dict[str, Any]] = json.loads(
    (ENGINE_DATA / "key-clocks.json").read_text(encoding="utf-8")
)
"""The engine's own cases of the frames of keys, as ``test_clip_keys.py`` reads them."""

AT = {"timeline": "t1", "clip": "c1"}


def case(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(one for one in CASES if one["name"] == name))


def film(fake: Fake) -> None:
    """On V1 of t1, at 25 frames a second: c1 of moving media, c3 a title and c4 a still; c9 of
    sound on A1. No clip has a stabilizer.

    c1 is on frames 100 to 149 and shows its media from its frame 19 on, at its own speed: its
    frame ``f`` of the timeline is the frame ``f - 81`` of its media. c3 is on frames 40 to 59,
    and c4 on frames 60 to 79.
    """
    names = {"c1": "media at the timeline's rate", "c3": "a title", "c4": "a still"}
    cases = {id: case(name) for id, name in names.items()}
    for id, each in cases.items():
        each["clip"]["id"] = id
    pictures = sorted((each["clip"] for each in cases.values()), key=lambda clip: clip["start"])
    rate = (cases["c1"]["rate"]["num"], cases["c1"]["rate"]["den"])
    timeline = timeline_of(
        track_of("V1", pictures, kind="video"), track_of("A1", [clip_of("c9", 0, 50)]), rate=rate
    )
    media = {each["media"]["id"]: each["media"] for each in cases.values() if each.get("media")}
    fake.project.update(schema=5, next_id=20, media=list(media.values()), timelines=[timeline])


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        film(fake)
        yield desk, fake


def clip(fake: Fake, id: str = "c1") -> dict[str, Any]:
    clips = fake.project["timelines"][0]["tracks"][0]["clips"]
    return next(one for one in clips if one["id"] == id)


async def stabilized(desk: Desk, **asked: Any) -> dict[str, Any]:
    arguments: dict[str, Any] = {"clip": "c1", **asked}
    return result(await desk.call("video_stabilize", **arguments))


async def refused(desk: Desk, asked: dict[str, Any]) -> tuple[str, str]:
    """The code and the words of a call that fails."""
    arguments: dict[str, Any] = {"clip": "c1", **asked}
    outcome = await desk.call("video_stabilize", **arguments)
    assert outcome.error is not None, (asked, outcome.text)
    return outcome.error.code, outcome.text


def measured(frames: range, method: str = "similarity") -> dict[str, Any]:
    """An analysis of a pan of half a pixel a frame, over the frames of the source given."""
    first = frames[0]
    path = {str(one): [0.5 * (one - first), 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0] for one in frames}
    return {"method": method, "size": [1280, 720], "path": path}


# ---------------------------------------------------------------------------------------------
# Reading
# ---------------------------------------------------------------------------------------------


async def test_a_clip_without_a_stabilizer_is_told_to_show_its_source(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = await stabilized(desk)
    assert (told["timeline"], told["clip"], told["stabilizer"]) == ("t1", "c1", False)
    assert "its source as it is" in told["shows"]
    assert "analyse measures the clip" in told["shows"]
    assert {"settings", "analysis", "notes"}.isdisjoint(told)
    # Reading sends nothing, and gives the clip no stabilizer.
    assert fake.transactions == []
    assert "stabilize" not in clip(fake)
    title = await stabilized(desk, clip="c3")
    assert "clip c3 is a title, which has no frames of a source" in title["notes"][0]


async def test_a_stabilizer_is_told_by_its_controls_and_the_frames_that_were_measured(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    analysis = measured(range(19, 69))
    controls = {**stabilize.DEFAULTS, "method": "similarity", "smooth": 0.6}
    clip(fake)["stabilize"] = {**controls, "analysis": analysis}

    told = await stabilized(desk)

    assert told["stabilizer"] is True
    assert told["settings"] == controls
    # The frames of the source that were measured are told at the frames of the timeline.
    assert told["analysis"] == {"method": "similarity", "frames": 50, "timeline_frames": [100, 149]}
    assert told["analyse_again"] is False
    assert "notes" not in told
    # Another method asks for a new analysis; the old one is kept meanwhile.
    clip(fake)["stabilize"]["method"] = "perspective"
    told = await stabilized(desk)
    assert told["analyse_again"] is True
    (note,) = told["notes"]
    assert "measured for similarity, and the method is perspective" in note
    assert "analyse measures the clip again" in note
    # So do frames of the clip that were not measured: the clip was made longer since.
    clip(fake)["stabilize"].update(method="similarity", analysis=measured(range(19, 41)))
    told = await stabilized(desk)
    assert told["analysis"]["timeline_frames"] == [100, 121]
    assert told["analyse_again"] is True
    (note,) = told["notes"]
    assert "28 frames of the clip show frames of its source that were not measured" in note


# ---------------------------------------------------------------------------------------------
# Setting
# ---------------------------------------------------------------------------------------------


async def test_set_sends_only_the_controls_that_change_and_keeps_the_analysis(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    steps = len(fake.steps)

    told = await stabilized(desk, action="set", smooth=0.5)

    # A clip without a stabilizer is given one, with the controls given.
    sent = {"op": "set_stabilize_settings", **AT, "settings": {"smooth": 0.5}}
    assert fake.transactions == [[sent]]
    assert clip(fake)["stabilize"] == {**stabilize.DEFAULTS, "smooth": 0.5}
    assert (told["stabilizer"], told["settings"]["smooth"], told["analysis"]) == (True, 0.5, None)
    assert any("not measured yet" in note for note in told["notes"])
    assert told["revision"] == fake.revision
    assert len(fake.steps) == steps + 1
    # Of those given, only those that change are sent.
    await stabilized(desk, action="set", smooth=0.5, strength=0.8, zoom=False)
    sent = {"op": "set_stabilize_settings", **AT, "settings": {"strength": 0.8, "zoom": False}}
    assert fake.transactions[-1] == [sent]
    # Where nothing would change, nothing is sent.
    code, said = await refused(desk, {"action": "set", "smooth": 0.5, "zoom": False})
    assert code == "VIDEO_STABILIZE"
    assert "every value given is what the stabilizer of clip c1 has already" in said
    assert len(fake.transactions) == 2

    # A new method keeps the analysis, which is then of another method.
    await stabilized(desk, action="analyse")
    told = await stabilized(desk, action="set", method="similarity")
    sent = {"op": "set_stabilize_settings", **AT, "settings": {"method": "similarity"}}
    assert fake.transactions[-1] == [sent]
    assert clip(fake)["stabilize"]["analysis"]["method"] == "perspective"
    assert told["analyse_again"] is True
    assert any("analyse measures the clip again" in note for note in told["notes"])
    # The other controls are drawn at once from the analysis that there is.
    told = await stabilized(desk, action="set", method="perspective", camera_lock=True)
    assert (told["analyse_again"], told["settings"]["camera_lock"]) == (False, True)
    assert len(clip(fake)["stabilize"]["analysis"]["path"]) == 50
    assert clip(fake)["stabilize"]["strength"] == 0.8


async def test_what_is_wrong_is_said_before_anything_is_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told: list[tuple[dict[str, Any], str]] = [
        (
            {"action": "set", "cropping_ratio": 1.5},
            "cropping_ratio is 0 to 1, and 1.5 is not (the engine refuses it as out_of_range "
            "stabilize.cropping_ratio)",
        ),
        ({"action": "set", "smooth": -0.1}, "out_of_range stabilize.smooth"),
        ({"action": "set", "strength": 2}, "strength is -1 to 1"),
        ({"action": "set"}, "set needs one of method, camera_lock, zoom"),
        ({"smooth": 0.5}, "smooth is not for info, which takes the clip alone"),
        ({"action": "analyse", "zoom": True}, "zoom is not for analyse, which takes method"),
        ({"action": "remove"}, "clip c1 has no stabilizer to take away"),
        ({"clip": "c9"}, "c9 is a clip of sound on A1"),
        ({"clip": "c9", "action": "set", "smooth": 0.5}, "wrong_track stabilize"),
        ({"clip": "c9", "action": "analyse"}, "c9 is a clip of sound on A1"),
        ({"clip": "c7"}, "there is no clip c7 on timeline t1"),
        ({"timeline": "t9"}, "there is no timeline t9"),
        ({"clip": "c3", "action": "set", "zoom": False}, "clip c3 is a title"),
        ({"clip": "c3", "action": "analyse"}, "clip c3 is a title: it has no frames of a moving"),
        ({"clip": "c4", "action": "analyse"}, "clip c4 is a still"),
    ]
    for asked, said in told:
        code, text = await refused(desk, asked)
        assert code == "VIDEO_STABILIZE", (asked, text)
        assert said in text, (asked, text)
    # What the arguments themselves do not allow.
    code, text = await refused(desk, {"action": "analyse", "method": "wobble"})
    assert code == "INVALID_ARGS"
    assert "- method:" in text
    assert fake.transactions == []
    assert "track.stabilize" not in fake.asked


async def test_a_refusal_of_the_engine_is_told_with_what_can_be_done_about_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told: list[tuple[str, dict[str, Any], dict[str, Any], str, str]] = [
        ("track.stabilize", {"reason": "busy"}, {"action": "analyse"}, "BUSY", "one at a time"),
        (
            "track.stabilize",
            {"reason": "untrackable"},
            {"action": "analyse"},
            "UNTRACKABLE",
            "has no frames to measure",
        ),
        (
            "track.stabilize",
            {"reason": "stale_revision"},
            {"action": "analyse"},
            "STALE_REVISION",
            "call again",
        ),
        (
            "transaction.apply",
            {"index": 0, "reason": "out_of_range", "what": "stabilize.smooth"},
            {"action": "set", "smooth": 0.5},
            "OUT_OF_RANGE",
            "smooth is 0 to 1",
        ),
        (
            "transaction.apply",
            {"index": 0, "reason": "wrong_track", "what": "stabilize"},
            {"action": "set", "zoom": False},
            "WRONG_TRACK",
            "a stabilizer is of a clip of a picture track",
        ),
    ]
    for method, data, asked, code, said in told:
        fake.refusals[method] = {"code": -32001, "message": "refused", "data": data}
        failed, text = await refused(desk, asked)
        del fake.refusals[method]
        assert failed == f"VIDEO_{code}", (method, data, text)
        assert said in text, (method, data, text)
    assert "stabilize" not in clip(fake)


# ---------------------------------------------------------------------------------------------
# Measuring
# ---------------------------------------------------------------------------------------------


async def test_a_clip_is_measured_and_its_analysis_written_in_one_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    revision, steps = fake.revision, len(fake.steps)

    told = await stabilized(desk, action="analyse")

    # For the clip's method, perspective where it has none.
    asked = {**AT, "revision": revision, "method": "perspective"}
    assert fake.stabilize_starts == [asked]
    assert (told["phase"], told["method"]) == ("done", "perspective")
    assert "reason" not in told
    # What was measured is told at the frames of the timeline that show it.
    assert (told["frames"], told["lost"], told["analysed"]) == (50, 0, [100, 149])
    assert any("video_undo takes it back" in note for note in told["notes"])
    assert any("video_look" in note for note in told["notes"])
    kept = clip(fake)["stabilize"]
    assert {name: kept[name] for name in stabilize.DEFAULTS} == stabilize.DEFAULTS
    assert kept["analysis"]["method"] == "perspective"
    assert sorted(map(int, kept["analysis"]["path"])) == list(range(19, 69))
    assert len(fake.steps) == steps + 1
    info = (await stabilized(desk))["analysis"]
    assert info == {"method": "perspective", "frames": 50, "timeline_frames": [100, 149]}
    # video_info tells the analysis in short, without the motion of each frame.
    listed = result(await desk.call("video_info"))
    clips = listed["timelines"][0]["tracks"][0]["clips"]
    shown = next(one for one in clips if one["id"] == "c1")["stabilize"]
    assert shown["analysis"] == {"method": "perspective", "frames": 50, "source_frames": [19, 68]}
    assert shown["smooth"] == stabilize.DEFAULTS["smooth"]
    # The whole measure is one step, which video_undo takes back.
    result(await desk.call("video_undo"))
    assert "stabilize" not in clip(fake)


async def test_a_clip_is_measured_for_the_method_given_and_keeps_its_controls(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    await stabilized(desk, action="set", strength=0.5)
    steps = len(fake.steps)

    told = await stabilized(desk, action="analyse", method="translation")

    # The method is written with the analysis: one step.
    assert fake.stabilize_starts[-1]["method"] == "translation"
    assert told["method"] == "translation"
    kept = clip(fake)["stabilize"]
    assert (kept["method"], kept["strength"]) == ("translation", 0.5)
    assert kept["analysis"]["method"] == "translation"
    assert len(fake.steps) == steps + 1
    # Without a method, the clip's; where the picture is lost, that is said.
    fake.stabilize_lost = {30, 31}
    told = await stabilized(desk, action="analyse")
    assert fake.stabilize_starts[-1]["method"] == "translation"
    assert told["lost"] == 2
    assert any("the picture was lost at 2 frames" in note for note in told["notes"])


async def test_a_measure_that_takes_too_long_is_stopped_and_writes_nothing(
    studio: tuple[Desk, Fake], monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, fake = studio
    monkeypatch.setattr(stabilize, "_MEASURE_SECONDS", (0.3, 0.0))
    # The measure reads one frame, and goes on no further until it is stopped.
    fake.track_hold = True
    steps = len(fake.steps)

    told = await stabilized(desk, action="analyse")

    assert fake.asked.count("track.stop") == 1
    assert (told["phase"], told["reason"], told["analysed"]) == ("stopped", "timeout", None)
    assert "took longer than 0.3 s and was stopped" in told["notes"][0]
    assert "stabilize" not in clip(fake)
    assert len(fake.steps) == steps
    # It is over: the next measure goes.
    fake.track_hold = False
    told = await stabilized(desk, action="analyse")
    assert (told["phase"], told["analysed"]) == ("done", [100, 149])


async def test_a_measure_is_one_track_of_the_engine_and_one_stopped_writes_nothing(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    directory = desk.work / ".columbina" / "studio"
    one = await Engine.attach(directory, program=str(Path(fake_engine.__file__)))
    fake.track_hold = True
    revision, _ = await one.project()
    asked: dict[str, Any] = {**AT, "revision": revision, "method": "similarity"}

    tracking = await one.track(asked, session=one.session, method="track.stabilize")
    with pytest.raises(TimeoutError):
        await asyncio.wait_for(tracking.ended(), 0.3)
    progress = tracking.last
    # One track at a time, of either kind: the tool is told so in words.
    code, said = await refused(desk, {"action": "analyse"})
    await tracking.stop()
    end = await asyncio.wait_for(tracking.ended(), 5)
    await one.aclose()

    assert tracking.started == {
        "job": tracking.job,
        "first": 19,
        "last": 68,
        "frame_size": [1280, 720],
        "method": "similarity",
    }
    assert progress is not None
    assert (progress["done"], progress["total"]) == (1, 50)
    assert code == "VIDEO_BUSY"
    assert "one at a time" in said
    assert (end["phase"], end["reason"], end["analysed"]) == ("stopped", "stop", None)
    assert end["method"] == "similarity"
    # A measure that is stopped writes nothing.
    assert "stabilize" not in clip(fake)


# ---------------------------------------------------------------------------------------------
# Taking it away, and the tool
# ---------------------------------------------------------------------------------------------


async def test_the_stabilizer_is_taken_away_in_one_step(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    await stabilized(desk, action="analyse")
    steps = len(fake.steps)

    told = await stabilized(desk, action="remove")

    assert fake.transactions[-1] == [{"op": "set_stabilize", **AT, "value": None}]
    assert "stabilize" not in clip(fake)
    assert (told["stabilizer"], len(fake.steps)) == (False, steps + 1)
    result(await desk.call("video_undo"))
    assert clip(fake)["stabilize"]["analysis"]["method"] == "perspective"


def test_the_tool_is_offered_and_costs_few_characters() -> None:
    tools = {tool.name: tool for tool in tools_of(Videos(Config(), None))}
    tool = tools["video_stabilize"]
    assert tool.effect == "write"
    told = tool.schema.description
    for word in (
        "measured once", "info", "analyse", "set", "remove", "perspective", "similarity",
        "translation", "camera_lock", "zoom", "cropping_ratio", "smooth", "strength",
        "act at once", "a new method needs analyse", "video_look",
    ):  # fmt: skip
        assert word in told, word
    for value in ("0.5", "0.25", "-1 to 1 (1)"):
        assert value in told, value
    properties = cast("dict[str, Any]", tool.schema.parameters["properties"])
    controls = {"method", "camera_lock", "zoom", "cropping_ratio", "smooth", "strength"}
    assert set(properties) == {"project", "timeline", "clip", "action"} | controls
    assert set(stabilize.SETTINGS) == controls
    assert properties["action"]["enum"] == list(get_args(stabilize.Action))
    assert properties["action"]["default"] == "info"
    assert properties["method"]["enum"] == list(stabilize.METHODS)
    # What the tool costs on every request.
    assert len(told) < 700
    assert len(told) + len(json.dumps(tool.schema.parameters)) < 1400


async def test_the_sandbox_lets_the_tool_reach_the_studio_project(
    ctx: Context, tmp_path: Path
) -> None:
    desk = await desk_of(ctx, tmp_path)
    (desk.work / ".columbina" / "studio").mkdir(parents=True)
    desk.sandbox.set_mode(desk.session, "workspace-write")
    studio = desk.fs.locate(desk.session, desk.work, ".columbina/studio")
    settings = desk.fs.locate(desk.session, desk.work, ".columbina/permissions.yml")

    def access(tool: str, where: Any) -> Access:
        return Access(session=desk.session, cwd=desk.work, where=where, kind="write", tool=tool)

    assert desk.sandbox.restricts(access("video_stabilize", studio)) is None
    outside = desk.sandbox.restricts(access("video_stabilize", settings))
    assert outside is not None
    assert outside.code == "PROTECTED"
