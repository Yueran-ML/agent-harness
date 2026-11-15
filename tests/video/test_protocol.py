"""The Python client, its fake engine and the tools say what ``engine/PROTOCOL.md`` says (T17).

Every method that :class:`Engine` calls, every notification that it reads and every method that the
fake engine answers is one of the protocol; the fake engine answers every method that the client
calls, but those that are tested against the engine only; every edit and operation that
:mod:`columbina.video.edits` writes is one of the protocol; and every operation that names a
timeline is one whose timeline ``video_edit`` fills (``_TIMELINE_OPERATIONS``), or one that the
agent is given an edit for.
"""

import inspect
import re
from pathlib import Path

from columbina.video import editing, edits
from columbina.video import engine as client
from support import fake_engine

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = (ROOT / "engine" / "PROTOCOL.md").read_text(encoding="utf-8")


def _cells(line: str) -> list[str] | None:
    row = line.strip()
    if not (row.startswith("|") and row.endswith("|")):
        return None
    return [cell.strip() for cell in re.split(r"(?<!\\)\|", row[1:-1])]


def _rows(header: list[str]) -> dict[str, str]:
    """The first column of the tables whose header is ``header``: each name, and its second cell
    (a row whose second cell is ``same`` takes that of the row above it)."""
    rows: dict[str, str] = {}
    lines = PROTOCOL.splitlines()
    at = 0
    while at < len(lines):
        head = _cells(lines[at])
        at += 1
        if head != header:
            continue
        at += 1
        before = ""
        while at < len(lines) and (row := _cells(lines[at])) is not None:
            second = before if row[1] == "same" else row[1]
            for name in re.findall(r"`([^`]+)`", row[0]):
                rows[name] = second
            before = second
            at += 1
    return rows


METHODS = {name for name in _rows(["method", "params", "result"]) if not name.endswith(".*")}
NOTIFICATIONS = set(_rows(["method", "params"]))
OPERATIONS = _rows(["op", "fields"]) | _rows(["op", "fields", "inverse"])
_EDITS_OF_TIMELINES = _rows(["edit", "fields besides timeline", "behavior"])
EDITS = _EDITS_OF_TIMELINES | _rows(["edit", "fields", "behavior"])

ONLY_WITH_THE_ENGINE = {
    "gallery.album.create", "gallery.album.delete", "gallery.album.rename", "gallery.list",
    "gallery.thumbnail", "grade.append_node", "grade.export_lut", "still.apply", "still.copy",
    "still.delete", "still.export", "still.get", "still.grab", "still.import", "still.label",
    "still.move", "still.restore",
}  # fmt: skip
"""Methods of the client that the fake engine does not answer: the Gallery is tested against the
engine in ``tests/video/test_stills.py``, Generate LUT in ``tests/video/test_engine.py``
(``COLUMBINA_REQUIRE_ENGINE=1``)."""

LEFT_TO_EDITS = {
    "add_cue", "add_transition", "move_clip", "remove_cue", "remove_transition", "set_clip_group",
    "set_cue", "set_transition", "shift_clips", "shift_cues", "trim_clip",
}  # fmt: skip
"""Operations that name a timeline whose timeline ``video_edit`` does not fill: the agent is told
the edits of the same names, or the edit commands that ripple, move and trim (which the engine
works out into these, by its rules of locks, links and transitions); ``set_clip_group`` is of
the grades of the Color page, whose tools write it with its timeline."""


def _names_in(source: str, pattern: str) -> set[str]:
    return set(re.findall(pattern, source, flags=re.DOTALL))


def _called_by_the_client() -> set[str]:
    source = inspect.getsource(client)
    return _names_in(
        source, r'(?:_call\(|_sent\(\s*\w+,\s*|_asked_on\(\s*\w+,\s*|method\s*=\s*)\s*"([a-z_.]+)"'
    )


def _answered_by_the_fake() -> set[str]:
    """The methods that the fake answers: those that it has a method of the name of, with
    ``.`` written ``_``, that takes the connection that asks and the params."""
    answered: set[str] = set()
    for name, found in inspect.getmembers(fake_engine.Fake, inspect.isfunction):
        taken = list(inspect.signature(found).parameters)[1:]
        if taken in (["asking", "params"], ["_asking", "params"]):
            answered.add(name)
    return answered


def test_the_tables_of_the_protocol_are_read() -> None:
    assert {"hello", "render.start", "automation.record", "transport.scrub_audio"} <= METHODS
    assert {"project.changed", "render.progress", "automation.written"} <= NOTIFICATIONS
    assert "`timeline`" in OPERATIONS["set_track_audio"]
    assert "`timeline`" not in OPERATIONS["set_media_streams"]
    assert {"add_serial", "lane_points", "set_track_audio", "overwrite"} <= set(EDITS)
    assert "`source`" in EDITS["overwrite"], "a row that takes the same as the one above it"


def test_the_client_calls_methods_and_reads_notifications_of_the_protocol() -> None:
    called = _called_by_the_client()
    assert {"project.get", "render.start", "render.cancel", "still.grab"} <= called
    assert called - METHODS == set()
    # Only the notification dispatcher receives a bare `method`. The same name in
    # analyze_sound is a requested operation, not an incoming notification.
    dispatcher = inspect.getsource(client.Engine._told)  # pyright: ignore[reportPrivateUsage]
    read = _names_in(dispatcher, r'(?<![.\w])method == "([a-z_.]+)"')
    read |= _names_in(inspect.getsource(client), r'told\.method (?:==|!=) "([a-z_.]+)"')
    assert {"engine.yielding", "project.changed"} <= read
    assert {"audio.analysis", "render.progress"} <= read
    assert read - NOTIFICATIONS == set()
    assert {"render.progress"} <= _names_in(inspect.getsource(client), r'"(render\.progress)"')


def test_the_fake_engine_answers_methods_of_the_protocol_and_every_one_the_client_calls() -> None:
    answered = _answered_by_the_fake()
    of_the_protocol = {method for method in METHODS if method.replace(".", "_") in answered}
    assert answered == {method.replace(".", "_") for method in of_the_protocol}
    assert {"hello", "transaction.apply", "render.start"} <= of_the_protocol
    assert _called_by_the_client() - of_the_protocol == ONLY_WITH_THE_ENGINE
    assert ONLY_WITH_THE_ENGINE <= METHODS


def test_the_fake_engine_tells_and_knows_only_what_the_protocol_has() -> None:
    source = inspect.getsource(fake_engine)
    told = _names_in(source, r'tell\(\s*"([a-z_.]+)"')
    told |= _names_in(source, r'_told\(.*?"(project\.changed)"')
    assert {"render.progress"} <= told
    assert told - NOTIFICATIONS == set()
    known = _names_in(source, r'(?:name|op\["op"\]) == "([a-z_]+)"') | set(fake_engine._KINDS)  # pyright: ignore[reportPrivateUsage]
    assert {"set_track_audio", "set_lane", "ripple_delete_range"} <= known
    assert known - set(OPERATIONS) - set(EDITS) == set()


def test_every_edit_and_operation_that_is_written_here_is_of_the_protocol() -> None:
    source = inspect.getsource(edits)
    written_edits = _names_in(source, r'"edit": "([a-z_]+)"')
    written_edits |= _names_in(source, r'_placed\(\s*"([a-z_]+)"')
    written_ops = _names_in(source, r'"op": "([a-z_]+)"')
    assert {"set_track_audio", "lane_points", "insert", "lift"} <= written_edits
    assert {"add_bus", "add_insert", "set_clip"} <= written_ops
    assert written_edits - set(EDITS) == set()
    assert written_ops - set(OPERATIONS) == set()


def test_the_timeline_of_every_operation_that_names_one_is_filled_or_left_to_an_edit() -> None:
    timelines = {op for op, fields in OPERATIONS.items() if "`timeline`" in fields}
    # A timeline added names its own (`add_timeline`), not one that it is in.
    timelines.discard("add_timeline")
    filled = editing._TIMELINE_OPERATIONS  # pyright: ignore[reportPrivateUsage]
    assert filled <= timelines
    assert timelines - filled == LEFT_TO_EDITS


def test_the_edits_of_speed_points_are_of_the_protocol_and_known_to_the_fake_engine() -> None:
    retiming = fake_engine._RETIMING  # pyright: ignore[reportPrivateUsage]
    assert retiming == {
        "add_speed_point", "set_segment_speed", "freeze_frame", "reverse_segment",
        "clear_speed_point", "reset_speed", "set_speed_points",
    }  # fmt: skip
    assert retiming <= set(EDITS)
    assert retiming <= fake_engine._EDITS  # pyright: ignore[reportPrivateUsage]
    assert "set_retime" in OPERATIONS
    assert "set_retime" in editing._TIMELINE_OPERATIONS  # pyright: ignore[reportPrivateUsage]
    assert retiming | {"set_retime"} <= set(edits.__all__)
    # Every field that a builder writes is one that the row of its edit names.
    half = edits.Speed(num=1, den=2)
    points = edits.Retime(points=[edits.SpeedPoint(frame=0)])
    written = [
        edits.add_speed_point("t1", "c1", 1),
        edits.set_segment_speed("t1", "c1", 1, half, ripple=True),
        edits.freeze_frame("t1", "c1", 1, 2, ripple=True),
        edits.reverse_segment("t1", "c1", 1),
        edits.clear_speed_point("t1", "c1", 1),
        edits.reset_speed("t1", "c1", ripple=True),
        edits.set_speed_points("t1", "c1", points, keep="end"),
        edits.set_retime("t1", "c1", points, length=5),
    ]
    wrong: list[str] = []
    for one in written:
        name = one.get("edit") or one["op"]
        row = (EDITS if "edit" in one else OPERATIONS)[name]
        taken = _fields_of(row) | {"edit", "op", "timeline"}
        wrong += [f"{name}: {key}" for key in one if key not in taken]
    assert wrong == []


def _fields_of(row: str) -> set[str]:
    """The fields that a row of a table names: `` `index?` ``, `` `action: add|remove` ``."""
    return {re.split(r"[?: ]", quote, maxsplit=1)[0] for quote in re.findall(r"`([^`]+)`", row)}


def test_every_field_that_a_builder_of_sound_writes_is_one_that_the_engine_takes() -> None:
    # The rows of PROTOCOL.md name every field that serde takes (the engine's protocol tests).
    at = edits.Time(ticks=48000, scale=48000)
    track, bus = {"track": "A1"}, {"bus": "B1"}
    written = [
        edits.set_track_audio("t1", "A1", {}, follow_groups=False),
        edits.add_bus("t1", {}, index=0),
        edits.set_bus("t1", "B1", {}),
        edits.remove_bus("t1", "B1"),
        edits.add_vca("t1", {}, index=0),
        edits.set_vca("t1", "VCA1", {}),
        edits.remove_vca("t1", "VCA1"),
        edits.add_track_group("t1", {}, index=0),
        edits.set_track_group("t1", "TG1", {}),
        edits.remove_track_group("t1", "TG1"),
        edits.set_timeline_audio("t1", {}),
        edits.add_insert("t1", track, {}, index=0),
        edits.set_insert("t1", track, 0, None),
        edits.move_insert("t1", track, 0, 1, to_of=bus),
        edits.copy_effects("t1", track, [bus]),
        edits.clip_gain_keys(
            "t1", "c1", "move", at, param="pan", value=1.0, to=at, interp="linear"
        ),
        edits.clip_gain_range("t1", "c1", at, at, 1.0),
        edits.nudge_clip_gain("t1", ["c1"], 1.0),
        edits.paste_attributes("t1", "c1", ["c2"], ["volume"]),
        edits.remove_attributes("t1", ["c1"], ["volume"]),
        edits.fade_to_playhead("t1", ["c1"], "in", 1),
        edits.set_lane("t1", track, "fader_db", {"points": []}),
        edits.lane_points("t1", track, "fader_db", "add", points=[(at, 0.0)], delta=1.0),
        edits.lane_points("t1", track, "fader_db", "move", at=[at], by=at),
        edits.lane_range(
            "t1", track, "fader_db", at, at, "paste", points=[(at, 0.0)], delta=1.0, tolerance=0.5
        ),
        edits.lane_draw("t1", track, "fader_db", at, at, [(at, 0.0)], tolerance=0.5),
        edits.clear_lanes("t1", track, paths=["fader_db"], then="hold", at=at),
        edits.set_track(
            "t1",
            "A1",
            name="a",
            enabled=True,
            locked=False,
            language="en",
            mute=True,
            solo=True,
            solo_safe=True,
        ),
        edits.set_transition(
            "t1",
            "x1",
            kind="crossfade",
            length=2,
            align="center",
            fade_in_level_db=3,
            fade_out_level_db=-3,
        ),
        edits.set_clip(
            "t1",
            "c1",
            enabled=True,
            gain_db=0.0,
            pan=0.0,
            fade_in=1,
            fade_out=1,
            audio_fade_in={"length": at.model_dump()},
            audio_fade_out=None,
            eq={},
            effects=[],
            pitch_correction=True,
            pitch={},
            channels=None,
        ),
    ]
    wrong: list[str] = []
    for one in written:
        name = one.get("edit") or one["op"]
        row = (EDITS if "edit" in one else OPERATIONS)[name]
        taken = _fields_of(row) | {"edit", "op", "timeline"}
        wrong += [f"{name}: {key}" for key in one if key not in taken]
    assert wrong == []
    fields = _fields_of(r"`clips: [id, ...]`, `index?`, `action: add\|remove`")
    assert fields == {"clips", "index", "action"}
