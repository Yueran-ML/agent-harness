"""The Planar Tracker of the Fusion page as the agent adds it, sets it, reads it and tracks it."""

import copy
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.video import edits, fusion
from columbina.video.engine import Engine, EngineError
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
STILL = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
"""The homography that moves nothing, as a path writes it."""


def case(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(one for one in CASES if one["name"] == name))


def film(fake: Fake) -> None:
    """c1 of media on frames 100 to 149 of t1 (its frame ``f`` is ``f - 81`` of its media), c3 a
    title on frames 40 to 59, and c9 of sound; none composed."""
    names = ("media at the timeline's rate", "a title")
    cases = [case(name) for name in names]
    for each, id in zip(cases, ("c1", "c3"), strict=True):
        each["clip"]["id"] = id
    pictures = sorted((each["clip"] for each in cases), key=lambda clip: clip["start"])
    rate = (cases[0]["rate"]["num"], cases[0]["rate"]["den"])
    timeline = timeline_of(
        track_of("V1", pictures, kind="video"), track_of("A1", [clip_of("c9", 0, 50)]), rate=rate
    )
    media = {each["media"]["id"]: each["media"] for each in cases if each.get("media")}
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


def params(fake: Fake, node: str, id: str = "c1") -> dict[str, Any]:
    nodes = clip(fake, id)["fusion"]["nodes"]
    return next(one for one in nodes if one["id"] == node)["params"]


def told_node(told: dict[str, Any], id: str) -> dict[str, Any]:
    return next(one for one in told["nodes"] if one["id"] == id)


def path_frames(fake: Fake, node: str) -> list[int]:
    return sorted(map(int, params(fake, node)["path"]))


async def node(desk: Desk, action: str, clip: str = "c1", **asked: Any) -> dict[str, Any]:
    return result(await desk.call("video_fusion_node", clip=clip, action=action, **asked))


async def planar(desk: Desk, **asked: Any) -> str:
    """Give c1 a planar tracker after its MediaIn1, and tell its id."""
    told = await node(desk, "add", kind="planar_tracker", after="MediaIn1", **asked)
    return told["added"]


async def refused(desk: Desk, fake: Fake, tool: str, **asked: Any) -> str:
    """What a call that is refused before anything is sent says."""
    sent, tracks = len(fake.transactions), len(fake.track_starts)
    outcome = await desk.call(tool, **{"clip": "c1", **asked})
    assert outcome.error is not None, outcome.text
    assert outcome.error.code == "VIDEO_FUSION", outcome.text
    assert (len(fake.transactions), len(fake.track_starts)) == (sent, tracks)
    return outcome.text


async def tracked(desk: Desk, **asked: Any) -> dict[str, Any]:
    return result(await desk.call("video_fusion_track", **{"clip": "c1", **asked}))


# ---------------------------------------------------------------------------------------------
# The node
# ---------------------------------------------------------------------------------------------


async def test_a_planar_tracker_is_added_whole_and_told_without_its_path(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    square = [[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]]
    settings = {"operation": "steady", "region": square, "corners.tl.x": 0.2, "reference": 110}
    told = await node(desk, "add", kind="planar_tracker", after="MediaIn1", settings=settings)
    added = told["added"]

    # Every setting is written; the region as places, the reference at the frame of the media
    # that frame 110 of the timeline shows, and the corner given in the corners of a new one.
    (_, sent) = fake.transactions[-1]
    new = fusion.new_params("planar_tracker")
    assert sent["node"]["params"] == {
        **new,
        "operation": "steady",
        "reference": 29,
        "region": [{"x": x, "y": y} for x, y in square],
        "corners": {**new["corners"], "tl": {"x": 0.2, "y": 0.75}},
    }
    assert sent["links"] == [
        {"from": "MediaIn1", "to": "", "input": "input"},
        {"from": "", "to": "MediaOut1", "input": "input"},
    ]
    shown = told_node(told, added)
    assert shown["free"] == ["foreground", "mask"]
    assert shown["settings"]["operation"] == "steady"
    assert shown["settings"]["motion"] == "perspective"
    assert shown["settings"]["reference"] == {"timeline_frame": 110, "frame": 29}
    assert shown["settings"]["region"] == {"places": 4}
    assert shown["settings"]["corners.tl.x"] == 0.2
    assert shown["settings"]["path"] == "none: not tracked"
    assert shown["note"].startswith("not tracked yet: video_fusion_track follows its region")
    assert "video_fusion_track" in told["settings_of"]["planar_tracker"]["path"]

    # The reference tells it whole.
    reference = result(await desk.call("video_fusion_info", reference=True))
    kind = reference["kinds"]["planar_tracker"]
    assert kind["inputs"] == ["input", "foreground", "mask"]
    assert "3 to 64 places" in kind["settings"]["region"]
    assert "corner_pin" in kind["settings"]["operation"]
    assert "planar_tracker" in reference["tracking"]
    assert "planar_tracker" in reference["keys"]


async def test_what_a_planar_tracker_does_not_take_is_said_before_anything_is_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk, settings={"operation": "steady"})

    async def wrong(**settings: Any) -> str:
        asked = {"action": "set", "node": plane, "settings": settings}
        return await refused(desk, fake, "video_fusion_node", **asked)

    said = await wrong(region=[[0.3, 0.3], [0.7, 0.3]])
    assert "region is a list of 3 to 64 places" in said
    assert "out_of_range planar_tracker.region" in said
    said = await wrong(region=[[0.3, 0.3], [0.7, 17], [0.5, 0.5]])
    assert "region.1.y is 17, out of range" in said
    assert "out_of_range planar_tracker.region" in said
    said = await wrong(region=[[0.3, 0.3, 0], [0.7, 0.3], [0.5, 0.5]])
    assert "region is a list of 3 to 64 places" in said
    said = await wrong(corners={"br": {"y": -16}})
    assert "corners.br.y is -16, out of range" in said
    assert "out_of_range planar_tracker.corners" in said
    said = await wrong(path={"30": STILL})
    assert "path is filled by tracking (video_fusion_track): {} clears it" in said
    said = await wrong(operation="stabilize")
    assert "is one of none, steady, corner_pin" in said
    said = await wrong(motion="affine")
    assert "is one of translation, translation_rotation, translation_rotation_scale" in said
    said = await wrong(reference=99)
    assert "reference is 99, not a frame of the clip, which is at frames 100 to 149" in said
    said = await wrong(**{"corners.tl.z": 0})
    assert "a planar_tracker has no setting 'corners.tl.z'" in said
    said = await wrong(operation="steady")
    assert "nothing changes" in said
    # Nothing of it takes keys: its path is what moves.
    for setting in ("reference", "corners.tl.x", "operation", "path"):
        asked = {"path": f"{plane}.{setting}", "action": "put", "frame": 110, "value": 0.5}
        said = await refused(desk, fake, "video_fusion_keys", **asked)
        assert f"{setting} of a planar_tracker takes no keys" in said


async def test_the_path_of_a_planar_tracker_is_told_by_its_frames_and_cleared_frame_by_frame(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk)
    # Tracking writes the path at frames of the clip's media: frames 100 and 101 of t1.
    params(fake, plane)["path"] = {"19": STILL, "20": [1.0, 0.0, 0.01, 0.0, 1.0, 0.0, 0.0, 0.0]}
    shown = told_node(result(await desk.call("video_fusion_info", clip="c1")), plane)
    assert shown["settings"]["path"] == {
        "frames": 2,
        "first": {"timeline_frame": 100, "frame": 19},
        "last": {"timeline_frame": 101, "frame": 20},
    }
    assert "note" not in shown

    # A region drawn again is taken, and the path that was tracked with the old one is kept.
    triangle = {"region": [[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]]}
    told = await node(desk, "set", node=plane, settings=triangle)
    assert f"the path of node {plane} was tracked with the region" in told["notes"][0]
    assert path_frames(fake, plane) == [19, 20]
    # {} clears the path: each of its frames is sent null, which takes it away.
    told = await node(desk, "set", node=plane, settings={"path": {}})
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": plane, "params": {"path": {"19": None, "20": None}}}
    ]
    cleared = f"the path of node {plane} is cleared: video_fusion_track tracks it again"
    assert told["notes"] == [cleared]
    assert params(fake, plane)["path"] == {}
    said = await refused(
        desk, fake, "video_fusion_node", action="set", node=plane, settings={"path": {}}
    )
    assert "nothing changes" in said


# ---------------------------------------------------------------------------------------------
# Tracking
# ---------------------------------------------------------------------------------------------


async def test_a_planar_tracker_is_tracked_from_its_reference_frame_in_one_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk, settings={"reference": 110})
    revision, steps = fake.revision, len(fake.steps)

    told = await tracked(desk, node=plane)

    # From frame 110 of the timeline, where the region was drawn, which shows frame 29 of the
    # media: the target is named planar, and not fusion.
    aim = {"timeline": "t1", "clip": "c1", "node": plane}
    assert fake.track_starts == [
        {"planar": aim, "revision": revision, "frame": 29, "direction": "forward"}
    ]
    assert told["phase"] == "done"
    assert "reason" not in told
    assert told["from"] == {"timeline_frame": 110, "frame": 29}
    assert (told["tracked"], told["frames"], told["lost"]) == ([110, 149], 39, 0)
    assert "patterns" not in told
    assert any("video_undo takes the whole track back" in note for note in told["notes"])
    assert any("operation steady" in note for note in told["notes"])
    assert path_frames(fake, plane) == list(range(29, 69))
    assert params(fake, plane)["path"]["29"] == STILL
    assert params(fake, plane)["path"]["39"] == pytest.approx([1, 0, 0.01, 0, 1, 0, 0, 0])
    info = result(await desk.call("video_fusion_info", clip="c1"))
    assert told_node(info, plane)["settings"]["path"] == {
        "frames": 40,
        "first": {"timeline_frame": 110, "frame": 29},
        "last": {"timeline_frame": 149, "frame": 68},
    }
    # The whole track is one step, which video_undo takes back.
    assert len(fake.steps) == steps + 1
    result(await desk.call("video_undo"))
    assert params(fake, plane)["path"] == {}

    # Asked again from where it was, nothing is new: nothing is written, and it says so.
    await tracked(desk, node=plane)
    told = await tracked(desk, node=plane)
    assert (told["phase"], told["tracked"]) == ("done", None)
    assert any("nothing was written" in note for note in told["notes"])


async def test_where_the_plane_is_lost_and_what_cannot_be_tracked_is_told(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk)

    # A new planar tracker's reference, frame 0 of the media, is shown at no frame of c1: the
    # track starts at its first frame. Frames 40 and 41 of the media are lost, and not written.
    fake.planar_lost = {40, 41}
    told = await tracked(desk, node=plane)
    assert fake.track_starts[-1]["frame"] == 19
    assert (told["tracked"], told["frames"], told["lost"]) == ([100, 149], 49, 2)
    assert any("the plane was lost at some frames" in note for note in told["notes"])
    assert path_frames(fake, plane) == [one for one in range(19, 69) if one not in (40, 41)]

    # A region that holds nothing to follow fails, and writes nothing.
    fake.planar_features = False
    told = await tracked(desk, node=plane, frame=120, direction="reverse")
    assert (told["phase"], told["reason"], told["tracked"]) == ("failed", "no_features", None)
    assert len(told["notes"]) == 1
    assert "the region holds nothing that can be followed" in told["notes"][0]

    # A refusal of the engine is told with what can be done about it.
    fake.refusals["track.start"] = {
        "code": -32001,
        "message": "refused",
        "data": {"reason": "region"},
    }
    outcome = await desk.call("video_fusion_track", clip="c1", node=plane)
    assert outcome.error is not None
    assert outcome.error.code == "VIDEO_REGION"
    assert "the region cannot be followed from there" in outcome.text
    del fake.refusals["track.start"]

    # What the project tells is said before anything is sent.
    said = await refused(desk, fake, "video_fusion_track", node=plane, trackers=[0])
    assert f"node {plane} is a planar_tracker, which follows its region whole" in said
    params(fake, plane)["region"] = [{"x": 0.5, "y": 0.5}] * 2
    said = await refused(desk, fake, "video_fusion_track", node=plane)
    assert f"the region of node {plane} has 2 places" in said
    assert "region" in said
    on_title = await node(desk, "add", clip="c3", kind="planar_tracker")
    said = await refused(desk, fake, "video_fusion_track", clip="c3", node=on_title["added"])
    assert "clip c3 is a title: a title, a colour or a still has no frames to track" in said
    blur = (await node(desk, "add", kind="blur"))["added"]
    said = await refused(desk, fake, "video_fusion_track", node=blur)
    assert "or the region of a planar_tracker" in said


async def test_a_refusal_of_a_planar_tracker_is_told_with_what_can_be_done_about_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk)

    async def answered(**data: Any) -> Any:
        fake.refusals["transaction.apply"] = {"code": -32001, "message": "refused", "data": data}
        settings = {"operation": "corner_pin"}
        outcome = await desk.call(
            "video_fusion_node", clip="c1", action="set", node=plane, settings=settings
        )
        assert outcome.error is not None
        return outcome

    outcome = await answered(reason="out_of_range", what="planar_tracker.region", index=0)
    assert outcome.error.code == "VIDEO_OUT_OF_RANGE"
    assert "a region is 3 to 64 places, each x and y -15.5 to 16.5" in outcome.text
    outcome = await answered(reason="out_of_range", what="planar_tracker.corners", index=0)
    assert "each corner of a planar_tracker are -15.5 to 16.5" in outcome.text
    outcome = await answered(reason="out_of_range", what="planar_tracker.reference", index=0)
    assert "reference of a planar_tracker is Reference Frame" in outcome.text
    assert params(fake, plane)["operation"] == "none"


async def test_the_fake_engine_checks_a_planar_tracker_as_the_engine_does(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    plane = await planar(desk)
    region = [{"x": 0.5, "y": 0.5}, {"x": 0.6, "y": 0.5}, {"x": 0.5, "y": 20.0}]
    wrong = [edits.set_fusion_node("t1", "c1", plane, params={"region": region})]
    with pytest.raises(EngineError) as refused_region:
        await apply_with(desk, wrong)
    assert refused_region.value.data is not None
    assert refused_region.value.data["what"] == "planar_tracker.region"
    corner = {"corners": {"tl": {"x": -16.0}}}
    with pytest.raises(EngineError) as refused_corner:
        await apply_with(desk, [edits.set_fusion_node("t1", "c1", plane, params=corner)])
    assert refused_corner.value.data is not None
    assert refused_corner.value.data["what"] == "planar_tracker.corners"
    # A track names one target: a window, fusion or planar.
    engine = await Engine.attach(desk.work / ".columbina" / "studio", program=FAKE)
    try:
        revision, _ = await engine.project()
        aim = {"timeline": "t1", "clip": "c1", "node": plane}
        both = {"planar": aim, "fusion": aim, "revision": revision, "frame": 19}
        with pytest.raises(EngineError) as two:
            await engine.track({**both, "direction": "forward"})
        assert two.value.data is not None
        assert two.value.data["reason"] == "fusion"
    finally:
        await engine.aclose()
    assert params(fake, plane)["region"] == fusion.new_params("planar_tracker")["region"]


FAKE = str(Path(fake_engine.__file__))
"""The fake engine, run as the engine is."""


async def apply_with(desk: Desk, ops: list[dict[str, Any]]) -> None:
    """Apply operations as another client of the project would, at its revision."""
    engine = await Engine.attach(desk.work / ".columbina" / "studio", program=FAKE)
    try:
        revision, _ = await engine.project()
        await engine.apply(ops, revision=revision, session=engine.session)
    finally:
        await engine.aclose()
