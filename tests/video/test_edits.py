"""Edits and operations are written as the engine reads them."""

from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest
from pydantic import ValidationError

from columbina.video import edits
from columbina.video.edits import (
    LEFT_OUT,
    CueItem,
    Key,
    Keys,
    Rate,
    Speed,
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
