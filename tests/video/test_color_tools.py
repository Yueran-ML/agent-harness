"""The agent's tools of scopes, colour management and Shot Match, against the fake engine."""

import base64
import copy
import json
from collections.abc import AsyncIterator, Callable
from itertools import pairwise
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.video import Config, Videos, grading, tools_of
from columbina.video.shotmatch import ShotMatch
from support.desk import Desk
from support.fake_engine import Fake, RefusedError, frame_of, served
from support.scenes import SCENE, figures_of, png_of, through
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")

Measure = Callable[[dict[str, Any], dict[str, Any]], dict[str, Any]]
FRAME = SCENE[:3000]
CAST = ShotMatch(gain=(0.85, 1.0, 1.1), offset=(0.03, 0.0, -0.02))


def corrector(id: str) -> dict[str, Any]:
    return {
        "id": id,
        "kind": "corrector",
        "label": "",
        "enabled": True,
        "params": grading.new_corrector(),
    }


def graded(*nodes: dict[str, Any]) -> dict[str, Any]:
    ends = ["source", *(one["id"] for one in nodes), "output"]
    links = [{"from": one, "to": two, "kind": "rgb", "input": 0} for one, two in pairwise(ends)]
    return {"nodes": list(nodes), "links": links}


def project(fake: Fake) -> None:
    """c1, ungraded, of the scene; c2, ungraded, of the scene with a cast; c4 with a node."""
    pictures = [clip_of("c1", 0, 50), clip_of("c2", 50, 50), clip_of("c4", 100, 40)]
    for clip in pictures:
        clip["grade"] = copy.deepcopy(grading.EMPTY_GRADE)
        clip["group"] = None
    pictures[2]["grade"] = graded(corrector("n3"))
    timeline = timeline_of(track_of("V1", pictures, kind="video"))
    timeline["grade"] = copy.deepcopy(grading.EMPTY_GRADE)
    video = {"width": 1920, "height": 1080, "color": {"primaries": "bt709", "transfer": "bt709"}}
    fake.project.update(
        schema=3,
        next_id=10,
        timelines=[timeline],
        groups=[],
        color={"science": "yrgb", "output": "Rec.709 Gamma 2.4"},
        media=[
            {"id": "m1", "name": "a.mov", "path": "a.mov", "facts": {"video": video}},
            {"id": "m2", "name": "b.wav", "path": "b.wav", "facts": {"video": None}},
        ],
    )


def measured(casts: dict[str, ShotMatch]) -> Measure:
    """The engine's figures of the picture at a node of a clip's grade: the scene through the
    clip's cast, then through the shot match of the node where it is measured at its output."""

    def measure(params: dict[str, Any], project: dict[str, Any]) -> dict[str, Any]:
        bins = int(params.get("bins", 256))
        if "grade" not in params:
            return frame_of(FRAME[:8], bins)
        clip = params["grade"]["clip"]
        frame = through(FRAME, casts.get(clip, ShotMatch()))
        clips = project["timelines"][0]["tracks"][0]["clips"]
        grade = next(one for one in clips if one["id"] == clip)["grade"]
        node = next(one for one in grade["nodes"] if one["id"] == params["node"])
        held = node["params"].get("shot_match")
        if held is not None and params.get("at", "output") == "output":
            frame = through(frame, ShotMatch.of(held))
        return frame_of(frame, bins)

    return measure


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        project(fake)
        fake.measure = measured({"c2": CAST})
        yield desk, fake


def clip_grade(fake: Fake, id: str) -> dict[str, Any]:
    clips = fake.project["timelines"][0]["tracks"][0]["clips"]
    return next(one for one in clips if one["id"] == id)["grade"]


def gaps(measured: dict[str, Any], target: dict[str, Any]) -> float:
    """The largest difference of the percentiles of R, G and B of what video_grade_scopes
    measured and of a target that video_shot_match told."""
    return max(
        abs(measured[name]["percentiles"][level] - target["percentiles"][name][level])
        for name in ("red", "green", "blue")
        for level in target["percentiles"][name]
    )


async def test_a_frame_or_a_node_is_measured_off_screen_by_its_percentiles(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(await desk.call("video_grade_scopes", frame=12))
    assert fake.looked[-1] == ("scopes.statistics", {"timeline": "t1", "frame": 12, "bins": 1024})
    assert told["measured"] == "the frame as an export draws it"
    assert set(told["red"]["percentiles"]) == {"1", "5", "25", "50", "75", "95", "99"}
    assert {"min", "max", "mean"} <= set(told["luma"])
    assert "histogram" not in told
    # A clip is measured at its middle; with a node, the picture at the node.
    result(await desk.call("video_grade_scopes", clip="c2"))
    assert fake.looked[-1] == ("scopes.statistics", {"timeline": "t1", "frame": 75, "bins": 1024})
    result(await desk.call("video_grade_scopes", clip="c4", node=1, at="input"))
    assert fake.looked[-1][1] == {
        "timeline": "t1",
        "frame": 120,
        "grade": {"level": "clip", "timeline": "t1", "clip": "c4"},
        "node": "n3",
        "at": "input",
        "bins": 1024,
    }
    whole = result(await desk.call("video_grade_scopes", clip="c4", node="n3", histogram=8))
    assert whole["measured"]["node"] == "n3"
    assert len(whole["histogram"]["red"]) == 8
    assert sum(whole["histogram"]["red"]) == pytest.approx(100, abs=0.1)
    assert whole["mid_grey"] == pytest.approx(
        figures_of(through(FRAME, ShotMatch())).grey, abs=0.002
    )
    for asked, said in [
        ({}, "give frame, or clip"),
        ({"node": 1}, "node needs the grade"),
        ({"grade": "c4"}, "give node too"),
        ({"clip": "c9"}, "there is no clip c9"),
    ]:
        refused = await desk.call("video_grade_scopes", **asked)
        assert refused.error is not None
        assert said in refused.text


async def test_colour_management_is_read_and_set_as_one_step(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    told = result(await desk.call("video_color_settings"))
    assert told["color"] == {"science": "yrgb", "output": "Rec.709 Gamma 2.4"}
    assert told["in_force"] == {"science": "yrgb", "managed": False}
    steps = len(fake.steps)
    patch = {"science": "yrgb_managed", "preset": "custom", "timeline": "Rec.2020 Gamma 2.4"}
    told = result(await desk.call("video_color_settings", settings=patch))
    assert fake.transactions[-1] == [{"op": "set_color_settings", "settings": patch}]
    assert len(fake.steps) == steps + 1
    assert told["in_force"]["timeline"] == "Rec.2020 Gamma 2.4"
    assert told["in_force"]["output"] == "Rec.709 Gamma 2.4"
    assert grading.in_force({"science": "yrgb_managed"})["timeline"] == "Rec.709 Gamma 2.4"
    assert grading.in_force({"science": "yrgb_managed", "preset": "davinci_wide_gamut"})[
        "timeline"
    ] == ("DaVinci Wide Gamut/DaVinci Intermediate")
    assert grading.in_force(
        {"science": "yrgb_managed", "automatic": True, "processing_mode": "hdr"}
    )["timeline"] == ("Rec.2100 ST2084")
    assert grading.in_force({"science": "acescct"})["timeline"] == "ACEScct"
    fake.refusals["transaction.apply"] = {
        "code": -32001,
        "message": "timeline names no space",
        "data": {"index": 0, "reason": "unknown_space", "setting": "timeline", "name": "x"},
    }
    refused = await desk.call("video_color_settings", settings={"timeline": "x"})
    assert refused.error is not None
    assert refused.error.code == "VIDEO_UNKNOWN_SPACE"
    assert "reference true lists the spaces" in refused.text


async def test_the_colour_of_media_is_listed_and_set_for_many_in_one_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    listed = result(await desk.call("video_media_color"))
    assert listed["media"] == [
        {
            "media": "m1",
            "name": "a.mov",
            "color_override": {"input": None, "levels": "auto"},
            "file": {"primaries": "bt709", "transfer": "bt709"},
        }
    ]
    steps = len(fake.steps)
    told = result(
        await desk.call("video_media_color", media=["m1", "m2"], input="Apple Log", levels="full")
    )
    assert fake.transactions[-1] == [
        {"op": "set_media_color", "id": "m1", "input": "Apple Log", "levels": "full"},
        {"op": "set_media_color", "id": "m2", "input": "Apple Log", "levels": "full"},
    ]
    assert len(fake.steps) == steps + 1
    assert told["media"][0]["color_override"] == {"input": "Apple Log", "levels": "full"}
    result(await desk.call("video_media_color", media=["m1"], input="default"))
    assert fake.transactions[-1] == [{"op": "set_media_color", "id": "m1", "input": None}]
    refused = await desk.call("video_media_color", media=["m7"], levels="video")
    assert refused.error is not None
    assert "there is no media m7" in refused.text


async def test_a_clip_is_shot_matched_invisibly_in_one_step_and_measures_as_its_target(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    target = clip_grade(fake, "c1")
    steps = len(fake.steps)
    told = result(await desk.call("video_shot_match", clips=["c1", "c2"], target="c1"))
    # c2 had no node: one is appended, and the target, measured by a node of its own, keeps
    # none, all in one gesture: one step of the history.
    assert len(fake.steps) == steps + 1
    first, last = fake.gestures[-2], fake.gestures[-1]
    assert first is not None
    assert last == {"id": first["id"], "end": True}
    assert clip_grade(fake, "c1") == target
    (matched,) = told["matched"]
    assert told["skipped"] == [{"clip": "c1", "why": "it is the target"}]
    assert matched["clip"] == "c2"
    assert matched["added"] is True
    assert matched["node"] == 1
    assert matched["after"] < 0.005 < matched["before"]
    assert told["target"]["clip"] == "c1"
    assert told["target"]["frame"] == 25
    assert "note" not in told
    (node,) = clip_grade(fake, "c2")["nodes"]
    assert node["params"]["shot_match"] == matched["shot_match"]
    assert node["params"]["shot_match"]["gain"]["r"] == pytest.approx(1 / 0.85, abs=0.02)

    # The grade shows the match, and no setting that the palettes show changed.
    info = result(await desk.call("video_grade_info", grade="c2"))
    assert info["nodes"][0]["shot_match"] is True
    assert info["nodes"][0]["settings"] == {}
    # What comes out of c2 now measures as c1 did.
    matched_figures = result(await desk.call("video_grade_scopes", clip="c2", node=1))
    assert gaps(matched_figures, told["target"]) < 0.006

    # One undo takes it all back.
    result(await desk.call("video_undo"))
    assert clip_grade(fake, "c2") == grading.EMPTY_GRADE
    assert clip_grade(fake, "c1") == target


async def test_a_node_with_a_match_is_matched_again_from_its_picture_without_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    fake.measure = measured({"c4": CAST})
    first = result(await desk.call("video_shot_match", clips=["c4"], target="c1"))
    again = result(await desk.call("video_shot_match", clips=["c4"], target="c1", node=1))
    # Measured with its match undone, the node is matched as before; the target keeps no node.
    assert clip_grade(fake, "c1") == grading.EMPTY_GRADE
    one, other = first["matched"][0]["shot_match"], again["matched"][0]["shot_match"]
    for name in ("gamma", "gain", "offset"):
        assert other[name] == pytest.approx(one[name], abs=0.003)
    assert other["contrast"] == pytest.approx(one["contrast"], abs=0.003)
    assert again["matched"][0]["before"] == pytest.approx(first["matched"][0]["before"])
    # Reset takes it away, of the node named or of every node with one.
    result(await desk.call("video_shot_match", clips=["c4"], reset=True))
    assert clip_grade(fake, "c4")["nodes"][0]["params"]["shot_match"] is None
    refused = await desk.call("video_shot_match", clips=["c4"], reset=True)
    assert refused.error is not None
    assert "none of the clips has a shot match" in refused.text


async def test_auto_color_goes_into_the_visible_lift_and_gain(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    fake.measure = measured({"c4": CAST})
    told = result(await desk.call("video_shot_match", clips=["c4"], auto=True))
    (done,) = told["auto"]
    params = clip_grade(fake, "c4")["nodes"][0]["params"]
    assert params["shot_match"] is None
    lift, gain = params["primaries"]["lift"], params["primaries"]["gain"]
    assert done["settings"] == {
        **{f"primaries.lift.{c}": lift[c] for c in "rgb"},
        **{f"primaries.gain.{c}": gain[c] for c in "rgb"},
    }
    assert gain["r"] > 1 > gain["b"]
    info = result(await desk.call("video_grade_info", grade="c4"))
    assert "primaries.gain.r" in info["nodes"][0]["settings"]


async def test_a_clip_is_matched_to_a_still_by_its_picture(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    width = 60
    pixels = FRAME[: width * 40]
    rows = [
        bytes(round(v * 255) for pixel in pixels[y * width : (y + 1) * width] for v in pixel)
        for y in range(40)
    ]
    png = base64.b64encode(png_of(rows, width, 3)).decode()
    asked: list[dict[str, Any]] = []

    def thumbnail(_asking: object, params: dict[str, Any]) -> dict[str, Any]:
        asked.append(params)
        if params["still"] != "s1":
            raise RefusedError(-32602, "no still", {"reason": "missing", "what": "still"})
        return {"width": width, "height": 40, "png": png}

    setattr(fake, "gallery_thumbnail", thumbnail)  # noqa: B010
    told = result(await desk.call("video_shot_match", clips=["c2"], target={"still": "s1"}))
    assert asked == [{"still": "s1", "height": 270}]
    assert told["target"]["still"] == "s1"
    # The thumbnail is of 8 bits: a match within its steps.
    assert told["matched"][0]["after"] < 0.01 < told["matched"][0]["before"]
    assert clip_grade(fake, "c2")["nodes"][0]["params"]["shot_match"]["gain"]["r"] > 1.1
    refused = await desk.call("video_shot_match", clips=["c4"], target={"still": "s9"})
    assert refused.error is not None
    assert "there is no still s9" in refused.text


async def test_what_a_shot_match_is_asked_is_checked_before_anything_is_done(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    sent = len(fake.transactions)
    for asked, said in [
        ({"clips": ["c2"]}, "target names the clip"),
        ({"clips": ["c2"], "target": "c1", "auto": True}, "no target or frame"),
        ({"clips": ["c2"], "auto": True, "reset": True}, "give one"),
        ({"clips": ["c2", "c2"], "target": "c1"}, "named twice"),
        ({"clips": ["c1"], "target": "c1"}, "no clip to match but the target"),
        ({"clips": ["c2"], "target": "c1", "frame": 70}, "frame 70 is not of clip c1"),
        ({"clips": ["c2"], "target": {"still": "s1"}, "frame": 3}, "frame is of a target clip"),
        ({"clips": ["c2"], "target": {"clip": "c1"}}, "Field required"),
        ({"clips": ["c9"], "target": "c1"}, "there is no clip c9"),
    ]:
        refused = await desk.call("video_shot_match", **asked)
        assert refused.error is not None, asked
        assert said in refused.text, (asked, refused.text)
    assert len(fake.transactions) == sent


async def test_a_failed_measure_takes_back_the_nodes_added_for_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio

    def broken(params: dict[str, Any], project: dict[str, Any]) -> dict[str, Any]:
        raise RefusedError(-32602, "no picture", {"reason": "clip"})

    fake.measure = broken
    steps, before = len(fake.steps), copy.deepcopy(fake.project)
    refused = await desk.call("video_shot_match", clips=["c2"], target="c1")
    assert refused.error is not None
    # The gesture ended where it began: no step, the project as it was but for next_id.
    assert len(fake.steps) == steps
    assert {**fake.project, "next_id": 0} == {**before, "next_id": 0}
    first = fake.gestures[-2]
    assert first is not None
    assert fake.gestures[-1] == {"id": first["id"], "end": True}


async def test_the_reference_of_settings_is_told_on_demand(studio: tuple[Desk, Fake]) -> None:
    desk, _ = studio
    told = result(await desk.call("video_grade_info", reference=True))
    assert told["corrector"]["primaries.saturation"] == (
        "Saturation (Sat): 0 to 100, 50 changes nothing"
    )
    assert "shot_match.gain.r" in told["corrector"]
    assert "Rec.2100 HLG" in told["spaces"]
    assert "keys" in told["keyframes"]
    assert set(told["mixers"]) == {"parallel_mixer", "layer_mixer", "key_mixer"}
    refused = await desk.call("video_grade_info", reference=True, grade="c1")
    assert refused.error is not None


def test_the_tools_of_grades_cost_few_characters_on_every_request() -> None:
    sizes = {
        tool.name: len(
            json.dumps(
                {
                    "name": tool.schema.name,
                    "description": tool.schema.description,
                    "parameters": tool.schema.parameters,
                }
            )
        )
        for tool in tools_of(Videos(Config(), None))
        if tool.name.startswith(("video_grade", "video_color", "video_media", "video_shot"))
    }
    assert len(sizes) == 10
    # The six tools of T14 were 15,745 characters, a reference table of the settings within;
    # it is now told by video_grade_info with reference true.
    six = ("info", "set", "node", "copy", "lut", "group")
    assert sum(sizes[f"video_grade_{name}"] for name in six) < 9000
    assert sum(sizes.values()) < 13000
    assert max(sizes.values()) < 1800


async def test_the_engine_matches_a_cast_shot_of_bars_to_the_same_bars(
    ctx: Context, tmp_path: Path
) -> None:
    from columbina.video import edits
    from support.media import made, real_engine

    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    bars = made(
        desk.work, "bars.png", "-f", "lavfi", "-i", "smptebars=size=160x120", "-frames:v", "1"
    )
    (media,) = result(await desk.call("video_import", paths=[str(bars)]))["added"]
    batch = [
        edits.add_timeline(
            "Film", edits.Rate(num=25, den=1), id="$film", width=160, height=120,
            tracks=[edits.track("video")],
        ),
        *(
            edits.insert("$film", edits.of_media(media, length=50), at, audio_track=None)
            for at in (0, 50, 100)
        ),
    ]  # fmt: skip
    result(await desk.call("video_edit", edits=batch))
    clips = result(await desk.call("video_info"))["timelines"][0]["tracks"][0]["clips"]
    target, cast, same = (one["id"] for one in clips)

    # Colour management and the colour of media, read and set, each one step to undo.
    told = result(await desk.call("video_color_settings"))
    assert told["in_force"] == {"science": "yrgb", "managed": False}
    told = result(await desk.call("video_color_settings", settings={"science": "yrgb_managed"}))
    assert told["in_force"]["timeline"] == "Rec.709 Gamma 2.4"
    result(await desk.call("video_undo"))
    assert result(await desk.call("video_color_settings"))["color"]["science"] == "yrgb"
    told = result(await desk.call("video_media_color", media=[media], levels="full"))
    assert told["media"][0]["color_override"]["levels"] == "full"
    result(await desk.call("video_undo"))
    listed = result(await desk.call("video_media_color"))["media"]
    assert listed[0]["color_override"] == {"input": None, "levels": "auto"}

    # A cast of a grade on the second clip: red up, blue down, green lifted.
    cast_settings = {
        "primaries.gain.r": 1.12,
        "primaries.gain.b": 0.88,
        "primaries.lift.g": 0.04,
        "primaries.lum_mix": 0.0,
    }
    result(await desk.call("video_grade_node", grade=cast, action="append", settings=cast_settings))
    wanted = result(await desk.call("video_grade_scopes", clip=target))
    before = result(await desk.call("video_grade_scopes", clip=cast, node=1))
    assert wanted["measured"] == "the frame as an export draws it"
    assert gaps(before, figures_told(wanted)) > 0.03

    told = result(await desk.call("video_shot_match", clips=[cast, same], target=target))
    matched, neutral = told["matched"]
    assert matched["clip"] == cast
    assert "added" not in matched
    assert neutral["added"] is True
    after = result(await desk.call("video_grade_scopes", clip=cast, node=1))
    # The bars of the matched clip measure as the target's, each level within 0.01.
    assert gaps(after, told["target"]) < 0.01
    assert gaps(after, figures_told(wanted)) < 0.01
    for name in ("gain", "gamma"):
        assert all(abs(one - 1) < 0.01 for one in neutral["shot_match"][name].values())
    assert all(abs(one) < 0.01 for one in neutral["shot_match"]["offset"].values())

    # Invisible: the grade shows the match, and its settings are the cast's alone.
    info = result(await desk.call("video_grade_info", grade=cast))
    assert info["nodes"][0]["shot_match"] is True
    assert info["nodes"][0]["settings"] == cast_settings
    assert result(await desk.call("video_grade_info", grade=target))["nodes"] == []

    # One undo takes back both matches and the node added for one.
    result(await desk.call("video_undo"))
    info = result(await desk.call("video_grade_info", grade=cast))
    assert "shot_match" not in info["nodes"][0]
    assert info["nodes"][0]["settings"] == cast_settings
    assert result(await desk.call("video_grade_info", grade=same))["nodes"] == []
    back = result(await desk.call("video_grade_scopes", clip=cast, node=1))
    assert gaps(back, figures_told(before)) < 1e-6


def figures_told(measured: dict[str, Any]) -> dict[str, Any]:
    """The percentiles of an answer of video_grade_scopes, as video_shot_match tells a target."""
    return {
        "percentiles": {name: measured[name]["percentiles"] for name in ("red", "green", "blue")}
    }
