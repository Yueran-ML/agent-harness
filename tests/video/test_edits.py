"""Edits and operations are written as the engine reads them."""

from fractions import Fraction
from pathlib import Path
from typing import Any, get_args

import pytest
from pydantic import ValidationError

from columbina.video import edits
from columbina.video.edits import (
    LEFT_OUT,
    CueItem,
    Key,
    Keys,
    Rate,
    Retime,
    SignedSpeed,
    Speed,
    SpeedPoint,
    Time,
    TitleStyle,
    Transform,
    of_color,
    of_media,
    of_title,
)
from columbina.video.engine import Engine
from support import fake_engine
from support.fake_engine import served


def test_fairlight_track_color_and_final_index_operation() -> None:
    assert edits.move_track("t1", "A3", 1) == {
        "op": "move_track",
        "timeline": "t1",
        "id": "A3",
        "index": 1,
    }
    assert edits.set_track("t1", "A1", color="#5aa0e0ff")["color"] == "#5aa0e0ff"
    assert edits.set_track("t1", "A1", color=None)["color"] is None
    assert "color" not in edits.set_track("t1", "A1", name="Dialog")


FROM = Time(ticks=0, scale=12800)


def test_a_source_is_put_on_a_timeline_by_the_edits_that_place_it() -> None:
    media = of_media("m3", start=FROM, length=100)

    inserted = edits.insert("t1", media, 250, video_track="V1", audio_track=None)
    overwritten = edits.overwrite(
        "t1", of_media("m3", start=FROM, end=Time(ticks=9, scale=12800)), 0
    )
    on_top = edits.place_on_top("t1", of_color("#000000ff", 25), 0)
    appended = edits.append("t1", of_title("Hello", 50), audio_track="A2")

    assert inserted == {
        "edit": "insert",
        "timeline": "t1",
        "source": {"kind": "media", "media": "m3", "from": {"ticks": 0, "scale": 12800},
                   "length": 100},
        "at": 250,
        "video_track": "V1",
        "audio_track": None,
    }  # fmt: skip
    # A track that is left out is the first of its kind, and is not written.
    assert overwritten["source"]["to"] == {"ticks": 9, "scale": 12800}
    assert "video_track" not in overwritten
    assert "audio_track" not in overwritten
    assert on_top == {
        "edit": "place_on_top",
        "timeline": "t1",
        "source": {"kind": "color", "color": "#000000ff", "length": 25},
        "at": 0,
    }
    assert appended == {
        "edit": "append",
        "timeline": "t1",
        "source": {
            "kind": "title",
            "text": "Hello",
            "style": {"font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5,
                      "align": "center"},
            "length": 50,
        },
        "audio_track": "A2",
    }  # fmt: skip
    for edit in (edits.replace, edits.smart_insert, edits.ripple_overwrite):
        assert edit("t1", media, 7)["edit"] == edit.__name__


def test_clips_are_taken_away_cut_and_moved_with_what_is_linked_to_them() -> None:
    assert edits.lift("t1", ["c4"]) == {
        "edit": "lift", "timeline": "t1", "clips": ["c4"], "linked": True,
    }  # fmt: skip
    assert edits.ripple_delete("t1", ("c4", "c5"), linked=False) == {
        "edit": "ripple_delete", "timeline": "t1", "clips": ["c4", "c5"], "linked": False,
    }  # fmt: skip
    assert edits.ripple_delete_range("t1", 10, 20) == {
        "edit": "ripple_delete_range", "timeline": "t1", "from": 10, "to": 20,
    }  # fmt: skip
    assert edits.ripple_delete_range("t1", 10, 20, tracks=["A1"])["tracks"] == ["A1"]
    assert edits.close_gap("t1", "V1", 30) == {
        "edit": "close_gap", "timeline": "t1", "track": "V1", "at": 30,
    }  # fmt: skip
    # Refusing is what the engine does unless told, and is not written, as the engine writes it.
    assert edits.close_gap("t1", "V1", 30, straddling="refuse") == edits.close_gap("t1", "V1", 30)
    assert edits.close_gap("t1", "V1", 30, straddling="stay")["straddling"] == "stay"
    assert edits.blade("t1", 12) == {"edit": "blade", "timeline": "t1", "at": 12, "linked": True}
    assert edits.blade("t1", 12, tracks=["V2"], linked=False)["tracks"] == ["V2"]
    assert edits.move("t1", ["c4"], -5) == {
        "edit": "move", "timeline": "t1", "clips": ["c4"], "delta": -5, "overwrite": False,
        "linked": True,
    }  # fmt: skip
    assert edits.move("t1", ["c4"], 5, to_track="V2", overwrite=True)["to_track"] == "V2"
    assert edits.insert_move("t1", ["c4"], 0) == {
        "edit": "insert_move", "timeline": "t1", "clips": ["c4"], "at": 0, "linked": True,
    }  # fmt: skip


def test_the_four_trims_move_edges_and_say_whether_they_ripple() -> None:
    assert edits.trim("t1", "c2", "out", -3, ripple=True) == {
        "edit": "trim", "timeline": "t1", "clip": "c2", "edge": "out", "delta": -3,
        "ripple": True, "linked": True,
    }  # fmt: skip
    assert edits.roll("t1", "c2", "c3", 4) == {
        "edit": "roll", "timeline": "t1", "left": "c2", "right": "c3", "delta": 4,
        "linked": True,
    }  # fmt: skip
    assert edits.slip("t1", "c2", 2) == {
        "edit": "slip", "timeline": "t1", "clip": "c2", "delta": 2, "linked": True,
    }  # fmt: skip
    assert edits.slide("t1", "c2", -2, linked=False)["linked"] is False


def test_transitions_speed_and_subtitles_are_written_with_what_is_given() -> None:
    assert edits.add_transition("t1", "c2", "c3", 12) == {
        "edit": "add_transition", "timeline": "t1", "left": "c2", "right": "c3", "length": 12,
        "align": "center",
    }  # fmt: skip
    assert edits.add_transition("t1", "c2", "c3", 12, kind="dip_black")["kind"] == "dip_black"
    assert edits.set_transition("t1", "x7", length=6) == {
        "edit": "set_transition", "timeline": "t1", "id": "x7", "length": 6,
    }  # fmt: skip
    assert edits.remove_transition("t1", "x7") == {
        "edit": "remove_transition", "timeline": "t1", "id": "x7",
    }  # fmt: skip
    assert edits.set_speed("t1", "c2", Speed.of(Fraction(1, 2)), reverse=True) == {
        "edit": "set_speed", "timeline": "t1", "clip": "c2", "speed": {"num": 1, "den": 2},
        "reverse": True, "ripple": False, "linked": True,
    }  # fmt: skip
    assert "reverse" not in edits.set_speed("t1", "c2", Speed.of(2))
    assert edits.add_cue("t1", "S1", 0, 50, "Hello") == {
        "edit": "add_cue", "timeline": "t1", "track": "S1", "start": 0, "length": 50,
        "text": "Hello",
    }  # fmt: skip
    assert edits.set_cue("t1", "u3", text="Hi") == {
        "edit": "set_cue", "timeline": "t1", "id": "u3", "text": "Hi",
    }  # fmt: skip
    assert edits.remove_cue("t1", "u3")["edit"] == "remove_cue"
    assert edits.cues_from("t1", "S1", [CueItem(start=0, length=5, text="a")]) == {
        "edit": "cues_from", "timeline": "t1", "track": "S1",
        "items": [{"start": 0, "length": 5, "text": "a"}],
    }  # fmt: skip
    assert edits.add_track("t1", "subtitle", language="zh") == {
        "edit": "add_track", "timeline": "t1", "kind": "subtitle", "language": "zh",
    }  # fmt: skip
    assert edits.remove_track("t1", "A2") == {
        "edit": "remove_track", "timeline": "t1", "track": "A2",
    }  # fmt: skip


def test_subtitles_are_put_in_order_and_none_lasts_over_the_next() -> None:
    def cue(start: int, length: int, text: str) -> edits.CueItem:
        return edits.CueItem(start=start, length=length, text=text)

    assert edits.untangled(
        [cue(30, 10, "c"), cue(0, 12, "a"), cue(10, 5, "b"), cue(40, 0, "x"), cue(45, 5, "d")]
    ) == [cue(0, 10, "a"), cue(10, 5, "b"), cue(30, 10, "c"), cue(45, 5, "d")]
    # One that begins with the next is shown for no frame, and is left out.
    assert edits.untangled([cue(5, 4, "a"), cue(5, 6, "b")]) == [cue(5, 6, "b")]
    assert edits.untangled([]) == []


def test_operations_set_what_is_given_and_tell_null_from_left_out() -> None:
    timeline = edits.add_timeline(
        "Cut",
        Rate(num=30000, den=1001),
        id="$t",
        tracks=[edits.track("video"), edits.track("subtitle", language="en")],
    )
    clip = edits.set_clip(
        "t1",
        "c2",
        enabled=False,
        transform=Transform(scale=0.5, opacity=0.8),
        fade_in=5,
        link=None,
        source=of_title("Hi", 1, TitleStyle(size=40)),
    )

    assert timeline == {
        "op": "add_timeline",
        "timeline": {
            "id": "$t",
            "name": "Cut",
            "rate": {"num": 30000, "den": 1001},
            "width": 1920,
            "height": 1080,
            "sample_rate": 48000,
            "tracks": [
                {"id": "", "kind": "video", "name": "", "enabled": True, "locked": False,
                 "clips": [], "transitions": []},
                {"id": "", "kind": "subtitle", "name": "", "enabled": True, "locked": False,
                 "language": "en", "cues": []},
            ],
        },
    }  # fmt: skip
    assert clip == {
        "op": "set_clip",
        "timeline": "t1",
        "id": "c2",
        "enabled": False,
        "transform": {"x": 0.0, "y": 0.0, "scale": 0.5, "rotation": 0.0, "opacity": 0.8},
        "fade_in": 5,
        # The length of a source is the clip's, and is not written.
        "source": {"kind": "title", "text": "Hi",
                   "style": {"font": "", "size": 40, "color": "#ffffffff", "x": 0.5, "y": 0.5,
                             "align": "center"}},
        "link": None,
    }  # fmt: skip
    assert "link" not in edits.set_clip("t1", "c2", gain_db=-6.0)
    assert edits.set_clip("t1", "c2", link="")["link"] == ""
    assert edits.set_media("m1", bin=None, mark_in=FROM) == {
        "op": "set_media", "id": "m1", "bin": None, "mark_in": {"ticks": 0, "scale": 12800},
    }  # fmt: skip
    assert edits.set_media("m1", name="Talk", mark_out=LEFT_OUT) == {
        "op": "set_media", "id": "m1", "name": "Talk",
    }  # fmt: skip
    assert edits.set_track("t1", "A1", locked=True) == {
        "op": "set_track", "timeline": "t1", "id": "A1", "locked": True,
    }  # fmt: skip
    assert edits.set_timeline("t1", width=1080, height=1920) == {
        "op": "set_timeline", "id": "t1", "width": 1080, "height": 1920,
    }  # fmt: skip
    assert edits.add_bin("Shots", id="$b") == {
        "op": "add_bin", "bin": {"id": "$b", "name": "Shots", "parent": None},
    }  # fmt: skip
    assert edits.set_bin("b1", parent=None) == {"op": "set_bin", "id": "b1", "parent": None}
    assert edits.remove_bin("b1") == {"op": "remove_bin", "id": "b1"}
    assert edits.remove_media("m1") == {"op": "remove_media", "id": "m1"}
    assert edits.remove_timeline("t1") == {"op": "remove_timeline", "id": "t1"}


def test_values_that_the_engine_could_not_read_are_refused_before_they_are_sent() -> None:
    for wrong in (
        lambda: of_color("black", 1),
        lambda: of_color("#000000FF", 1),
        lambda: Time(ticks=0, scale=0),
        lambda: Speed(num=0, den=1),
        lambda: Rate(num=25, den=0),
        lambda: TitleStyle(size=0),
        lambda: SignedSpeed(num=-1, den=0),
        lambda: SpeedPoint(frame=-1),
        lambda: Retime(points=[]),
        lambda: Retime(points=[SpeedPoint(frame=frame) for frame in range(1025)]),
    ):
        with pytest.raises(ValidationError):
            wrong()

    assert Time.at(1_500_000, 12800) == Time(ticks=19200, scale=12800)
    assert Time.at(1, 25) == Time(ticks=0, scale=25)
    assert Speed.of(Fraction(4, 2)) == Speed(num=2, den=1)


async def test_every_edit_is_taken_by_the_engine_beside_operations(tmp_path: Path) -> None:
    every: list[dict[str, Any]] = [
        edits.insert("t1", of_color("#000000ff", 5), 0),
        edits.overwrite("t1", of_color("#000000ff", 5), 0),
        edits.replace("t1", of_color("#000000ff", 5), 0),
        edits.smart_insert("t1", of_color("#000000ff", 5), 0),
        edits.ripple_overwrite("t1", of_color("#000000ff", 5), 0),
        edits.place_on_top("t1", of_color("#000000ff", 5), 0),
        edits.append("t1", of_color("#000000ff", 5)),
        edits.blade("t1", 2),
        edits.move("t1", ["c3"], 1),
        edits.insert_move("t1", ["c3"], 0),
        edits.trim("t1", "c3", "in", 1),
        edits.roll("t1", "c3", "c4", 1),
        edits.slip("t1", "c3", 1),
        edits.slide("t1", "c3", 1),
        edits.add_transition("t1", "c3", "c4", 2),
        edits.set_transition("t1", "x1", length=1),
        edits.remove_transition("t1", "x1"),
        edits.set_speed("t1", "c3", Speed.of(2)),
        edits.add_track("t1", "subtitle"),
        edits.add_cue("t1", "S1", 0, 5, "a"),
        edits.set_cue("t1", "u1", text="b"),
        edits.remove_cue("t1", "u1"),
        edits.cues_from("t1", "S1", []),
        edits.close_gap("t1", "V1", 0),
        edits.lift("t1", ["c3"]),
        edits.ripple_delete("t1", ["c4"]),
        edits.ripple_delete_range("t1", 0, 1),
        edits.remove_track("t1", "S1"),
    ]
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            applied = await one.apply(every)

        assert [one["edit"] for one in fake.edits] == [one["edit"] for one in every]
        assert all("op" in one for one in applied.done)


def test_the_sound_of_a_timeline_is_written_as_the_engine_reads_it() -> None:
    second = Time(ticks=48000, scale=48000)
    limiter = {"effect": {"kind": "limiter", "ceiling_db": -1.0}}

    assert edits.set_track_audio("t1", "A1", {"strip": {"fader_db": -6}}) == {
        "edit": "set_track_audio",
        "timeline": "t1",
        "track": "A1",
        "patch": {"strip": {"fader_db": -6}},
    }
    assert (
        edits.set_track_audio("t1", "A1", {"mute": True}, follow_groups=False)["follow_groups"]
        is False
    )
    assert edits.set_bus("t1", "B1", {"strip": {"fader_db": -3}})["edit"] == "set_bus"
    assert edits.add_bus("t1", {"id": "B2", "name": "Dialogue"}, index=1) == {
        "op": "add_bus",
        "timeline": "t1",
        "bus": {"id": "B2", "name": "Dialogue"},
        "index": 1,
    }
    assert edits.add_insert("t1", {"bus": "B1"}, limiter) == {
        "op": "add_insert",
        "timeline": "t1",
        "of": {"bus": "B1"},
        "insert": limiter,
    }
    assert edits.set_insert("t1", {"track": "A1"}, 0, None)["insert"] is None
    assert edits.move_insert("t1", {"track": "A1"}, 0, 2, to_of={"bus": "B1"}) == {
        "edit": "move_insert",
        "timeline": "t1",
        "of": {"track": "A1"},
        "from": 0,
        "to": 2,
        "to_of": {"bus": "B1"},
    }
    assert edits.clip_gain_keys("t1", "c3", "add", second, param="pan", value=0.5) == {
        "edit": "clip_gain_keys",
        "timeline": "t1",
        "clip": "c3",
        "action": "add",
        "at": {"ticks": 48000, "scale": 48000},
        "param": "pan",
        "value": 0.5,
    }
    assert "param" not in edits.clip_gain_keys("t1", "c3", "remove", second)
    assert edits.clip_gain_range("t1", "c3", FROM, second, -6.0)["from"] == {
        "ticks": 0,
        "scale": 12800,
    }
    assert edits.nudge_clip_gain("t1", ["c3"], 3.0)["linked"] is True
    assert edits.paste_attributes("t1", "c3", ["c4"], ["volume"])["items"] == ["volume"]
    assert edits.fade_to_playhead("t1", ["c3"], "out", 40)["side"] == "out"
    assert edits.lane_points(
        "t1", {"track": "A1"}, "fader_db", "add", points=[(FROM, -10.0), (second, None)]
    )["points"] == [
        {"at": {"ticks": 0, "scale": 12800}, "value": -10.0},
        {"at": second.model_dump()},
    ]
    removed = edits.lane_points("t1", {"vca": "VCA1"}, "fader_db", "remove", at=[second])
    assert (removed["at"], "points" in removed) == ([second.model_dump()], False)
    assert edits.clear_lanes("t1", {"bus": "B1"}, then="hold", at=second)["then"] == "hold"
    assert edits.set_track("t1", "A1", mute=True) == {
        "op": "set_track",
        "timeline": "t1",
        "id": "A1",
        "mute": True,
    }
    assert edits.set_transition("t1", "x1", fade_in_level_db=3)["fade_in_level_db"] == 3


def test_the_sound_of_a_clip_and_the_lanes_are_written_as_the_engine_reads_them() -> None:
    second = Time(ticks=48000, scale=48000)
    fade = {"length": {"ticks": 1, "scale": 2}, "curve": 0.5, "skew": 0.0}

    assert edits.set_clip("t1", "c3", audio_fade_in=fade, channels=None, pitch_correction=True) == {
        "op": "set_clip",
        "timeline": "t1",
        "id": "c3",
        "audio_fade_in": fade,
        "channels": None,
        "pitch_correction": True,
    }
    assert "audio_fade_out" not in edits.set_clip("t1", "c3", gain_db=-3.0)
    mapped = edits.set_clip("t1", "c3", channels=[{"source": {"stream": 1, "channel": 0}}])
    assert mapped["channels"] == [{"source": {"stream": 1, "channel": 0}}]
    assert edits.set_lane("t1", {"bus": "B1"}, "fader_db", None) == {
        "op": "set_lane",
        "timeline": "t1",
        "of": {"bus": "B1"},
        "path": "fader_db",
        "lane": None,
    }
    assert edits.lane_range(
        "t1", {"track": "A1"}, "pan.x", FROM, second, "shift_value", delta=0.2
    ) == {
        "edit": "lane_range",
        "timeline": "t1",
        "of": {"track": "A1"},
        "path": "pan.x",
        "from": {"ticks": 0, "scale": 12800},
        "to": {"ticks": 48000, "scale": 48000},
        "action": "shift_value",
        "delta": 0.2,
    }
    assert "grouped" not in edits.lane_draw("t1", {"track": "A1"}, "fader_db", FROM, second, [])
    assert (
        edits.lane_points("t1", {"track": "A1"}, "fader_db", "remove", at=[second], grouped=True)[
            "grouped"
        ]
        is True
    )


def test_a_setting_of_a_transform_takes_keys_and_two_operations_set_them() -> None:
    keys = Keys(keys=[Key(frame=0, value=-0.5), Key(frame=24, value=0.0, interp="hold")])
    written = [
        {"frame": 0, "value": -0.5, "interp": "linear"},
        {"frame": 24, "value": 0.0, "interp": "hold"},
    ]

    clip = edits.set_clip("t1", "c2", transform=Transform(x=keys, scale=2))

    assert clip["transform"] == {
        "x": {"keys": written},
        "y": 0.0,
        "scale": 2.0,
        "rotation": 0.0,
        "opacity": 1.0,
    }
    assert edits.set_clip_keys("t1", "c2", "transform.opacity", 0.5) == {
        "op": "set_clip_keys",
        "timeline": "t1",
        "id": "c2",
        "path": "transform.opacity",
        "param": 0.5,
    }
    assert edits.set_clip_keys("t1", "c2", "transform.x", keys)["param"] == {"keys": written}
    eased = Key(frame=30, value=1.0, interp={"bezier": {"out": [0.3, 0.0], "in": [0.3, 0.0]}})
    assert edits.splice_clip_keys("t1", "c2", "transform.x", 1, 1, [eased]) == {
        "op": "splice_clip_keys",
        "timeline": "t1",
        "id": "c2",
        "path": "transform.x",
        "at": 1,
        "remove": 1,
        "keys": [
            {
                "frame": 30,
                "value": 1.0,
                "interp": {"bezier": {"out": [0.3, 0.0], "in": [0.3, 0.0]}},
            }
        ],
    }


async def test_the_keys_of_a_setting_of_a_clip_are_set_by_its_path(tmp_path: Path) -> None:
    def opacity(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]["transform"]["opacity"]

    first, last = Key(frame=0, value=0.0), Key(frame=24, value=1.0)
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            id = fake.project["timelines"][0]["tracks"][0]["clips"][0]["id"]
            path = "transform.opacity"

            await one.apply([edits.splice_clip_keys("t1", id, path, 0, 0, [first, last])])
            assert opacity(fake) == {
                "keys": [
                    {"frame": 0, "value": 0.0, "interp": "linear"},
                    {"frame": 24, "value": 1.0, "interp": "linear"},
                ]
            }
            await one.apply([edits.splice_clip_keys("t1", id, path, 0, 1, [])])
            assert opacity(fake) == {"keys": [{"frame": 24, "value": 1.0, "interp": "linear"}]}
            await one.apply([edits.set_clip_keys("t1", id, path, 0.5)])

        assert opacity(fake) == 0.5


async def test_the_fake_engine_keeps_the_masks_of_a_composition(tmp_path: Path) -> None:
    def composition(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]["fusion"]

    def node(name: str, kind: str, **params: Any) -> dict[str, Any]:
        return {"id": name, "kind": kind, "label": "", "params": params}

    def mask(kind: str, **shape: Any) -> dict[str, Any]:
        # Every setting of a mask is written, as the engine writes a node: none is left out.
        common = {
            "level": 1.0,
            "soft_edge": 0.0,
            "border_width": 0.0,
            "invert": False,
            "solid": True,
            "paint_mode": "merge",
            "center": {"x": 0.5, "y": 0.5},
        }
        return node("", kind, **common, **shape)

    def links(fake: fake_engine.Fake) -> list[tuple[str, str, str]]:
        return [(link["from"], link["to"], link["input"]) for link in composition(fake)["links"]]

    triangle = [{"x": -0.1, "y": -0.1}, {"x": 0.1, "y": -0.1}, {"x": 0.0, "y": 0.1}]
    rising = {
        "keys": [
            {"frame": 0, "value": 0.1, "interp": "linear"},
            {"frame": 24, "value": 0.3, "interp": "linear"},
        ]
    }
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            at = {
                "timeline": "t1",
                "clip": fake.project["timelines"][0]["tracks"][0]["clips"][0]["id"],
            }
            colour = {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
            first = {
                "nodes": [node("bg", "background", **colour), node("out", "media_out")],
                "links": [{"from": "bg", "to": "out", "input": "input"}],
            }
            await one.apply([{"op": "set_fusion", **at, "value": first}])

            # An ellipse is the effect mask of the background, and a polygon comes into it.
            ellipse = mask("ellipse", width=0.5, height=0.5, angle=0.0)
            into = [{"from": "", "to": "bg", "input": "mask"}]
            await one.apply([{"op": "add_fusion_node", **at, "node": ellipse, "links": into}])
            e = composition(fake)["nodes"][-1]["id"]
            polygon = mask("polygon", size=1.0, angle=0.0, points=triangle, fill="alternate")
            into = [{"from": "", "to": e, "input": "mask"}]
            await one.apply([{"op": "add_fusion_node", **at, "node": polygon, "links": into}])
            p = composition(fake)["nodes"][-1]["id"]
            kinds = [each["kind"] for each in composition(fake)["nodes"]]
            assert kinds == ["background", "media_out", "ellipse", "polygon"]
            # The links are in the engine's order: by the node, then by the name of the input.
            assert links(fake) == sorted(
                [(e, "bg", "mask"), (p, e, "mask"), ("bg", "out", "input")],
                key=lambda link: (link[1], link[2]),
            )

            # A number of a point is keyed by the place of the point in the list.
            path = f"{p}.points.2.y"
            await one.apply([{"op": "set_fusion_keys", **at, "path": path, "param": rising}])
            points = composition(fake)["nodes"][-1]["params"]["points"]
            assert points == [triangle[0], triangle[1], {"x": 0.0, "y": rising}]
            taken = {"op": "splice_fusion_keys", **at, "path": path, "at": 1, "remove": 1}
            await one.apply([{**taken, "keys": []}])
            assert composition(fake)["nodes"][-1]["params"]["points"][2]["y"] == {
                "keys": [rising["keys"][0]]
            }
            soft = {"op": "set_fusion_keys", **at, "path": f"{e}.soft_edge", "param": 0.25}
            await one.apply([soft])
            assert composition(fake)["nodes"][2]["params"]["soft_edge"] == 0.25

            # The controls of a mask are set by a patch, and the points of a polygon whole.
            patch = {"fill": "non_zero_winding", "invert": True, "points": [{"x": 0.0, "y": 0.0}]}
            await one.apply([{"op": "set_fusion_node", **at, "id": p, "params": patch}])
            params = composition(fake)["nodes"][-1]["params"]
            assert (params["fill"], params["invert"], params["solid"]) == (
                "non_zero_winding",
                True,
                True,
            )
            assert params["points"] == [{"x": 0.0, "y": 0.0}]

            # A mask is unlinked as another input is.
            await one.apply([{"op": "disconnect_fusion", **at, "to": "bg", "input": "mask"}])

        assert links(fake) == sorted(
            [(p, e, "mask"), ("bg", "out", "input")], key=lambda link: (link[1], link[2])
        )


async def test_the_fake_engine_keeps_a_blur_and_a_mosaic_of_a_composition(tmp_path: Path) -> None:
    def composition(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]["fusion"]

    def node(name: str, kind: str, **params: Any) -> dict[str, Any]:
        return {"id": name, "kind": kind, "label": "", "params": params}

    def links(fake: fake_engine.Fake) -> list[tuple[str, str, str]]:
        return [(link["from"], link["to"], link["input"]) for link in composition(fake)["links"]]

    def params(fake: fake_engine.Fake, name: str) -> Any:
        return next(each for each in composition(fake)["nodes"] if each["id"] == name)["params"]

    # Every setting of each is written, as the engine writes a node: none is left out.
    blur = node("", "blur", filter="gaussian", lock_xy=True, size={"x": 5.0, "y": 5.0}, blend=1.0)
    mosaic = node(
        "",
        "mosaic",
        frequency=32.0,
        aspect=1.0,
        offset={"x": 0.0, "y": 0.0},
        whole_cells_only=False,
    )
    rising = {
        "keys": [
            {"frame": 0, "value": 0.0, "interp": "linear"},
            {"frame": 24, "value": 40.0, "interp": "linear"},
        ]
    }
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            at = {
                "timeline": "t1",
                "clip": fake.project["timelines"][0]["tracks"][0]["clips"][0]["id"],
            }
            colour = {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
            first = {
                "nodes": [node("bg", "background", **colour), node("out", "media_out")],
                "links": [{"from": "bg", "to": "out", "input": "input"}],
            }
            await one.apply([{"op": "set_fusion", **at, "value": first}])

            # A blur of the background goes to the media_out in its place, and a mosaic of the
            # blur after it, keyed by the background.
            into = [
                {"from": "bg", "to": "", "input": "input"},
                {"from": "", "to": "out", "input": "input"},
            ]
            await one.apply([{"op": "add_fusion_node", **at, "node": blur, "links": into}])
            b = composition(fake)["nodes"][-1]["id"]
            into = [
                {"from": b, "to": "", "input": "input"},
                {"from": "", "to": "out", "input": "input"},
                {"from": "bg", "to": "", "input": "mask"},
            ]
            await one.apply([{"op": "add_fusion_node", **at, "node": mosaic, "links": into}])
            m = composition(fake)["nodes"][-1]["id"]
            kinds = [each["kind"] for each in composition(fake)["nodes"]]
            assert kinds == ["background", "media_out", "blur", "mosaic"]
            # The links are in the engine's order: by the node, then by the name of the input.
            assert links(fake) == sorted(
                [(m, "out", "input"), ("bg", b, "input"), (b, m, "input"), ("bg", m, "mask")],
                key=lambda link: (link[1], link[2]),
            )

            # A number is keyed by its path in the params of its node.
            path = f"{b}.size.x"
            await one.apply([{"op": "set_fusion_keys", **at, "path": path, "param": rising}])
            assert params(fake, b)["size"] == {"x": rising, "y": 5.0}
            taken = {"op": "splice_fusion_keys", **at, "path": path, "at": 1, "remove": 1}
            await one.apply([{**taken, "keys": []}])
            assert params(fake, b)["size"]["x"] == {"keys": [rising["keys"][0]]}
            down = {"op": "set_fusion_keys", **at, "path": f"{m}.offset.y", "param": -6.0}
            await one.apply([down])
            assert params(fake, m)["offset"] == {"x": 0.0, "y": -6.0}

            # The other settings are set by a patch, which leaves what it does not name.
            patch = {"filter": "box", "lock_xy": False, "size": {"y": 12.5}, "blend": 0.5}
            await one.apply([{"op": "set_fusion_node", **at, "id": b, "params": patch}])
            assert (params(fake, b)["filter"], params(fake, b)["lock_xy"]) == ("box", False)
            assert params(fake, b)["size"]["y"] == 12.5
            assert params(fake, b)["blend"] == 0.5
            patch = {"frequency": 24.0, "whole_cells_only": True}
            await one.apply([{"op": "set_fusion_node", **at, "id": m, "params": patch}])
            assert params(fake, m) == {
                "frequency": 24.0,
                "aspect": 1.0,
                "offset": {"x": 0.0, "y": -6.0},
                "whole_cells_only": True,
            }

            # The key of a mosaic is unlinked as another input is.
            await one.apply([{"op": "disconnect_fusion", **at, "to": m, "input": "mask"}])

        assert links(fake) == sorted(
            [(m, "out", "input"), ("bg", b, "input"), (b, m, "input")],
            key=lambda link: (link[1], link[2]),
        )


async def test_the_fake_engine_keeps_a_delta_keyer_of_a_composition(tmp_path: Path) -> None:
    def composition(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]["fusion"]

    def node(name: str, kind: str, **params: Any) -> dict[str, Any]:
        return {"id": name, "kind": kind, "label": "", "params": params}

    def links(fake: fake_engine.Fake) -> list[tuple[str, str, str]]:
        return [(link["from"], link["to"], link["input"]) for link in composition(fake)["links"]]

    def params(fake: fake_engine.Fake, name: str) -> Any:
        return next(each for each in composition(fake)["nodes"] if each["id"] == name)["params"]

    def at_first() -> dict[str, Any]:
        # Every setting is written, as the engine writes a node: none is left out.
        return {
            "screen": {"r": 0.0, "g": 1.0, "b": 0.0},
            "gain": 1.0,
            "balance": 0.5,
            "pre_blur": 0.0,
            "low": 0.0,
            "high": 1.0,
            "erode_dilate": 0.0,
            "matte_blur": 0.0,
            "clean_foreground": 0.0,
            "clean_background": 0.0,
            "replace": "soft_color",
            "replace_color": {"r": 0.5, "g": 0.5, "b": 0.5},
            "garbage_invert": False,
            "spill_method": "medium",
            "spill_suppression": 1.0,
            "view": "final_result",
        }

    keyer = node("", "delta_keyer", **at_first())
    rising = {
        "keys": [
            {"frame": 0, "value": 1.0, "interp": "linear"},
            {"frame": 24, "value": 3.0, "interp": "linear"},
        ]
    }
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            at = {
                "timeline": "t1",
                "clip": fake.project["timelines"][0]["tracks"][0]["clips"][0]["id"],
            }
            colour = {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
            first = {
                "nodes": [node("bg", "background", **colour), node("out", "media_out")],
                "links": [{"from": "bg", "to": "out", "input": "input"}],
            }
            await one.apply([{"op": "set_fusion", **at, "value": first}])

            # A keyer of the background goes to the media_out in its place, the background its
            # garbage matte and its effect mask too.
            into = [
                {"from": "bg", "to": "", "input": "input"},
                {"from": "", "to": "out", "input": "input"},
                {"from": "bg", "to": "", "input": "garbage_matte"},
                {"from": "bg", "to": "", "input": "mask"},
            ]
            await one.apply([{"op": "add_fusion_node", **at, "node": keyer, "links": into}])
            k = composition(fake)["nodes"][-1]["id"]
            kinds = [each["kind"] for each in composition(fake)["nodes"]]
            assert kinds == ["background", "media_out", "delta_keyer"]
            assert params(fake, k) == at_first()
            # Its solid matte is linked as another input is.
            solid = {"from": "bg", "to": k, "input": "solid_matte"}
            await one.apply([{"op": "connect_fusion", **at, "link": solid}])
            # The links are in the engine's order: by the node, then by the name of the input.
            into_keyer = [
                ("bg", k, "garbage_matte"),
                ("bg", k, "input"),
                ("bg", k, "mask"),
                ("bg", k, "solid_matte"),
            ]
            assert links(fake) == sorted(
                [(k, "out", "input"), *into_keyer],
                key=lambda link: (link[1], link[2]),
            )
            assert [link for link in links(fake) if link[1] == k] == into_keyer

            # A number is keyed by its path in the params of its node.
            path = f"{k}.gain"
            await one.apply([{"op": "set_fusion_keys", **at, "path": path, "param": rising}])
            assert params(fake, k)["gain"] == rising
            taken = {"op": "splice_fusion_keys", **at, "path": path, "at": 1, "remove": 1}
            await one.apply([{**taken, "keys": []}])
            assert params(fake, k)["gain"] == {"keys": [rising["keys"][0]]}
            green = {"op": "set_fusion_keys", **at, "path": f"{k}.screen.g", "param": 0.45}
            await one.apply([green])
            assert params(fake, k)["screen"] == {"r": 0.0, "g": 0.45, "b": 0.0}
            blue = {"op": "set_fusion_keys", **at, "path": f"{k}.replace_color.b", "param": 0.25}
            await one.apply([blue])
            assert params(fake, k)["replace_color"] == {"r": 0.5, "g": 0.5, "b": 0.25}
            low = {"op": "set_fusion_keys", **at, "path": f"{k}.low", "param": 0.125}
            await one.apply([low])
            assert params(fake, k)["low"] == 0.125

            # The other settings are set by a patch, which leaves what it does not name.
            patch = {
                "replace": "source",
                "spill_method": "well_done",
                "garbage_invert": True,
                "view": "matte",
                "erode_dilate": -2.5,
            }
            await one.apply([{"op": "set_fusion_node", **at, "id": k, "params": patch}])
            assert params(fake, k) == {
                **at_first(),
                "screen": {"r": 0.0, "g": 0.45, "b": 0.0},
                "gain": {"keys": [rising["keys"][0]]},
                "low": 0.125,
                "erode_dilate": -2.5,
                "replace": "source",
                "replace_color": {"r": 0.5, "g": 0.5, "b": 0.25},
                "garbage_invert": True,
                "spill_method": "well_done",
                "view": "matte",
            }

            # A matte of a keyer is unlinked as another input is.
            await one.apply([{"op": "disconnect_fusion", **at, "to": k, "input": "solid_matte"}])

        left = [(k, "out", "input"), *into_keyer[:3]]
        assert links(fake) == sorted(left, key=lambda link: (link[1], link[2]))


async def test_the_fake_engine_keeps_a_tracker_of_a_composition(tmp_path: Path) -> None:
    def composition(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]["fusion"]

    def node(name: str, kind: str, **params: Any) -> dict[str, Any]:
        return {"id": name, "kind": kind, "label": "", "params": params}

    def links(fake: fake_engine.Fake) -> list[tuple[str, str, str]]:
        return [(link["from"], link["to"], link["input"]) for link in composition(fake)["links"]]

    def params(fake: fake_engine.Fake, name: str) -> Any:
        return next(each for each in composition(fake)["nodes"] if each["id"] == name)["params"]

    def pattern(name: str, x: float, path: dict[str, list[float]]) -> dict[str, Any]:
        # A pattern as the engine writes one: where it was put, how large it and its search
        # area are, its offset, and its path by the frame.
        return {
            "name": name,
            "enabled": True,
            "pattern": {"x": x, "y": 0.5, "width": 0.05, "height": 0.05},
            "search": {"width": 0.125, "height": 0.125},
            "offset": {"x": 0.0, "y": 0.0},
            "path": path,
        }

    def at_first() -> dict[str, Any]:
        # Every setting is written, as the engine writes a node: none is left out.
        return {
            "operation": "none",
            "merge": "bg_only",
            "position": True,
            "rotation": True,
            "scaling": True,
            "pivot": "tracker_average",
            "pivot_tracker": 0,
            "pivot_point": {"x": 0.5, "y": 0.5},
            "reference": 0,
            "trackers": [],
        }

    tracker = node("", "tracker", **at_first())
    ellipse = node(
        "",
        "ellipse",
        level=1.0,
        soft_edge=0.0,
        border_width=0.0,
        invert=False,
        solid=True,
        paint_mode="merge",
        center={"x": 0.5, "y": 0.5},
        width=0.5,
        height=0.5,
        angle=0.0,
    )
    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            at = {
                "timeline": "t1",
                "clip": fake.project["timelines"][0]["tracks"][0]["clips"][0]["id"],
            }
            colour = {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
            first = {
                "nodes": [node("bg", "background", **colour), node("out", "media_out")],
                "links": [{"from": "bg", "to": "out", "input": "input"}],
            }
            await one.apply([{"op": "set_fusion", **at, "value": first}])

            # A tracker of the background goes to the media_out in its place, the background
            # its foreground too.
            into = [
                {"from": "bg", "to": "", "input": "input"},
                {"from": "", "to": "out", "input": "input"},
                {"from": "bg", "to": "", "input": "foreground"},
            ]
            await one.apply([{"op": "add_fusion_node", **at, "node": tracker, "links": into}])
            t = composition(fake)["nodes"][-1]["id"]
            kinds = [each["kind"] for each in composition(fake)["nodes"]]
            assert kinds == ["background", "media_out", "tracker"]
            assert params(fake, t) == at_first()
            # An ellipse is its effect mask: the links are in the engine's order, by the node
            # and then by the name of the input.
            mask = [{"from": "", "to": t, "input": "mask"}]
            await one.apply([{"op": "add_fusion_node", **at, "node": ellipse, "links": mask}])
            e = composition(fake)["nodes"][-1]["id"]
            into_tracker = [("bg", t, "foreground"), ("bg", t, "input"), (e, t, "mask")]
            assert links(fake) == sorted(
                [(t, "out", "input"), *into_tracker],
                key=lambda link: (link[1], link[2]),
            )
            assert [link for link in links(fake) if link[1] == t] == into_tracker

            # The patterns are set whole, with their paths, as the tracking writes them; the
            # other settings by a patch, which leaves what it does not name.
            patterns = [
                pattern("Left", 0.25, {"0": [0.25, 0.5], "1": [0.26, 0.5]}),
                pattern("Right", 0.75, {"0": [0.75, 0.5], "1": [0.76, 0.5]}),
            ]
            await one.apply(
                [{"op": "set_fusion_node", **at, "id": t, "params": {"trackers": patterns}}]
            )
            settings: dict[str, Any] = {
                "operation": "match_move",
                "merge": "fg_over_bg",
                "reference": 1,
            }
            await one.apply([{"op": "set_fusion_node", **at, "id": t, "params": settings}])
            assert params(fake, t) == {**at_first(), **settings, "trackers": patterns}
            further = {"0": [0.25, 0.5], "1": [0.26, 0.5], "2": [0.27, 0.5]}
            longer = [pattern("Left", 0.25, further)]
            await one.apply(
                [{"op": "set_fusion_node", **at, "id": t, "params": {"trackers": longer}}]
            )
            assert params(fake, t)["trackers"] == longer

            # The centre of the ellipse follows a pattern of the tracker, by a patch of the
            # centre, which keeps its own numbers; and lets go of it by a follow of null.
            follow: dict[str, Any] = {"node": t, "output": "tracker", "tracker": 0}
            centre: dict[str, Any] = {"center": {"follow": follow}}
            await one.apply([{"op": "set_fusion_node", **at, "id": e, "params": centre}])
            assert params(fake, e)["center"] == {"x": 0.5, "y": 0.5, "follow": follow}
            centre = {"center": {"follow": {"output": "steady_position"}}}
            await one.apply([{"op": "set_fusion_node", **at, "id": e, "params": centre}])
            assert params(fake, e)["center"]["follow"] == {**follow, "output": "steady_position"}
            centre = {"center": {"follow": None}}
            await one.apply([{"op": "set_fusion_node", **at, "id": e, "params": centre}])
            assert params(fake, e)["center"] == {"x": 0.5, "y": 0.5, "follow": None}

            # The foreground of a tracker is unlinked as another input is.
            await one.apply([{"op": "disconnect_fusion", **at, "to": t, "input": "foreground"}])

        left = [(t, "out", "input"), ("bg", t, "input"), (e, t, "mask")]
        assert links(fake) == sorted(left, key=lambda link: (link[1], link[2]))


def test_a_crop_and_a_composite_mode_are_written_as_the_engine_reads_them() -> None:
    opening = Keys(keys=[Key(frame=0, value=1.0), Key(frame=20, value=0.0)])

    clip = edits.set_clip("t1", "c2", crop=edits.Crop(left=opening, top=0.1), composite="screen")

    assert clip == {
        "op": "set_clip",
        "timeline": "t1",
        "id": "c2",
        "crop": {
            "left": {
                "keys": [
                    {"frame": 0, "value": 1.0, "interp": "linear"},
                    {"frame": 20, "value": 0.0, "interp": "linear"},
                ]
            },
            "right": 0.0,
            "top": 0.1,
            "bottom": 0.0,
            "softness": 0.0,
        },
        "composite": "screen",
    }
    assert edits.set_clip_keys("t1", "c2", "crop.softness", 0.25)["path"] == "crop.softness"


def test_the_size_and_the_place_of_a_title_are_numbers_or_keys() -> None:
    # A title of plain numbers is written as it was before its style took keys, to the letter:
    # a whole size without a fraction, whichever way it was given.
    plain = '{"font":"","size":72,"color":"#ffffffff","x":0.5,"y":0.5,"align":"center"}'
    assert TitleStyle().model_dump_json() == plain
    assert TitleStyle.model_validate_json(plain).model_dump_json() == plain
    assert TitleStyle(size=72.0).model_dump_json() == plain
    assert type(TitleStyle(size=72.0).size) is int
    # A size between two whole numbers is one too, as a key put between two keys leaves it.
    assert TitleStyle(size=72.5).model_dump(mode="json")["size"] == 72.5
    # Each of the three is a number or keys, as the settings of a transform are.
    grows = Keys(keys=[Key(frame=0, value=48), Key(frame=24, value=96.5, interp="hold")])
    rolls = Keys(keys=[Key(frame=0, value=1.25), Key(frame=49, value=-0.25)])
    style = TitleStyle(size=grows, x=0.25, y=rolls, align="left")
    written = {
        "font": "",
        "size": {
            "keys": [
                {"frame": 0, "value": 48.0, "interp": "linear"},
                {"frame": 24, "value": 96.5, "interp": "hold"},
            ]
        },
        "color": "#ffffffff",
        "x": 0.25,
        "y": {
            "keys": [
                {"frame": 0, "value": 1.25, "interp": "linear"},
                {"frame": 49, "value": -0.25, "interp": "linear"},
            ]
        },
        "align": "left",
    }
    assert style.model_dump(mode="json") == written
    assert TitleStyle.model_validate(written) == style
    assert TitleStyle.model_validate_json(style.model_dump_json()) == style
    # The style is set whole by set_clip with the source, keys and all.
    clip = edits.set_clip("t1", "c2", source=of_title("Hi", 1, style))
    assert clip["source"] == {"kind": "title", "text": "Hi", "style": written}
    # And one setting of it by its path.
    assert edits.set_clip_keys("t1", "c2", "title.size", grows) == {
        "op": "set_clip_keys",
        "timeline": "t1",
        "id": "c2",
        "path": "title.size",
        "param": written["size"],
    }
    assert edits.splice_clip_keys("t1", "c2", "title.y", 0, 0, rolls.keys)["path"] == "title.y"
    # A size that the engine would refuse, as a number or at a key, is refused here.
    for wrong in (
        lambda: TitleStyle(size=0.5),
        lambda: TitleStyle(size=float("inf")),
        lambda: TitleStyle(size=float("nan")),
        lambda: TitleStyle(size=Keys(keys=[Key(frame=0, value=48), Key(frame=9, value=0.5)])),
    ):
        with pytest.raises(ValidationError):
            wrong()


def test_a_retime_process_is_written_as_the_engine_reads_it() -> None:
    blend = edits.set_clip("t1", "c2", retime_process="frame_blend")

    assert blend == {
        "op": "set_clip",
        "timeline": "t1",
        "id": "c2",
        "retime_process": "frame_blend",
    }
    # Nearest is set by its name, and a clip that is not asked about it is told nothing of it.
    assert edits.set_clip("t1", "c2", retime_process="nearest")["retime_process"] == "nearest"
    assert "retime_process" not in edits.set_clip("t1", "c2", composite="screen")
    # So is Optical Flow, the third process of DaVinci's menu.
    flow = edits.set_clip("t1", "c2", retime_process="optical_flow")
    assert flow["retime_process"] == "optical_flow"
    assert get_args(edits.RetimeProcess) == ("nearest", "frame_blend", "optical_flow")


async def test_the_retime_process_of_a_clip_is_kept_where_it_mixes_two_pictures(
    tmp_path: Path,
) -> None:
    def clip(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]

    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_color("#000000ff", 50), 0)])
            id = clip(fake)["id"]
            assert "retime_process" not in clip(fake)

            await one.apply([edits.set_clip("t1", id, retime_process="frame_blend")])
            assert clip(fake)["retime_process"] == "frame_blend"
            # Another setting leaves it as it is.
            await one.apply([edits.set_clip("t1", id, fade_in=5)])
            assert clip(fake)["retime_process"] == "frame_blend"
            # Optical Flow is kept by its name in its place.
            await one.apply([edits.set_clip("t1", id, retime_process="optical_flow")])
            assert clip(fake)["retime_process"] == "optical_flow"
            # A clip that shows the nearest picture is written without it, as it always was.
            await one.apply([edits.set_clip("t1", id, retime_process="nearest")])

        assert "retime_process" not in clip(fake)


def test_the_edits_of_speed_points_are_written_with_the_frame_of_the_clip() -> None:
    assert edits.add_speed_point("t1", "c2", 12) == {
        "edit": "add_speed_point", "timeline": "t1", "clip": "c2", "at": 12, "linked": True,
    }  # fmt: skip
    assert edits.set_segment_speed("t1", "c2", 12, Speed.of(Fraction(1, 2)), ripple=True) == {
        "edit": "set_segment_speed", "timeline": "t1", "clip": "c2", "at": 12,
        "speed": {"num": 1, "den": 2}, "ripple": True, "linked": True,
    }  # fmt: skip
    assert edits.freeze_frame("t1", "c2", 12, 25, linked=False) == {
        "edit": "freeze_frame", "timeline": "t1", "clip": "c2", "at": 12, "hold": 25,
        "ripple": False, "linked": False,
    }  # fmt: skip
    assert edits.reverse_segment("t1", "c2", 0) == {
        "edit": "reverse_segment", "timeline": "t1", "clip": "c2", "at": 0, "linked": True,
    }  # fmt: skip
    assert edits.clear_speed_point("t1", "c2", 30, linked=False) == {
        "edit": "clear_speed_point", "timeline": "t1", "clip": "c2", "at": 30, "linked": False,
    }  # fmt: skip
    assert edits.reset_speed("t1", "c2", ripple=True) == {
        "edit": "reset_speed", "timeline": "t1", "clip": "c2", "ripple": True, "linked": True,
    }  # fmt: skip


def test_the_speed_points_of_a_clip_are_set_whole_or_taken_away() -> None:
    points = Retime(
        points=[
            SpeedPoint(frame=0),
            SpeedPoint(frame=4, speed=SignedSpeed(num=0, den=1)),
            SpeedPoint(frame=7, speed=SignedSpeed.of(Fraction(-1, 2))),
        ]
    )

    assert edits.set_retime("t1", "c2", points, length=10) == {
        "op": "set_retime",
        "timeline": "t1",
        "id": "c2",
        "retime": {
            "points": [
                {"frame": 0, "speed": {"num": 1, "den": 1}},
                {"frame": 4, "speed": {"num": 0, "den": 1}},
                {"frame": 7, "speed": {"num": -1, "den": 2}},
            ]
        },
        "length": 10,
    }
    # The speed of the source, forward, is written as null, and the length is left out.
    assert edits.set_retime("t1", "c2", None) == {
        "op": "set_retime", "timeline": "t1", "id": "c2", "retime": None,
    }  # fmt: skip
    assert SignedSpeed.of(0) == SignedSpeed(num=0, den=1)
    assert SignedSpeed.of(Fraction(6, -4)) == SignedSpeed(num=-3, den=2)


async def test_the_edits_of_speed_points_change_the_speed_points_of_a_clip(tmp_path: Path) -> None:
    def clip(fake: fake_engine.Fake) -> Any:
        return fake.project["timelines"][0]["tracks"][0]["clips"][0]

    def points(fake: fake_engine.Fake) -> list[tuple[int, int, int]]:
        written = clip(fake)["retime"]["points"]
        return [(one["frame"], one["speed"]["num"], one["speed"]["den"]) for one in written]

    async with served(tmp_path / "film") as fake:
        async with await Engine.attach(tmp_path / "film", program=fake_engine.__file__) as one:
            await one.apply(
                [edits.add_timeline("Cut", Rate(num=25, den=1), tracks=[edits.track("video")])]
            )
            await one.apply([edits.insert("t1", of_media("m1", start=FROM, length=10), 0)])
            id = clip(fake)["id"]

            # The numbers of the tests of retime.rs: a frame held for 3 frames more,
            # a_freeze_holds_a_picture_and_the_clip_plays_on_from_it.
            applied = await one.apply([edits.freeze_frame("t1", id, 4, 3)])
            assert [done["op"] for done in applied.done] == ["set_retime"]
            assert points(fake) == [(0, 1, 1), (4, 0, 1), (7, 1, 1)]
            assert clip(fake)["length"] == 13
            # The last segment turned round, and then played twice as fast over what it showed.
            await one.apply([edits.reverse_segment("t1", id, 9)])
            assert points(fake) == [(0, 1, 1), (4, 0, 1), (7, -1, 1)]
            await one.apply([edits.set_segment_speed("t1", id, 9, Speed.of(2))])
            assert points(fake) == [(0, 1, 1), (4, 0, 1), (7, -2, 1)]
            assert clip(fake)["length"] == 10
            # A speed point where one is changes nothing, and one elsewhere splits its segment.
            again, split = edits.add_speed_point("t1", id, 4), edits.add_speed_point("t1", id, 2)
            await one.apply([again, split])
            assert points(fake) == [(0, 1, 1), (2, 1, 1), (4, 0, 1), (7, -2, 1)]
            # Cleared, the freeze is shared out over the segment before it:
            # a_speed_point_that_is_cleared_leaves_a_straight_line_between_its_neighbours.
            await one.apply([edits.clear_speed_point("t1", id, 5)])
            assert points(fake) == [(0, 1, 1), (2, 2, 5), (7, -2, 1)]
            # At its own speed it shows once the 6 frames that its path went through.
            await one.apply([edits.reset_speed("t1", id)])
            assert "retime" not in clip(fake)
            assert clip(fake)["length"] == 6
            # The operation sets them whole.
            turned = SpeedPoint(frame=3, speed=SignedSpeed.of(-1))
            whole = Retime(points=[SpeedPoint(frame=0), turned])
            await one.apply([edits.set_retime("t1", id, whole)])
            assert points(fake) == [(0, 1, 1), (3, -1, 1)]

        assert [one["edit"] for one in fake.edits[1:]] == [
            "freeze_frame", "reverse_segment", "set_segment_speed", "add_speed_point",
            "add_speed_point", "clear_speed_point", "reset_speed",
        ]  # fmt: skip
