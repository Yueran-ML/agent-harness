"""The agent's tools of the Fusion page: a composition read, and changed one transaction a call."""

import copy
import json
from collections.abc import AsyncIterator, Iterator
from pathlib import Path
from typing import Any, cast

import pytest

from columbina.kernel import Context
from columbina.video import Config, Videos, edits, fusion, tools_of
from columbina.video.grading import Choice, Flag
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
FIRST = {"op": "set_fusion", **AT, "value": edits.first_composition()}
"""What goes before the first change of a clip that has no composition."""


def case(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(one for one in CASES if one["name"] == name))


def film(fake: Fake) -> None:
    """Three clips of picture on t1, of 25 frames a second, and c9 of sound, none composed.

    c1 is on frames 100 to 149 and shows its media from its frame 19 on, at its own speed: its
    frame ``f`` of the timeline is the frame ``f - 81`` of its media. c2 is on frames 10 to 39
    and plays its media twice as fast, backward: it shows the frame 83 of its media first. c3 is
    a title on frames 40 to 59.
    """
    names = ("media at the timeline's rate", "twice as fast, backward", "a title")
    cases = [case(name) for name in names]
    for number, each in enumerate(cases, 1):
        each["clip"]["id"] = f"c{number}"
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


def linked(fake: Fake, id: str = "c1") -> set[str]:
    """The links of the composition of a clip as the fake engine keeps them."""
    links = clip(fake, id)["fusion"]["links"]
    return {f"{link['from']} -> {link['to']}.{link['input']}" for link in links}


def params(fake: Fake, node: str, id: str = "c1") -> dict[str, Any]:
    nodes = clip(fake, id)["fusion"]["nodes"]
    return next(one for one in nodes if one["id"] == node)["params"]


def told_node(told: dict[str, Any], id: str) -> dict[str, Any]:
    return next(one for one in told["nodes"] if one["id"] == id)


async def node(desk: Desk, action: str, clip: str = "c1", **asked: Any) -> dict[str, Any]:
    return result(await desk.call("video_fusion_node", clip=clip, action=action, **asked))


async def refused(desk: Desk, fake: Fake, tool: str, **asked: Any) -> str:
    """What a call that is refused before anything is sent says."""
    sent = len(fake.transactions)
    outcome = await desk.call(tool, clip="c1", **asked)
    assert outcome.error is not None, outcome.text
    assert outcome.error.code == "VIDEO_FUSION", outcome.text
    assert len(fake.transactions) == sent
    return outcome.text


async def blurred(desk: Desk, **asked: Any) -> str:
    """Give c1 a blur after its MediaIn1, and tell the id of the blur."""
    return (await node(desk, "add", kind="blur", after="MediaIn1", **asked))["added"]


# ---------------------------------------------------------------------------------------------
# The kinds and their settings
# ---------------------------------------------------------------------------------------------


def test_the_settings_and_their_bounds_are_those_of_the_engines_catalog() -> None:
    catalog = json.loads((ENGINE_DATA / "catalog.json").read_text(encoding="utf-8"))
    with_numbers: set[str] = set()
    for kind, known in fusion.KINDS.items():
        numbers = {
            path: spec for path, spec in known.settings.items() if isinstance(spec, fusion.Setting)
        }
        listed = catalog.get(f"fusion_{kind}", [])
        if numbers:
            with_numbers.add(f"fusion_{kind}")
        if kind == "polygon":
            # A number of a point is named by the place of the point, which the catalog stars.
            points = [one for one in listed if one["path"].startswith("points.*.")]
            listed = [one for one in listed if one not in points]
            assert [one["path"] for one in points] == [f"points.*.{axis}" for axis in fusion.POINT]
            numbers = {**numbers, **{f"points.*.{axis}": fusion.POINT[axis] for axis in "xy"}}
            listed += points
        assert [one["path"] for one in listed] == list(numbers), kind
        for one in listed:
            spec = numbers[one["path"]]
            found = (spec.default, spec.low, spec.high, spec.unit)
            assert found == (one["default"], one["low"], one["high"], one["unit"]), one["path"]
            assert (one["clock"], one["page"]) == ("source_frame", "fusion")
    assert {name for name in catalog if name.startswith("fusion_")} == with_numbers
    assert set(fusion.KINDS) - {name.removeprefix("fusion_") for name in with_numbers} == {
        "media_in",
        "media_out",
    }


def nodes_in(value: object) -> Iterator[dict[str, Any]]:
    """Every node of a composition in what the engine wrote: one has ``pass_through``."""
    if isinstance(value, dict):
        found = cast("dict[str, Any]", value)
        if {"id", "kind", "params", "pass_through"} <= set(found):
            yield found
        for inner in found.values():
            yield from nodes_in(inner)
    elif isinstance(value, list):
        for inner in cast("list[Any]", value):
            yield from nodes_in(inner)


def shape(value: object) -> object:
    """The fields of settings without their values: a number with keys is a number still."""
    if isinstance(value, dict):
        inner = cast("dict[str, Any]", value)
        return "number" if "keys" in inner else {name: shape(one) for name, one in inner.items()}
    if isinstance(value, bool):
        return "flag"
    if isinstance(value, int | float):
        return "number"
    return "list" if isinstance(value, list) else "word"


def test_a_new_node_has_every_setting_that_the_engine_writes_and_its_defaults() -> None:
    examples = json.loads((ENGINE_DATA / "ops-examples.json").read_text(encoding="utf-8"))
    written: dict[str, list[dict[str, Any]]] = {}
    for one in nodes_in(examples):
        written.setdefault(one["kind"], []).append(one["params"])
    assert set(written) <= set(fusion.KINDS)
    assert set(fusion.KINDS) - set(written) <= {"rectangle"}
    for kind, each in written.items():
        new = fusion.new_params(kind)
        for found in each:
            # The same fields, each of the same sort: none left out, none that the engine has not.
            assert shape(found) == shape(new), kind
            for path, spec in fusion.KINDS[kind].settings.items():
                value: Any = found
                for part in path.split("."):
                    value = value[part]
                if isinstance(spec, Choice):
                    assert value in spec.choices, (kind, path)
                elif isinstance(spec, Flag):
                    assert isinstance(value, bool), (kind, path)
    # The examples add these kinds as the engine makes them, every setting at its default.
    as_new = {kind for kind, each in written.items() if fusion.new_params(kind) in each}
    assert as_new >= {"media_in", "media_out", "merge", "transform", "background"}
    assert as_new >= {"ellipse", "blur"}
    # A rectangle, which the examples have not, is an ellipse with corners.
    rectangle, ellipse = fusion.new_params("rectangle"), fusion.new_params("ellipse")
    assert rectangle == {**ellipse, "corner_radius": 0}
    assert fusion.new_params("polygon")["points"] == []
    assert edits.first_composition() == {
        "nodes": [
            {"id": id, "kind": kind, "label": "", "pass_through": False, "pos": None, "params": {}}
            for id, kind in (("MediaIn1", "media_in"), ("MediaOut1", "media_out"))
        ],
        "links": [{"from": "MediaIn1", "to": "MediaOut1", "input": "input"}],
    }


def test_the_operations_of_a_composition_are_written_as_the_engine_reads_them() -> None:
    link = edits.fusion_link("n1", "", "mask")
    assert link == {"from": "n1", "to": "", "input": "mask"}
    colour = {"color": {"r": 0.1, "g": 0.2, "b": 0.6, "a": 1.0}}
    made = edits.fusion_node(
        "background", colour, id="$sky", label="Sky", pass_through=True, pos={"x": 120, "y": 30}
    )
    assert made == {
        "id": "$sky",
        "kind": "background",
        "label": "Sky",
        "pass_through": True,
        "pos": {"x": 120, "y": 30},
        "params": colour,
    }
    assert edits.add_fusion_node("t1", "c1", made, index=0, links=[link]) == {
        "op": "add_fusion_node",
        **AT,
        "node": made,
        "index": 0,
        "links": [link],
    }
    assert edits.add_fusion_node("t1", "c1", made) == {"op": "add_fusion_node", **AT, "node": made}
    assert edits.remove_fusion_node("t1", "c1", "n1", links=[]) == {
        "op": "remove_fusion_node",
        **AT,
        "id": "n1",
        "links": [],
    }
    # What is left out stays as it is; a place of None leaves the node for the editor to lay out.
    assert edits.set_fusion_node("t1", "c1", "n1") == {"op": "set_fusion_node", **AT, "id": "n1"}
    assert edits.set_fusion_node("t1", "c1", "n1", label="", pos=None, pass_through=False) == {
        "op": "set_fusion_node",
        **AT,
        "id": "n1",
        "label": "",
        "pass_through": False,
        "pos": None,
    }
    placed = edits.set_fusion_node("t1", "c1", "n1", pos={"x": 1.0, "y": 2.0}, params=colour)
    assert (placed["pos"], placed["params"]) == ({"x": 1.0, "y": 2.0}, colour)
    key = edits.Key(frame=24, value=0.5, interp="hold")
    assert edits.set_fusion_keys("t1", "c1", "n1.blend", edits.Keys(keys=[key])) == {
        "op": "set_fusion_keys",
        **AT,
        "path": "n1.blend",
        "param": {"keys": [{"frame": 24, "value": 0.5, "interp": "hold"}]},
    }
    assert edits.splice_fusion_keys("t1", "c1", "n1.blend", 1, 0, [key]) == {
        "op": "splice_fusion_keys",
        **AT,
        "path": "n1.blend",
        "at": 1,
        "remove": 0,
        "keys": [{"frame": 24, "value": 0.5, "interp": "hold"}],
    }
    assert edits.set_fusion("t1", "c1", None) == {"op": "set_fusion", **AT, "value": None}
    assert edits.connect_fusion("t1", "c1", link) == {"op": "connect_fusion", **AT, "link": link}
    assert edits.disconnect_fusion("t1", "c1", "n2", "mask") == {
        "op": "disconnect_fusion",
        **AT,
        "to": "n2",
        "input": "mask",
    }


def test_the_tools_of_the_fusion_page_are_given_and_tell_what_a_composition_is() -> None:
    tools = {tool.name: tool for tool in tools_of(Videos(Config(), None))}
    names = [name for name in tools if name.startswith("video_fusion")]
    assert names == ["video_fusion_info", "video_fusion_node", "video_fusion_keys"]
    assert [tools[name].effect for name in names] == ["read", "write", "write"]
    told = tools["video_fusion_node"].schema.description
    for kind in fusion.KINDS:
        assert kind in told, kind
    for word in (
        "LOWER LEFT", "counter-clockwise", "MediaIn1 -> MediaOut1.input", "mask_of",
        "Blur or pixelate a face", "Green screen", "picture-in-picture", "video_fusion_keys",
        "video_fusion_info", "video_grade_scopes",
    ):  # fmt: skip
        assert word in told, word
    schema = tools["video_fusion_node"].schema.parameters
    properties = cast("dict[str, Any]", schema["properties"])
    taken = {"clip", "action", "node", "kind", "settings", "label", "pass_through", "after"}
    assert taken | {"mask_of", "links", "from", "input", "bridge"} <= set(properties)
    assert properties["kind"]["enum"] == list(fusion.KINDS)
    sizes = {
        name: len(tools[name].schema.description) + len(json.dumps(tools[name].schema.parameters))
        for name in names
    }
    # What the three cost on every request: the node tool carries what a composition is.
    assert sizes["video_fusion_info"] < 2000
    assert sizes["video_fusion_keys"] < 1600
    assert sum(sizes.values()) < 9000


async def test_the_reference_tells_every_kind_with_its_inputs_and_settings(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(await desk.call("video_fusion_info", reference=True))
    assert list(told["kinds"]) == list(fusion.KINDS)
    assert told["kinds"]["merge"]["inputs"] == ["background", "foreground", "mask"]
    assert told["kinds"]["media_out"]["output"] is False
    assert told["kinds"]["blur"]["settings"]["size.x"] == (
        "Blur Size across: 0 to 500, 5 at first, in pixels of a picture 1080 high"
    )
    view = told["kinds"]["delta_keyer"]["settings"]["view"]
    assert view.startswith("View Mode")
    assert "one of pre_matte, matte, status, final_result; final_result at first" in view
    assert "LOWER LEFT" in told["coordinates"]
    assert "video_fusion_keys" in told["keys"]
    alone = await refused(desk, fake, "video_fusion_info", reference=True)
    assert "reference is told alone" in alone


# ---------------------------------------------------------------------------------------------
# Reading
# ---------------------------------------------------------------------------------------------


async def test_a_clip_without_a_composition_is_told_to_show_its_source(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(await desk.call("video_fusion_info", clip="c1"))
    assert (told["timeline"], told["clip"], told["composition"]) == ("t1", "c1", False)
    assert "shows its source" in told["shows"]
    assert "MediaIn1 -> MediaOut1.input" in told["shows"]
    assert "nodes" not in told
    assert result(await desk.call("video_fusion_info")) == {"timeline": "t1", "compositions": []}
    # Reading sends nothing, and gives the clip no composition.
    assert fake.transactions == []
    assert "fusion" not in clip(fake)

    sound = await desk.call("video_fusion_info", clip="c9")
    assert sound.error is not None
    assert sound.error.code == "VIDEO_FUSION"
    assert "c9 is a clip of sound on A1" in sound.text
    assert "wrong_track fusion" in sound.text
    none = await desk.call("video_fusion_info", clip="c7")
    assert none.error is not None
    assert "there is no clip c7 on timeline t1" in none.text


async def test_a_composition_is_told_with_its_nodes_its_links_and_what_nothing_reads(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk, label="Face", settings={"size.x": 20, "filter": "box"})
    await node(desk, "add", kind="background")
    told = result(await desk.call("video_fusion_info", clip="c1"))

    assert told["composition"] is True
    assert [(one["id"], one["kind"]) for one in told["nodes"]] == [
        ("MediaIn1", "media_in"),
        ("MediaOut1", "media_out"),
        (blur, "blur"),
        ("n21", "background"),
    ]
    assert told_node(told, "MediaIn1") == {
        "id": "MediaIn1",
        "kind": "media_in",
        "label": "",
        "pass_through": False,
    }
    assert told_node(told, "MediaOut1")["inputs"] == {"input": blur}
    assert told_node(told, blur) == {
        "id": blur,
        "kind": "blur",
        "label": "Face",
        "pass_through": False,
        "inputs": {"input": "MediaIn1"},
        "free": ["mask"],
        "settings": {"filter": "box", "lock_xy": True, "size.x": 20, "size.y": 5, "blend": 1},
    }
    assert told_node(told, "n21")["free"] == ["mask"]
    assert told_node(told, "n21")["settings"] == {
        "color.r": 0,
        "color.g": 0,
        "color.b": 0,
        "color.a": 1,
    }
    assert set(told["links"]) == {f"MediaIn1 -> {blur}.input", f"{blur} -> MediaOut1.input"}
    assert set(told["links"]) == linked(fake)
    assert told["output"] == blur
    # The background is linked to nothing: nothing reads it, and the picture is made without it.
    assert (told["unread"], told["unused"]) == (["n21"], ["n21"])
    assert set(told["settings_of"]) == {"blur", "background"}
    assert told["settings_of"]["blur"]["size.x"] == (
        "Blur Size across: 0 to 500, 5 at first, in pixels of a picture 1080 high"
    )
    assert told["settings_of"]["blur"]["filter"] == (
        "Filter: one of gaussian, box; gaussian at first; no keys"
    )
    assert told["key_frames"] == "frames of the clip's media, from 0"
    listed = result(await desk.call("video_fusion_info"))
    assert listed == {
        "timeline": "t1",
        "compositions": [
            {
                "clip": "c1",
                "track": "V1",
                "start": 100,
                "kinds": ["media_in", "media_out", "blur", "background"],
            }
        ],
    }


# ---------------------------------------------------------------------------------------------
# The graph
# ---------------------------------------------------------------------------------------------


async def test_a_node_added_after_another_is_laid_into_the_chain_in_one_transaction(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = await node(
        desk,
        "add",
        kind="blur",
        after="MediaIn1",
        label="Face",
        settings={"size.x": 20, "filter": "box"},
    )

    # The clip had no composition: the first one goes before the node, in the same transaction.
    blur = {
        "id": "",
        "kind": "blur",
        "label": "Face",
        "pass_through": False,
        "pos": None,
        "params": {"filter": "box", "lock_xy": True, "size": {"x": 20, "y": 5}, "blend": 1},
    }
    into = [
        {"from": "MediaIn1", "to": "", "input": "input"},
        {"from": "", "to": "MediaOut1", "input": "input"},
    ]
    assert fake.transactions == [
        [FIRST, {"op": "add_fusion_node", **AT, "node": blur, "links": into}]
    ]
    assert (told["revision"], told["operations"], told["added"]) == (1, 2, "n20")
    assert linked(fake) == {"MediaIn1 -> n20.input", "n20 -> MediaOut1.input"}
    assert told["output"] == "n20"
    assert (told["unread"], told["unused"]) == ([], [])
    assert len(fake.steps) == 1

    # A clip that has one is sent the node alone; the node goes between n20 and what read it.
    told = await node(desk, "add", kind="transform", after="n20", settings={"size": 1.5})
    (added,) = fake.transactions[-1]
    assert added["node"]["params"] == {
        "center": {"x": 0.5, "y": 0.5},
        "size": 1.5,
        "angle": 0,
        "flip_horizontal": False,
        "flip_vertical": False,
    }
    assert added["links"] == [
        {"from": "n20", "to": "", "input": "input"},
        {"from": "", "to": "MediaOut1", "input": "input"},
    ]
    assert told["added"] == "n21"
    assert linked(fake) == {
        "MediaIn1 -> n20.input",
        "n20 -> n21.input",
        "n21 -> MediaOut1.input",
    }
    # A node that is linked to nothing is added so, and told as one that nothing reads.
    told = await node(desk, "add", kind="media_in")
    assert "links" not in fake.transactions[-1][0]
    assert (told["added"], told["unread"]) == ("n22", ["n22"])
    # Each call is one step: undone, the last node is gone; undone thrice, the composition.
    result(await desk.call("video_undo"))
    assert [one["id"] for one in clip(fake)["fusion"]["nodes"]][2:] == ["n20", "n21"]
    result(await desk.call("video_undo"))
    result(await desk.call("video_undo"))
    assert "fusion" not in clip(fake)

    # What cannot come after a node, or be added at all, is said before anything is sent.
    said = await refused(
        desk, fake, "video_fusion_node", action="add", kind="background", after="MediaIn1"
    )
    assert "a background has no input of a picture to come after MediaIn1 by" in said
    said = await refused(
        desk, fake, "video_fusion_node", action="add", kind="blur", after="MediaOut1"
    )
    assert "MediaOut1 is the media_out, which has no output" in said
    assert "no_output" in said
    said = await refused(desk, fake, "video_fusion_node", action="add", kind="media_out")
    assert "one_media_out" in said
    said = await refused(desk, fake, "video_fusion_node", action="add", kind="blur", after="n9")
    assert "the composition has no node n9" in said
    assert "its nodes are MediaIn1 (media_in), MediaOut1 (media_out)" in said
    said = await refused(desk, fake, "video_fusion_node", action="add")
    assert "add needs kind" in said
    said = await refused(desk, fake, "video_fusion_node", action="add", kind="blur", node="n20")
    assert "node is not for add" in said
    said = await refused(
        desk, fake, "video_fusion_node", action="add", kind="ellipse", after="n1", mask_of="n2"
    )
    assert "give one of after, mask_of and links" in said


async def test_a_mask_is_linked_into_the_mask_of_the_node_that_it_limits(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)
    face = {"center.x": 0.4, "center": {"y": 0.6}, "width": 0.2, "height": 0.25, "soft_edge": 0.02}
    told = await node(desk, "add", kind="ellipse", mask_of=blur, settings=face)

    (added,) = fake.transactions[-1]
    assert added["links"] == [{"from": "", "to": blur, "input": "mask"}]
    # Every setting of the mask is written: what is not given is as a new ellipse has it.
    assert added["node"]["params"] == {
        "level": 1,
        "soft_edge": 0.02,
        "border_width": 0,
        "invert": False,
        "solid": True,
        "paint_mode": "merge",
        "center": {"x": 0.4, "y": 0.6},
        "width": 0.2,
        "height": 0.25,
        "angle": 0,
    }
    ellipse = told["added"]
    assert told_node(told, blur)["inputs"] == {"input": "MediaIn1", "mask": ellipse}
    assert "free" not in told_node(told, blur)
    # The mask is read through the mask of the blur: the picture depends on it.
    assert (told["unread"], told["unused"]) == ([], [])

    # An input takes one link: a second mask is combined with the first, through its mask.
    said = await refused(
        desk, fake, "video_fusion_node", action="add", kind="rectangle", mask_of=blur
    )
    assert f"the mask of {blur} is linked from {ellipse}" in said
    assert f"mask_of {ellipse}" in said
    told = await node(desk, "add", kind="polygon", mask_of=ellipse)
    polygon = told["added"]
    assert f"{polygon} -> {ellipse}.mask" in linked(fake)
    assert told_node(told, polygon)["settings"]["points"] == []
    assert told_node(told, polygon)["note"] == "fewer than three points: a mask of nothing"

    said = await refused(desk, fake, "video_fusion_node", action="add", kind="ellipse", after=blur)
    assert "an ellipse is a mask, which goes into no chain of pictures: give mask_of" in said
    said = await refused(desk, fake, "video_fusion_node", action="add", kind="blur", mask_of=blur)
    assert "mask_of is for a mask (rectangle, ellipse, polygon)" in said
    said = await refused(
        desk, fake, "video_fusion_node", action="add", kind="ellipse", mask_of="MediaIn1"
    )
    assert "MediaIn1 is a media_in, which has no input 'mask': it has no inputs" in said
    assert "no_input" in said


async def test_a_keyer_is_merged_over_a_background_by_links_that_are_given(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    screen = {"screen": {"r": 0.08, "g": 0.45, "b": 0.12}, "gain": 1.5}
    told = await node(desk, "add", kind="delta_keyer", after="MediaIn1", settings=screen)
    keyer = told["added"]
    assert params(fake, keyer) == {
        **fusion.new_params("delta_keyer"),
        "screen": {"r": 0.08, "g": 0.45, "b": 0.12},
        "gain": 1.5,
    }
    back = (await node(desk, "add", kind="background", settings={"color.b": 0.6}))["added"]
    links = [
        {"from": back, "to": "", "input": "background"},
        {"from": keyer, "to": "", "input": "foreground"},
        {"from": "", "to": "MediaOut1", "input": "input"},
    ]
    told = await node(desk, "add", kind="merge", links=links)

    (added,) = fake.transactions[-1]
    assert added["links"] == links
    assert added["node"]["params"] == fusion.new_params("merge")
    merge = told["added"]
    # The link into the media_out takes the place of the one that it had.
    assert linked(fake) == {
        f"MediaIn1 -> {keyer}.input",
        f"{back} -> {merge}.background",
        f"{keyer} -> {merge}.foreground",
        f"{merge} -> MediaOut1.input",
    }
    assert told["output"] == merge
    assert told_node(told, merge)["inputs"] == {"background": back, "foreground": keyer}
    assert told_node(told, merge)["free"] == ["mask"]
    assert (told["unread"], told["unused"]) == ([], [])

    async def wrong(links: list[dict[str, str]], kind: str = "merge") -> str:
        return await refused(desk, fake, "video_fusion_node", action="add", kind=kind, links=links)

    # Each link has the new node at one of its ends, and names what there is.
    said = await wrong([{"from": back, "to": merge, "input": "mask"}])
    assert 'the new node being ""' in said
    assert "out_of_range links" in said
    said = await wrong([{"from": back, "to": ""}])
    assert 'the new node being ""' in said
    said = await wrong([{"from": "n99", "to": "", "input": "background"}])
    assert "the composition has no node n99" in said
    said = await wrong([{"from": back, "to": "", "input": "input"}])
    assert "a merge has no input 'input': it has background, foreground, mask" in said
    said = await wrong([{"from": "", "to": keyer, "input": "background"}])
    assert f"node {keyer} is a delta_keyer, which has no input 'background'" in said
    twice = [{"from": back, "to": "", "input": "mask"}, {"from": keyer, "to": "", "input": "mask"}]
    assert "input_linked" in await wrong(twice)

    # What the engine checks of a keyer's settings together is checked before it is sent.
    crossed = await refused(
        desk,
        fake,
        "video_fusion_node",
        action="set",
        node=keyer,
        settings={"low": 0.8, "high": 0.2},
    )
    assert "low (0.8) is above high (0.2)" in crossed
    assert "out_of_range delta_keyer.low" in crossed
    grey = await refused(
        desk,
        fake,
        "video_fusion_node",
        action="add",
        kind="delta_keyer",
        settings={"screen": {"r": 0.5, "g": 0.5, "b": 0.5}},
    )
    assert "screen is r 0.5, g 0.5, b 0.5" in grey
    assert "out_of_range delta_keyer.screen" in grey


async def test_a_node_is_removed_and_what_read_it_reads_what_it_read(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)
    turned = (await node(desk, "add", kind="transform", after=blur))["added"]
    mask = (await node(desk, "add", kind="ellipse", mask_of=blur))["added"]

    # The blur hands on its input: what it fed is joined to what fed that, and not to its mask.
    told = await node(desk, "remove", node=blur)
    assert fake.transactions[-1] == [
        {
            "op": "remove_fusion_node",
            **AT,
            "id": blur,
            "links": [{"from": "MediaIn1", "to": turned, "input": "input"}],
        }
    ]
    assert linked(fake) == {f"MediaIn1 -> {turned}.input", f"{turned} -> MediaOut1.input"}
    assert told["output"] == turned
    assert (told["unread"], told["unused"]) == ([mask], [mask])
    assert len(fake.steps) == 4

    # Without the bridge, what the node fed is left without a picture: the clip is transparent.
    told = await node(desk, "remove", node=turned, bridge=False)
    assert fake.transactions[-1] == [{"op": "remove_fusion_node", **AT, "id": turned}]
    assert linked(fake) == set()
    assert told["output"] is None
    assert told["unread"] == ["MediaIn1", mask]
    assert told_node(told, "MediaOut1")["free"] == ["input"]
    # A node that makes its picture of nothing has nothing to join across.
    await node(desk, "remove", node=mask)
    assert fake.transactions[-1] == [{"op": "remove_fusion_node", **AT, "id": mask}]

    said = await refused(desk, fake, "video_fusion_node", action="remove", node="MediaOut1")
    assert "the media_out is not taken away" in said
    assert "one_media_out" in said
    said = await refused(desk, fake, "video_fusion_node", action="remove", node=blur)
    assert f"the composition has no node {blur}" in said
    assert "its nodes are MediaIn1 (media_in), MediaOut1 (media_out)" in said
    said = await refused(desk, fake, "video_fusion_node", action="remove")
    assert "remove needs node" in said


async def test_links_are_connected_and_disconnected_one_at_a_time(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    # The first change of a clip without a composition gives it the first one, whatever it is.
    told = await node(desk, "disconnect", node="MediaOut1", input="input")
    assert fake.transactions == [
        [FIRST, {"op": "disconnect_fusion", **AT, "to": "MediaOut1", "input": "input"}]
    ]
    assert told["links"] == []
    assert told["output"] is None
    assert (told["unread"], told["unused"]) == (["MediaIn1"], ["MediaIn1"])

    link = {"from": "MediaIn1", "to": "MediaOut1", "input": "input"}
    told = await node(desk, "connect", **{"from": "MediaIn1"}, node="MediaOut1", input="input")
    assert fake.transactions[-1] == [{"op": "connect_fusion", **AT, "link": link}]
    assert told["links"] == ["MediaIn1 -> MediaOut1.input"]
    assert told["output"] == "MediaIn1"

    # A link into an input that has one takes its place.
    blur = (await node(desk, "add", kind="blur"))["added"]
    await node(desk, "connect", **{"from": "MediaIn1"}, node=blur, input="input")
    told = await node(desk, "connect", **{"from": blur}, node="MediaOut1", input="input")
    assert linked(fake) == {f"MediaIn1 -> {blur}.input", f"{blur} -> MediaOut1.input"}
    assert told["output"] == blur

    async def wrong(**asked: Any) -> str:
        return await refused(desk, fake, "video_fusion_node", **asked)

    said = await wrong(action="connect", **{"from": blur}, node="MediaOut1", input="input")
    assert f"{blur} is linked into MediaOut1.input already" in said
    assert "empty" in said
    said = await wrong(action="connect", **{"from": blur}, node="MediaOut1", input="mask")
    assert "MediaOut1 is a media_out, which has no input 'mask': it has input" in said
    said = await wrong(action="connect", **{"from": "MediaOut1"}, node=blur, input="mask")
    assert "MediaOut1 is the media_out, which has no output" in said
    # A node that takes its picture through another is not linked back into it.
    turned = (await node(desk, "add", kind="transform", after=blur))["added"]
    said = await wrong(action="connect", **{"from": turned}, node=blur, input="mask")
    assert f"{turned} takes its picture through {blur}: the link would close a loop" in said
    said = await wrong(action="connect", **{"from": blur}, node=blur, input="mask")
    assert "loop" in said
    said = await wrong(action="connect", **{"from": blur}, node=turned)
    assert "connect needs input" in said
    said = await wrong(action="disconnect", node=blur, input="mask")
    assert f"nothing is linked into {blur}.mask: {blur} has input from MediaIn1" in said
    said = await wrong(action="disconnect", node=blur, input="foreground")
    assert "which has no input 'foreground': it has input, mask" in said


async def test_the_composition_is_taken_away_and_the_clip_shows_its_source_again(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    said = await refused(desk, fake, "video_fusion_node", action="reset")
    assert "clip c1 has no composition to take away" in said
    await blurred(desk)
    told = await node(desk, "reset")
    assert fake.transactions[-1] == [{"op": "set_fusion", **AT, "value": None}]
    assert "fusion" not in clip(fake)
    assert told["composition"] is False
    assert "shows its source" in told["shows"]
    said = await refused(desk, fake, "video_fusion_node", action="reset", node="n20")
    assert "node is not for reset, which takes the clip alone" in said


# ---------------------------------------------------------------------------------------------
# The settings of a node
# ---------------------------------------------------------------------------------------------


async def test_the_settings_of_a_node_are_patched_and_what_is_wrong_is_said(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)

    async def wrong(**asked: Any) -> str:
        return await refused(desk, fake, "video_fusion_node", action="set", **asked)

    # A patch by paths with dots or by objects: only what is given is sent.
    told = await node(desk, "set", node=blur, settings={"size.x": 40, "lock_xy": False})
    patch = {"size": {"x": 40}, "lock_xy": False}
    assert fake.transactions[-1] == [{"op": "set_fusion_node", **AT, "id": blur, "params": patch}]
    assert told_node(told, blur)["settings"] == {
        "filter": "gaussian",
        "lock_xy": False,
        "size.x": 40,
        "size.y": 5,
        "blend": 1,
    }
    await node(desk, "set", node=blur, settings={"size": {"y": 12.5}, "filter": "box"})
    patch = {"size": {"y": 12.5}, "filter": "box"}
    assert fake.transactions[-1] == [{"op": "set_fusion_node", **AT, "id": blur, "params": patch}]
    assert params(fake, blur) == {
        "filter": "box",
        "lock_xy": False,
        "size": {"x": 40, "y": 12.5},
        "blend": 1,
    }
    # What is there already is not sent again; all of it there already is refused, as the
    # engine refuses a change that changes nothing.
    await node(desk, "set", node=blur, settings={"size.x": 40, "blend": 0.5})
    assert fake.transactions[-1][0]["params"] == {"blend": 0.5}
    said = await wrong(node=blur, settings={"size.x": 40, "filter": "box"})
    assert f"every value given is what node {blur} has already" in said
    assert "empty" in said

    # A number out of its range is refused with its bounds, its unit and the engine's name.
    said = await wrong(node=blur, settings={"size.x": 501})
    assert (
        "size.x is 501, out of range (Blur Size across: 0 to 500, 5 at first, in pixels of a "
        "picture 1080 high); the engine refuses it as out_of_range blur.size.x"
    ) in said
    said = await wrong(node=blur, settings={"blend": -0.1})
    assert "blend is -0.1, out of range" in said
    assert "0 to 1, 1 at first" in said
    # A setting that the kind has not, with the nearest that it has, and all of them.
    said = await wrong(node=blur, settings={"sise.x": 3})
    assert "a blur has no setting 'sise.x' (did you mean size.x?)" in said
    assert "it has filter, lock_xy, size.x, size.y, blend" in said
    said = await wrong(node=blur, settings={"size": 3})
    assert "a blur has no setting 'size' (did you mean size.x, size.y?)" in said
    said = await wrong(node=blur, settings={"radius": 3})
    assert "a blur has no setting 'radius'; it has filter" in said
    # A word is one of its spellings, a flag true or false, a number a number.
    said = await wrong(node=blur, settings={"filter": "median"})
    assert "filter (Filter) is one of gaussian, box" in said
    said = await wrong(node=blur, settings={"lock_xy": "yes"})
    assert "lock_xy (Lock X/Y" in said
    assert "is true or false" in said
    said = await wrong(node=blur, settings={"blend": "half"})
    assert "blend is a number (Blend" in said
    keyed = {"keys": [{"frame": 0, "value": 1}]}
    said = await wrong(node=blur, settings={"blend": keyed})
    assert "blend takes a number here" in said
    assert "video_fusion_keys puts its keys" in said
    # The node is one of the composition, and has settings.
    said = await wrong(node="n99", settings={"blend": 0.5})
    assert (
        "the composition has no node n99; its nodes are MediaIn1 (media_in), MediaOut1 "
        f"(media_out), {blur} (blur)"
    ) in said
    said = await wrong(node="MediaIn1", settings={"blend": 0.5})
    assert "a media_in has no settings" in said
    said = await wrong(node=blur)
    assert "give settings, label or pass_through to change" in said
    said = await wrong(node=blur, settings={"blend": 0.2}, after="MediaIn1")
    assert "after is not for set" in said


async def test_a_node_is_labelled_and_passed_through(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    blur = await blurred(desk)
    told = await node(desk, "set", node=blur, label="Soft", pass_through=True)
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": blur, "label": "Soft", "pass_through": True}
    ]
    shown = told_node(told, blur)
    assert (shown["label"], shown["pass_through"]) == ("Soft", True)
    # Only what changes is sent: the label is there already.
    await node(desk, "set", node=blur, label="Soft", pass_through=False)
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": blur, "pass_through": False}
    ]
    said = await refused(desk, fake, "video_fusion_node", action="set", node=blur, label="Soft")
    assert "nothing changes" in said
    said = await refused(desk, fake, "video_fusion_node", action="set", node=blur, label="x" * 101)
    assert "a label is at most 100 characters" in said
    # A node is labelled as it is added, too.
    await node(desk, "add", kind="mosaic", after=blur, label="Witness", pass_through=True)
    added = fake.transactions[-1][0]["node"]
    assert (added["label"], added["pass_through"]) == ("Witness", True)
    assert added["params"] == {
        "frequency": 32,
        "aspect": 1,
        "offset": {"x": 0, "y": 0},
        "whole_cells_only": False,
    }


async def test_the_points_of_a_polygon_are_set_whole_and_one_number_by_its_place(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)
    triangle = [[-0.2, -0.1], [0.2, -0.1], {"x": 0, "y": 0.1}]
    told = await node(desk, "add", kind="polygon", mask_of=blur, settings={"points": triangle})
    polygon = told["added"]
    points = [{"x": -0.2, "y": -0.1}, {"x": 0.2, "y": -0.1}, {"x": 0, "y": 0.1}]
    assert params(fake, polygon)["points"] == points
    assert told_node(told, polygon)["settings"]["points"] == points
    assert "note" not in told_node(told, polygon)
    assert "points.N.x" in told["settings_of"]["polygon"]["points"]

    async def wrong(**settings: Any) -> str:
        return await refused(
            desk, fake, "video_fusion_node", action="set", node=polygon, settings=settings
        )

    # One number of a point is put by the place of the point: the list is not sent again.
    raised = {"points.2.y": 0.2, "fill": "non_zero_winding"}
    await node(desk, "set", node=polygon, settings=raised)
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": polygon, "params": {"fill": "non_zero_winding"}},
        {"op": "set_fusion_keys", **AT, "path": f"{polygon}.points.2.y", "param": 0.2},
    ]
    assert params(fake, polygon)["points"][2] == {"x": 0, "y": 0.2}
    # The list is put whole, in place of the one there.
    square = [{"x": x, "y": y} for x, y in ((-0.2, -0.2), (0.2, -0.2), (0.2, 0.2), (-0.2, 0.2))]
    await node(desk, "set", node=polygon, settings={"points": square})
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": polygon, "params": {"points": square}}
    ]

    said = await wrong(**{"points.4.x": 0.1})
    assert f"node {polygon} has 4 points, numbered from 0: there is no points.4.x" in said
    said = await wrong(**{"points.0.x": 17})
    assert "points.0.x is 17, out of range" in said
    assert "out_of_range polygon.points.x" in said
    said = await wrong(points=[[0, 0], [0.1, 20], [0.2, 0]])
    assert "points.1.y is 20, out of range" in said
    assert "out_of_range polygon.points.y" in said
    said = await wrong(points=[[0, 0, 0]])
    assert "points is a list of" in said
    said = await wrong(points=[[0, 0]] * 129)
    assert "128 at most" in said
    assert "out_of_range polygon.points" in said
    said = await wrong(point=1)
    assert "a polygon has no setting 'point'" in said
    assert "points.N.x or points.N.y, N from 0" in said

    # A number that has keys is not put in their place by a patch: the keys would be lost.
    result(
        await desk.call(
            "video_fusion_keys",
            clip="c1",
            path=f"{polygon}.points.0.x",
            action="put",
            frame=110,
            value=-0.3,
        )
    )
    said = await wrong(**{"points.0.x": -0.25})
    assert f"points.0.x of node {polygon} has keys" in said
    said = await wrong(points=triangle)
    assert f"points of node {polygon} has keys" in said
    assert "video_fusion_keys" in said


# ---------------------------------------------------------------------------------------------
# Keys
# ---------------------------------------------------------------------------------------------


async def test_a_key_is_put_at_the_frame_of_the_picture_that_the_timeline_shows(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)
    path = f"{blur}.size.x"
    size = {"clip": "c1", "path": path}
    spliced = {"op": "splice_fusion_keys", **AT, "path": path}

    async def put(frame: int, value: float, **more: Any) -> dict[str, Any]:
        key = {"action": "put", "frame": frame, "value": value, **more}
        return result(await desk.call("video_fusion_keys", **size, **key))

    # A number takes its first key: frame 110 of the timeline is frame 29 of the clip's media.
    told = await put(110, 30)
    first = {"frame": 29, "value": 30.0, "interp": "linear"}
    assert fake.transactions[-1] == [{**spliced, "at": 0, "remove": 0, "keys": [first]}]
    assert params(fake, blur)["size"]["x"] == {"keys": [first]}
    assert told == {
        "revision": 2,
        "operations": 1,
        "timeline": "t1",
        "clip": "c1",
        "path": path,
        "key": {"timeline_frame": 110, "frame": 29},
        "key_frames": "frames of the clip's media, from 0",
        "setting": {"keys": [{"timeline_frame": 110, **first}]},
    }
    # A key before it goes before it, and one put where there is one takes its place.
    await put(100, 0, interp="hold")
    held = {"frame": 19, "value": 0.0, "interp": "hold"}
    assert fake.transactions[-1] == [{**spliced, "at": 0, "remove": 0, "keys": [held]}]
    told = await put(100, 5)
    assert fake.transactions[-1] == [
        {**spliced, "at": 0, "remove": 1, "keys": [{**held, "value": 5.0}]}
    ]
    assert [key["timeline_frame"] for key in told["setting"]["keys"]] == [100, 110]
    # The composition is told with the keys of the setting at the frames of the timeline.
    shown = result(await desk.call("video_fusion_info", clip="c1"))
    assert told_node(shown, blur)["settings"]["size.x"] == {
        "keys": [
            {"timeline_frame": 100, "frame": 19, "value": 5.0, "interp": "hold"},
            {"timeline_frame": 110, **first},
        ]
    }

    # A key is taken away by its frame of the timeline; the last leaves the number that it had.
    none = await refused(desk, fake, "video_fusion_keys", path=path, action="remove", frame=111)
    assert (
        f"{path} of clip c1 has no key at frame 111 of the timeline (frame 30 of the clip's "
        "media): its keys are at frames 100, 110 of the timeline"
    ) in none
    told = result(await desk.call("video_fusion_keys", **size, action="remove", frame=100))
    assert fake.transactions[-1] == [{**spliced, "at": 0, "remove": 1, "keys": []}]
    assert told["key"] == {"timeline_frame": 100, "frame": 19}
    told = result(await desk.call("video_fusion_keys", **size, action="remove", frame=110))
    number = {"op": "set_fusion_keys", **AT, "path": path}
    assert fake.transactions[-1] == [{**number, "param": 30.0}]
    assert told["setting"] == 30.0
    # Every key is cleared at once, to a value given or to that of the first.
    await put(105, 10)
    await put(120, 20)
    told = result(await desk.call("video_fusion_keys", **size, action="clear"))
    assert fake.transactions[-1] == [{**number, "param": 10.0}]
    assert "key" not in told
    await put(105, 10)
    result(await desk.call("video_fusion_keys", **size, action="clear", value=0))
    assert fake.transactions[-1] == [{**number, "param": 0}]
    assert params(fake, blur)["size"]["x"] == 0
    # Each call was one transaction of one operation, but the one that gave the composition.
    assert [len(ops) for ops in fake.transactions] == [2] + [1] * 10
    assert len(fake.steps) == 11

    # What is wrong is said before anything is sent.
    said = await refused(
        desk, fake, "video_fusion_keys", path=path, action="put", frame=99, value=3
    )
    assert "frame 99 is not in clip c1, which is at frames 100 to 149 of timeline t1" in said
    said = await refused(
        desk, fake, "video_fusion_keys", path=path, action="put", frame=110, value=501
    )
    assert "size.x is 501, out of range" in said
    assert "out_of_range blur.size.x" in said
    said = await refused(desk, fake, "video_fusion_keys", path=path, action="put", frame=110)
    assert "put needs value" in said
    said = await refused(desk, fake, "video_fusion_keys", path=path, action="remove")
    assert "remove needs frame" in said
    said = await refused(desk, fake, "video_fusion_keys", path=path, action="clear")
    assert "has no keys to clear" in said


async def test_the_keys_of_a_composition_count_the_frames_that_the_clip_counts(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    # c2 begins at frame 10 and plays twice as fast, backward: its first frame shows the frame
    # 83 of its media, as the engine's case says.
    backward = case("twice as fast, backward")
    assert dict(zip(backward["frames"], backward["keys"], strict=True))[10] == 83
    moved = (await node(desk, "add", "c2", kind="transform", after="MediaIn1"))["added"]
    asked = {"clip": "c2", "path": f"{moved}.center.x", "action": "put"}
    told = result(await desk.call("video_fusion_keys", **asked, frame=10, value=0.3))
    assert told["key"] == {"timeline_frame": 10, "frame": 83}
    assert told["key_frames"] == "frames of the clip's media, from 0"
    (sent,) = fake.transactions[-1]
    assert (sent["op"], sent["clip"], sent["path"]) == (
        "splice_fusion_keys",
        "c2",
        f"{moved}.center.x",
    )
    assert sent["keys"] == [{"frame": 83, "value": 0.3, "interp": "linear"}]
    # The keys rise by their own frames, so one later on the timeline goes before.
    told = result(await desk.call("video_fusion_keys", **asked, frame=39, value=0.7))
    assert told["key"] == {"timeline_frame": 39, "frame": 25}
    assert (fake.transactions[-1][0]["at"], fake.transactions[-1][0]["remove"]) == (0, 0)
    assert [key["timeline_frame"] for key in told["setting"]["keys"]] == [39, 10]
    assert params(fake, moved, "c2")["center"]["x"]["keys"][0]["frame"] == 25

    # A title has no frames of its own: its keys count the frames of the clip from its first.
    turned = (await node(desk, "add", "c3", kind="transform", after="MediaIn1"))["added"]
    told = result(
        await desk.call(
            "video_fusion_keys",
            clip="c3",
            path=f"{turned}.angle",
            action="put",
            frame=45,
            value=-90,
            interp="hold",
        )
    )
    assert told["key"] == {"timeline_frame": 45, "frame": 5}
    assert told["key_frames"] == "frames of the clip, from 0"
    assert told["setting"] == {
        "keys": [{"timeline_frame": 45, "frame": 5, "value": -90.0, "interp": "hold"}]
    }


async def test_only_a_number_of_a_node_that_is_there_takes_keys(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio

    async def wrong(path: str) -> str:
        asked = {"path": path, "action": "put", "frame": 110, "value": 0.5}
        return await refused(desk, fake, "video_fusion_keys", **asked)

    said = await wrong("n20.size.x")
    assert "clip c1 has no composition, and so no node n20" in said
    assert "missing fusion" in said
    blur = await blurred(desk)
    polygon = (await node(desk, "add", kind="polygon", mask_of=blur))["added"]

    said = await wrong(f"{blur}.filter")
    assert "filter (Filter) takes no keys: only a setting that is a number does" in said
    assert "video_fusion_node with action set sets it" in said
    said = await wrong(f"{blur}.lock_xy")
    assert "takes no keys" in said
    said = await wrong(f"{blur}.size")
    assert "a blur has no setting 'size' (did you mean size.x, size.y?)" in said
    said = await wrong(f"{blur}.blur")
    assert "a blur has no setting 'blur'" in said
    said = await wrong(blur)
    assert f"'{blur}' is not the path of a setting" in said
    said = await wrong("n99.blend")
    assert "the composition has no node n99; its nodes are MediaIn1 (media_in)" in said
    said = await wrong("MediaIn1.blend")
    assert "a media_in has no settings" in said
    said = await wrong(f"{polygon}.points")
    assert "points is a list, which takes no keys" in said
    said = await wrong(f"{polygon}.points.0.x")
    assert f"node {polygon} has 0 points, numbered from 0" in said


async def test_a_refusal_of_the_engine_is_told_with_what_can_be_done_about_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    blur = await blurred(desk)

    async def answered(**data: Any) -> Any:
        fake.refusals["transaction.apply"] = {"code": -32001, "message": "refused", "data": data}
        outcome = await desk.call(
            "video_fusion_node", clip="c1", action="set", node=blur, settings={"blend": 0.5}
        )
        assert outcome.error is not None
        return outcome

    outcome = await answered(reason="out_of_range", what="blur.size.x", index=0)
    assert outcome.error.code == "VIDEO_OUT_OF_RANGE"
    assert "size.x of a blur is Blur Size across: 0 to 500, 5 at first" in outcome.text
    outcome = await answered(reason="out_of_range", what="polygon.points.x", index=0)
    assert "points.x of a polygon is X of a point" in outcome.text
    outcome = await answered(reason="out_of_range", what="fusion_nodes", index=0)
    assert "a composition has 256 nodes at most" in outcome.text
    outcome = await answered(reason="loop", index=0)
    assert outcome.error.code == "VIDEO_LOOP"
    assert "links make no loop" in outcome.text
    outcome = await answered(reason="missing", what="fusion", id="c1", index=0)
    assert "video_fusion_node gives it one" in outcome.text
    outcome = await answered(reason="input_linked", node=blur, input="mask", index=0)
    assert "an input takes one link" in outcome.text
    # A reason that there is no more to say of is told as the engine told it.
    outcome = await answered(reason="locked", index=0)
    assert outcome.error.code == "VIDEO_LOCKED"
    assert params(fake, blur)["blend"] == 1
