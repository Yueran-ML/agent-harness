"""Text+ of the Fusion page as the agent adds it, styles it and animates it by a preset."""

import copy
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any, cast

import pytest

from columbina.fs import Access
from columbina.kernel import Context
from columbina.video import Config, Videos, edits, fusion, tools_of
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
    """As ``test_fusion_tools.py`` has it: on t1 of 25 frames a second, c1 on frames 100 to 149
    (its frame ``f`` shows frame ``f - 81`` of its media), c2 on frames 10 to 39 playing its
    media twice as fast, backward (frame 83 first), c3 a title on frames 40 to 59; c9 of sound."""
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


def params(fake: Fake, node: str, id: str = "c1") -> dict[str, Any]:
    nodes = clip(fake, id)["fusion"]["nodes"]
    return next(one for one in nodes if one["id"] == node)["params"]


def linked(fake: Fake, id: str = "c1") -> set[str]:
    links = clip(fake, id)["fusion"]["links"]
    return {f"{link['from']} -> {link['to']}.{link['input']}" for link in links}


def told_node(told: dict[str, Any], id: str) -> dict[str, Any]:
    return next(one for one in told["nodes"] if one["id"] == id)


def keyed(*pairs: tuple[int, float]) -> dict[str, Any]:
    """A setting with keys at frames of the clip's keys, each linear."""
    keys = [{"frame": frame, "value": value, "interp": "linear"} for frame, value in pairs]
    return {"keys": keys}


async def node(desk: Desk, action: str, clip: str = "c1", **asked: Any) -> dict[str, Any]:
    return result(await desk.call("video_fusion_node", clip=clip, action=action, **asked))


async def animate(desk: Desk, **asked: Any) -> dict[str, Any]:
    return result(await desk.call("video_text_animate", **{"clip": "c1", **asked}))


async def refused(desk: Desk, fake: Fake, tool: str, **asked: Any) -> str:
    """What a call that is refused before anything is sent says."""
    sent = len(fake.transactions)
    outcome = await desk.call(tool, **{"clip": "c1", **asked})
    assert outcome.error is not None, outcome.text
    assert outcome.error.code == "VIDEO_FUSION", outcome.text
    assert len(fake.transactions) == sent
    return outcome.text


# ---------------------------------------------------------------------------------------------
# The node
# ---------------------------------------------------------------------------------------------


async def test_a_text_plus_is_added_with_the_parts_that_it_is_given_filled_in(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    settings = {
        "text": "Hello",
        "size": 0.1,
        "outline.thickness": 0.05,
        "fill": {"color": {"g": 0.8, "b": 0}},
    }
    told = await node(desk, "add", kind="text_plus", settings=settings)
    text = told["added"]

    # Every setting is written, and the outline that a setting of it names is added whole.
    (_, added) = fake.transactions[-1]
    assert "links" not in added
    assert added["node"]["params"] == {
        **fusion.new_params("text_plus"),
        "text": "Hello",
        "size": 0.1,
        "fill": {"color": {"r": 1, "g": 0.8, "b": 0, "a": 1}},
        "outline": {**fusion.new_part("text_plus", "outline"), "thickness": 0.05},
    }
    # It is told by what is not as a new one has it, and by the parts that it has not.
    shown = told_node(told, text)
    assert shown["free"] == ["mask"]
    assert shown["settings"] == {
        "text": "Hello",
        "size": 0.1,
        "fill.color.g": 0.8,
        "fill.color.b": 0,
        "outline.thickness": 0.05,
        "outline.color.r": 0,
        "outline.color.g": 0,
        "outline.color.b": 0,
        "outline.color.a": 1,
        "border": None,
        "follower": None,
    }
    assert "settings as a new node has them are left out" in shown["note"]
    thickness = told["settings_of"]["text_plus"]["outline.thickness"]
    assert thickness == "Outline Thickness: 0 to 0.5, 0.02 at first"
    reference = result(await desk.call("video_fusion_info", reference=True))
    kind = reference["kinds"]["text_plus"]
    assert kind["inputs"] == ["mask"]
    assert '{"outline": {}} adds one as a new one is' in kind["parts"]
    assert '{"outline": null} takes it away' in kind["parts"]
    assert "4096 characters at most" in kind["settings"]["text"]
    assert "256 bytes of UTF-8 at most" in kind["settings"]["font"]
    # A long text is told short.
    await node(desk, "set", node=text, settings={"text": "word " * 100})
    shown = told_node(result(await desk.call("video_fusion_info", clip="c1")), text)
    assert shown["settings"]["text"].endswith("... (500 characters)")


async def test_a_part_of_a_text_plus_is_added_set_and_taken_away(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    text = (await node(desk, "add", kind="text_plus", settings={"text": "Hi"}))["added"]

    async def sent(settings: dict[str, Any]) -> dict[str, Any]:
        await node(desk, "set", node=text, settings=settings)
        (op,) = fake.transactions[-1]
        assert (op["op"], op["id"]) == ("set_fusion_node", text)
        return op["params"]

    # {} adds a part as a new one is; then a setting of it is sent alone.
    assert await sent({"border": {}}) == {"border": fusion.new_part("text_plus", "border")}
    assert await sent({"border.round": 0.2}) == {"border": {"round": 0.2}}
    assert params(fake, text)["border"]["round"] == 0.2
    # A setting of a part that it has not adds the part, the rest as new.
    follower = {**fusion.new_part("text_plus", "follower"), "opacity": 0.5}
    assert await sent({"follower.opacity": 0.5}) == {"follower": follower}
    # true adds one too, and null or false takes one away, which the engine does.
    assert await sent({"outline": True}) == {"outline": fusion.new_part("text_plus", "outline")}
    assert await sent({"outline": None}) == {"outline": None}
    assert "outline" not in params(fake, text)
    assert await sent({"follower": False}) == {"follower": None}
    assert "follower" not in params(fake, text)

    async def wrong(**settings: Any) -> str:
        asked = {"action": "set", "node": text, "settings": settings}
        return await refused(desk, fake, "video_fusion_node", **asked)

    # A part that is there, or not there to take away, changes nothing.
    said = await wrong(border={})
    assert "nothing changes" in said
    said = await wrong(outline=None)
    assert "nothing changes" in said
    said = await wrong(outline="thick")
    assert "outline is given by its settings" in said
    said = await wrong(text="x" * 4097)
    assert "text is 4097 characters long, and 4096 at most" in said
    assert "out_of_range text_plus.text" in said
    said = await wrong(font="字" * 86)
    assert "font is 258 bytes of UTF-8 long, and 256 at most" in said
    assert "out_of_range text_plus.font" in said
    said = await wrong(font=3)
    assert "font is words, a string" in said
    said = await wrong(size=2)
    assert (
        "size is 2, out of range (Size of the letters: 0 to 1, 0.08 at first, in heights of the"
        " picture)"
    ) in said
    assert "out_of_range text_plus.size" in said
    said = await wrong(**{"follower.delay": -1})
    assert "follower.delay is -1, out of range" in said
    assert "out_of_range text_plus.follower.delay" in said
    said = await wrong(justify="middle")
    assert "justify (Horizontal Justification) is one of left, center, right" in said
    said = await wrong(colour=1)
    assert "a text_plus has no setting 'colour'" in said
    assert "outline, border, follower as a whole: {} adds one, null takes it away" in said
    after = {"action": "add", "kind": "text_plus", "after": "MediaIn1"}
    said = await refused(desk, fake, "video_fusion_node", **after)
    assert "a text_plus has no input of a picture to come after MediaIn1" in said


async def test_a_number_of_a_text_plus_takes_keys_where_its_part_is_there(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    text = (await node(desk, "add", kind="text_plus", settings={"text": "Hi"}))["added"]

    async def wrong(path: str, value: float = 0.1) -> str:
        asked = {"path": f"{text}.{path}", "action": "put", "frame": 110, "value": value}
        return await refused(desk, fake, "video_fusion_keys", **asked)

    said = await wrong("border.extend")
    assert f"node {text} has no border" in said
    assert '{"border": {}} adds one as a new one is, and then its keys are put' in said
    said = await wrong("text")
    assert "text (Styled Text" in said
    assert "takes no keys" in said
    said = await wrong("size", 2)
    assert "out_of_range text_plus.size" in said
    # Frame 110 of the timeline shows frame 29 of c1's media.
    put = {"path": f"{text}.size", "action": "put", "frame": 110, "value": 0.2}
    result(await desk.call("video_fusion_keys", clip="c1", **put))
    assert fake.transactions[-1] == [
        {
            "op": "splice_fusion_keys",
            **AT,
            "path": f"{text}.size",
            "at": 0,
            "remove": 0,
            "keys": [{"frame": 29, "value": 0.2, "interp": "linear"}],
        }
    ]
    assert params(fake, text)["size"] == keyed((29, 0.2))
    # Once the part is there, its numbers take keys.
    await node(desk, "set", node=text, settings={"border": {}})
    extend = {"path": f"{text}.border.extend", "action": "put", "frame": 120, "value": 0.3}
    result(await desk.call("video_fusion_keys", clip="c1", **extend))
    assert params(fake, text)["border"]["extend"] == keyed((39, 0.3))


async def test_a_refusal_of_a_text_plus_is_told_with_what_can_be_done_about_it(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    text = (await node(desk, "add", kind="text_plus", settings={"text": "Hi"}))["added"]

    async def answered(**data: Any) -> Any:
        fake.refusals["transaction.apply"] = {"code": -32001, "message": "refused", "data": data}
        outcome = await desk.call(
            "video_fusion_node", clip="c1", action="set", node=text, settings={"size": 0.2}
        )
        assert outcome.error is not None
        return outcome

    outcome = await answered(reason="out_of_range", what="text_plus.outline.thickness", index=0)
    assert outcome.error.code == "VIDEO_OUT_OF_RANGE"
    assert "outline.thickness of a text_plus is Outline Thickness: 0 to 0.5" in outcome.text
    outcome = await answered(reason="out_of_range", what="text_plus.text", index=0)
    assert "the text of a text_plus is 4096 characters at most" in outcome.text
    outcome = await answered(reason="out_of_range", what="text_plus.font", index=0)
    assert "the font of a text_plus is 256 bytes of UTF-8 at most" in outcome.text


# ---------------------------------------------------------------------------------------------
# Animations
# ---------------------------------------------------------------------------------------------


async def test_a_typewriter_lays_a_new_text_over_the_picture_in_one_step(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    told = await animate(desk, preset="typewriter", text="Hello")

    # The clip had no composition: the first one, a Merge in the place of what MediaOut1 read,
    # and the text into its foreground, in one transaction. A second of 25 frames from the
    # first frame of c1: frames 100 and 125 of the timeline show frames 19 and 44 of its media.
    words = {
        **fusion.new_params("text_plus"),
        "text": "Hello",
        "write_on": {"start": 0, "end": keyed((19, 0.0), (44, 1.0))},
    }
    over = [
        {"from": "MediaIn1", "to": "", "input": "background"},
        {"from": "", "to": "MediaOut1", "input": "input"},
    ]
    merge = edits.fusion_node("merge", fusion.new_params("merge"), id="$over")
    text = edits.fusion_node("text_plus", words, id="$text")
    into = [{"from": "", "to": "$over", "input": "foreground"}]
    assert fake.transactions == [
        [
            FIRST,
            {"op": "add_fusion_node", **AT, "node": merge, "links": over},
            {"op": "add_fusion_node", **AT, "node": text, "links": into},
        ]
    ]
    assert linked(fake) == {
        "MediaIn1 -> n20.background",
        "n21 -> n20.foreground",
        "n20 -> MediaOut1.input",
    }
    assert told == {
        "revision": 1,
        "operations": 3,
        "timeline": "t1",
        "clip": "c1",
        "preset": "typewriter",
        "node": "n21",
        "merge": "n20",
        "frames": {"from": 100, "to": 125},
        "keys": {
            "write_on.end": [
                {"timeline_frame": 100, "frame": 19, "value": 0.0, "interp": "linear"},
                {"timeline_frame": 125, "frame": 44, "value": 1.0, "interp": "linear"},
            ]
        },
        "notes": [told["notes"][-1]],
    }
    assert "video_fusion_keys" in told["notes"][-1]
    # One step, which video_undo takes back whole.
    assert len(fake.steps) == 1
    result(await desk.call("video_undo"))
    assert "fusion" not in clip(fake)


PRESETS: list[tuple[str, str, dict[str, Any], dict[str, list[int]], tuple[int, int]]] = [
    (
        "fade_in",
        "left_to_right",
        {"opacity": keyed((19, 0.0), (29, 1.0))},
        {"follower.opacity": [100, 110]},
        (100, 120),
    ),
    (
        "rise",
        "left_to_right",
        {
            "opacity": keyed((19, 0.0), (29, 1.0)),
            "offset": {"x": 0, "y": keyed((19, -0.05), (29, 0.0))},
        },
        {"follower.opacity": [100, 110], "follower.offset.y": [100, 110]},
        (100, 120),
    ),
    (
        "pop",
        "inside_out",
        {"scale": keyed((19, 0.0), (26, 1.15), (29, 1.0))},
        {"follower.scale": [100, 107, 110]},
        (100, 120),
    ),
    (
        "fade_out",
        "left_to_right",
        {"opacity": keyed((48, 1.0), (58, 0.0))},
        {"follower.opacity": [129, 139]},
        (129, 149),
    ),
]
"""Each preset that moves the characters by the follower, over 20 frames of c1: its order, what
it keys of the follower, at which frames of the timeline, and the frames that it lasts."""


@pytest.mark.parametrize(("preset", "order", "moved", "frames", "lasts"), PRESETS)
async def test_a_preset_moves_each_character_by_the_follower(
    studio: tuple[Desk, Fake],
    preset: str,
    order: str,
    moved: dict[str, Any],
    frames: dict[str, list[int]],
    lasts: tuple[int, int],
) -> None:
    desk, fake = studio
    told = await animate(desk, preset=preset, text="Hi", length=20)

    # Each character moves for half the length, and the last starts 10 frames after the first.
    follower = {"order": order, "delay": 10, "delay_type": "first_to_last"}
    words = fake.transactions[-1][-1]["node"]["params"]
    assert words == {
        **fusion.new_params("text_plus"),
        "text": "Hi",
        "follower": {**fusion.new_part("text_plus", "follower"), **follower, **moved},
    }
    assert told["follower"] == follower
    assert told["frames"] == {"from": lasts[0], "to": lasts[1]}
    shown = {path: [key["timeline_frame"] for key in keys] for path, keys in told["keys"].items()}
    assert shown == frames
    assert len(fake.steps) == 1


async def test_an_animation_of_a_text_that_is_there_keeps_its_other_keys(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    text = (await animate(desk, preset="fade_in", text="Hi", length=20))["node"]

    # A fade out at the end of the clip is put among the keys of the fade in.
    told = await animate(desk, preset="fade_out", node=text, length=20)
    both = keyed((19, 0.0), (29, 1.0), (48, 1.0), (58, 0.0))
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": text, "params": {"follower": {"opacity": both}}}
    ]
    assert (told["node"], told["operations"]) == (text, 1)
    assert "merge" not in told
    assert len(told["notes"]) == 1
    assert params(fake, text)["follower"]["opacity"] == both
    # The same again changes nothing.
    said = await refused(desk, fake, "video_text_animate", preset="fade_out", node=text, length=20)
    assert f"node {text} has this animation already" in said
    # Another order is told: the follower has one for all that it moves.
    told = await animate(desk, preset="pop", node=text, length=20)
    scale = keyed((19, 0.0), (26, 1.15), (29, 1.0))
    assert fake.transactions[-1][0]["params"] == {
        "follower": {"order": "inside_out", "scale": scale}
    }
    assert told["notes"][0] == (
        f"the follower of node {text} had order left_to_right, and now order inside_out: "
        "whatever else of it has keys goes in that order too"
    )
    # Its text and style change with it where they are given.
    asked = {"text": "Bye", "color": "#ff000080", "outline": True, "length": 20}
    await animate(desk, preset="typewriter", node=text, **asked)
    assert fake.transactions[-1][0]["params"] == {
        "text": "Bye",
        "fill": {"color": {"g": 0.0, "b": 0.0, "a": 0.502}},
        "outline": fusion.new_part("text_plus", "outline"),
        "write_on": {"end": keyed((19, 0.0), (39, 1.0))},
    }
    await animate(desk, preset="typewriter", node=text, outline=False, length=10)
    assert "outline" not in params(fake, text)
    assert params(fake, text)["write_on"]["end"] == keyed((19, 0.0), (29, 1.0), (39, 1.0))


async def test_a_text_without_a_follower_is_given_one_whole(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    text = (await node(desk, "add", kind="text_plus", settings={"text": "Hi"}))["added"]

    told = await animate(desk, preset="rise", node=text, length=20)

    follower = {
        **fusion.new_part("text_plus", "follower"),
        "order": "left_to_right",
        "delay": 10,
        "delay_type": "first_to_last",
        "opacity": keyed((19, 0.0), (29, 1.0)),
        "offset": {"x": 0, "y": keyed((19, -0.05), (29, 0.0))},
    }
    assert fake.transactions[-1] == [
        {"op": "set_fusion_node", **AT, "id": text, "params": {"follower": follower}}
    ]
    assert (told["node"], told["operations"]) == (text, 1)
    assert "merge" not in told
    assert params(fake, text)["follower"] == follower


async def test_the_keys_of_an_animation_are_at_the_frames_that_the_clip_shows(
    studio: tuple[Desk, Fake],
) -> None:
    desk, _ = studio
    # c2 plays its media twice as fast, backward: frames 10 and 20 show frames 83 and 63, and
    # the keys rise by their own frames.
    told = await animate(desk, clip="c2", preset="typewriter", text="Hi", length=10)
    assert told["keys"]["write_on.end"] == [
        {"timeline_frame": 20, "frame": 63, "value": 1.0, "interp": "linear"},
        {"timeline_frame": 10, "frame": 83, "value": 0.0, "interp": "linear"},
    ]
    assert told["frames"] == {"from": 10, "to": 20}
    # A title counts the frames of the clip; its 20 frames hold 19 of a second.
    told = await animate(desk, clip="c3", preset="typewriter", text="Hi")
    assert [key["frame"] for key in told["keys"]["write_on.end"]] == [0, 19]
    assert told["frames"] == {"from": 40, "to": 59}


async def test_what_cannot_be_animated_is_said_before_anything_is_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio

    async def wrong(**asked: Any) -> str:
        return await refused(desk, fake, "video_text_animate", **asked)

    said = await wrong(preset="typewriter")
    assert "a new text_plus needs text" in said
    said = await wrong(preset="typewriter", text="Hi", length=0)
    assert "length is how many frames the animation lasts, 1 at least" in said
    said = await wrong(preset="typewriter", text="Hi", at=-1)
    assert "at is a frame of the clip, counted from its first" in said
    said = await wrong(preset="fade_in", text="Hi", at=40, length=20)
    assert "an animation of 20 frames from frame 40 of clip c1 does not fit in it" in said
    assert "the clip has frames 0 to 49" in said
    said = await wrong(preset="pop", text="Hi", length=3)
    assert "length is 4 frames at least" in said
    said = await wrong(preset="typewriter", text="Hi", color="red")
    assert "color is #rrggbb or #rrggbbaa" in said
    said = await wrong(preset="typewriter", text="Hi", size=2)
    assert "size is 2, out of range" in said
    said = await wrong(preset="typewriter", text="Hi", font="字" * 86)
    assert "font is 258 bytes of UTF-8 long" in said
    said = await wrong(preset="typewriter", text="x" * 4097)
    assert "text is 4097 characters long" in said
    sound = await desk.call("video_text_animate", clip="c9", preset="typewriter", text="Hi")
    assert sound.error is not None
    assert "c9 is a clip of sound on A1" in sound.text
    blur = (await node(desk, "add", kind="blur", after="MediaIn1"))["added"]
    said = await wrong(preset="typewriter", node=blur)
    assert f"node {blur} is a blur: video_text_animate animates a text_plus" in said
    said = await wrong(preset="typewriter", node="n99")
    assert "the composition has no node n99" in said
    # A style would take away the keys of a setting that has them.
    text = (await animate(desk, preset="typewriter", text="Hi"))["node"]
    put = {"path": f"{text}.size", "action": "put", "frame": 110, "value": 0.1}
    result(await desk.call("video_fusion_keys", clip="c1", **put))
    said = await wrong(preset="fade_in", node=text, size=0.2)
    assert f"size of node {text} has keys, which a style would take away" in said


def test_the_tool_is_offered_and_costs_few_characters() -> None:
    tools = {tool.name: tool for tool in tools_of(Videos(Config(), None))}
    tool = tools["video_text_animate"]
    assert tool.effect == "write"
    told = tool.schema.description
    for word in (
        "Text+", "one undo step", "typewriter", "fade_in", "rise", "pop", "fade_out", "merge",
        "MediaOut", "at + length", "video_fusion_keys", "video_look",
    ):  # fmt: skip
        assert word in told, word
    properties = cast("dict[str, Any]", tool.schema.parameters["properties"])
    taken = {"clip", "preset", "node", "text", "at", "length", "font", "size", "color", "outline"}
    assert set(properties) == {"project", "timeline"} | taken
    assert properties["preset"]["enum"] == list(fusion.PRESETS)
    # What the tool costs on every request.
    assert len(told) < 800
    assert len(told) + len(json.dumps(tool.schema.parameters)) < 1900


async def test_the_sandbox_lets_the_tool_reach_the_studio_project(
    ctx: Context, tmp_path: Path
) -> None:
    desk = await desk_of(ctx, tmp_path)
    (desk.work / ".columbina" / "studio").mkdir(parents=True)
    desk.sandbox.set_mode(desk.session, "workspace-write")
    studio = desk.fs.locate(desk.session, desk.work, ".columbina/studio")
    settings = desk.fs.locate(desk.session, desk.work, ".columbina/permissions.yml")

    def access(where: Any) -> Access:
        tool = "video_text_animate"
        return Access(session=desk.session, cwd=desk.work, where=where, kind="write", tool=tool)

    assert desk.sandbox.restricts(access(studio)) is None
    outside = desk.sandbox.restricts(access(settings))
    assert outside is not None
    assert outside.code == "PROTECTED"
