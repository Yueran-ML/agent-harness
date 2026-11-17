"""The agent looks at frames of a timeline, pictures drawn off screen, against the fake engine."""

import base64
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any, cast

import pytest

from columbina import pictures
from columbina.fs import Access
from columbina.kernel import Context
from columbina.video import Config, Videos, look, tools_of
from support import fake_engine
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")


def film(fake: Fake) -> None:
    """t1 of 25 frames a second, 1920 by 1080, 140 frames long.

    V1 has c1 on frames 0 to 49, c2 on 50 to 99 and c4 on 100 to 139; V2, above it, has c5 on
    20 to 39; A1 has c9, of sound, on 0 to 49.
    """
    first = [clip_of("c1", 0, 50), clip_of("c2", 50, 50), clip_of("c4", 100, 40)]
    timeline = timeline_of(
        track_of("V1", first, kind="video"),
        track_of("V2", [clip_of("c5", 20, 20)], kind="video"),
        track_of("A1", [clip_of("c9", 0, 50)]),
    )
    fake.project.update(next_id=10, timelines=[timeline])


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        film(fake)
        yield desk, fake


def drawn(fake: Fake) -> list[dict[str, Any]]:
    """What each ``timeline.thumbnail`` was asked, in order."""
    return [params for method, params in fake.looked if method == "timeline.thumbnail"]


def clip(fake: Fake, id: str) -> dict[str, Any]:
    tracks = fake.project["timelines"][0]["tracks"]
    return next(one for track in tracks for one in track["clips"] if one["id"] == id)


async def test_frames_are_drawn_as_exported_and_given_back_as_pictures_in_order(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    outcome = await desk.call("video_look", frames=[12])
    told = result(outcome)
    assert drawn(fake) == [{"timeline": "t1", "frame": 12, "height": 270}]
    (picture,) = outcome.pictures
    assert picture.media_type == "image/png"
    png = base64.b64decode(picture.data)
    assert png == fake_engine.png_of_grey(480, 270, 12)
    assert pictures.kind_of(png) == "image/png"
    assert pictures.size_of(png) == (480, 270)
    assert told["timeline"] == "t1"
    assert told["shows"].startswith("each frame as an export draws it")
    assert told["pictures"] == [
        {"frame": 12, "seconds": 0.48, "on_top": "c1", "width": 480, "height": 270}
    ]
    assert "shown beside this text, in this order" in told["note"]
    assert "left_out" not in told

    # Four frames, in the order asked; the clip on top is of the highest track that shows one.
    frames = [75, 25, 0, 139]
    outcome = await desk.call("video_look", frames=frames, height=90)
    told = result(outcome)
    assert drawn(fake)[1:] == [{"timeline": "t1", "frame": one, "height": 90} for one in frames]
    assert [one["frame"] for one in told["pictures"]] == frames
    assert [one["on_top"] for one in told["pictures"]] == ["c2", "c5", "c1", "c4"]
    assert [(one["width"], one["height"]) for one in told["pictures"]] == [(160, 90)] * 4
    assert [base64.b64decode(one.data) for one in outcome.pictures] == [
        fake_engine.png_of_grey(160, 90, one) for one in frames
    ]
    assert [one.media_type for one in outcome.pictures] == ["image/png"] * 4


async def test_a_clip_is_looked_at_in_its_middle_or_alone_by_its_own_grades(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(await desk.call("video_look", clip="c2"))
    assert drawn(fake) == [{"timeline": "t1", "frame": 75, "height": 270}]
    assert told["pictures"][0]["frame"] == 75
    assert told["shows"].startswith("each frame as an export draws it")

    # Alone, the layer of the clip is drawn by itself: the engine is given the clip.
    told = result(await desk.call("video_look", clip="c5", alone=True))
    assert drawn(fake)[1:] == [{"timeline": "t1", "frame": 30, "clip": "c5", "height": 270}]
    assert told["shows"].startswith("clip c5 alone, fitted to the frame")
    outcome = await desk.call("video_look", clip="c5", alone=True, frames=[20, 39], height=180)
    told = result(outcome)
    assert drawn(fake)[2:] == [
        {"timeline": "t1", "frame": 20, "clip": "c5", "height": 180},
        {"timeline": "t1", "frame": 39, "clip": "c5", "height": 180},
    ]
    assert [one["frame"] for one in told["pictures"]] == [20, 39]
    assert len(outcome.pictures) == 2
    # Given frames of a clip without alone, the whole frame is drawn.
    result(await desk.call("video_look", clip="c4", frames=[100]))
    assert drawn(fake)[-1] == {"timeline": "t1", "frame": 100, "height": 270}


async def test_what_is_asked_is_checked_before_a_frame_is_drawn(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told: list[tuple[dict[str, Any], str]] = [
        ({"clip": "c2", "frames": [49]}, "not of clip c2, which lies on frames 50 to 99"),
        ({"clip": "c2", "frames": [60, 100]}, "frame 100 is not of clip c2"),
        ({"frames": [140]}, "frame 140 is outside timeline t1, whose frames are 0 to 139"),
        ({"frames": [-1]}, "frame -1 is outside timeline t1"),
        ({}, "give frames, or clip"),
        ({"frames": [3], "alone": True}, "give clip too"),
        ({"frames": [3, 3]}, "frame 3 is named twice"),
        ({"clip": "c9"}, "c9 is a clip of sound on A1"),
        ({"clip": "c7"}, "there is no clip c7 on timeline t1"),
        ({"frames": [3], "timeline": "t9"}, "there is no timeline t9"),
    ]
    for given, said in told:
        outcome = await desk.call("video_look", **given)
        assert outcome.error is not None, given
        assert outcome.error.code == "VIDEO_LOOK", (given, outcome.text)
        assert said in outcome.text, (given, outcome.text)
    # What the arguments themselves do not allow: more than four frames, a height out of range.
    unfit: list[tuple[dict[str, Any], str]] = [
        ({"frames": [1, 2, 3, 4, 5]}, "frames"),
        ({"frames": [1], "height": 89}, "height"),
        ({"frames": [1], "height": 271}, "height"),
    ]
    for given, field in unfit:
        outcome = await desk.call("video_look", **given)
        assert outcome.error is not None, given
        assert outcome.error.code == "INVALID_ARGS", (given, outcome.text)
        assert f"- {field}:" in outcome.text, (given, outcome.text)
    assert drawn(fake) == []


async def test_a_refusal_of_the_engine_is_told_with_what_to_do(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    fake.refusals["timeline.thumbnail"] = {
        "code": -32004,
        "message": "the card could not draw the frame",
        "data": {"reason": "draw"},
    }
    outcome = await desk.call("video_look", frames=[3])
    assert outcome.error is not None
    assert outcome.error.code == "VIDEO_DRAW"
    assert "the card could not draw the frame" in outcome.text
    assert "video_grade_scopes" in outcome.text
    del fake.refusals["timeline.thumbnail"]

    # A clip that is off is not seen, alone or on top.
    clip(fake, "c5")["enabled"] = False
    outcome = await desk.call("video_look", clip="c5", alone=True)
    assert outcome.error is not None
    assert outcome.error.code == "VIDEO_CLIP"
    assert "clip c5 is not seen at frame 30 of the timeline" in outcome.text
    assert "while it is on" in outcome.text
    told = result(await desk.call("video_look", frames=[25]))
    assert told["pictures"][0]["on_top"] == "c1"


async def test_pictures_larger_than_a_model_takes_in_one_result_are_left_out(
    studio: tuple[Desk, Fake], monkeypatch: pytest.MonkeyPatch
) -> None:
    desk, _ = studio
    sizes = [len(fake_engine.png_of_grey(160, 90, frame)) for frame in (10, 60, 110)]
    monkeypatch.setattr(pictures, "MOST_BYTES", sizes[0] + sizes[1])
    outcome = await desk.call("video_look", frames=[10, 60, 110], height=90)
    told = result(outcome)
    assert [one["frame"] for one in told["pictures"]] == [10, 60]
    assert len(outcome.pictures) == 2
    assert told["left_out"]["frames"] == [110]
    assert "lower height" in told["left_out"]["why"]
    monkeypatch.setattr(pictures, "MOST_BYTES", sizes[0] - 1)
    refused = await desk.call("video_look", frames=[10], height=90)
    assert refused.error is not None
    assert refused.error.code == "VIDEO_LOOK"
    assert "larger than a model takes" in refused.text
    assert refused.pictures == ()


def test_the_tool_is_offered_reads_only_and_costs_few_characters() -> None:
    tools = {tool.name: tool for tool in tools_of(Videos(Config(), None))}
    tool = tools["video_look"]
    assert tool.effect == "read"
    told = tool.schema.description
    for word in ("as an export draws it", "1 to 4", "alone", "own grades", "video_grade_scopes"):
        assert word in told, word
    properties = cast("dict[str, Any]", tool.schema.parameters["properties"])
    assert set(properties) == {"project", "timeline", "frames", "clip", "alone", "height"}
    assert properties["frames"]["maxItems"] == look.MOST_FRAMES == 4
    assert (properties["height"]["minimum"], properties["height"]["maximum"]) == (90, 270)
    assert properties["height"]["default"] == 270
    # What the tool costs on every request.
    assert len(told) < 600
    assert len(told) + len(json.dumps(tool.schema.parameters)) < 1200


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

    assert desk.sandbox.restricts(access("video_look", studio)) is None
    refused = desk.sandbox.restricts(access("video_look", settings))
    assert refused is not None
    assert refused.code == "PROTECTED"
    # Only the tools of the studio reach it: one that is not among them is refused.
    other = desk.sandbox.restricts(access("video_peek", studio))
    assert other is not None
    assert other.code == "PROTECTED"
