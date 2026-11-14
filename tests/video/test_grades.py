"""The agent's tools of grades: each change one transaction of the engine, told in short."""

import copy
import json
from collections.abc import AsyncIterator
from itertools import pairwise
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.tools import ToolOutcome, ToolRun
from columbina.video import grading
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")


def corrector(id: str, **params: Any) -> dict[str, Any]:
    """A corrector as the engine writes it, its settings patched."""
    node: dict[str, Any] = {"id": id, "kind": "corrector", "label": "", "enabled": True}
    node["params"] = grading.new_corrector()
    for path, value in params.items():
        place = node["params"]
        *heads, last = path.split("__")
        for head in heads:
            place = place[head]
        place[last] = value
    return node


def chain(*ids: str) -> list[dict[str, Any]]:
    """The links of nodes in a row from the source to the output, in the engine's order."""
    ends = ["source", *ids, "output"]
    links = [{"from": one, "to": two, "kind": "rgb", "input": 0} for one, two in pairwise(ends)]
    return sorted(links, key=lambda one: one["to"])


def graded(*nodes: dict[str, Any]) -> dict[str, Any]:
    return {"nodes": list(nodes), "links": chain(*(one["id"] for one in nodes))}


def project(fake: Fake) -> None:
    """Three clips of picture and one of sound on t1; c2 graded with two nodes."""
    pictures = [clip_of("c1", 0, 50), clip_of("c2", 50, 50), clip_of("c4", 100, 50)]
    for clip in pictures:
        clip["grade"] = copy.deepcopy(grading.EMPTY_GRADE)
        clip["group"] = None
    pictures[1]["grade"] = graded(
        corrector("n3", primaries__saturation=30.0),
        corrector("n5", lut="LUTs/film.cube"),
    )
    timeline = timeline_of(
        track_of("V1", pictures, kind="video"), track_of("A1", [clip_of("c3", 0, 50)])
    )
    timeline["grade"] = copy.deepcopy(grading.EMPTY_GRADE)
    fake.project.update(schema=3, next_id=10, timelines=[timeline], groups=[])


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        project(fake)
        yield desk, fake


async def grouped(desk: Desk, **arguments: Any) -> ToolOutcome:
    """Call video_grade_group, whose name of a group is the name that Desk.call takes."""
    asked = ToolCallBlock(
        call_id="group", name="video_grade_group", arguments=json.dumps(arguments)
    )
    run = ToolRun("group", desk.work, desk.session)
    return await desk.tools.begin(desk.ctx, asked, run).execute()


def clip_grade(fake: Fake, id: str) -> dict[str, Any]:
    clips = fake.project["timelines"][0]["tracks"][0]["clips"]
    return next(one for one in clips if one["id"] == id)["grade"]


async def test_a_grade_is_told_by_its_nodes_and_only_what_they_change(
    studio: tuple[Desk, Fake],
) -> None:
    desk, _ = studio
    told = result(await desk.call("video_grade_info", grade="c2"))
    assert told["grade"] == {"level": "clip", "timeline": "t1", "clip": "c2"}
    assert told["complete"] is True
    assert told["output"] == "n5"
    first, second = told["nodes"]
    assert first == {
        "number": 1,
        "id": "n3",
        "kind": "corrector",
        "label": "",
        "enabled": True,
        "input": "source",
        "settings": {"primaries.saturation": 30.0},
    }
    assert second["input"] == "n3"
    assert second["settings"] == {}
    assert second["lut"] == "LUTs/film.cube"
    assert "lut_missing" not in second
    empty = result(await desk.call("video_grade_info", grade={"level": "timeline"}))
    assert empty["nodes"] == []
    assert empty["output"] == "source"


async def test_a_corrector_is_added_with_its_settings_in_one_undo_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(
        await desk.call(
            "video_grade_node",
            grade="c1",
            action="append",
            label="Mono",
            settings={"primaries.saturation": 0},
        )
    )
    assert fake.transactions == [
        [
            {"edit": "append_node", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"}},
            {
                "op": "set_node",
                "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
                "id": "n10",
                "label": "Mono",
                "params": {"primaries": {"saturation": 0}},
            },
        ]
    ]
    assert told["added"] == "n10"
    (summary,) = told["grades"]
    assert summary["nodes"][0]["label"] == "Mono"
    assert summary["nodes"][0]["settings"] == {"primaries.saturation": 0}
    assert len(fake.steps) == 1
    result(await desk.call("video_undo"))
    assert clip_grade(fake, "c1")["nodes"] == []


async def test_settings_out_of_range_or_unknown_are_told_in_davincis_units_and_not_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    calls: list[tuple[dict[str, Any], str]] = [
        ({"primaries.saturation": 150}, "Saturation (Sat): 0 to 100, 50 changes nothing"),
        ({"primaries.saturaton": 10}, "did you mean saturation?"),
        ({"primaries": {"lift": 0.1}}, "primaries.lift holds the settings y, r, g, b"),
        ({"primaries.offset.r": -80}, "Offset red: -75 to 125, 25 changes nothing"),
        ({"curves.y.points": [[0, 0]]}, "2 points at least"),
        ({"hsl.hue_vs_sat": [[0.5, 0.2], [0.4, 0.1]]}, "in from 0 to 1 rising"),
        (
            {"primaries.gain.y": {"keys": [{"frame": 5, "value": 1}, {"frame": 5, "value": 2}]}},
            "the frames of keys rise",
        ),
        ({"primaries.temperature": {"keys": []}}, "one key at least"),
        ({"curves.ganged": 1}, "is true or false"),
    ]
    for settings, said in calls:
        refused = await desk.call("video_grade_set", grade="c2", node=1, settings=settings)
        assert refused.error is not None
        assert refused.error.code == "VIDEO_GRADE"
        assert said in refused.text, refused.text
    assert fake.transactions == []


async def test_a_node_is_named_by_its_number_and_keyframes_are_checked_and_filled(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    keys = {"keys": [{"frame": 0, "value": 1}, {"frame": 24, "value": 1.5, "interp": "hold"}]}
    told = result(
        await desk.call(
            "video_grade_set",
            grade="c2",
            node=2,
            settings={"primaries.gain.y": keys, "primaries": {"contrast": 1.2}},
            enabled=False,
        )
    )
    (sent,) = fake.transactions[0]
    assert sent["id"] == "n5"
    assert sent["enabled"] is False
    assert sent["params"] == {
        "primaries": {
            "gain": {
                "y": {
                    "keys": [
                        {"frame": 0, "value": 1, "interp": "linear"},
                        {"frame": 24, "value": 1.5, "interp": "hold"},
                    ]
                }
            },
            "contrast": 1.2,
        }
    }
    node = told["grades"][0]["nodes"][1]
    assert node["enabled"] is False
    assert node["settings"]["primaries.contrast"] == 1.2
    assert node["settings"]["primaries.gain.y"]["keys"][1]["interp"] == "hold"
    missing = await desk.call("video_grade_set", grade="c2", node=3, enabled=True)
    assert missing.error is not None
    assert "has 2 nodes" in missing.text


def _sent(fake: Fake) -> dict[str, Any]:
    """The one operation of the last transaction, without the grade that it names."""
    (op,) = fake.transactions[-1]
    assert op["grade"] == {"level": "clip", "timeline": "t1", "clip": "c2"}
    return {name: value for name, value in op.items() if name != "grade"}


async def test_a_key_is_put_replaced_and_taken_away_without_the_others_being_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    gain = {"grade": "c2", "node": 1, "path": "primaries.gain.y"}
    spliced = {"op": "splice_keys", "id": "n3", "path": "primaries.gain.y"}

    async def put(frame: int, value: float, **more: Any) -> dict[str, Any]:
        asked = desk.call(
            "video_grade_keys", **gain, action="put", frame=frame, value=value, **more
        )
        return result(await asked)

    # A number takes its first key; later keys go where their frames are.
    await put(10, 1.2)
    assert _sent(fake) == {
        **spliced,
        "at": 0,
        "remove": 0,
        "keys": [{"frame": 10, "value": 1.2, "interp": "linear"}],
    }
    await put(0, 1.0, interp="hold")
    assert _sent(fake) == {
        **spliced,
        "at": 0,
        "remove": 0,
        "keys": [{"frame": 0, "value": 1.0, "interp": "hold"}],
    }
    told = await put(24, 1.5)
    assert (_sent(fake)["at"], _sent(fake)["remove"]) == (2, 0)
    keys = told["grades"][0]["nodes"][0]["settings"]["primaries.gain.y"]["keys"]
    assert [key["frame"] for key in keys] == [0, 10, 24]
    # A key put where there is one takes its place, and keeps its interpolation unless one is given.
    await put(0, 0.9)
    assert _sent(fake) == {
        **spliced,
        "at": 0,
        "remove": 1,
        "keys": [{"frame": 0, "value": 0.9, "interp": "hold"}],
    }
    bezier = {"bezier": {"out": [0.3, 0.0], "in": [0.7, 0.0]}}
    await put(10, 1.3, interp=bezier)
    assert _sent(fake) == {
        **spliced,
        "at": 1,
        "remove": 1,
        "keys": [{"frame": 10, "value": 1.3, "interp": bezier}],
    }
    # A key is taken away by its frame; the last leaves the number that it had.
    none = await desk.call("video_grade_keys", **gain, action="remove", frame=11)
    assert none.error is not None
    assert "its keys are at 0, 10, 24" in none.text
    result(await desk.call("video_grade_keys", **gain, action="remove", frame=10))
    assert _sent(fake) == {**spliced, "at": 1, "remove": 1, "keys": []}
    result(await desk.call("video_grade_keys", **gain, action="remove", frame=24))
    result(await desk.call("video_grade_keys", **gain, action="remove", frame=0))
    assert _sent(fake) == {
        "op": "set_keys",
        "id": "n3",
        "path": "primaries.gain.y",
        "param": 0.9,
    }
    assert clip_grade(fake, "c2")["nodes"][0]["params"]["primaries"]["gain"]["y"] == 0.9
    # Every key is cleared at once, to a value given or to that of the first.
    await put(5, 1.1)
    await put(9, 1.4)
    result(await desk.call("video_grade_keys", **gain, action="clear"))
    assert _sent(fake)["param"] == 1.1
    await put(5, 1.1)
    result(await desk.call("video_grade_keys", **gain, action="clear", value=1))
    assert _sent(fake) == {"op": "set_keys", "id": "n3", "path": "primaries.gain.y", "param": 1}


async def test_keys_are_of_a_number_setting_and_what_an_action_needs_is_asked_for(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    for asked, why in [
        ({"path": "primaries.gain", "action": "put", "frame": 0, "value": 1}, "name one that is"),
        ({"path": "curves.y.points", "action": "put", "frame": 0, "value": 1}, "takes no keys"),
        ({"path": "lut", "action": "put", "frame": 0, "value": 1}, "takes no keys"),
        (
            {"path": "primaries.gian.y", "action": "put", "frame": 0, "value": 1},
            "did you mean gain",
        ),
        ({"path": "primaries..y", "action": "put", "frame": 0, "value": 1}, "not a path"),
        ({"path": "primaries.gain.y.z", "action": "put", "frame": 0, "value": 1}, "holds no"),
        (
            {"path": "qualifier.hue.center", "action": "put", "frame": 0, "value": 1},
            "has no qualifier",
        ),
        ({"path": "primaries.gain.y", "action": "put", "frame": 0, "value": 99}, "out of range"),
        ({"path": "primaries.gain.y", "action": "put", "frame": 0}, "put needs value"),
        ({"path": "primaries.gain.y", "action": "put", "value": 1}, "put needs frame"),
        ({"path": "primaries.gain.y", "action": "remove", "frame": 0}, "it has no keys"),
        ({"path": "primaries.gain.y", "action": "clear"}, "no keys to clear"),
        (
            {"path": "primaries.gain.y", "action": "put", "frame": 0, "value": 1, "interp": "ease"},
            "interp is",
        ),
    ]:
        refused = await desk.call("video_grade_keys", grade="c2", node=1, **asked)
        assert refused.error is not None, asked
        assert why in refused.text, (asked, refused.text)
    assert fake.transactions == []


async def test_a_shot_match_given_in_part_where_there_is_none_is_made_whole(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(
        await desk.call(
            "video_grade_set", grade="c2", node="n3", settings={"shot_match.gain.r": 1.1}
        )
    )
    (sent,) = fake.transactions[0]
    assert sent["params"]["shot_match"] == {
        "gamma": {"r": 1.0, "g": 1.0, "b": 1.0},
        "gain": {"r": 1.1, "g": 1.0, "b": 1.0},
        "offset": {"r": 0.0, "g": 0.0, "b": 0.0},
        "contrast": 1.0,
        "pivot": 0.435,
    }
    assert told["grades"][0]["nodes"][0]["shot_match"] is True


async def test_the_blur_palette_is_set_in_its_units_and_told_and_out_of_range_refused(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    assert grading.new_corrector()["blur"] == {
        "mode": "blur",
        "radius": {"r": 0.5, "g": 0.5, "b": 0.5},
        "ratio": {"r": 0.5, "g": 0.5, "b": 0.5},
        "scaling": {"r": 0.25, "g": 0.25, "b": 0.25},
        "coring_softness": 0.0,
        "level": 0.0,
        "mix": 100.0,
        "linked": {"radius": True, "ratio": True, "scaling": True},
    }
    told = result(
        await desk.call(
            "video_grade_set",
            grade="c2",
            node=1,
            settings={"blur.mode": "mist", "blur.radius.r": 0.3, "blur.mix": 40},
        )
    )
    (sent,) = fake.transactions[0]
    assert sent["params"] == {"blur": {"mode": "mist", "radius": {"r": 0.3}, "mix": 40}}
    settings = told["grades"][0]["nodes"][0]["settings"]
    assert settings["blur.radius.r"] == 0.3
    assert settings["blur.mix"] == 40
    for refused_settings, said in [
        ({"blur.radius.g": 1.5}, "Blur Radius"),
        ({"blur.mode": "defocus"}, "blur, sharpen, mist"),
        ({"blur.mix": -1}, "Blur Mix (Mist): 0 to 100"),
        ({"blur.linked.radius": 1}, "is true or false"),
    ]:
        refused = await desk.call("video_grade_set", grade="c2", node=1, settings=refused_settings)
        assert refused.error is not None, refused_settings
        assert said in refused.text, refused.text
    assert len(fake.transactions) == 1


async def test_a_qualifier_given_in_part_where_there_is_none_is_made_whole_and_told(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(
        await desk.call(
            "video_grade_set",
            grade="c2",
            node="n3",
            settings={
                "qualifier.hue.center": 8,
                "qualifier.hue.width": 12,
                "qualifier.finesse.blur_radius": 30,
                "key.qualifier.gain": 0.8,
            },
        )
    )
    (sent,) = fake.transactions[0]
    qualifier = sent["params"]["qualifier"]
    assert qualifier["mode"] == "hsl"
    assert qualifier["hue"] == {"on": True, "center": 8, "width": 12, "soft": 0.0, "symmetry": 50.0}
    assert qualifier["lum"] == {
        "on": True, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0,
    }  # fmt: skip
    assert qualifier["finesse"]["white_clip"] == 100.0
    assert qualifier["finesse"]["morph"] == "shrink"
    assert sent["params"]["key"] == {"qualifier": {"gain": 0.8}}
    node = told["grades"][0]["nodes"][0]
    assert node["qualifier"] == "hsl"
    assert node["settings"]["qualifier.hue.center"] == 8
    assert node["settings"]["qualifier.finesse.blur_radius"] == 30
    assert node["settings"]["key.qualifier.gain"] == 0.8
    for settings, said in [
        ({"qualifier.finesse.blur_radius": 2500}, "Blur Radius"),
        ({"qualifier.mode": "3d"}, "hsl, rgb, lum"),
        ({"qualifier.finesse.morph": "erode"}, "shrink, grow, opening, closing"),
        ({"key.input_blur.ratio": 2}, "H/V Ratio"),
    ]:
        refused = await desk.call("video_grade_set", grade="c2", node="n3", settings=settings)
        assert refused.error is not None, settings
        assert said in refused.text, refused.text
    assert len(fake.transactions) == 1


async def test_an_outside_node_is_added_after_a_corrector(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    target = {"level": "clip", "timeline": "t1", "clip": "c2"}
    result(
        await desk.call(
            "video_grade_node",
            grade="c2",
            action="add_outside",
            node=1,
            settings={"primaries.saturation": 20},
        )
    )
    edit, set_node = fake.transactions[0]
    assert edit == {"edit": "add_outside", "grade": target, "node": "n3"}
    assert set_node["op"] == "set_node"
    assert set_node["id"] == "n10"
    assert set_node["params"] == {"primaries": {"saturation": 20}}


async def test_nodes_are_added_removed_reset_and_extracted_by_the_color_pages_commands(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    target = {"level": "clip", "timeline": "t1", "clip": "c2"}
    result(await desk.call("video_grade_node", grade="c2", action="add_serial", node=1))
    result(
        await desk.call("video_grade_node", grade="c2", action="add_serial_before", node="output")
    )
    result(await desk.call("video_grade_node", grade="c2", action="reset", node="n3"))
    result(await desk.call("video_grade_node", grade="c2", action="extract", node=2))
    told = result(await desk.call("video_grade_node", grade="c2", action="remove", node="n3"))
    assert [one[0] for one in fake.transactions] == [
        {"edit": "add_serial", "grade": target, "node": "n3"},
        {"edit": "add_serial_before", "grade": target, "node": "output"},
        {"edit": "reset_node", "grade": target, "node": "n3"},
        {"edit": "extract_node", "grade": target, "node": "n5"},
        {"op": "remove_node", "grade": target, "id": "n3"},
    ]
    assert len(fake.steps) == 5
    assert told["grades"][0]["complete"] is False
    assert "not linked" in told["grades"][0]["why"]
    refused = await desk.call(
        "video_grade_node", grade="c2", action="remove", node=1, settings={"primaries.hue": 40}
    )
    assert refused.error is not None
    assert "for a node that is added" in refused.text
    needs = await desk.call("video_grade_node", grade="c2", action="add_parallel")
    assert needs.error is not None
    assert "needs the node" in needs.text


async def test_grades_are_copied_appended_and_pasted_between_clips(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(
        await desk.call("video_grade_copy", **{"from": "c2"}, to=["c1", "c4"], preserve_nodes=1)
    )
    c2 = {"level": "clip", "timeline": "t1", "clip": "c2"}
    c1 = {"level": "clip", "timeline": "t1", "clip": "c1"}
    c4 = {"level": "clip", "timeline": "t1", "clip": "c4"}
    assert fake.transactions[0] == [
        {"edit": "copy_grade", "from": c2, "to": [c1, c4], "preserve_nodes": 1}
    ]
    assert len(fake.steps) == 1
    assert [len(one["nodes"]) for one in told["grades"]] == [2, 2]
    result(await desk.call("video_grade_copy", action="append", **{"from": "c2"}, to=["c4"]))
    assert fake.transactions[1] == [{"edit": "append_node_graph", "from": c2, "to": [c4]}]
    result(
        await desk.call(
            "video_grade_copy",
            action="paste_value",
            **{"from": "c2"},
            from_node=1,
            to=[{"clip": "c1"}],
            to_node=2,
            param="primaries.saturation",
        )
    )
    assert fake.transactions[2] == [
        {
            "edit": "paste_value",
            "from": c2,
            "from_node": "n3",
            "to": c1,
            "to_node": "n5",
            "param": "primaries.saturation",
        }
    ]
    assert clip_grade(fake, "c1")["nodes"][1]["params"]["primaries"]["saturation"] == 30.0
    wrong = await desk.call(
        "video_grade_copy",
        action="paste_value",
        **{"from": "c2"},
        from_node=1,
        to=["c1"],
        to_node=1,
        param="primaries.sat",
    )
    assert wrong.error is not None
    assert "not a setting of a corrector" in wrong.text
    sound = await desk.call("video_grade_copy", **{"from": "c2"}, to=["c3"])
    assert sound.error is not None
    assert "clip of sound" in sound.text
    assert len(fake.transactions) == 3


async def test_keys_of_a_copied_grade_go_where_keyframes_says(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    c2 = {"level": "clip", "timeline": "t1", "clip": "c2"}
    c1 = {"level": "clip", "timeline": "t1", "clip": "c1"}
    result(
        await desk.call("video_grade_copy", **{"from": "c2"}, to=["c1"], keyframes="start_frame")
    )
    assert fake.transactions[0] == [
        {"edit": "copy_grade", "from": c2, "to": [c1], "keyframes": "start_frame"}
    ]
    result(
        await desk.call(
            "video_grade_copy",
            action="append",
            **{"from": "c2"},
            to=["c1"],
            keyframes="source_timecode",
        )
    )
    assert fake.transactions[1] == [
        {"edit": "append_node_graph", "from": c2, "to": [c1], "keyframes": "source_timecode"}
    ]
    pasted = await desk.call(
        "video_grade_copy",
        action="paste_node",
        **{"from": "c2"},
        from_node=1,
        to=["c1"],
        to_node=1,
        keyframes="start_frame",
    )
    assert pasted.error is not None
    assert "keyframes is for copy and append" in pasted.text
    wrong = await desk.call("video_grade_copy", **{"from": "c2"}, to=["c1"], keyframes="frame")
    assert wrong.error is not None
    assert len(fake.transactions) == 2


async def test_a_lut_is_set_by_its_name_in_the_library_and_a_missing_one_is_told(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    listed = result(await desk.call("video_grade_lut", action="list"))
    assert listed["luts"] == [{"name": "LUTs/film.cube", "kind": "3d"}]
    assert [one["place"] for one in listed["roots"]] == ["project", "library"]
    told = result(await desk.call("video_grade_lut", grade="c1", lut="LUTs/film.cube"))
    assert fake.transactions[0][0] == {
        "edit": "append_node",
        "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
    }
    assert fake.transactions[0][1]["params"] == {"lut": "LUTs/film.cube"}
    assert told["grades"][0]["nodes"][0]["lut"] == "LUTs/film.cube"
    unknown = await desk.call("video_grade_lut", grade="c1", node=1, lut="LUTs/flim.cube")
    assert unknown.error is not None
    assert "the closest are LUTs/film.cube" in unknown.text
    assert len(fake.transactions) == 1
    result(await desk.call("video_grade_lut", action="clear", grade="c2", node=2))
    assert fake.transactions[1][0]["params"] == {"lut": None}
    result(await desk.call("video_grade_set", grade="c2", node=1, settings={"lut": "gone.cube"}))
    info = result(await desk.call("video_grade_info", grade="c2"))
    message = "there is no LUT gone.cube in the folder of the project or in the library"
    assert info["nodes"][0]["lut_missing"] == message
    assert info["missing_luts"] == [{"name": "gone.cube", "message": message}]
    listing = result(await desk.call("video_info"))
    clips = listing["timelines"][0]["tracks"][0]["clips"]
    assert clips[1]["grade"]["nodes"][0]["lut_missing"] == message


async def test_a_lut_that_does_not_parse_is_listed_with_why_and_not_set(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    root = {"place": "project", "path": str(desk.work), "exists": True}
    lut = {"root": 0, "size": 9, "modified_us": None, "shadowed": False}
    fake.results["luts.list"] = {
        "roots": [root],
        "luts": [
            {**lut, "name": "Log.cube", "kind": "1d", "status": "ok"},
            {**lut, "name": "Broken.cube", "kind": "3d", "status": "error", "message": "line 3"},
            {**lut, "name": "Old.cube"},
        ],
        "truncated": False,
    }
    listed = result(await desk.call("video_grade_lut", action="list"))
    assert listed["luts"] == [
        {"name": "Log.cube", "kind": "1d"},
        {"name": "Broken.cube", "kind": "3d", "error": "line 3"},
        {"name": "Old.cube"},
    ]
    refused = await desk.call("video_grade_lut", grade="c1", lut="broken.cube")
    assert refused.error is not None
    assert "the LUT Broken.cube cannot be used: line 3" in refused.text
    assert fake.transactions == []
    result(await desk.call("video_grade_lut", grade="c1", lut="Log.cube"))
    result(await desk.call("video_grade_lut", grade="c1", lut="Old.cube"))
    assert [ops[1]["params"] for ops in fake.transactions] == [
        {"lut": "Log.cube"},
        {"lut": "Old.cube"},
    ]


async def test_a_group_is_made_with_its_clips_and_a_pre_grade_in_one_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = result(
        await grouped(
            desk,
            action="create",
            name="Interview",
            clips=["c1", "c4"],
            level="pre",
            **{"from": "c2"},
        )
    )
    sent = fake.transactions[0]
    assert [one.get("op", one.get("edit")) for one in sent] == [
        "add_group",
        "set_clip_group",
        "set_clip_group",
        "copy_grade",
    ]
    assert sent[3]["to"] == [{"level": "pre_clip", "group": "$group"}]
    assert len(fake.steps) == 1
    group = told["group"]
    assert group["id"] == told["names"]["$group"] == "k10"
    assert group["clips"] == [{"timeline": "t1", "clip": "c1"}, {"timeline": "t1", "clip": "c4"}]
    assert [one["id"] for one in group["pre"]["nodes"]] == ["n3", "n5"]
    assert group["post"] is None
    graded = result(
        await desk.call(
            "video_grade_node",
            grade={"level": "post_clip", "group": "k10"},
            action="append",
            settings={"primaries.contrast": 1.1},
        )
    )
    assert graded["grades"][0]["nodes"][0]["settings"] == {"primaries.contrast": 1.1}
    overview = result(await desk.call("video_grade_info"))
    assert [one["clip"] for one in overview["clips"]] == ["c1", "c2", "c4"]
    assert overview["clips"][0] == {
        "clip": "c1",
        "track": "V1",
        "start": 0,
        "group": "k10",
        "grade": None,
    }
    assert overview["groups"][0]["post"]["nodes"][0]["settings"] == {"primaries.contrast": 1.1}
    clip = result(await desk.call("video_grade_info", grade="c1"))
    assert clip["group"] == "k10"
    result(await grouped(desk, action="rename", group="k10", name="Talk"))
    result(await desk.call("video_grade_group", action="collapse", group="k10"))
    assert fake.transactions[-1] == [
        {"edit": "collapse_group_grades", "timeline": "t1", "clips": ["c1", "c4"]}
    ]
    result(await desk.call("video_grade_group", action="add", group="k10", clips=["c1"]))
    deleted = result(await desk.call("video_grade_group", action="delete", group="k10"))
    assert fake.transactions[-1] == [
        {"op": "set_clip_group", "timeline": "t1", "clip": "c1", "group": None},
        {"op": "remove_group", "id": "k10"},
    ]
    assert "group" not in deleted
    assert fake.project["groups"] == []


async def test_a_refusal_of_the_engine_is_told_with_what_to_do(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    fake.refusals["transaction.apply"] = {
        "code": -32001,
        "message": "primaries.saturation is out of range",
        "data": {"index": 0, "reason": "out_of_range", "what": "primaries.saturation"},
    }
    refused = await desk.call(
        "video_grade_set", grade="c2", node=1, settings={"primaries.saturation": 20}
    )
    assert refused.error is not None
    assert refused.error.code == "VIDEO_OUT_OF_RANGE"
    assert "Saturation (Sat): 0 to 100, 50 changes nothing" in refused.text


async def test_a_change_meanwhile_is_worked_out_again_on_the_project_as_it_is(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    result(await desk.call("video_grade_info"))  # Connected, and the project read once.
    fake.then["project.get"] = lambda: fake.change_by("desktop")
    told = result(
        await desk.call(
            "video_grade_node",
            timeline="t1",
            grade="c1",
            action="append",
            settings={"primaries.hue": 60},
        )
    )
    # The first try named n10, which the desktop's timeline took; the second names n11.
    assert fake.asked.count("transaction.apply") == 2
    assert fake.transactions[0][1]["id"] == told["added"] == "n11"
    assert told["grades"][0]["nodes"][0]["settings"] == {"primaries.hue": 60}


async def test_a_grade_is_of_a_picture_and_a_target_names_what_it_needs(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    for grade, said in [
        ("c3", "clip of sound"),
        ("c9", "there is no clip c9"),
        ({"level": "pre_clip"}, "needs its group"),
        ({"level": "pre_clip", "group": "k1"}, "there is no group k1"),
        ({"level": "timeline", "clip": "c1"}, "names only its timeline"),
    ]:
        refused = await desk.call("video_grade_info", grade=grade)
        assert refused.error is not None
        assert said in refused.text, refused.text
    timeline = result(
        await desk.call("video_grade_node", grade={"level": "timeline"}, action="append")
    )
    assert timeline["grades"][0]["grade"] == {"level": "timeline", "timeline": "t1"}
    assert len(fake.project["timelines"][0]["grade"]["nodes"]) == 1


async def test_video_info_tells_grades_in_short(studio: tuple[Desk, Fake]) -> None:
    desk, _ = studio
    listing = result(await desk.call("video_info"))
    timeline = listing["timelines"][0]
    assert "grade" not in timeline
    clips = timeline["tracks"][0]["clips"]
    assert "grade" not in clips[0]
    assert "group" not in clips[0]
    assert clips[1]["grade"] == {
        "nodes": [
            {
                "number": 1,
                "id": "n3",
                "kind": "corrector",
                "label": "",
                "enabled": True,
                "settings": {"primaries.saturation": 30.0},
            },
            {
                "number": 2,
                "id": "n5",
                "kind": "corrector",
                "label": "",
                "enabled": True,
                "settings": {},
                "lut": "LUTs/film.cube",
            },
        ]
    }
    assert "groups" not in listing


async def test_the_engine_grades_as_the_tools_ask(
    ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from columbina.video import edits
    from support.media import real_engine

    library = tmp_path / "library"
    library.mkdir()
    monkeypatch.setenv("COLUMBINA_LUTS", str(library))
    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    rate = edits.Rate(num=25, den=1)
    batch = [
        edits.add_timeline(
            "Film", rate, id="$film", width=160, height=120, tracks=[edits.track("video")]
        ),
        edits.insert("$film", edits.of_color("#808080ff", 50), 0, audio_track=None),
        edits.insert("$film", edits.of_color("#406080ff", 50), 50, audio_track=None),
    ]
    result(await desk.call("video_edit", edits=batch))
    clips = result(await desk.call("video_info"))["timelines"][0]["tracks"][0]["clips"]
    first, second = (one["id"] for one in clips)

    # A corrector with saturation 0: every other setting of a new one is as the tools think.
    added = result(
        await desk.call(
            "video_grade_node", grade=first, action="append", settings={"primaries.saturation": 0}
        )
    )
    told = result(await desk.call("video_grade_info", grade=first))
    (node,) = told["nodes"]
    assert node["id"] == added["added"]
    assert node["settings"] == {"primaries.saturation": 0.0}
    assert told["output"] == node["id"]
    assert node["input"] == "source"
    assert told["complete"] is True

    # One undo step takes the node and its settings away together.
    result(await desk.call("video_undo"))
    assert result(await desk.call("video_grade_info", grade=first))["nodes"] == []
    result(await desk.call("video_undo", redo=True))

    copied = result(await desk.call("video_grade_copy", **{"from": first}, to=[second]))
    assert copied["grades"][0]["nodes"][0]["settings"] == {"primaries.saturation": 0.0}
    again = result(await desk.call("video_grade_info", grade=second))
    assert again["nodes"][0]["settings"] == {"primaries.saturation": 0.0}

    made = result(await grouped(desk, action="create", name="Look", clips=[first, second]))
    group = made["group"]["id"]
    pre = {"level": "pre_clip", "group": group}
    result(
        await desk.call(
            "video_grade_node", grade=pre, action="append", settings={"primaries.contrast": 1.3}
        )
    )
    look = result(await desk.call("video_grade_info", grade=pre))
    assert look["nodes"][0]["settings"] == {"primaries.contrast": 1.3}
    assert result(await desk.call("video_grade_info", grade=first))["group"] == group
    overview = result(await desk.call("video_grade_info"))
    assert overview["groups"][0]["clips"] == [
        {"timeline": overview["timeline"], "clip": first},
        {"timeline": overview["timeline"], "clip": second},
    ]

    # A LUT that is not there is told so; one in the folder of the project is not.
    result(await desk.call("video_grade_set", grade=first, node=1, settings={"lut": "nope.cube"}))
    missing = result(await desk.call("video_grade_info", grade=first))
    assert missing["missing_luts"][0]["name"] == "nope.cube"
    assert "nope.cube" in missing["nodes"][0]["lut_missing"]
    cube = "LUT_3D_SIZE 2\n" + "".join(
        f"{r} {g} {b}\n" for b in (0, 1) for g in (0, 1) for r in (0, 1)
    )
    (desk.work / ".columbina" / "studio" / "film.cube").write_text(cube, encoding="utf-8")
    listed = result(await desk.call("video_grade_lut", action="list"))
    assert {"name": "film.cube", "kind": "3d"} in listed["luts"]
    result(await desk.call("video_grade_lut", grade=first, node=1, lut="film.cube"))
    found = result(await desk.call("video_grade_info", grade=first))
    assert found["nodes"][0]["lut"] == "film.cube"
    assert "missing_luts" not in found
    refused = await desk.call(
        "video_grade_set", grade=first, node=1, settings={"key.output.gain": 3}
    )
    assert refused.error is not None
    assert "Key Output gain (how strongly the node works): 0 to 2" in refused.text


async def test_video_edit_sends_edits_of_grades_as_the_engine_reads_them(
    ctx: Context, tmp_path: Path
) -> None:
    from columbina.video import edits
    from support.media import real_engine

    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    rate = edits.Rate(num=25, den=1)
    batch = [
        edits.add_timeline(
            "Film", rate, id="$film", width=160, height=120, tracks=[edits.track("video")]
        ),
        edits.insert("$film", edits.of_color("#808080ff", 50), 0, audio_track=None),
        edits.insert("$film", edits.of_color("#406080ff", 50), 50, audio_track=None),
    ]
    result(await desk.call("video_edit", edits=batch))
    timeline = result(await desk.call("video_info"))["timelines"][0]
    first, second = (one["id"] for one in timeline["tracks"][0]["clips"])

    # An edit of a grade names its grade by a target and takes no timeline of its own: none is
    # added to it.
    target = {"level": "clip", "timeline": timeline["id"], "clip": first}
    result(await desk.call("video_edit", edits=[{"edit": "append_node", "grade": target}]))
    assert len(result(await desk.call("video_grade_info", grade=first))["nodes"]) == 1

    # A target of a clip that leaves its timeline out is given the tool's, as an edit is.
    result(
        await desk.call(
            "video_edit",
            edits=[
                {"edit": "append_node", "grade": {"level": "clip", "clip": first}},
                {
                    "edit": "copy_grade",
                    "from": {"level": "clip", "clip": first},
                    "to": [{"level": "clip", "clip": second}],
                },
            ],
        )
    )
    assert len(result(await desk.call("video_grade_info", grade=first))["nodes"]) == 2
    assert len(result(await desk.call("video_grade_info", grade=second))["nodes"]) == 2


async def test_a_grade_is_exported_as_a_cube_that_the_engine_bakes(
    ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from columbina.video import edits
    from support.media import real_engine

    monkeypatch.setenv("COLUMBINA_LUTS", str(tmp_path / "library"))
    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    rate = edits.Rate(num=25, den=1)
    batch = [
        edits.add_timeline(
            "Film", rate, id="$film", width=160, height=120, tracks=[edits.track("video")]
        ),
        edits.insert("$film", edits.of_color("#808080ff", 50), 0, audio_track=None),
    ]
    result(await desk.call("video_edit", edits=batch))
    (clip,) = result(await desk.call("video_info"))["timelines"][0]["tracks"][0]["clips"]
    first = clip["id"]
    result(
        await desk.call(
            "video_grade_node", grade=first, action="append", settings={"primaries.saturation": 0}
        )
    )
    folder = desk.work / ".columbina" / "studio" / "looks"
    folder.mkdir()

    # Node 1, as DaVinci numbers it, baked into 17 points on an edge in the project's folder.
    told = result(
        await desk.call(
            "video_grade_lut", action="export", grade=first, node=1, size=17, path="looks/grey.cube"
        )
    )
    assert told["name"] == "looks/grey.cube"
    assert told["size"] == 17
    assert told["ignored"] == []
    lines = (folder / "grey.cube").read_text(encoding="utf-8").splitlines()
    assert "LUT_3D_SIZE 17" in lines
    rows = [line for line in lines if line[:1].isdigit() or line.startswith("-")]
    assert len(rows) == 17**3
    # The point of pure red, made grey by saturation 0: its three values are one.
    red = [float(value) for value in rows[16].split()]
    assert red[0] > 0.0
    assert red[0] == red[1] == red[2]

    refused = await desk.call("video_grade_lut", action="export", grade=first)
    assert refused.error is not None
    assert "export needs path" in refused.text
    outside = await desk.call("video_grade_lut", action="export", grade=first, path="../x.cube")
    assert outside.error is not None
    assert not (desk.work / ".columbina" / "x.cube").exists()
