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
"""Methods of the client that the fake engine does not answer: the Gallery and Generate LUT are
tested against the engine (``tests/video/test_stills.py``, ``COLUMBINA_REQUIRE_ENGINE=1``)."""

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
    # What it is told, not what it asked (`asked.method == "project.get"`).
    read = _names_in(inspect.getsource(client), r'(?<![.\w])method == "([a-z_.]+)"')
    assert {"engine.yielding", "project.changed"} <= read
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
