"""The Fusion tools of the agent against the engine itself: what the fake engine takes, the
engine takes too, for Text+, its animation by a preset and the Planar Tracker."""

from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.video import edits
from support.media import made, real_engine
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")


def nodes_of(told: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """The nodes of the composition that `video_fusion_info` tells, by their ids."""
    return {one["id"]: one for one in told["nodes"]}


async def test_a_text_plus_animated_by_presets_and_a_planar_tracker_are_taken_by_the_engine(
    ctx: Context, tmp_path: Path
) -> None:
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
        edits.insert("$film", edits.of_media(media, length=50), 0, audio_track=None),
    ]  # fmt: skip
    result(await desk.call("video_edit", edits=batch))
    (clip,) = result(await desk.call("video_info"))["timelines"][0]["tracks"][0]["clips"]
    at = clip["id"]

    # A typewriter on a clip without a composition: the first composition, a Merge and the
    # text over the picture, as one step that the engine takes as the fake does.
    told = result(await desk.call("video_text_animate", clip=at, preset="typewriter", text="Hello"))
    shown = result(await desk.call("video_fusion_info", clip=at))
    kinds = {one["kind"] for one in shown["nodes"]}
    assert {"merge", "text_plus"} <= kinds, shown
    text = next(one["id"] for one in shown["nodes"] if one["kind"] == "text_plus")
    assert text in str(told)

    # A second preset on the same text, its follower made whole; then a key of its size and an
    # outline added, each taken by the engine.
    result(await desk.call("video_text_animate", clip=at, preset="fade_in", node=text))
    put = {"path": f"{text}.size", "action": "put", "frame": 10, "value": 0.2}
    result(await desk.call("video_fusion_keys", clip=at, **put))
    result(
        await desk.call(
            "video_fusion_node", clip=at, action="set", node=text, settings={"outline": {}}
        )
    )
    shown = nodes_of(result(await desk.call("video_fusion_info", clip=at)))
    assert text in shown

    # A Planar Tracker after MediaIn1 with a region and Steady: the engine takes it as it is
    # written, and says it has no path yet.
    square = [[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]]
    settings = {"operation": "steady", "region": square, "motion": "perspective"}
    added = result(
        await desk.call(
            "video_fusion_node",
            clip=at,
            action="add",
            kind="planar_tracker",
            after="MediaIn1",
            settings=settings,
        )
    )["added"]
    shown = nodes_of(result(await desk.call("video_fusion_info", clip=at)))
    assert shown[added]["kind"] == "planar_tracker"
    assert shown[added]["settings"]["operation"] == "steady"
    assert shown[added]["settings"]["region"] == {"places": 4}

    # Each was one step: undone one by one, the clip has no Planar Tracker, then no outline.
    result(await desk.call("video_undo"))
    shown = nodes_of(result(await desk.call("video_fusion_info", clip=at)))
    assert added not in shown
