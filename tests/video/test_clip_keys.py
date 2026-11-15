"""The agent's tool of the keys of a clip's picture: one key, one operation, one undo step."""

import copy
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from columbina.kernel import Context
from columbina.video import Config, Videos, clip_keys, tools_of
from columbina.video.clip_keys import KeyClock
from columbina.video.edits import Rate
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")

ENGINE_DATA = Path(__file__).resolve().parents[2] / "engine" / "crates" / "model" / "tests" / "data"
CASES: list[dict[str, Any]] = json.loads(
    (ENGINE_DATA / "key-clocks.json").read_text(encoding="utf-8")
)
"""The engine's own cases of the frames of keys: a clip, the rate of its timeline, its media, and
the frame of its keys (``keys``) at each of some frames of the timeline (``frames``)."""


def case(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(one for one in CASES if one["name"] == name))


def clock_of(one: dict[str, Any]) -> KeyClock:
    return KeyClock.of(one["clip"], Rate.model_validate(one["rate"]), one.get("media"))


def cut(fake: Fake, one: dict[str, Any], *more: dict[str, Any]) -> None:
    """Timeline t1 of the rate of a case: its clip and those of ``more`` on V1, sound on A1."""
    cases = [one, *more]
    for number, each in enumerate(cases, 1):
        each["clip"]["id"] = f"c{number}"
    pictures = sorted((each["clip"] for each in cases), key=lambda clip: clip["start"])
    rate = (one["rate"]["num"], one["rate"]["den"])
    timeline = timeline_of(
        track_of("V1", pictures, kind="video"), track_of("A1", [clip_of("c9", 0, 50)]), rate=rate
    )
    media = {each["media"]["id"]: each["media"] for each in cases if each.get("media")}
    fake.project.update(schema=5, next_id=20, media=list(media.values()), timelines=[timeline])


def film(fake: Fake) -> None:
    """Three clips of picture on t1, of 25 frames a second, and c9 of sound.

    c1 is on frames 100 to 149 and shows its media from its frame 19 on, at its own speed: its
    frame ``f`` of the timeline is the frame ``f - 81`` of its media. c2 is on frames 10 to 39
    and plays its media twice as fast, backward: it shows the frame 83 of its media first and
    the frame 25 last. c3 is a title on frames 40 to 59.
    """
    late, backward = case("media at the timeline's rate"), case("twice as fast, backward")
    cut(fake, late, backward, case("a title"))


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        film(fake)
        yield desk, fake


def setting(fake: Fake, clip: str, path: str) -> Any:
    of, name = path.split(".")
    clips = fake.project["timelines"][0]["tracks"][0]["clips"]
    return next(one for one in clips if one["id"] == clip)[of][name]


def sent(fake: Fake, clip: str = "c1") -> dict[str, Any]:
    """The one operation of the last transaction, without the clip that it names."""
    (op,) = fake.transactions[-1]
    assert (op["timeline"], op["id"]) == ("t1", clip)
    return {name: value for name, value in op.items() if name not in ("timeline", "id")}


def test_the_frame_of_a_key_is_counted_as_the_engines_cases_count_it() -> None:
    assert len(CASES) == 12
    for one in CASES:
        clock = clock_of(one)
        assert [clock.at(frame) for frame in one["frames"]] == one["keys"], one["name"]
        assert (clock.picture is not None) == (
            one["name"] not in ("a still", "media without a picture", "a title", "a colour")
        )
    # A clip of media that the project does not have counts its own frames, as a title does.
    lost = case("twice as fast, backward")
    assert KeyClock.of(lost["clip"], Rate(num=25, den=1), None).at(12) == 2


def test_a_key_is_shown_at_the_first_frame_of_the_timeline_that_is_at_its_frame() -> None:
    for one in CASES:
        clock = clock_of(one)
        clip = one["clip"]
        frames = range(clip["start"], clip["start"] + clip["length"])
        first: dict[int, int] = {}
        for frame in frames:
            first.setdefault(clock.at(frame), frame)
        for key in range(min(first) - 2, max(first) + 3):
            assert clock.shown(key) == first.get(key), (one["name"], key)
    # Twice as fast, the clip passes over every other picture; half as fast, it shows each twice.
    assert clock_of(case("twice as fast, backward")).shown(82) is None
    slow = clock_of(case("half as fast"))
    assert (slow.at(6), slow.at(7), slow.shown(7)) == (7, 7, 6)
    empty = case("a title")
    empty["clip"]["length"] = 0
    assert clock_of(empty).shown(0) is None


def test_the_keys_of_a_clip_with_speed_points_are_at_the_pictures_that_its_course_shows() -> None:
    # The clip and the numbers of the engine's test of this name (keyframes.rs): c1 on frames
    # [0, 100) of a timeline of 25 a second, from 2 seconds into m1, frame 50 of its 25 a
    # second: 40 frames forward, a picture held for 20, and 40 frames back at half the speed.
    one = case("media at the timeline's rate")
    one["clip"].update(start=0, length=100)
    one["clip"]["source"]["in"]["ticks"] = 2 * 15360
    points = [(0, 1, 1), (40, 0, 1), (60, -1, 2)]
    one["clip"]["retime"] = {
        "points": [{"frame": at, "speed": {"num": num, "den": den}} for at, num, den in points]
    }
    clock = clock_of(one)

    assert clock.counts == "the clip's media"
    # On the way up, a picture a frame; the frame before the clip goes on before it.
    assert (clock.at(0), clock.at(39), clock.at(-1)) == (50, 89, 49)
    # The freeze holds picture 90, with one setting on all its frames.
    assert {clock.at(frame) for frame in range(40, 60)} == {90}
    # On the way back each picture is shown twice, the last first.
    assert [clock.at(60 + back) for back in range(40)] == [
        50 + (79 - back) // 2 for back in range(40)
    ]
    # A key is shown at the first frame that is at it: on the way up, though the way back
    # comes by the picture again; the held picture at the first frame of the freeze.
    first: dict[int, int] = {}
    for frame in range(100):
        first.setdefault(clock.at(frame), frame)
    for key in range(47, 94):
        assert clock.shown(key) == first.get(key), key
    assert (clock.shown(50), clock.shown(70), clock.shown(89), clock.shown(90)) == (0, 20, 39, 40)
    assert (clock.shown(49), clock.shown(91)) == (None, None)


def test_the_settings_and_their_bounds_are_those_of_the_engines_catalog() -> None:
    catalog = json.loads((ENGINE_DATA / "catalog.json").read_text(encoding="utf-8"))["clip"]
    assert [one["path"] for one in catalog] == list(clip_keys.SETTINGS)
    for one in catalog:
        spec = clip_keys.SETTINGS[one["path"]]
        assert (spec.default, spec.low, spec.high) == (one["default"], one["low"], one["high"])
        assert one["clock"] == "source_frame"


async def test_a_key_is_put_replaced_and_taken_away_without_the_others_being_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    x = {"clip": "c1", "path": "transform.x"}
    spliced = {"op": "splice_clip_keys", "path": "transform.x"}

    async def put(frame: int, value: float, **more: Any) -> dict[str, Any]:
        asked = desk.call("video_clip_keys", **x, action="put", frame=frame, value=value, **more)
        return result(await asked)

    def frames() -> list[int]:
        return [key["frame"] for key in setting(fake, "c1", "transform.x")["keys"]]

    # A number takes its first key: frame 110 of the timeline is frame 29 of the clip's media.
    told = await put(110, 0.25)
    first = {"frame": 29, "value": 0.25, "interp": "linear"}
    assert sent(fake) == {**spliced, "at": 0, "remove": 0, "keys": [first]}
    assert setting(fake, "c1", "transform.x") == {"keys": [first]}
    assert told == {
        "revision": 1,
        "operations": 1,
        "timeline": "t1",
        "clip": "c1",
        "path": "transform.x",
        "key": {"timeline_frame": 110, "frame": 29},
        "key_frames": "frames of the clip's media, from 0",
        "setting": {"keys": [{"timeline_frame": 110, **first}]},
    }
    # Later keys go where their frames are: before, after and between.
    await put(100, 0.0, interp="hold")
    assert sent(fake) == {
        **spliced,
        "at": 0,
        "remove": 0,
        "keys": [{"frame": 19, "value": 0.0, "interp": "hold"}],
    }
    await put(140, 0.5)
    assert (sent(fake)["at"], sent(fake)["remove"]) == (2, 0)
    told = await put(120, 0.3)
    assert (sent(fake)["at"], sent(fake)["remove"]) == (2, 0)
    assert frames() == [19, 29, 39, 59]
    assert [key["timeline_frame"] for key in told["setting"]["keys"]] == [100, 110, 120, 140]
    # A key put where there is one takes its place, and keeps its interpolation unless one is given.
    await put(100, -0.1)
    assert sent(fake) == {
        **spliced,
        "at": 0,
        "remove": 1,
        "keys": [{"frame": 19, "value": -0.1, "interp": "hold"}],
    }
    bezier = {"bezier": {"out": [0.3, 0.0], "in": [0.7, 0.0]}}
    await put(110, 0.2, interp=bezier)
    assert sent(fake) == {
        **spliced,
        "at": 1,
        "remove": 1,
        "keys": [{"frame": 29, "value": 0.2, "interp": bezier}],
    }
    assert frames() == [19, 29, 39, 59]
    # A key is taken away by its frame of the timeline; the last leaves the number that it had.
    none = await desk.call("video_clip_keys", **x, action="remove", frame=111)
    assert none.error is not None
    assert none.error.code == "VIDEO_KEYS"
    assert (
        "transform.x of clip c1 has no key at frame 111 of the timeline (frame 30 of the clip's "
        "media): its keys are at frames 100, 110, 120, 140 of the timeline"
    ) in none.text
    told = result(await desk.call("video_clip_keys", **x, action="remove", frame=120))
    assert sent(fake) == {**spliced, "at": 2, "remove": 1, "keys": []}
    assert told["key"] == {"timeline_frame": 120, "frame": 39}
    assert frames() == [19, 29, 59]
    result(await desk.call("video_clip_keys", **x, action="remove", frame=140))
    result(await desk.call("video_clip_keys", **x, action="remove", frame=110))
    told = result(await desk.call("video_clip_keys", **x, action="remove", frame=100))
    assert sent(fake) == {"op": "set_clip_keys", "path": "transform.x", "param": -0.1}
    assert setting(fake, "c1", "transform.x") == -0.1
    assert told["setting"] == -0.1
    # Every key is cleared at once, to a value given or to that of the first.
    await put(105, 0.4)
    await put(109, 0.6)
    told = result(await desk.call("video_clip_keys", **x, action="clear"))
    assert sent(fake) == {"op": "set_clip_keys", "path": "transform.x", "param": 0.4}
    assert "key" not in told
    assert told["setting"] == 0.4
    await put(105, 0.4)
    result(await desk.call("video_clip_keys", **x, action="clear", value=0))
    assert sent(fake) == {"op": "set_clip_keys", "path": "transform.x", "param": 0}
    # Each call was one transaction of one operation, and one step to undo.
    assert all(len(ops) == 1 for ops in fake.transactions)
    assert len(fake.steps) == len(fake.transactions) == 15
    result(await desk.call("video_undo"))
    assert frames() == [24]


async def test_a_frame_of_the_timeline_is_the_frame_of_the_picture_shown_there(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    crop = {"clip": "c2", "path": "crop.left"}

    async def put(frame: int, value: float) -> dict[str, Any]:
        asked = desk.call("video_clip_keys", **crop, action="put", frame=frame, value=value)
        return result(await asked)

    # c2 begins late, at frame 10, and plays twice as fast, backward: its first frame shows the
    # frame 83 of its media and its last, 39, the frame 25, as the engine's case says.
    backward = case("twice as fast, backward")
    assert dict(zip(backward["frames"], backward["keys"], strict=True))[10] == 83
    told = await put(10, 0.5)
    assert told["key"] == {"timeline_frame": 10, "frame": 83}
    assert sent(fake, "c2")["keys"] == [{"frame": 83, "value": 0.5, "interp": "linear"}]
    # The keys rise by their own frames, so one later on the timeline goes before.
    assert (await put(39, 0.0))["key"] == {"timeline_frame": 39, "frame": 25}
    assert (sent(fake, "c2")["at"], sent(fake, "c2")["remove"]) == (0, 0)
    told = await put(20, 0.25)
    assert told["key"] == {"timeline_frame": 20, "frame": 63}
    assert (sent(fake, "c2")["at"], sent(fake, "c2")["remove"]) == (1, 0)
    assert told["key_frames"] == "frames of the clip's media, from 0"
    assert told["setting"] == {
        "keys": [
            {"timeline_frame": 39, "frame": 25, "value": 0.0, "interp": "linear"},
            {"timeline_frame": 20, "frame": 63, "value": 0.25, "interp": "linear"},
            {"timeline_frame": 10, "frame": 83, "value": 0.5, "interp": "linear"},
        ]
    }
    # A key at a picture that the clip passes over is told as shown at no frame of the timeline.
    between = {"frame": 62, "value": 0.2, "interp": "hold"}
    setting(fake, "c2", "crop.left")["keys"].insert(1, between)
    none = await desk.call("video_clip_keys", **crop, action="remove", frame=21)
    assert none.error is not None
    assert (
        "crop.left of clip c2 has no key at frame 21 of the timeline (frame 61 of the clip's "
        "media): its keys are at frames 39, 20, 10 of the timeline; its keys at frames 62 of "
        "the clip's media are shown at no frame of the clip (video_edit's splice_clip_keys "
        "takes one away by its place among the keys)"
    ) in none.text
    told = result(await desk.call("video_clip_keys", **crop, action="remove", frame=20))
    assert sent(fake, "c2") == {
        "op": "splice_clip_keys",
        "path": "crop.left",
        "at": 2,
        "remove": 1,
        "keys": [],
    }
    assert told["setting"]["keys"][1] == {"timeline_frame": None, **between}
    # A title has no frames of its own: its keys count the frames of the clip from its first.
    told = result(
        await desk.call(
            "video_clip_keys",
            clip="c3",
            path="transform.opacity",
            action="put",
            frame=45,
            value=0.5,
            interp="hold",
        )
    )
    assert told["key"] == {"timeline_frame": 45, "frame": 5}
    assert told["key_frames"] == "frames of the clip, from 0"
    assert told["setting"] == {
        "keys": [{"timeline_frame": 45, "frame": 5, "value": 0.5, "interp": "hold"}]
    }


async def test_every_case_of_the_engine_is_keyed_at_the_frame_that_the_engine_counts(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    for one in CASES:
        one = copy.deepcopy(one)
        cut(fake, one)
        clip = one["clip"]
        first, last = clip["start"], clip["start"] + clip["length"] - 1
        keyed = dict(zip(one["frames"], one["keys"], strict=True))
        shown: dict[int, int] = {}
        for frame in range(first, last + 1):
            shown.setdefault(keyed[frame], frame)
        for frame in sorted({first, first + 1, (first + last) // 2, last}):
            told = result(
                await desk.call(
                    "video_clip_keys",
                    clip="c1",
                    path="transform.scale",
                    action="put",
                    frame=frame,
                    value=2,
                )
            )
            assert told["key"] == {"timeline_frame": frame, "frame": keyed[frame]}, one["name"]
            assert [key["frame"] for key in sent(fake)["keys"]] == [keyed[frame]]
            # Its keys are in the order of their frames, each told with where it is first shown.
            now = setting(fake, "c1", "transform.scale")["keys"]
            assert [key["frame"] for key in now] == sorted({key["frame"] for key in now})
            assert [key["timeline_frame"] for key in told["setting"]["keys"]] == [
                shown[key["frame"]] for key in now
            ]
        # A frame of the timeline that the clip is not on is refused, either side of it.
        sends = len(fake.transactions)
        for frame in (first - 1, last + 1):
            refused = await desk.call(
                "video_clip_keys",
                clip="c1",
                path="transform.scale",
                action="put",
                frame=frame,
                value=2,
            )
            assert refused.error is not None
            assert (
                f"frame {frame} is not in clip c1, which is at frames {first} to {last} of "
                "timeline t1; frame is a frame of the timeline"
            ) in refused.text
        assert len(fake.transactions) == sends


async def test_what_is_wrong_is_told_before_anything_is_sent(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    listed = "it has transform.x, y, scale, rotation, opacity and crop.left, right, top, bottom"
    put = {"action": "put", "frame": 110, "value": 0.5}
    for asked, why in [
        ({"path": "transform.zoom", **put}, "(did you mean transform.scale?)"),
        ({"path": "transform.opacty", **put}, "(did you mean transform.opacity?)"),
        ({"path": "opacity", **put}, "(did you mean transform.opacity?)"),
        ({"path": "transform.left", **put}, f"(did you mean crop.left?); {listed}"),
        ({"path": "transform", **put}, f"no setting 'transform' that takes keys; {listed}"),
        ({"path": "gain_db", **put}, f"no setting 'gain_db' that takes keys; {listed}"),
        (
            {"path": "transform.scale", "action": "put", "frame": 110, "value": 0},
            "transform.scale is 0, out of range (Zoom, a factor: 0.01 to 16, 1 changes nothing)",
        ),
        (
            {"path": "transform.x", "action": "put", "frame": 110, "value": -16.5},
            "transform.x is -16.5, out of range (Position X, right, in widths of the timeline's "
            "picture: -16 to 16, 0 changes nothing)",
        ),
        (
            {"path": "crop.top", "action": "clear", "value": 1.5},
            "crop.top is 1.5, out of range (Crop Top, a share of the height of the clip's "
            "picture: 0 to 1, 0 changes nothing)",
        ),
        (
            {"path": "transform.rotation", "action": "put", "frame": 110},
            "put needs value, what transform.rotation is at the key (Rotation Angle, in degrees "
            "clockwise: -3600 to 3600, 0 changes nothing)",
        ),
        (
            {"path": "transform.x", "action": "put", "value": 0.5},
            "put needs frame, the frame of the timeline that the key is at",
        ),
        (
            {"path": "transform.x", "action": "remove"},
            "remove needs frame, the frame of the timeline that the key is at",
        ),
        (
            {"path": "transform.x", "action": "remove", "frame": 110},
            "transform.x of clip c1 has no key at frame 110 of the timeline (frame 29 of the "
            "clip's media): it has no keys",
        ),
        ({"path": "transform.x", "action": "clear"}, "transform.x has no keys to clear"),
        ({"path": "transform.x", **put, "interp": "ease"}, "interp is"),
        (
            {"path": "transform.x", **put, "frame": 99},
            "frame 99 is not in clip c1, which is at frames 100 to 149 of timeline t1",
        ),
        ({"path": "transform.x", **put, "frame": 150}, "frame 150 is not in clip c1"),
        (
            {"path": "transform.x", **put, "clip": "c7"},
            "there is no clip c7 on timeline t1; video_info lists the clips",
        ),
        (
            {"path": "transform.x", **put, "clip": "c9"},
            "c9 is a clip of sound on A1; a transform and a crop are of the clips of picture",
        ),
        (
            {"path": "transform.x", **put, "timeline": "t5"},
            "there is no timeline t5; video_info lists the timelines",
        ),
    ]:
        refused = await desk.call("video_clip_keys", **{"clip": "c1", **asked})
        assert refused.error is not None, asked
        assert refused.error.code == "VIDEO_KEYS", asked
        assert why in refused.text, (asked, refused.text)
    assert fake.transactions == []
    assert fake.steps == []
    # What the model's arguments cannot be is refused as any tool's are, and nothing is sent.
    unknown = await desk.call("video_clip_keys", clip="c1", path="transform.x", action="add")
    assert unknown.error is not None
    assert fake.asked.count("transaction.apply") == 0


async def test_a_change_meanwhile_is_worked_out_again_and_a_clip_gone_after_is_not_told(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    x = {"timeline": "t1", "clip": "c1", "path": "transform.x"}
    result(await desk.call("video_info"))  # Connected, and the project read once.
    fake.then["project.get"] = lambda: fake.change_by("desktop")
    told = result(await desk.call("video_clip_keys", **x, action="put", frame=100, value=1))
    assert fake.asked.count("transaction.apply") == 2
    assert len(fake.transactions) == 1
    assert told["key"] == {"timeline_frame": 100, "frame": 19}

    # The desktop took the clip away right after: the change is told, and no setting.
    def taken() -> None:
        fake.project["timelines"][0]["tracks"][0]["clips"].pop()

    fake.then["transaction.apply"] = taken
    told = result(await desk.call("video_clip_keys", **x, action="put", frame=101, value=0.5))
    assert told["key"] == {"timeline_frame": 101, "frame": 20}
    assert "setting" not in told
    assert "key_frames" not in told


async def test_a_refusal_of_the_engine_is_told_by_its_reason(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    fake.refusals["transaction.apply"] = {
        "code": -32001,
        "message": "track V1 is locked",
        "data": {"index": 0, "reason": "locked", "what": "track"},
    }
    refused = await desk.call(
        "video_clip_keys", clip="c1", path="crop.softness", action="put", frame=100, value=0.1
    )
    assert refused.error is not None
    assert refused.error.code == "VIDEO_LOCKED"
    assert "track V1 is locked" in refused.text


def test_the_tool_costs_few_characters_on_every_request() -> None:
    (tool,) = [one for one in tools_of(Videos(Config(), None)) if one.name == "video_clip_keys"]
    told = tool.schema.description
    size = len(
        json.dumps(
            {"name": tool.schema.name, "description": told, "parameters": tool.schema.parameters}
        )
    )
    # As video_grade_keys, which is under 1,300: the paths and their units are in the description.
    assert size < 1300
    assert len(told) < 520
    for said in (
        "transform.x|y|scale|rotation|opacity",
        "crop.left|right|top|bottom|softness",
        "widths, heights of the timeline's picture",
        "degrees clockwise",
        "shares 0-1",
        "a timeline frame within the clip",
        "one undo step",
    ):
        assert said in told
    properties = tool.schema.parameters["properties"]
    assert isinstance(properties, dict)
    assert list(properties) == [
        "project", "timeline", "clip", "path", "action", "frame", "value", "interp",
    ]  # fmt: skip
