"""An engine of video for the tests: in the process of a test, or as a program of its own.

It speaks protocol 2 of ``engine/PROTOCOL.md`` over a named pipe of Windows or a socket of
Unix, one message a line: ``hello``, ``project.get``, ``transaction.apply`` (with names of the
form ``$name``, and edits beside operations), ``history.undo``, ``history.redo``,
``media.import``, ``timeline.duration``, ``render.start`` (with the ``audio`` of the sound of an
export, or the sound alone with ``video`` ``null``) and ``render.cancel``. Every change is
told to every connection that said hello before it is answered, as the engine does. A test makes
it misbehave through the attributes of :class:`Fake`.

Its project is of schema 2. Of the rules of the engine it has few: an edit is answered with
operations only, as the engine answers it, but they are made up simply. An edit that puts a
source on a timeline becomes one ``insert_clip`` and nothing moves; ``lift`` and
``ripple_delete`` take their clips away; ``ripple_delete_range`` cuts the frames out of every
track that is not locked, as the engine does for clips that play forward; ``add_track`` and
``remove_track`` do what they say; an edit of the Retime Controls (``add_speed_point``,
``set_segment_speed``, ``freeze_frame``, ``reverse_segment``, ``clear_speed_point``,
``reset_speed``) becomes one ``set_retime`` of the clip that it names, its speed points counted
as the engine counts them, and nothing else moves; ``set_speed_points`` becomes one
``set_retime`` of the speed points that it gives; any other edit is answered with a
``set_timeline`` that changes nothing. An edit that names a locked track, or a clip on one, is
refused with ``locked``. A render writes a few bytes where it is told to, and tells its progress
to every connection.

Grades (schema 3) are kept as simply: the operations of grades and groups do what they say, an
edit that adds a corrector becomes one ``add_node`` linked as the engine links it (beside a node,
only its input is linked), a corrector that it adds has no settings but ``lut`` and
``shot_match``, ``copy_grade`` and ``paste_*`` copy, ``reset_node`` takes the settings away,
``collapse_group_grades`` only takes the clips out of their groups, and ``append_node_graph``
changes nothing.

Compositions (the Fusion page) are kept as simply: the operations on the composition of a clip
do what they say, on a clip that has one, with few of the engine's checks of it: a node of any
kind (the masks ``rectangle``, ``ellipse`` and ``polygon`` among them) and a link into any input
(``mask`` among them) are kept as they are given, the links in the engine's order, and a setting
is found by its path in the ``params`` of its node, a point of a polygon by its place in the
list of its points; a link names a node added before in the transaction by its ``$name``. The
numbers of a Planar Tracker and of a Text+, its text and its font, are checked as the engine
checks them (``out_of_range``); a frame of the path of a Planar Tracker, or a part of a Text+,
that is set ``null`` is taken away; and the keys of a setting that is not there are ``missing``.

A track of the patterns of a Tracker of a composition (``track.start`` with ``fusion``) is refused
as the engine refuses one, and is made up: each pattern followed is found a step further to the
right each frame, from where it starts to the end of the clip each way that the track goes; the
paths are written as one change, and ``track.progress`` and ``track.state`` are told to the
connection that started it alone. ``track.stop`` ends it. A window of a grade is not tracked. So
is a track of the region of a Planar Tracker (``track.start`` with ``planar``): the plane is
found a step further to the right each frame, but at the frames of ``planar_lost``, and nothing
is found where ``planar_features`` is false.

The stabilizer of a clip is kept as simply: ``set_stabilize`` puts it whole and
``set_stabilize_settings`` the controls given (with those of a first stabilizer where the clip
has none), refused as the engine refuses a clip of sound or a number out of its range. A measure
of a clip (``track.stabilize``) is refused as the engine refuses one, and is made up: the picture
pans a step further to the right each frame, but at the frames of ``stabilize_lost``; its
analysis is written at the end as one change, and nothing where it is stopped.

In a test, :func:`served` serves a project in the loop of the test. As a program, run it the way
the engine is run, ``python fake_engine.py --project DIR --headless [--idle-seconds N]``, and with
``--mode`` it ends as an engine that cannot start ends instead:

- ``listen``: it takes the project, writes ``engine.lock``, says where it listens, logs a line
  on stderr for each connection, and ends once no client has been connected for the idle time.
- ``held ADDRESS``: another engine has the project open at ``ADDRESS`` (exit code 3).
- ``held-moved ADDRESS``: the same, naming an address where nothing listens, and having
  written ``ADDRESS`` into ``engine.lock``, as an engine that took over meanwhile would.
- ``contended``: another engine is starting (exit code 3). ``contended-once`` says so the first
  time only, and listens the next.
- ``error``: the project cannot be read (exit code 2).
- ``exit``: it ends with exit code 5, logging lines that say nothing of why.
- ``libraries``: it ends as a program of Windows ends whose libraries are not found.
- ``babble``: its first line is not where it listens, and it waits.
- ``elsewhere``: it says that it listens at a pipe or a socket that is not an engine's, and waits.
- ``silent``: it says nothing, and waits.

As the engine, it refuses an idle time that is not a whole number of seconds.

It imports nothing of Columbina.
"""

import argparse
import asyncio
import base64
import contextlib
import copy
import json
import os
import struct
import sys
import time
import zlib
from collections.abc import AsyncGenerator, Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from fractions import Fraction
from pathlib import Path
from typing import Any, cast

__all__ = ["LIMIT", "Fake", "served"]

LIMIT = 16 * 1024 * 1024
"""The longest line that a client reads."""


def argv_of(*mode: str, idle: int | None = None) -> Callable[[str, Path, int | None], list[str]]:
    """What the engine is started with, for this file to be started in its place.

    It is a script, run by the Python of the test, in a mode (``listen`` if none is given), and
    with ``idle`` for its idle time, if given, in place of the one that it is started with.
    """

    def argv(program: str, directory: Path, idle_seconds: int | None) -> list[str]:
        seconds = idle if idle is not None else idle_seconds
        idling = ["--idle-seconds", str(seconds)] if seconds is not None else []
        return [sys.executable, __file__, "--project", str(directory), "--headless", *idling,
                "--mode", *(mode or ("listen",))]  # fmt: skip

    return argv


def address_of() -> str:
    """A new address of the form of an engine's, that nothing listens at yet."""
    digits = os.urandom(8).hex()
    if sys.platform == "win32":
        return rf"\\.\pipe\columbina-engine-{digits}"
    return str(_sockets() / f"{digits}.sock")


def _sockets() -> Path:
    """The directory where the engine makes its sockets, made as the engine makes it."""
    assert sys.platform != "win32"
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime and Path(runtime).is_absolute():
        made = Path(runtime) / "columbina"
    else:
        temporary = os.environ.get("TMPDIR")
        base = Path(temporary) if temporary and Path(temporary).is_absolute() else Path("/tmp")
        made = base / f"columbina-{os.geteuid()}"
    made.mkdir(mode=0o700, exist_ok=True)
    return made


class RefusedError(Exception):
    """An answer of JSON-RPC that is an error."""

    def __init__(self, code: int, message: str, data: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.error: dict[str, Any] = {"code": code, "message": message}
        if data is not None:
            self.error["data"] = data


_AUDIO_FIELDS = {"bus", "rate", "codec", "bitrate", "normalize"}
"""What the ``audio`` of ``render.start`` takes (``PROTOCOL.md``, The sound of an export)."""

_KINDS = {"add_media": "media", "add_timeline": "timeline", "add_track": "track", "add_bin": "bin"}
_LETTERS = {"media": "m", "timeline": "t", "clip": "c", "bin": "b"}
_TRACK_LETTERS = {"video": "V", "audio": "A", "subtitle": "S"}
_GRADE_OPS = {
    "add_node",
    "remove_node",
    "connect",
    "disconnect",
    "set_node",
    "set_keys",
    "splice_keys",
    "set_grade",
    "add_group",
    "remove_group",
    "set_group",
    "set_clip_group",
}
_FUSION_OPS = {
    "set_fusion",
    "add_fusion_node",
    "remove_fusion_node",
    "connect_fusion",
    "disconnect_fusion",
    "set_fusion_node",
    "set_fusion_keys",
    "splice_fusion_keys",
}
_STABILIZE_OPS = {"set_stabilize", "set_stabilize_settings"}
_METHODS = ("perspective", "similarity", "translation")
_STABILIZER: dict[str, Any] = {
    "method": "perspective",
    "camera_lock": False,
    "zoom": True,
    "cropping_ratio": 0.5,
    "smooth": 0.25,
    "strength": 1.0,
}
"""The stabilizer that a clip is given at first, as the Inspector shows it."""

_STABILIZE_RANGES = {"cropping_ratio": (0.0, 1.0), "smooth": (0.0, 1.0), "strength": (-1.0, 1.0)}
"""The bounds of the numbers of a stabilizer."""

_GRADE_EDITS = {
    "append_node",
    "add_serial",
    "add_serial_before",
    "add_parallel",
    "add_layer",
    "add_outside",
    "extract_node",
    "reset_node",
    "copy_grade",
    "append_node_graph",
    "append_grade",
    "collapse_group_grades",
    "paste_value",
    "paste_node",
}
_PLACING = {
    "insert",
    "overwrite",
    "replace",
    "smart_insert",
    "ripple_overwrite",
    "place_on_top",
    "append",
}
_RETIMING = {
    "add_speed_point",
    "set_segment_speed",
    "freeze_frame",
    "reverse_segment",
    "clear_speed_point",
    "reset_speed",
    "set_speed_points",
}
"""The edits of the Retime Controls, which change the speed points of a clip."""

_BACKWARD = {
    "forward": "reverse",
    "reverse": "forward",
    "one_forward": "one_reverse",
    "one_reverse": "one_forward",
}
"""The way that a track goes in the source of a clip that plays it backward."""

_WINDOW = {"grade", "node", "window", "tracker", "components"}
"""What ``track.start`` takes of a track of a window of a grade."""

_EDITS = (
    _PLACING
    | _RETIMING
    | {
        "lift",
        "ripple_delete",
        "ripple_delete_range",
        "close_gap",
        "blade",
        "move",
        "insert_move",
        "trim",
        "roll",
        "slip",
        "slide",
        "add_transition",
        "set_transition",
        "remove_transition",
        "set_speed",
        "add_cue",
        "set_cue",
        "remove_cue",
        "cues_from",
        "add_track",
        "remove_track",
    }
)


class Fake:
    """The engine: one project, its connections, and what a test makes it do.

    Attributes:
        refusals: The error that a method is answered with, instead of its result, as it is
            written into the answer.
        results: The result that a method is answered with, instead of its own.
        drop: A method at which the connection is closed instead of answered, once.
        huge: A method that is answered once with a line of ``huge_bytes``.
        anonymous: A method that is answered once with an error whose id is null, as the
            engine answers a line that it cannot read.
        then: What is done once right after a method is answered.
        gap: Whether the next change is told as if a revision had been missed.
        hold: How many of the next answers are held back, and sent the other way round.
        noise: Lines sent to a connection after its answer to hello.
        project_dir: The directory that hello names, if not the one of the project.
        headless: Whether hello says that it is headless; nothing, if ``None``.
        asked: Every method that was asked, in order.
        edits: Every edit of every transaction that was applied, as it was sent.
        clamped: The places that the next transaction is answered as clamped at.
        render_frames: How many frames a render writes, if not as many as its timeline lasts.
        render_end: How a render ends, unless it is cancelled: ``done`` or ``failed``.
        render_hold: Whether a render holds on after it begins, until it is cancelled.
        jobs: The state of each render, of each analysis of sound and of each track, by its job.
        sounds: What each ``audio.analyze``, ``audio.normalize`` and ``audio.level_match`` was
            asked, in order.
        sound_end: How an analysis of sound ends, unless it is cancelled: ``done`` or ``failed``.
        sound_hold: Whether an analysis of sound holds on after it begins, until it is cancelled.
        sound_found: What an analysis of sound that is done tells besides its job and state.
        track_starts: What each ``track.start`` was asked, in order.
        track_plans: What each track that began follows, and what it found, by its job.
        track_hold: Whether a track goes one frame each way and holds on there, until it is
            stopped.
        track_step: How much further to the right a pattern is found each frame from the one
            that it starts from, in widths of the picture.
        track_lost: The frame of the clip's keys at which a pattern is lost, by its place.
        planar_lost: The frames of the clip's keys at which the plane of a Planar Tracker is
            lost, and not written.
        planar_features: Whether the region of a Planar Tracker holds anything to follow: a
            track of one that does not fails with ``no_features``.
        stabilize_starts: What each ``track.stabilize`` was asked, in order.
        stabilize_lost: The frames of a clip's source at which a measure loses the picture.
        rendered: What each ``render.start`` was asked, in order.
        looked: What each ``scopes.statistics``, ``timeline.thumbnail`` and ``luts.list`` was
            asked, in order.
        transactions: The operations and edits of every ``transaction.apply``, as they were sent.
        gestures: The ``gesture`` of every ``transaction.apply``, or ``None``, in order.
        measure: What ``scopes.statistics`` of a frame drawn off screen answers, from its
            params and the project; a grey frame unless a test says.
    """

    def __init__(self, directory: Path, address: str) -> None:
        self.directory = directory
        self.address = address
        self.revision = 0
        self.project: dict[str, Any] = {
            "schema": 2,
            "next_id": 1,
            "bins": [],
            "media": [],
            "timelines": [],
        }
        self.steps: list[Step] = []
        self.undone: list[Step] = []
        self.connections: list[Connection] = []
        self.refusals: dict[str, dict[str, Any]] = {}
        self.results: dict[str, Any] = {}
        self.drop: str | None = None
        self.huge: str | None = None
        self.huge_bytes = LIMIT + 1
        self.anonymous: str | None = None
        self.then: dict[str, Callable[[], None]] = {}
        self.gap = False
        self.hold = 0
        self.noise: list[bytes] = []
        self.project_dir: str | None = None
        self.headless: bool | None = True
        self.asked: list[str] = []
        self.edits: list[dict[str, Any]] = []
        self.clamped: list[int] = []
        self.render_frames: int | None = None
        self.render_end = "done"
        self.render_hold = False
        self.jobs: dict[str, str] = {}
        self.sounds: list[tuple[str, dict[str, Any]]] = []
        self.sound_end = "done"
        self.sound_hold = False
        self.sound_found: dict[str, Any] = {}
        self.track_starts: list[dict[str, Any]] = []
        self.track_plans: dict[str, dict[str, Any]] = {}
        self.track_hold = False
        self.track_step = 0.001
        self.track_lost: dict[int, int] = {}
        self.planar_lost: set[int] = set()
        self.planar_features = True
        self.stabilize_starts: list[dict[str, Any]] = []
        self.stabilize_lost: set[int] = set()
        self.standards: list[dict[str, Any]] = []
        self.rendered: list[dict[str, Any]] = []
        self.looked: list[tuple[str, dict[str, Any]]] = []
        self.transactions: list[list[dict[str, Any]]] = []
        self.gestures: list[dict[str, Any] | None] = []
        self.gesture: tuple[str, int] | None = None
        self.measure: Callable[[dict[str, Any], dict[str, Any]], dict[str, Any]] = (
            lambda params, project: grey_frame(int(params.get("bins", 256)))
        )
        self.last_left = time.monotonic()
        self.log: Callable[[dict[str, Any]], None] = lambda line: None

    @property
    def greeted(self) -> list["Connection"]:
        """The connections that said hello."""
        return [one for one in self.connections if one.client is not None]

    def tell(self, method: str, params: dict[str, Any]) -> None:
        """Send a notification to every connection that said hello."""
        for one in self.greeted:
            one.send({"jsonrpc": "2.0", "method": method, "params": params})

    def say(self, *lines: bytes) -> None:
        """Send lines as they are to every connection that said hello."""
        for one in self.greeted:
            for line in lines:
                cast("asyncio.Transport", one.transport).write(line + b"\n")

    def changed(self, by: str, done: list[dict[str, Any]], before: dict[str, Any]) -> None:
        """Make a revision of a change, and tell it."""
        self.revision += 2 if self.gap else 1
        self.gap = False
        self.steps.append(Step(before, copy.deepcopy(self.project), done, by))
        self.undone.clear()
        self._told(by, by, done)

    def change_by(self, client: str) -> None:
        """Add a timeline, as another client would."""
        before = copy.deepcopy(self.project)
        timeline: dict[str, Any] = {
            "id": "",
            "name": "Other",
            "rate": {"num": 25, "den": 1},
            "width": 1920,
            "height": 1080,
            "sample_rate": 48000,
            "tracks": [],
        }
        done, _, _ = self._applied([{"op": "add_timeline", "timeline": timeline}])
        self.changed(client, done, before)

    def _told(self, by: str, made_by: str, done: list[dict[str, Any]]) -> None:
        self.tell(
            "project.changed",
            {
                "revision": self.revision,
                "by": by,
                "made_by": made_by,
                "done": done,
                "next_id": self.project["next_id"],
            },
        )

    # The methods

    def hello(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        if asking.client is not None:
            raise RefusedError(-32600, "hello was said before")
        if params.get("protocol") != 2:
            raise RefusedError(
                -32600, "protocol 2 is spoken", {"reason": "protocol", "supported": [2]}
            )
        asking.client = str(params.get("client"))
        said: dict[str, Any] = {
            "engine": "0.1.0-fake",
            "protocol": 2,
            "revision": self.revision,
            "project": self.project_dir or str(self.directory),
            "pid": os.getpid(),
        }
        if self.headless is not None:
            said["headless"] = self.headless
        return said

    def automation_playback(self, _asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """Wire-only session flag. No fake DSP or project edits."""
        self.looked.append(("automation.playback", copy.deepcopy(params)))
        self.automation_playback_on = params["on"]
        return {"on": params["on"]}

    def automation_parameters(
        self, _asking: "Connection", params: dict[str, Any]
    ) -> dict[str, Any]:
        """Tests can inject a model catalogue through Fake.results; no second catalogue."""
        self.looked.append(("automation.parameters", copy.deepcopy(params)))
        return {"revision": self.revision, "of": copy.deepcopy(params["of"]), "parameters": []}

    def project_get(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return {
            "revision": self.revision,
            "next_id": self.project["next_id"],
            "project": copy.deepcopy(self.project),
        }

    def transaction_apply(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        given = cast("dict[str, Any] | None", params.get("gesture"))
        self.gestures.append(copy.deepcopy(given))
        key = None if given is None else f"{id(asking)}:{given['id']}"
        going = self.gesture is not None and key == self.gesture[0]
        if going and self.steps and self.gesture is not None:
            fresh = self.gesture[1] <= params.get("revision", self.revision) <= self.revision
        else:
            fresh = params.get("revision", self.revision) == self.revision
        if not fresh:
            if given is not None and given.get("end"):
                self.gesture = None
            raise RefusedError(-32001, "the project changed", {"reason": "stale_revision"})
        if not going:
            self.gesture = None
        ops = cast("list[dict[str, Any]]", params.get("ops") or [])
        self.transactions.append(copy.deepcopy(ops))
        if not ops:
            message = "the transaction has no operations"
            raise RefusedError(-32001, message, {"reason": "empty", "message": message})
        before = copy.deepcopy(self.project)
        try:
            done, names, clamped = self._applied(copy.deepcopy(ops))
        except RefusedError:
            self.project = before
            if given is not None and given.get("end"):
                self.gesture = None
            raise
        self.edits.extend(copy.deepcopy(op) for op in ops if "edit" in op)
        if going and self.gesture is not None:
            step = self.steps[-1]
            self.revision += 1
            self.steps[-1] = Step(
                step.before, copy.deepcopy(self.project), step.done + done, step.made_by
            )
            self._told(str(asking.client), str(asking.client), done)
        else:
            self.changed(str(asking.client), done, before)
            if key is not None:
                self.gesture = (key, self.revision - 1)
        if given is not None and given.get("end"):
            self.gesture = None
            last = self.steps[-1]
            if _same_project(last.before, self.project):
                self.steps.pop()
        return {
            "revision": self.revision,
            "done": done,
            "next_id": self.project["next_id"],
            "names": names,
            "clamped": clamped,
        }

    def history_undo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        self.gesture = None
        return self._stepped(asking, self.steps, self.undone, back=True)

    def history_redo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        self.gesture = None
        return self._stepped(asking, self.undone, self.steps, back=False)

    def media_import(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        items = self.media_inspect(asking, params)["media"]
        ops: list[dict[str, Any]] = [{"op": "add_media", "media": item} for item in items]
        bin = params.get("bin")
        if bin is not None and not any(one["id"] == bin for one in self.project["bins"]):
            raise RefusedError(-32001, "no such bin", {"reason": "missing"})
        for op in ops:
            op["media"]["bin"] = bin
        before = copy.deepcopy(self.project)
        done, _, _ = self._applied(ops)
        self.changed(str(asking.client), done, before)
        return {
            "revision": self.revision,
            "added": [one["media"]["id"] for one in done],
            "next_id": self.project["next_id"],
        }

    def media_inspect(self, _asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        paths = cast("list[str]", params.get("paths") or [])
        for index, path in enumerate(paths):
            if not Path(path).is_absolute():
                raise RefusedError(
                    -32602,
                    f"media.import takes absolute paths, and {path} is not one",
                    {"index": index, "path": path, "reason": "relative"},
                )
        for path in paths:
            if not Path(path).is_file():
                message = "it cannot be read: no such file"
                raise RefusedError(
                    -32002,
                    f"a file cannot be read: {path}: {message}",
                    {"path": path, "reason": "not_found", "message": message},
                )
        return {"media": [_media(Path(path)) for path in paths]}

    def _stepped(
        self, asking: "Connection", taken: list["Step"], given: list["Step"], *, back: bool
    ) -> dict[str, Any]:
        if not taken:
            return {"revision": self.revision, "done": None}
        step = taken.pop()
        given.append(step)
        next_id = self.project["next_id"]
        self.project = copy.deepcopy(step.before if back else step.after)
        self.project["next_id"] = next_id
        undone = [{"op": "undo", "of": one} for one in reversed(step.done)] if back else step.done
        self.revision += 1
        self._told(str(asking.client), step.made_by, undone)
        return {"revision": self.revision, "done": undone, "next_id": next_id}

    def timeline_duration(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        timeline = self._timeline(str(params.get("timeline")), 0, code=-32602)
        return {"frames": _duration(timeline)}

    def render_start(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        self.rendered.append(copy.deepcopy(params))
        timeline = self._timeline(str(params.get("timeline")), 0, code=-32602)
        if params.get("revision", self.revision) != self.revision:
            told = {"reason": "stale_revision", "expected": params["revision"]}
            raise RefusedError(-32001, "the project changed", {**told, "actual": self.revision})
        path = Path(str(params.get("path")))
        if not path.is_absolute():
            message = f"render.start takes an absolute path, and {path} is not one"
            raise RefusedError(-32602, message, {"path": str(path), "reason": "relative"})
        if path.exists() and not params.get("replace"):
            message = f"{path} is there already"
            raise RefusedError(-32002, message, {"path": str(path), "reason": "exists"})
        if "subtitles" in params:
            raise RefusedError(-32602, "render.start takes no subtitles", {"reason": "params"})
        alone = "video" in params
        if alone and (params["video"] is not None or {"codec", "quality"} & set(params)):
            message = "the sound alone is rendered with video null, and no codec or quality"
            raise RefusedError(-32602, message, {"reason": "params"})
        audio: object = params.get("audio", {})
        if not isinstance(audio, dict) or set(cast("dict[str, Any]", audio)) - _AUDIO_FIELDS:
            message = "render.start: audio is not as it is taken"
            raise RefusedError(-32602, message, {"reason": "params"})
        chosen = cast("list[str]", params.get("subtitle_tracks") or [])
        subtitles = {one["id"] for one in timeline["tracks"] if one["kind"] == "subtitle"}
        for at, track in enumerate(chosen):
            if track not in subtitles or track in chosen[:at]:
                message = f"subtitle_tracks: track {track} is not one subtitle track to write"
                raise RefusedError(-32602, message, {"reason": "subtitle_track", "track": track})
        job = f"r{len(self.rendered)}"
        frames = self.render_frames if self.render_frames is not None else _duration(timeline)
        self.jobs[job] = "running"
        asyncio.get_running_loop().call_soon(self._progress, job, path, frames, 0)
        return {"job": job}

    def scopes_statistics(self, _asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """The figures of a grey frame of 4 x 2, as a monitor of the desktop's engine shows it."""
        self.looked.append(("scopes.statistics", copy.deepcopy(params)))
        if "timeline" in params:
            said = self.measure(copy.deepcopy(params), copy.deepcopy(self.project))
            said.update(timeline=params["timeline"], frame=params["frame"])
            if "node" in params:
                said["node"] = {
                    "grade": params["grade"],
                    "node": params["node"],
                    "at": params.get("at", "output"),
                }
            if "fusion" in params:
                # The picture of one node of a composition, as the Fusion view shows it.
                said["fusion"] = {**params["fusion"], "shown": True}
            return said
        if self.headless is not False:
            raise RefusedError(-32601, "there is no method scopes.statistics in a headless engine")
        bins = params.get("bins", 256)
        grey = 128 / 255
        channel = {"min": grey, "max": grey, "mean": grey}
        level = round(grey * (bins - 1))
        counts = [8 if each == level else 0 for each in range(bins)]
        names = ("red", "green", "blue", "luma")
        return {
            "view": params["view"],
            "frame": 12,
            "width": 4,
            "height": 2,
            "samples": 8,
            **dict.fromkeys(names, channel),
            "chroma": {
                "centroid": [0.0, 0.0],
                "spread": [0.0, 0.0],
                "radius": 0.0,
                "saturation": 0.0,
                "hue": 0.0,
            },
            "percentiles": {name: {"1": grey, "50": grey, "99": grey} for name in names},
            "histogram": {"bins": bins, **dict.fromkeys(names, counts)},
        }

    def timeline_thumbnail(self, _asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """A frame of a timeline as an opaque PNG, every pixel the grey of its number (mod 256).

        It is ``height`` high (90 unless asked, no higher than the timeline) and as wide as the
        timeline's aspect makes it; with ``clip``, only of a clip that is on and covers the frame.
        Refused as the engine refuses it.
        """
        self.looked.append(("timeline.thumbnail", copy.deepcopy(params)))
        named, frame = params.get("timeline"), params.get("frame")
        timeline = next((one for one in self.project["timelines"] if one["id"] == named), None)
        if timeline is None:
            raise RefusedError(-32602, f"there is no timeline {named}", {"reason": "timeline"})
        height = params.get("height", 90)
        if type(height) is not int or not 1 <= height <= 270:
            message = f"a thumbnail is from 1 to 270 pixels high, not {height}"
            raise RefusedError(-32602, message, {"reason": "height"})
        if type(frame) is not int:
            raise RefusedError(-32602, "timeline.thumbnail takes a frame", {"reason": "params"})
        clip = params.get("clip")
        if clip is not None and not any(
            one["id"] == clip
            and one.get("enabled", True)
            and one["start"] <= frame < one["start"] + one["length"]
            for track in timeline["tracks"]
            for one in track.get("clips", [])
        ):
            message = f"clip {clip} is not seen at frame {frame} of the timeline"
            raise RefusedError(-32602, message, {"reason": "clip"})
        high = min(height, timeline["height"])
        wide = max(round(timeline["width"] * high / timeline["height"]), 1)
        png = base64.b64encode(png_of_grey(wide, high, frame % 256)).decode()
        return {"width": wide, "height": high, "png": png, "frame": frame}

    def luts_list(self, _asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """One LUT in the folder of the project, and a library that is not there."""
        self.looked.append(("luts.list", copy.deepcopy(params)))
        folder = params.get("folder")
        if isinstance(folder, str) and ".." in folder.replace("\\", "/").split("/"):
            raise RefusedError(
                -32602,
                f"the folder {folder} is not a path within the folders of LUTs",
                {"reason": "outside_roots", "folder": folder},
            )
        return {
            "roots": [
                {"place": "project", "path": str(self.directory), "exists": True},
                {"place": "library", "path": str(self.directory / "library"), "exists": False},
            ],
            "luts": [
                {
                    "name": "LUTs/film.cube",
                    "root": 0,
                    "size": 120,
                    "modified_us": 1_790_000_000_000_000,
                    "shadowed": False,
                    "kind": "3d",
                    "status": "ok",
                }
            ],
            "truncated": False,
        }

    def render_cancel(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        job = str(params.get("job"))
        if job not in self.jobs:
            message = f"there is no render {job}"
            raise RefusedError(-32602, message, {"reason": "missing", "what": "job", "id": job})
        if self.jobs[job] == "running":
            self.jobs[job] = "cancelled"
            told = {"job": job, "state": "cancelled", "frame": 0, "frames": 0, "fps": 0.0}
            self.tell("render.progress", told)
        return {}

    def audio_standards(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return {"standards": copy.deepcopy(self.standards)}

    def audio_eq_response(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio.eq_response", params)

    def audio_dynamics_curve(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio.dynamics_curve", params)

    def _processor_curve(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        """Use an explicit reply fixture; the fake never implements a second DSP formula."""
        self.sounds.append((method, copy.deepcopy(params)))
        if method not in self.results:
            raise RefusedError(-32000, f"no static curve fixture configured for {method}")
        return copy.deepcopy(self.results[method])

    def audio_fx_meters(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio.fx_meters", params)

    def audio_soft_clip_curve(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio.soft_clip_curve", params)

    def audio_level_profile(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._sound("audio.level_profile", params)

    def audio_noise_learn(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._sound("audio.noise_learn", params)

    def audio_presets_list(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio_presets.list", params)

    def audio_presets_save(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio_presets.save", params)

    def audio_presets_update(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio_presets.update", params)

    def audio_presets_delete(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._processor_curve("audio_presets.delete", params)

    def audio_analyze(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._sound("audio.analyze", params)

    def audio_normalize(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._sound("audio.normalize", params)

    def audio_level_match(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._sound("audio.level_match", params)

    def audio_cancel(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        job = str(params.get("job"))
        if self.jobs.get(job) != "running":
            raise RefusedError(
                -32602, f"there is no analysis {job} running", {"reason": "missing", "job": job}
            )
        self.jobs[job] = "cancelled"
        self.tell("audio.analysis", {"job": job, "state": "cancelled"})
        return {}

    def _sound(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        """Echo analysis requests, including sum targets/candidates; never emulate their DSP."""
        self.sounds.append((method, copy.deepcopy(params)))
        job = f"audio-{len(self.sounds)}"
        self.jobs[job] = "running"
        asyncio.get_running_loop().call_later(0.01, self._measured, job, method)
        return {"job": job}

    def _measured(self, job: str, method: str) -> None:
        if self.jobs.get(job) != "running":
            return
        self.tell("audio.analysis", {"job": job, "state": "running", "done": 1.0, "total": 2.0})
        if self.sound_hold:
            return
        self.jobs[job] = self.sound_end
        told: dict[str, Any] = {"job": job, "state": self.sound_end}
        if self.sound_end == "done":
            told.update(method=method, **self.sound_found)
        else:
            told["message"] = "a clip cannot be read"
        self.tell("audio.analysis", told)

    def track_start(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """Follow the patterns of a Tracker (``fusion``) or the region of a Planar Tracker
        (``planar``), refused as the engine refuses it."""
        self.track_starts.append(copy.deepcopy(params))
        aims = [key for key in ("fusion", "planar") if key in params]
        if len(aims) != 1 or not isinstance(params[aims[0]], dict) or _WINDOW & set(params):
            message = "track.start takes a window, fusion or planar, one of them"
            raise RefusedError(-32602, message, {"reason": "fusion"})
        if any(job.startswith("track-") and now == "running" for job, now in self.jobs.items()):
            raise RefusedError(-32001, "a track is going on", {"reason": "busy"})
        if params.get("revision") != self.revision:
            told = {"reason": "stale_revision", "expected": params.get("revision")}
            raise RefusedError(-32001, "the project changed", {**told, "actual": self.revision})
        if aims == ["planar"]:
            return self._planar_started(params, asking)
        aim = cast("dict[str, Any]", params["fusion"])
        clip, node = self._tracker_of(aim)
        source = clip["source"]
        media = next(
            (one for one in self.project["media"] if one["id"] == source.get("media")), None
        )
        video = None if media is None else media["facts"].get("video")
        if source["kind"] != "media" or video is None or video.get("still"):
            message = "this Tracker cannot be tracked: it has no moving picture"
            raise RefusedError(-32001, message, {"reason": "untrackable"})
        patterns: list[dict[str, Any]] = node["params"].get("trackers", [])
        named = aim.get("trackers")
        followed = [at for at, one in enumerate(patterns) if one.get("enabled", True)]
        if named is not None:
            for at in cast("list[int]", named):
                if at >= len(patterns) or not patterns[at].get("enabled", True):
                    message = f"pattern {at} cannot be followed"
                    raise RefusedError(-32001, message, {"reason": "tracker", "tracker": at})
            followed = sorted(set(cast("list[int]", named)))
        if not followed:
            message = "the Tracker has no pattern to follow"
            raise RefusedError(-32001, message, {"reason": "no_trackers"})
        # The frames of the media that the clip shows, as a clip of one speed shows them: the
        # speed points of a clip are not followed here.
        begins, speed, rate = source["in"], clip["speed"], video["rate"]
        first = begins["ticks"] * rate["num"] // (begins["scale"] * rate["den"])
        last = first + (clip["length"] - 1) * speed["num"] // speed["den"]
        frame = params.get("frame")
        if not isinstance(frame, int) or not first <= frame <= last:
            message = f"frame {frame} is outside the clip, from {first} to {last}"
            raise RefusedError(-32001, message, {"reason": "frame"})
        direction = str(params.get("direction"))
        if clip.get("reverse"):
            direction = _BACKWARD.get(direction, direction)
        starts: dict[int, list[float]] = {}
        for at in followed:
            pattern = patterns[at]
            place = pattern.get("path", {}).get(str(frame))
            starts[at] = place or [pattern["pattern"]["x"], pattern["pattern"]["y"]]
        job = f"track-{len(self.track_starts)}"
        self.jobs[job] = "running"
        self.track_plans[job] = {
            "asking": asking,
            "timeline": aim["timeline"],
            "clip": aim["clip"],
            "node": aim["node"],
            "followed": followed,
            "starts": starts,
            "frame": frame,
            "first": first,
            "last": last,
            "direction": direction,
        }
        asyncio.get_running_loop().call_later(0.01, self._followed, job)
        told = {"job": job, "first": first, "last": last, "frame_size": [1280, 720]}
        return {**told, "direction": direction, "trackers": followed}

    def track_stop(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        job = str(params.get("job"))
        plan = self.track_plans.get(job)
        if plan is None or self.jobs.get(job) != "running" or plan["asking"] is not asking:
            raise RefusedError(-32001, "no such track is going on", {"reason": "job"})
        # It ends between frames, after the answer, as the engine's thread ends it.
        self.jobs[job] = "stopped"
        if "measure" in plan:
            ended = self._measure_ended
        elif "planar" in plan:
            ended = self._planar_ended
        else:
            ended = self._track_ended
        asyncio.get_running_loop().call_soon(ended, job, "stopped", "stop")
        return {}

    def track_stabilize(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        """Measure the motion of the picture of a clip, refused as the engine refuses it."""
        self.stabilize_starts.append(copy.deepcopy(params))
        method = params.get("method")
        if method is not None and method not in _METHODS:
            raise RefusedError(-32602, f"there is no method {method}", {"reason": "method"})
        if any(job.startswith("track-") and now == "running" for job, now in self.jobs.items()):
            raise RefusedError(-32001, "a track is going on", {"reason": "busy"})
        if params.get("revision") != self.revision:
            told = {"reason": "stale_revision", "expected": params.get("revision")}
            raise RefusedError(-32001, "the project changed", {**told, "actual": self.revision})
        found = self._clip_on(params)
        if found is None:
            raise RefusedError(-32001, "there is no such clip", {"reason": "target"})
        track, clip = found
        source = clip["source"]
        media = next(
            (one for one in self.project["media"] if one["id"] == source.get("media")), None
        )
        video = None if media is None else media["facts"].get("video")
        if track["kind"] != "video" or source["kind"] != "media" or not video or video.get("still"):
            message = "this clip cannot be measured: it has no moving picture"
            raise RefusedError(-32001, message, {"reason": "untrackable"})
        # The frames of the media that the clip shows, as a clip of one speed shows them.
        begins, speed, rate = source["in"], clip["speed"], video["rate"]
        first = begins["ticks"] * rate["num"] // (begins["scale"] * rate["den"])
        last = first + (clip["length"] - 1) * speed["num"] // speed["den"]
        measured = method or clip.get("stabilize", _STABILIZER)["method"]
        job = f"track-s{len(self.stabilize_starts)}"
        self.jobs[job] = "running"
        self.track_plans[job] = {
            "asking": asking,
            "timeline": params["timeline"],
            "clip": params["clip"],
            "measure": measured,
            "first": first,
            "last": last,
        }
        asyncio.get_running_loop().call_later(0.01, self._stabilizer_measured, job)
        told: dict[str, Any] = {"job": job, "first": first, "last": last}
        return {**told, "frame_size": [1280, 720], "method": measured}

    def _clip_on(self, aim: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]] | None:
        """The track and the clip that ``timeline`` and ``clip`` name, if there are such."""
        for timeline in self.project["timelines"]:
            if timeline["id"] != aim.get("timeline"):
                continue
            for track in timeline["tracks"]:
                for clip in track.get("clips", []):
                    if clip["id"] == aim.get("clip"):
                        return track, clip
        return None

    def _stabilizer_measured(self, job: str) -> None:
        """Measure a clip, made up, and write its analysis as one change of the project.

        The picture pans ``track_step`` of the width further to the right each frame from the
        first, but at the frames of ``stabilize_lost``, where it is lost and holds the motion of
        the frame before. A measure held (``track_hold``) measures its first frame, and waits
        there to be stopped, writing nothing.
        """
        plan = self.track_plans[job]
        if self.jobs.get(job) != "running":
            return
        first, last = plan["first"], plan["last"]
        frames = list(range(first, last + 1))[: 1 if self.track_hold else None]
        path: dict[str, list[float]] = {}
        pan, lost = 0.0, 0
        for frame in frames:
            if frame in self.stabilize_lost:
                lost += 1
            else:
                pan = round(1280 * self.track_step * (frame - first), 4)
            path[str(frame)] = [pan, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0]
        plan["report"] = {"frames": len(frames), "lost": lost, "frame": frames[-1]}
        progress: dict[str, Any] = {"job": job, "frame": frames[-1], "timeline_frame": None}
        progress.update(done=len(frames), total=last - first + 1, lost=lost)
        self._told_to(plan, "track.progress", {**progress, "revision": self.revision})
        if self.track_hold:
            return
        found = self._clip_on(plan)
        if found is None:
            self._measure_ended(job, "stopped", "target")
            return
        value = copy.deepcopy(found[1].get("stabilize", _STABILIZER))
        value["method"] = plan["measure"]
        value["analysis"] = {"method": plan["measure"], "size": [1280, 720], "path": path}
        op: dict[str, Any] = {"op": "set_stabilize", "timeline": plan["timeline"]}
        op.update(clip=plan["clip"], value=value)
        before = copy.deepcopy(self.project)
        done, _, _ = self._applied([op])
        self.changed(str(cast("Connection", plan["asking"]).client), done, before)
        plan["analysed"] = [first, last]
        self._measure_ended(job, "done", None)

    def _measure_ended(self, job: str, phase: str, reason: str | None) -> None:
        """End a measure: ``track.state`` to the connection that started it, as the engine does."""
        plan = self.track_plans[job]
        self.jobs[job] = phase
        report = plan.get("report") or {"frames": 0, "lost": 0, "frame": plan["first"]}
        said: dict[str, Any] = {
            "job": job,
            "phase": phase,
            "reason": reason,
            **report,
            "revision": self.revision,
            "analysed": plan.get("analysed"),
            "method": plan["measure"],
        }
        self._told_to(plan, "track.state", said)

    def _tracker_of(self, aim: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        """The clip and the Tracker that a track names, refused as the engine refuses them."""
        timelines = [one for one in self.project["timelines"] if one["id"] == aim.get("timeline")]
        for timeline in timelines:
            for track in timeline["tracks"]:
                for clip in track.get("clips", []):
                    if clip["id"] != aim.get("clip"):
                        continue
                    nodes = clip.get("fusion", {}).get("nodes", [])
                    node = next((one for one in nodes if one["id"] == aim.get("node")), None)
                    if node is None:
                        break
                    if node["kind"] != "tracker":
                        data = {"reason": "wrong_node", "node": node["id"], "kind": node["kind"]}
                        raise RefusedError(-32001, f"{node['id']} is not a tracker", data)
                    return clip, node
        message = "the clip's composition has no such node"
        raise RefusedError(-32001, message, {"reason": "target"})

    def _followed(self, job: str) -> None:
        """Write the paths of a track of a Tracker, made up, as one change of the project.

        Each pattern followed is found ``track_step`` further to the right a frame from where
        it starts, on to the end of the clip each way that the track goes, but at the frame of
        ``track_lost``, where it is lost, and beyond it that way. A track held (``track_hold``)
        goes one frame each way, and waits there to be stopped.
        """
        plan = self.track_plans[job]
        if self.jobs.get(job) != "running":
            return
        start, first, last = plan["frame"], plan["first"], plan["last"]
        ways: dict[str, list[tuple[int, int]]] = {
            "forward": [(1, last)],
            "reverse": [(-1, first)],
            "forward_and_back": [(1, last), (-1, first)],
            "one_forward": [(1, min(start + 1, last))],
            "one_reverse": [(-1, max(start - 1, first))],
        }
        followed: list[int] = plan["followed"]
        paths: dict[int, dict[str, list[float]]] = {at: {} for at in followed}
        found = dict.fromkeys(followed, 0)
        lost: dict[int, list[int]] = {at: [] for at in followed}
        stepped, reached = 0, start
        for way, end in ways[plan["direction"]]:
            frames = list(range(start + way, end + way, way))[: 1 if self.track_hold else None]
            for at in followed:
                x, y = plan["starts"][at]
                paths[at][str(start)] = [x, y]
                for frame in frames:
                    if self.track_lost.get(at) == frame:
                        lost[at].append(frame)
                        break
                    paths[at][str(frame)] = [round(x + self.track_step * (frame - start), 7), y]
                    found[at] += 1
            stepped += len(frames)
            reached = frames[-1] if frames else reached
        written = any(found.values())
        if written:
            try:
                self._paths_written(plan, paths)
            except RefusedError:
                self._track_ended(job, "stopped", "target")
                return
        frames_of = sorted(int(frame) for path in paths.values() for frame in path)
        trackers = [{"tracker": at, "frames": found[at], "lost": lost[at]} for at in followed]
        report: dict[str, Any] = {
            "frames": stepped,
            "lost": sum(len(one) for one in lost.values()),
            "frame": reached,
            "tracked": [frames_of[0], frames_of[-1]] if written else None,
            "trackers": trackers,
        }
        plan["report"] = report
        progress = {"job": job, "frame": reached, "timeline_frame": None, "done": stepped}
        following = [at for at in followed if not lost[at]]
        self._told_to(
            plan,
            "track.progress",
            {
                **progress,
                "total": stepped,
                "lost": report["lost"],
                "revision": self.revision,
                "following": following,
            },
        )
        if not self.track_hold:
            self._track_ended(job, "done", None)

    def _paths_written(
        self, plan: dict[str, Any], paths: dict[int, dict[str, list[float]]]
    ) -> None:
        """Write the paths found into the Tracker, its list of patterns set whole, one change."""
        _, node = self._tracker_of(plan)
        trackers = copy.deepcopy(node["params"]["trackers"])
        for at, path in paths.items():
            trackers[at]["path"] = {**trackers[at].get("path", {}), **path}
        op = {
            "op": "set_fusion_node",
            "timeline": plan["timeline"],
            "clip": plan["clip"],
            "id": plan["node"],
            "params": {"trackers": trackers},
        }
        before = copy.deepcopy(self.project)
        done, _, _ = self._applied([op])
        self.changed(str(cast("Connection", plan["asking"]).client), done, before)

    def _track_ended(self, job: str, phase: str, reason: str | None) -> None:
        """End a track: ``track.state``, with what it found, to the connection that started it."""
        plan = self.track_plans[job]
        self.jobs[job] = phase
        report: dict[str, Any] = plan.get("report") or {
            "frames": 0,
            "lost": 0,
            "frame": plan["frame"],
            "tracked": None,
            "trackers": [{"tracker": at, "frames": 0, "lost": []} for at in plan["followed"]],
        }
        said: dict[str, Any] = {
            "job": job,
            "phase": phase,
            "reason": reason,
            **report,
            "revision": self.revision,
        }
        self._told_to(plan, "track.state", said)

    def _planar_started(self, params: dict[str, Any], asking: "Connection") -> dict[str, Any]:
        """Follow the region of a Planar Tracker, refused as the engine refuses it.

        It takes the params first, so that it is not read as a method of the protocol.
        """
        aim = cast("dict[str, Any]", params["planar"])
        clip, node = self._planar_of(aim)
        source = clip["source"]
        media = next(
            (one for one in self.project["media"] if one["id"] == source.get("media")), None
        )
        video = None if media is None else media["facts"].get("video")
        if source["kind"] != "media" or video is None or video.get("still"):
            message = "this Planar Tracker cannot be tracked: it has no moving picture"
            raise RefusedError(-32001, message, {"reason": "untrackable"})
        # The frames of the media that the clip shows, as a clip of one speed shows them.
        begins, speed, rate = source["in"], clip["speed"], video["rate"]
        first = begins["ticks"] * rate["num"] // (begins["scale"] * rate["den"])
        last = first + (clip["length"] - 1) * speed["num"] // speed["den"]
        frame = params.get("frame")
        if not isinstance(frame, int) or not first <= frame <= last:
            message = f"frame {frame} is outside the clip, from {first} to {last}"
            raise RefusedError(-32001, message, {"reason": "frame"})
        if len(node["params"].get("region", [])) < 3:
            message = "the region cannot be followed"
            raise RefusedError(-32001, message, {"reason": "region", "node": node["id"]})
        direction = str(params.get("direction"))
        if clip.get("reverse"):
            direction = _BACKWARD.get(direction, direction)
        job = f"track-p{len(self.track_starts)}"
        self.jobs[job] = "running"
        self.track_plans[job] = {
            "asking": asking,
            "timeline": aim["timeline"],
            "clip": aim["clip"],
            "node": aim["node"],
            "planar": True,
            "frame": frame,
            "first": first,
            "last": last,
            "direction": direction,
        }
        asyncio.get_running_loop().call_later(0.01, self._planar_followed, job)
        told = {"job": job, "first": first, "last": last, "frame_size": [1280, 720]}
        return {**told, "direction": direction}

    def _planar_of(self, aim: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        """The clip and the Planar Tracker that a track names, refused as the engine refuses."""
        found = self._clip_on(aim)
        nodes: list[dict[str, Any]] = (
            [] if found is None else found[1].get("fusion", {}).get("nodes", [])
        )
        node = next((one for one in nodes if one["id"] == aim.get("node")), None)
        if node is None:
            message = "the clip's composition has no such node"
            raise RefusedError(-32001, message, {"reason": "target"})
        if node["kind"] != "planar_tracker":
            data = {"reason": "wrong_node", "node": node["id"], "kind": node["kind"]}
            raise RefusedError(-32001, f"{node['id']} is not a planar tracker", data)
        assert found is not None
        return found[1], node

    def _planar_followed(self, job: str) -> None:
        """Write the path of a track of a Planar Tracker, made up, as one change of the project.

        The plane is found ``track_step`` of the width further to the right each frame from the
        one that it starts from, on to the end of the clip each way that the track goes, but at
        the frames of ``planar_lost``, where it is lost and not written, and the track goes on.
        Where ``planar_features`` is false it fails, and writes nothing. A track held
        (``track_hold``) goes one frame each way, and waits there to be stopped.
        """
        plan = self.track_plans[job]
        if self.jobs.get(job) != "running":
            return
        if not self.planar_features:
            self._planar_ended(job, "failed", "no_features")
            return
        start, first, last = plan["frame"], plan["first"], plan["last"]
        ways: dict[str, list[tuple[int, int]]] = {
            "forward": [(1, last)],
            "reverse": [(-1, first)],
            "forward_and_back": [(1, last), (-1, first)],
            "one_forward": [(1, min(start + 1, last))],
            "one_reverse": [(-1, max(start - 1, first))],
        }
        path: dict[str, list[float]] = {str(start): [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]}
        stepped, reached, lost = 0, start, 0
        for way, end in ways[plan["direction"]]:
            frames = list(range(start + way, end + way, way))[: 1 if self.track_hold else None]
            for frame in frames:
                if frame in self.planar_lost:
                    lost += 1
                    continue
                shift = round(self.track_step * (frame - start), 9)
                path[str(frame)] = [1.0, 0.0, shift, 0.0, 1.0, 0.0, 0.0, 0.0]
            stepped += len(frames)
            reached = frames[-1] if frames else reached
        try:
            written = self._planar_written(plan, path)
        except RefusedError:
            self._planar_ended(job, "stopped", "target")
            return
        frames_of = sorted(int(frame) for frame in path)
        report: dict[str, Any] = {
            "frames": stepped,
            "lost": lost,
            "frame": reached,
            "tracked": [frames_of[0], frames_of[-1]] if written else None,
        }
        plan["report"] = report
        progress: dict[str, Any] = {"job": job, "frame": reached, "timeline_frame": None}
        progress.update(done=stepped, total=stepped, lost=lost, revision=self.revision)
        self._told_to(plan, "track.progress", progress)
        if not self.track_hold:
            self._planar_ended(job, "done", None)

    def _planar_written(self, plan: dict[str, Any], path: dict[str, list[float]]) -> bool:
        """Write the frames of a path that the Planar Tracker has not so, as one change."""
        _, node = self._planar_of(plan)
        had: dict[str, Any] = node["params"].get("path", {})
        patch = {frame: one for frame, one in path.items() if had.get(frame) != one}
        if not patch:
            return False
        op = {
            "op": "set_fusion_node",
            "timeline": plan["timeline"],
            "clip": plan["clip"],
            "id": plan["node"],
            "params": {"path": patch},
        }
        before = copy.deepcopy(self.project)
        done, _, _ = self._applied([op])
        self.changed(str(cast("Connection", plan["asking"]).client), done, before)
        return True

    def _planar_ended(self, job: str, phase: str, reason: str | None) -> None:
        """End a track of a Planar Tracker: ``track.state`` to the connection that started it."""
        plan = self.track_plans[job]
        self.jobs[job] = phase
        report: dict[str, Any] = plan.get("report") or {
            "frames": 0,
            "lost": 0,
            "frame": plan["frame"],
            "tracked": None,
        }
        said = {"job": job, "phase": phase, "reason": reason, **report, "revision": self.revision}
        self._told_to(plan, "track.state", said)

    def _told_to(self, plan: dict[str, Any], method: str, params: dict[str, Any]) -> None:
        """Tell the connection that started a track, and no other, as the engine tells it."""
        asking = cast("Connection", plan["asking"])
        if asking in self.connections:
            asking.send({"jsonrpc": "2.0", "method": method, "params": params})

    def _progress(self, job: str, path: Path, frames: int, frame: int) -> None:
        """Tell how far a render has come, and go on with it a moment later."""
        if self.jobs.get(job) != "running":
            return
        told: dict[str, Any] = {"job": job, "frame": frame, "frames": frames, "fps": 50.0}
        held = self.render_hold and frame == 0
        if frame < frames or held:
            self.tell("render.progress", {**told, "state": "running"})
            if not held:
                later = min(frames, frame + max(1, frames // 2))
                asyncio.get_running_loop().call_later(
                    0.01, self._progress, job, path, frames, later
                )
            return
        self.jobs[job] = self.render_end
        if self.render_end == "done":
            path.write_bytes(b"a film of the fake engine")
            told.update(path=str(path), encoder="h264_fake")
        else:
            told["message"] = "the encoder failed"
        self.tell("render.progress", {**told, "state": self.render_end})

    def _applied(
        self, ops: list[dict[str, Any]]
    ) -> tuple[list[dict[str, Any]], dict[str, str], list[int]]:
        names: dict[str, str] = {}
        done: list[dict[str, Any]] = []
        clamped, self.clamped = self.clamped, []
        for index, op in enumerate(ops):
            worked = self._worked(op, names, index) if "edit" in op else [op]
            for one in worked:
                self._one(one, names, index)
                done.append(one)
        return done, names, clamped

    def _one(self, op: dict[str, Any], names: dict[str, str], index: int) -> None:
        """Apply an operation."""
        if op["op"] in _GRADE_OPS:
            self._graded(op, names, index)
            return
        if op["op"] in _FUSION_OPS:
            self._fused(op, names, index)
            return
        if op["op"] in _STABILIZE_OPS:
            self._stabilized(op, names, index)
            return
        kind = _KINDS.get(op["op"], "clip" if op["op"] == "insert_clip" else None)
        for key in ("timeline", "track", "id"):
            if key in op and key != kind:
                op[key] = self._named(op[key], names, index)
        clip = op.get("clip")
        if isinstance(clip, dict):
            source = cast("dict[str, Any]", clip).get("source")
            if isinstance(source, dict) and "media" in source:
                source = cast("dict[str, Any]", source)
                source["media"] = self._named(source["media"], names, index)
        thing = cast("dict[str, Any] | None", op.get(kind) if kind else None)
        if kind is None or thing is None:
            self._changed(op, index)
            return
        given = str(thing.get("id", ""))
        if not given or given.startswith("$"):
            if kind == "track":
                tracks = self._timeline(str(op["timeline"]), index)["tracks"]
                letter = _TRACK_LETTERS[str(thing.get("kind", "video"))]
                count = sum(1 for one in tracks if one["id"].startswith(letter))
                thing["id"] = f"{letter}{count + 1}"
            else:
                # Fixtures may supply ids inside a whole timeline. New ids must not reuse
                # those explicit ids when the fake expands a later edit into inserted clips.
                used = {str(one["id"]) for one in self.project["media"]}
                for timeline in self.project["timelines"]:
                    used.add(str(timeline["id"]))
                    for track in timeline["tracks"]:
                        used.add(str(track["id"]))
                        used.update(str(clip["id"]) for clip in track.get("clips", []))
                while f"{_LETTERS[kind]}{self.project['next_id']}" in used:
                    self.project["next_id"] += 1
                thing["id"] = f"{_LETTERS[kind]}{self.project['next_id']}"
                self.project["next_id"] += 1
            if given:
                names[given] = thing["id"]
        self._added(op, kind, thing, index)

    def _worked(
        self, edit: dict[str, Any], names: dict[str, str], index: int
    ) -> list[dict[str, Any]]:
        """Work an edit out into operations, simply."""
        name = edit.get("edit")
        if name in _GRADE_EDITS:
            return self._grade_worked(edit, names, index)
        if name not in _EDITS:
            message = f"there is no edit {name}"
            raise RefusedError(-32602, message, {"index": index, "reason": "unknown_edit"})
        edit = {key: self._named(value, names, index) for key, value in edit.items()}
        timeline = self._timeline(str(edit.get("timeline")), index)
        self._unlocked(timeline, edit, index)
        on = timeline["id"]
        if name in _PLACING:
            return [self._placed(timeline, edit, index)]
        if name in _RETIMING:
            return [self._retimed(timeline, edit, index)]
        if name in ("lift", "ripple_delete"):
            return [{"op": "remove_clip", "timeline": on, "id": one} for one in edit["clips"]]
        if name == "ripple_delete_range":
            return self._cut_out(timeline, int(edit["from"]), int(edit["to"]), edit.get("tracks"))
        if name == "add_track":
            kind = str(edit["kind"])
            # A timeline shows one subtitle track at a time: a new one is hidden while one is.
            shown = kind == "subtitle" and _shown(timeline) is not None
            track: dict[str, Any] = {
                "id": "",
                "kind": kind,
                "name": edit.get("name", ""),
                "enabled": not shown,
                "locked": False,
            }
            if kind == "subtitle":
                track.update(language=edit.get("language", ""), cues=[])
            else:
                track.update(clips=[], transitions=[])
            return [{"op": "add_track", "timeline": on, "track": track}]
        if name == "remove_track":
            return [{"op": "remove_track", "timeline": on, "id": edit["track"]}]
        return [{"op": "set_timeline", "id": on, "name": timeline["name"]}]

    def _unlocked(self, timeline: dict[str, Any], edit: dict[str, Any], index: int) -> None:
        """Refuse an edit that names a locked track, or a clip on one."""
        keys = ("track", "video_track", "audio_track", "to_track")
        named: list[object] = [edit.get(key) for key in keys]
        named += cast("list[object]", edit.get("tracks") or [])
        clips: set[object] = {edit.get(key) for key in ("clip", "left", "right")}
        clips |= set(cast("list[object]", edit.get("clips") or []))
        for track in timeline["tracks"]:
            on = {clip["id"] for clip in track.get("clips", [])}
            if track["locked"] and (track["id"] in named or on & clips):
                message = f"track {track['id']} is locked"
                data = {"index": index, "reason": "locked", "track": track["id"]}
                raise RefusedError(-32001, message, {**data, "message": message})

    def _placed(self, timeline: dict[str, Any], edit: dict[str, Any], index: int) -> dict[str, Any]:
        """The clip that an edit puts on a timeline: on one track, and nothing moves."""
        source = cast("dict[str, Any]", edit["source"])
        kinds = ["video", "audio"] if source["kind"] == "media" else ["video"]
        usable = [
            one
            for one in timeline["tracks"]
            if one["kind"] in kinds and edit.get(f"{one['kind']}_track", "") is not None
        ]
        wanted = [edit.get(f"{kind}_track") for kind in kinds]
        track = next((one for one in usable if one["id"] in wanted), None)
        track = track or next(iter(usable), None)
        if track is None:
            message = f"there is no {kinds[0]} track to put it on"
            data = {"index": index, "reason": "no_track", "kind": kinds[0]}
            raise RefusedError(-32001, message, {**data, "message": message})
        length = int(source.get("length") or 25)
        at = edit.get("at")
        if at is None:
            at = max((clip["start"] + clip["length"] for clip in track["clips"]), default=0)
        if source["kind"] == "media":
            begins = source.get("from") or {"ticks": 0, "scale": 25}
            given: dict[str, Any] = {"kind": "media", "media": source["media"], "in": begins}
        else:
            given = {key: value for key, value in source.items() if key != "length"}
            _spans_checked(given, index)
        clip = _clip(given, int(at), length)
        return {"op": "insert_clip", "timeline": timeline["id"], "track": track["id"], "clip": clip}

    def _retimed(
        self, timeline: dict[str, Any], edit: dict[str, Any], index: int
    ) -> dict[str, Any]:
        """The ``set_retime`` that an edit of the Retime Controls becomes, for its clip alone.

        The speed points are counted as ``retime.rs`` of the engine counts them. Of its checks
        only those that this count needs are copied: the clip has the frame, a freeze holds one
        frame at least, a speed is above 0, and the first segment has no speed point to clear.
        Not copied: the other checks (the range and the unit of the speeds, the kind of the
        source, what the media has, an overlap), the move of ``source.in`` to the lowest place
        of the path, the ripple, the linked clips, the transitions, a speed kept as it is
        given, and one speed written as the speed of the clip; an edit that changes nothing is
        answered with its operation all the same. ``set_speed_points`` gives the speed points
        themselves, which are set as they are given.
        """
        clip = self._found_clip(timeline["id"], edit["clip"], index)
        name = edit["edit"]
        if name == "set_speed_points":
            return {
                "op": "set_retime",
                "timeline": timeline["id"],
                "id": clip["id"],
                "retime": edit["retime"],
            }
        length = int(clip["length"])
        retime: Any = clip.get("retime")
        written: list[Any] = retime["points"] if retime else []
        points: list[tuple[int, Fraction]] = []
        for one in written:
            # A speed that is left out is that of the source, as the engine reads it.
            given = one.get("speed") or {"num": 1, "den": 1}
            points.append((int(one["frame"]), Fraction(given["num"], given["den"])))
        if not points:
            whole = Fraction(clip["speed"]["num"], clip["speed"]["den"])
            points = [(0, -whole if clip["reverse"] else whole)]
        on: dict[str, Any] = {"op": "set_retime", "timeline": timeline["id"], "id": clip["id"]}
        if name == "reset_speed":
            # As many frames as show once, at the speed of the source, what the path went by.
            place = low = high = Fraction(0)
            for number, (frame, speed) in enumerate(points):
                ends = points[number + 1][0] if number + 1 < len(points) else length
                place += speed * (ends - frame)
                low, high = min(low, place), max(high, place)
            return {**on, "retime": None, "length": max(1, int((high - low) // 1))}
        at = int(edit["at"])
        if not 0 <= at < length:
            raise self._refused(index, "out_of_range", f"the clip has no frame {at}", what="frame")
        number = max(number for number, (frame, _) in enumerate(points) if frame <= at)
        first, speed = points[number]
        ends = points[number + 1][0] if number + 1 < len(points) else length
        head, tail = points[:number], points[number + 1 :]
        if name == "add_speed_point":
            here = [(first, speed)] if first == at else [(first, speed), (at, speed)]
            points = [*head, *here, *tail]
        elif name == "reverse_segment":
            points = [*head, (first, -speed), *tail]
        elif name == "set_segment_speed":
            new = Fraction(edit["speed"]["num"], edit["speed"]["den"])
            if new <= 0:
                raise self._refused(index, "out_of_range", "no speed of a segment", what="speed")
            # It shows what it showed, rounded down, in one frame at least; a freeze keeps its
            # frames, and plays forward over them.
            frames = ends - first
            lasts = max(1, int(frames * abs(speed) // new)) if speed else frames
            later = [(frame + lasts - frames, one) for frame, one in tail]
            points = [*head, (first, -new if speed < 0 else new), *later]
            length += lasts - frames
        elif name == "freeze_frame":
            hold = int(edit["hold"])
            if hold < 1:
                raise self._refused(index, "out_of_range", "a freeze holds a frame", what="hold")
            # A frame of a segment that goes backward shows where it ends: it is held from
            # the frame after.
            begins = at + 1 if speed < 0 else at
            length += hold
            later = [(frame + hold, one) for frame, one in tail]
            held: list[tuple[int, Fraction]] = [(first, speed)] if first < begins else []
            held.append((begins, Fraction(0)))
            if begins + hold < length and (not later or later[0][0] != begins + hold):
                held.append((begins + hold, speed))
            points = [*head, *held, *later]
        else:
            if number == 0:
                message = "the first segment has no speed point to clear"
                raise self._refused(index, "out_of_range", message, what="speed_point")
            # One speed, in a straight line through what the two segments went by.
            before, slower = points[number - 1]
            rise = slower * (first - before) + speed * (ends - first)
            points = [*points[: number - 1], (before, rise / (ends - before)), *tail]
        told = [
            {"frame": frame, "speed": {"num": one.numerator, "den": one.denominator}}
            for frame, one in points
        ]
        return {**on, "retime": {"points": told}, "length": length}

    def _cut_out(
        self, timeline: dict[str, Any], start: int, end: int, tracks: object
    ) -> list[dict[str, Any]]:
        """Cut frames out of tracks, and close the gap, for clips that play forward."""
        cut = end - start
        rate = timeline["rate"]
        on = timeline["id"]
        ops: list[dict[str, Any]] = []
        for track in timeline["tracks"]:
            chosen = tracks is None or track["id"] in cast("list[str]", tracks)
            if track["kind"] == "subtitle" or track["locked"] or not chosen:
                continue
            for clip in track["clips"]:
                first, last = clip["start"], clip["start"] + clip["length"]
                where = {"timeline": on, "id": clip["id"]}
                if last <= start:
                    continue
                if first >= end:
                    ops.append(
                        {"op": "move_clip", **where, "track": track["id"], "start": first - cut}
                    )
                elif start <= first and last <= end:
                    ops.append({"op": "remove_clip", **where})
                elif first < start and last > end:
                    ops.append(
                        {"op": "trim_clip", **where, "start": first, "length": start - first}
                    )
                    tail = _clip(_later(clip, end - first, rate), start, last - end)
                    tail.update(speed=clip["speed"], reverse=clip["reverse"])
                    ops.append(
                        {"op": "insert_clip", "timeline": on, "track": track["id"], "clip": tail}
                    )
                elif first < start:
                    ops.append(
                        {"op": "trim_clip", **where, "start": first, "length": start - first}
                    )
                else:
                    begins = _later(clip, end - first, rate).get("in")
                    trimmed = {"source_in": begins, "start": start, "length": last - end}
                    ops.append({"op": "trim_clip", **where, **trimmed})
        return ops

    # Grades

    def _refused(self, index: int, reason: str, message: str, **data: object) -> RefusedError:
        return RefusedError(
            -32001, message, {"index": index, "reason": reason, **data, "message": message}
        )

    def _target(self, target: dict[str, Any], names: dict[str, str], index: int) -> dict[str, Any]:
        return {key: self._named(value, names, index) for key, value in target.items()}

    def _group(self, id: object, index: int) -> dict[str, Any]:
        for group in self.project.setdefault("groups", []):
            if group["id"] == id:
                return cast("dict[str, Any]", group)
        raise self._refused(index, "missing", f"there is no group {id}", what="group", id=id)

    def _found_clip(self, timeline: object, id: object, index: int) -> dict[str, Any]:
        for track in self._timeline(str(timeline), index)["tracks"]:
            for clip in track.get("clips", []):
                if clip["id"] == id:
                    return cast("dict[str, Any]", clip)
        raise self._refused(index, "missing", f"there is no clip {id}", what="clip", id=id)

    def _grade_of(self, target: dict[str, Any], index: int) -> dict[str, Any]:
        """The grade that a target names, in the project."""
        level = target.get("level")
        if level in ("pre_clip", "post_clip"):
            group = self._group(target.get("group"), index)
            return cast("dict[str, Any]", group["pre" if level == "pre_clip" else "post"])
        if level == "timeline":
            timeline = self._timeline(str(target.get("timeline")), index)
            return cast("dict[str, Any]", timeline.setdefault("grade", _empty_grade()))
        clip = self._found_clip(target.get("timeline"), target.get("clip"), index)
        return cast("dict[str, Any]", clip.setdefault("grade", _empty_grade()))

    def _graded(self, op: dict[str, Any], names: dict[str, str], index: int) -> None:
        """Apply an operation of grades or groups."""
        name = op["op"]
        if name == "add_group":
            group = op["group"]
            given = str(group.get("id", ""))
            if not given or given.startswith("$"):
                group["id"] = f"k{self.project['next_id']}"
                self.project["next_id"] += 1
                if given:
                    names[given] = group["id"]
            groups = self.project.setdefault("groups", [])
            op.setdefault("index", len(groups))
            groups.insert(op["index"], group)
            return
        if name in ("remove_group", "set_group"):
            op["id"] = self._named(op["id"], names, index)
            group = self._group(op["id"], index)
            if name == "set_group":
                group.update({k: v for k, v in op.items() if k not in ("op", "id")})
                return
            for timeline in self.project["timelines"]:
                for track in timeline["tracks"]:
                    for clip in track.get("clips", []):
                        if clip.get("group") == op["id"]:
                            message = f"clip {clip['id']} is in group {op['id']}"
                            raise self._refused(
                                index, "grouped", message, group=op["id"], clip=clip["id"]
                            )
            self.project["groups"].remove(group)
            return
        if name == "set_clip_group":
            for key in ("timeline", "clip", "group"):
                op[key] = self._named(op[key], names, index)
            if op["group"] is not None:
                self._group(op["group"], index)
            self._found_clip(op["timeline"], op["clip"], index)["group"] = op["group"]
            return
        op["grade"] = self._target(op["grade"], names, index)
        grade = self._grade_of(op["grade"], index)
        nodes: list[dict[str, Any]] = grade["nodes"]
        if name == "set_grade":
            grade.clear()
            grade.update(copy.deepcopy(op["value"]))
            return
        if name == "add_node":
            node = op["node"]
            given = str(node.get("id", ""))
            if not given or given.startswith("$"):
                node["id"] = f"n{self.project['next_id']}"
                self.project["next_id"] += 1
                if given:
                    names[given] = node["id"]
            op.setdefault("index", len(nodes))
            nodes.insert(op["index"], node)
            links = [
                {**link, **{end: node["id"] for end in ("from", "to") if link[end] in ("", given)}}
                for link in op.get("links", [])
            ]
            for link in links:
                _linked(grade, link)
            op["links"] = links
            return
        if name in ("connect", "disconnect"):
            if name == "connect":
                _linked(grade, op["link"])
            else:
                grade["links"] = [one for one in grade["links"] if not _into(one, op)]
            return
        id = op["id"] = self._named(op["id"], names, index)
        found = [one for one in nodes if one["id"] == id]
        if not found:
            raise self._refused(index, "missing", f"there is no node {id}", what="node", id=id)
        if name == "set_node":
            found[0].update({k: op[k] for k in ("label", "enabled") if k in op})
            if "params" in op:
                found[0]["params"] = _patched(found[0].get("params", {}), op["params"])
            return
        if name in ("set_keys", "splice_keys"):
            place: Any = found[0]["params"]
            *heads, last = (
                int(part) if part.isdecimal() else part for part in op["path"].split(".")
            )
            for head in heads:
                place = place[head]
            if name == "set_keys":
                place[last] = op["param"]
                return
            now = place[last]
            keys: list[Any] = (
                list(cast("dict[str, Any]", now)["keys"]) if isinstance(now, dict) else []
            )
            keys[op["at"] : op["at"] + op["remove"]] = op["keys"]
            if not keys:
                raise self._refused(
                    index, "out_of_range", "a setting keeps one key at least", what="params"
                )
            place[last] = {"keys": keys}
            return
        # remove_node: what came into it goes where it went, unless links are given.
        up = next((one["from"] for one in grade["links"] if _into(one, {"to": id})), None)
        went = [one for one in grade["links"] if one["from"] == id]
        grade["links"] = [one for one in grade["links"] if id not in (one["from"], one["to"])]
        bridged = op.get("links")
        if bridged is None:
            bridged = [{**one, "from": up} for one in went] if up is not None else []
        for link in bridged:
            _linked(grade, link)
        op["links"] = bridged
        nodes.remove(found[0])

    def _grade_worked(
        self, edit: dict[str, Any], names: dict[str, str], index: int
    ) -> list[dict[str, Any]]:
        """Work an edit of grades out into operations, simply."""
        name = edit["edit"]
        if name == "collapse_group_grades":
            on = self._named(edit["timeline"], names, index)
            return [
                {"op": "set_clip_group", "timeline": on, "clip": clip, "group": None}
                for clip in edit["clips"]
            ]
        if name in ("copy_grade", "append_node_graph", "append_grade", "paste_node", "paste_value"):
            source = self._target(edit["from"], names, index)
            into: list[dict[str, Any]] = edit["to"] if "from_node" not in edit else [edit["to"]]
            targets = [self._target(one, names, index) for one in into]
            copied = copy.deepcopy(self._grade_of(source, index))
            if name == "copy_grade":
                return [{"op": "set_grade", "grade": one, "value": copied} for one in targets]
            if name in ("append_node_graph", "append_grade"):
                kept = [copy.deepcopy(self._grade_of(one, index)) for one in targets]
                return [
                    {"op": "set_grade", "grade": one, "value": value}
                    for one, value in zip(targets, kept, strict=True)
                ]
            node = next(one for one in copied["nodes"] if one["id"] == edit["from_node"])
            params: dict[str, Any] = node["params"]
            if name == "paste_value":
                parts = edit["param"].split(".")
                patch: dict[str, Any] = {}
                place, value = patch, params
                for part in parts[:-1]:
                    place, value = place.setdefault(part, {}), value[part]
                place[parts[-1]] = value[parts[-1]]
                params = patch
            to_node = edit["to_node"]
            return [{"op": "set_node", "grade": targets[0], "id": to_node, "params": params}]
        target = self._target(edit["grade"], names, index)
        grade = self._grade_of(target, index)
        node = edit.get("node", "output")
        ids = {one["id"] for one in grade["nodes"]} | {"source", "output"}
        if node not in ids:
            raise self._refused(index, "missing", f"there is no node {node}", what="node", id=node)
        up = next((one["from"] for one in grade["links"] if _into(one, {"to": node})), None)
        if name == "reset_node":
            params = {"lut": None, "shot_match": None}
            return [{"op": "set_node", "grade": target, "id": node, "params": params}]
        if name == "extract_node":
            kept = copy.deepcopy(next(one for one in grade["nodes"] if one["id"] == node))
            return [
                {"op": "remove_node", "grade": target, "id": node},
                {"op": "add_node", "grade": target, "node": kept, "links": []},
            ]
        if name == "add_serial":
            links = [_link(node, "")] + [
                {**one, "from": ""} for one in grade["links"] if one["from"] == node
            ]
        elif name == "add_outside":
            links = [_link(node, ""), {"from": node, "to": "", "kind": "key", "input": 0}]
            links += [{**one, "from": ""} for one in grade["links"] if one["from"] == node]
        elif name in ("append_node", "add_serial_before"):
            links = [_link("", node)] + ([_link(up, "")] if up is not None else [])
        else:
            links = [_link(up, "")] if up is not None else []
        corrector = {
            "id": "",
            "kind": "corrector",
            "label": "",
            "enabled": True,
            "params": {"lut": None, "shot_match": None},
        }
        return [{"op": "add_node", "grade": target, "node": corrector, "links": links}]

    def _fused(self, op: dict[str, Any], names: dict[str, str], index: int) -> None:
        """Apply an operation on the composition of a clip, with few of the engine's checks."""
        name = op["op"]
        for key in ("timeline", "clip"):
            op[key] = self._named(op[key], names, index)
        clip = self._found_clip(op["timeline"], op["clip"], index)
        if name == "set_fusion":
            # A clip that has no composition is written without one, as the engine writes it.
            if op["value"] is None:
                clip.pop("fusion", None)
            else:
                clip["fusion"] = copy.deepcopy(op["value"])
                links: list[dict[str, Any]] = clip["fusion"]["links"]
                links.sort(key=lambda one: (one["to"], one["input"]))
            return
        if "fusion" not in clip:
            message = f"there is no fusion {clip['id']}"
            raise self._refused(index, "missing", message, what="fusion", id=clip["id"])
        composition = cast("dict[str, Any]", clip["fusion"])
        nodes: list[dict[str, Any]] = composition["nodes"]
        if name == "add_fusion_node":
            node = op["node"]
            given = str(node.get("id", ""))
            if not given or given.startswith("$"):
                node["id"] = f"n{self.project['next_id']}"
                self.project["next_id"] += 1
                if given:
                    names[given] = node["id"]
            op.setdefault("index", len(nodes))
            nodes.insert(op["index"], node)
            # A link names the node by the id that it was given, and a node added before in the
            # transaction by its $name.
            wired: list[dict[str, Any]] = []
            for link in op.get("links", []):
                one = dict(link)
                for end in ("from", "to"):
                    own = one[end] in ("", given)
                    one[end] = node["id"] if own else self._named(one[end], names, index)
                wired.append(one)
            for link in wired:
                _fusion_linked(composition, link)
            op["links"] = wired
            self._kind_checked(node, index)
            return
        if name == "connect_fusion":
            _fusion_linked(composition, op["link"])
            return
        if name == "disconnect_fusion":
            op["to"] = self._named(op["to"], names, index)
            end = (op["to"], op["input"])
            links = composition["links"]
            composition["links"] = [one for one in links if (one["to"], one["input"]) != end]
            return
        if name in ("set_fusion_keys", "splice_fusion_keys"):
            # The id of the node, a dot, and the path of the setting in its params: a point of a
            # polygon is named by its place in the list of its points (``points.2.x``).
            node_id, _, path = str(op["path"]).partition(".")
            keyed = self._fusion_node(nodes, node_id, index)
            place: Any = keyed["params"]
            *heads, last = path.split(".")
            for head in heads:
                # A part of a node that it has not (the outline of a Text+) has no setting.
                inner: Any = None
                if isinstance(place, list):
                    items = cast("list[Any]", place)
                    inner = items[int(head)] if head.isdigit() and int(head) < len(items) else None
                elif isinstance(place, dict):
                    inner = cast("dict[str, Any]", place).get(head)
                if inner is None:
                    message = f"there is no setting {op['path']}"
                    raise self._refused(index, "missing", message, what="setting", path=op["path"])
                place = inner
            if name == "set_fusion_keys":
                place[last] = op["param"]
                self._kind_checked(keyed, index)
                return
            now = place[last]
            keys: list[Any] = (
                list(cast("dict[str, Any]", now)["keys"]) if isinstance(now, dict) else []
            )
            keys[op["at"] : op["at"] + op["remove"]] = op["keys"]
            if not keys:
                message = "a setting keeps one key at least"
                raise self._refused(index, "params", message, node=node_id)
            place[last] = {"keys": keys}
            self._kind_checked(keyed, index)
            return
        id = op["id"] = self._named(op["id"], names, index)
        node = self._fusion_node(nodes, id, index)
        if name == "set_fusion_node":
            node.update({k: op[k] for k in ("label", "pass_through", "pos") if k in op})
            if "params" in op:
                node["params"] = _patched(node.get("params", {}), op["params"])
                _nulls_taken(node)
                self._kind_checked(node, index)
            return
        # remove_fusion_node: its links go, and those that are given are put in their place.
        nodes.remove(node)
        links = composition["links"]
        composition["links"] = [one for one in links if id not in (one["from"], one["to"])]
        op["links"] = list(op.get("links") or [])
        for link in op["links"]:
            _fusion_linked(composition, link)

    def _stabilized(self, op: dict[str, Any], names: dict[str, str], index: int) -> None:
        """Set the stabilizer of a clip, whole or some of its controls, checked as the engine does.

        A clip of sound has none, and a number of one is within its range; the analysis is not
        checked here.
        """
        for key in ("timeline", "clip"):
            op[key] = self._named(op[key], names, index)
        found = self._clip_on(op)
        if found is None:
            raise self._refused(
                index, "missing", f"there is no clip {op['clip']}", what="clip", id=op["clip"]
            )
        track, clip = found
        if op["op"] == "set_stabilize" and op["value"] is None:
            # A clip that has no stabilizer is written without one, as the engine writes it.
            clip.pop("stabilize", None)
            return
        kind = track["kind"]
        if kind != "video":
            message = "a clip of sound has no stabilizer"
            raise self._refused(index, "wrong_track", message, what="stabilize", kind=kind)
        if op["op"] == "set_stabilize":
            value = copy.deepcopy(op["value"])
        else:
            value = {**copy.deepcopy(clip.get("stabilize", _STABILIZER)), **op["settings"]}
        for name, (low, high) in _STABILIZE_RANGES.items():
            if not low <= value[name] <= high:
                message = f"{name} is {low} to {high}"
                raise self._refused(index, "out_of_range", message, what=f"stabilize.{name}")
        clip["stabilize"] = value

    def _fusion_node(self, nodes: list[dict[str, Any]], id: object, index: int) -> dict[str, Any]:
        for node in nodes:
            if node["id"] == id:
                return node
        raise self._refused(index, "missing", f"there is no node {id}", what="node", id=id)

    def _kind_checked(self, node: dict[str, Any], index: int) -> None:
        """Check a Planar Tracker or a Text+ as the engine does: its numbers within their ranges,
        the text and the font of a Text+ no longer than they may be (``out_of_range``)."""
        params = cast("dict[str, Any]", node.get("params", {}))
        if node["kind"] == "planar_tracker":
            region = cast("list[Any]", params.get("region", []))
            if not 3 <= len(region) <= 64 or not all(map(_placed, region)):
                message = "a region is 3 to 64 places, each -15.5 to 16.5"
                raise self._refused(index, "out_of_range", message, what="planar_tracker.region")
            corners = cast("dict[str, Any]", params.get("corners", {}))
            if not all(map(_placed, corners.values())):
                message = "a corner is -15.5 to 16.5 across and up"
                raise self._refused(index, "out_of_range", message, what="planar_tracker.corners")
        if node["kind"] != "text_plus":
            return
        if len(str(params.get("text", ""))) > 4096:
            message = "the text is 4096 characters at most"
            raise self._refused(index, "out_of_range", message, what="text_plus.text")
        if len(str(params.get("font", "")).encode("utf-8")) > 256:
            message = "the font is 256 bytes at most"
            raise self._refused(index, "out_of_range", message, what="text_plus.font")
        for path, (low, high) in _TEXT_PLUS_RANGES.items():
            if any(not low <= value <= high for value in _numbers_at(params, path)):
                message = f"{path} is {low} to {high}"
                raise self._refused(index, "out_of_range", message, what=f"text_plus.{path}")

    def _named(self, value: object, names: dict[str, str], index: int) -> object:
        if isinstance(value, str) and value.startswith("$"):
            if value not in names:
                raise RefusedError(
                    -32602,
                    f"{value} is named before an operation gives it",
                    {"index": index, "name": value, "reason": "unknown_name"},
                )
            return names[value]
        return value

    def _timeline(self, id: str, index: int, *, code: int = -32001) -> dict[str, Any]:
        for timeline in self.project["timelines"]:
            if timeline["id"] == id:
                return timeline
        message = f"there is no timeline {id}"
        raise RefusedError(
            code,
            message,
            {"index": index, "reason": "missing", "what": "timeline", "id": id, "message": message},
        )

    def _added(self, op: dict[str, Any], kind: str, thing: dict[str, Any], index: int) -> None:
        if kind == "bin":
            places = self.project["bins"]
        elif kind == "media":
            places = self.project["media"]
        elif kind == "timeline":
            places = self.project["timelines"]
            tracks = cast("list[dict[str, Any]]", thing.get("tracks", []))
            for at, track in enumerate(tracks):
                if track["kind"] == "subtitle" and track.get("enabled", True):
                    _one_shown({"tracks": tracks[:at]}, track, index)
        elif kind == "track":
            timeline = self._timeline(str(op["timeline"]), index)
            places = timeline["tracks"]
            if thing["kind"] == "subtitle" and thing.get("enabled", True):
                _one_shown(timeline, thing, index)
        else:
            places = self._track(str(op["timeline"]), str(op["track"]), index)["clips"]
        if kind != "clip":
            op.setdefault("index", len(places))
        places.append(thing)
        if kind == "clip":
            places.sort(key=_start)

    def _changed(self, op: dict[str, Any], index: int) -> None:
        """Apply an operation that adds nothing."""
        name = op["op"]
        if name == "remove_media":
            self.project["media"] = [m for m in self.project["media"] if m["id"] != op["id"]]
        elif name == "remove_timeline":
            self.project["timelines"] = [
                t for t in self.project["timelines"] if t["id"] != op["id"]
            ]
        elif name == "set_timeline":
            timeline = self._timeline(str(op["id"]), index)
            timeline.update({key: value for key, value in op.items() if key not in ("op", "id")})
        elif name == "remove_track":
            timeline = self._timeline(str(op["timeline"]), index)
            timeline["tracks"] = [one for one in timeline["tracks"] if one["id"] != op["id"]]
        elif name == "set_track":
            track = self._track(str(op["timeline"]), str(op["id"]), index)
            if track["kind"] == "subtitle" and op.get("enabled") is True:
                _one_shown(self._timeline(str(op["timeline"]), index), track, index)
            track.update({k: v for k, v in op.items() if k not in ("op", "id", "timeline")})
        elif name in (
            "remove_clip", "move_clip", "trim_clip", "set_clip", "set_clip_keys",
            "splice_clip_keys", "set_retime",
        ):  # fmt: skip
            self._clip_changed(op, index)
        elif name == "set_track_audio":
            # Only the patch is put in: the engine's checks of sound are not copied here.
            track = self._track(str(op["timeline"]), str(op["track"]), index)
            track.update(_patched(track, op["patch"]))
        elif name == "set_lane" and "track" in op["of"]:
            # A lane of a track, put in or taken away; the engine's checks are not copied here.
            track = self._track(str(op["timeline"]), str(op["of"]["track"]), index)
            lanes = track.setdefault("lanes", {})
            if op["lane"] is None:
                lanes.pop(str(op["path"]), None)
            else:
                lanes[str(op["path"])] = op["lane"]
        elif name == "set_color_settings":
            self.project["color"] = _patched(self.project.get("color", {}), op["settings"])
        elif name == "set_media_color":
            media = next((m for m in self.project["media"] if m["id"] == op["id"]), None)
            if media is None:
                raise self._refused(index, "missing", "no such media", what="media", id=op["id"])
            told = media.setdefault("color_override", {"input": None, "levels": "auto"})
            told.update({key: op[key] for key in ("input", "levels") if key in op})

    def _track(self, timeline: str, id: str, index: int) -> dict[str, Any]:
        found = [one for one in self._timeline(timeline, index)["tracks"] if one["id"] == id]
        if not found:
            message = f"there is no track {id}"
            data = {"index": index, "reason": "missing", "what": "track", "id": id}
            raise RefusedError(-32001, message, {**data, "message": message})
        return found[0]

    def _clip_changed(self, op: dict[str, Any], index: int) -> None:
        timeline = self._timeline(str(op["timeline"]), index)
        for track in timeline["tracks"]:
            for clip in list(track.get("clips", [])):
                if clip["id"] != op["id"]:
                    continue
                if op["op"] == "remove_clip":
                    track["clips"].remove(clip)
                elif op["op"] == "move_clip":
                    clip["start"] = op["start"]
                    if op["track"] != track["id"]:
                        track["clips"].remove(clip)
                        self._track(timeline["id"], str(op["track"]), index)["clips"].append(clip)
                elif op["op"] == "trim_clip":
                    clip.update(start=op["start"], length=op["length"])
                    if op.get("source_in") is not None:
                        clip["source"]["in"] = op["source_in"]
                elif op["op"] in ("set_clip_keys", "splice_clip_keys"):
                    self._clip_keys(clip, op, index)
                elif op["op"] == "set_retime":
                    # The speed points are put as they are sent: the engine's checks, and its
                    # writing of one speed as the speed of the clip, are not copied here.
                    if op["retime"] is None:
                        clip.pop("retime", None)
                    else:
                        clip["retime"] = op["retime"]
                    if op.get("length") is not None:
                        clip["length"] = op["length"]
                else:
                    if "source" in op:
                        _spans_checked(cast("dict[str, Any]", op["source"]), index)
                    clip.update({k: v for k, v in op.items() if k not in ("op", "id", "timeline")})
                    # A clip that shows the nearest picture is written without its Retime
                    # Process, as the engine writes it.
                    if clip.get("retime_process") == "nearest":
                        del clip["retime_process"]
                for one in timeline["tracks"]:
                    one.get("clips", []).sort(key=_start)
                return
        message = f"there is no clip {op['id']}"
        data = {"index": index, "reason": "missing", "what": "clip", "id": op["id"]}
        raise RefusedError(-32001, message, {**data, "message": message})

    def _clip_keys(self, clip: dict[str, Any], op: dict[str, Any], index: int) -> None:
        """Set the keys of a setting of a clip; the engine's checks of values are not copied."""
        path = str(op["path"])
        if path not in _CLIP_KEYED:
            message = f"there is no setting {path}"
            data = {"index": index, "reason": "missing", "what": "setting", "id": path}
            raise RefusedError(-32001, message, {**data, "message": message})
        of, _, name = path.partition(".")
        if of == "crop":
            # A clip that a test built without a crop has none, as one that the engine reads.
            none = {"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0, "softness": 0.0}
            clip.setdefault("crop", none)
        if of == "title":
            # The size and the place of a title are in the style of its source, and the width
            # of its stroke and the like in that part of its style. A clip of anything else, or
            # a title without the part, has no such setting, and is refused as a path that is
            # not known is.
            source = cast("dict[str, Any]", clip["source"])
            holder = cast("dict[str, Any]", source.get("style") or {})
            part, _, name = name.rpartition(".")
            if part:
                holder = cast("dict[str, Any]", holder.get(part) or {})
            if source.get("kind") != "title" or not holder:
                message = f"there is no setting {path}"
                data = {"index": index, "reason": "missing", "what": "setting", "id": path}
                raise RefusedError(-32001, message, {**data, "message": message})
        else:
            holder = cast("dict[str, Any]", clip[of])
        if op["op"] == "set_clip_keys":
            holder[name] = op["param"]
            return
        now = holder[name]
        keys: list[Any] = list(cast("dict[str, Any]", now)["keys"]) if isinstance(now, dict) else []
        keys[op["at"] : op["at"] + op["remove"]] = op["keys"]
        if not keys:
            message = "a setting keeps one key at least"
            data = {"index": index, "reason": "out_of_range", "what": "keys"}
            raise RefusedError(-32001, message, {**data, "message": message})
        holder[name] = {"keys": keys}


_CLIP_KEYED = frozenset(
    [f"transform.{name}" for name in ("x", "y", "scale", "rotation", "opacity")]
    + [f"crop.{name}" for name in ("left", "right", "top", "bottom", "softness")]
    + [f"title.{name}" for name in ("size", "x", "y")]
    + ["title.stroke.width"]
    + [f"title.shadow.{name}" for name in ("x", "y", "blur")]
    + [f"title.background.{name}" for name in ("padding", "radius")]
)
"""The settings of a clip that take keys, as the engine's catalog lists them. Those of ``title``
are of the style of a clip whose source is a title, those of its parts of a title that has the
part."""


def _spans_checked(source: dict[str, Any], index: int) -> None:
    """Refuse the spans of a title as the engine does (``out_of_range``, ``spans``).

    They are runs of its text by Unicode scalars, as a ``str`` of Python counts them: each ends
    after it begins, in the order of the text, none over another, 256 at the most, and none past
    the end of the text. The other checks of a style, its stroke, shadow and background, are not
    copied here.
    """
    if source.get("kind") != "title":
        return
    style = cast("dict[str, Any]", source.get("style") or {})
    spans = cast("list[dict[str, Any]]", style.get("spans") or [])
    reached = 0
    ordered = True
    for span in spans:
        ordered = ordered and reached <= span["from"] < span["to"]
        reached = span["to"]
    if not ordered or len(spans) > 256 or reached > len(str(source.get("text", ""))):
        message = "the spans of a title are runs of its text, in order, none over another"
        data = {"index": index, "reason": "out_of_range", "what": "spans"}
        raise RefusedError(-32001, message, {**data, "message": message})


def _clip(source: dict[str, Any], start: int, length: int) -> dict[str, Any]:
    """A clip of schema 2, as it is put on a track, with no id."""
    return {
        "id": "",
        "start": start,
        "length": length,
        "source": source,
        "enabled": True,
        "speed": {"num": 1, "den": 1},
        "reverse": False,
        "link": None,
        "transform": {"x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "opacity": 1.0},
        "crop": {"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0, "softness": 0.0},
        "composite": "normal",
        "gain_db": 0.0,
        "pan": 0.0,
        "fade_in": 0,
        "fade_out": 0,
    }


def _later(clip: dict[str, Any], frames: int, rate: dict[str, int]) -> dict[str, Any]:
    """The source of a clip that plays forward, ``frames`` frames into it, as the engine counts."""
    source = copy.deepcopy(cast("dict[str, Any]", clip["source"]))
    if source["kind"] == "media":
        begins, speed = source["in"], clip["speed"]
        whole = frames * rate["den"] * begins["scale"] * speed["num"]
        ticks = whole // (speed["den"] * rate["num"])
        source["in"] = {"ticks": begins["ticks"] + ticks, "scale": begins["scale"]}
    return source


def _start(thing: dict[str, Any]) -> int:
    return thing["start"]


def _shown(timeline: dict[str, Any]) -> dict[str, Any] | None:
    """The subtitle track that a timeline shows, if any: it shows one at a time."""
    return next(
        (one for one in timeline["tracks"] if one["kind"] == "subtitle" and one["enabled"]), None
    )


def _one_shown(timeline: dict[str, Any], track: dict[str, Any], index: int) -> None:
    """Refuse to show ``track`` while another subtitle track of the timeline is shown."""
    shown = _shown({"tracks": [one for one in timeline["tracks"] if one is not track]})
    if shown is not None:
        message = (
            f"subtitle track {shown['id']} is shown, and a timeline shows one subtitle track "
            "at a time"
        )
        data = {"index": index, "reason": "subtitle_shown", "track": shown["id"]}
        raise RefusedError(-32001, message, {**data, "message": message})


def _empty_grade() -> dict[str, Any]:
    return {"nodes": [], "links": [_link("source", "output")]}


def _link(source: str, to: str) -> dict[str, Any]:
    return {"from": source, "to": to, "kind": "rgb", "input": 0}


def _into(link: dict[str, Any], end: dict[str, Any]) -> bool:
    """Whether a link goes into the input that ``end`` names: its first RGB one, unless said."""
    return (
        link["to"] == end["to"]
        and link["kind"] == end.get("kind", "rgb")
        and link["input"] == end.get("input", 0)
    )


def _linked(grade: dict[str, Any], link: dict[str, Any]) -> None:
    """Put a link in place of the one into its input, keeping the links in the engine's order."""
    grade["links"] = [one for one in grade["links"] if not _into(one, link)] + [link]
    grade["links"].sort(key=lambda one: (one["to"], one["kind"] != "rgb", one["input"]))


def _fusion_linked(composition: dict[str, Any], link: dict[str, Any]) -> None:
    """Put a link of a composition in place of the one into its input, in the engine's order."""
    end = (link["to"], link["input"])
    links = [one for one in composition["links"] if (one["to"], one["input"]) != end]
    links.append(link)
    links.sort(key=lambda one: (one["to"], one["input"]))
    composition["links"] = links


_TEXT_PLUS_RANGES: dict[str, tuple[float, float]] = {
    "size": (0.0, 1.0),
    "center.x": (-15.5, 16.5),
    "center.y": (-15.5, 16.5),
    "line_spacing": (0.0, 10.0),
    "tracking": (-1.0, 10.0),
    "angle": (-3600.0, 3600.0),
    "write_on.start": (0.0, 1.0),
    "write_on.end": (0.0, 1.0),
    "transform.offset.x": (-10.0, 10.0),
    "transform.offset.y": (-10.0, 10.0),
    "transform.angle": (-3600.0, 3600.0),
    "transform.scale": (0.0, 100.0),
    **{
        f"{part}.color.{channel}": (0.0, 1.0)
        for part in ("fill", "outline", "border")
        for channel in "rgba"
    },
    "outline.thickness": (0.0, 0.5),
    "border.extend": (0.0, 1.0),
    "border.round": (0.0, 1.0),
    "follower.delay": (0.0, 10000.0),
    "follower.opacity": (0.0, 1.0),
    "follower.offset.x": (-10.0, 10.0),
    "follower.offset.y": (-10.0, 10.0),
    "follower.angle": (-3600.0, 3600.0),
    "follower.scale": (0.0, 100.0),
}
"""The bounds of the numbers of a Text+, by their paths; those of a part that it has not are not
read."""

_TEXT_PLUS_PARTS = ("outline", "border", "follower")
"""The parts of a Text+, which it may be without."""


def _placed(place: Any) -> bool:
    """Whether a place of a Planar Tracker is within its bounds, across and up."""
    if not isinstance(place, dict):
        return False
    axes = cast("dict[str, Any]", place)
    numbers = [axes.get(one) for one in "xy"]
    return all(isinstance(one, int | float) and -15.5 <= one <= 16.5 for one in numbers)


def _numbers_at(params: dict[str, Any], path: str) -> list[float]:
    """The numbers of a setting at a path: its value, or the values of its keys; none where the
    node has no such setting."""
    now: Any = params
    for part in path.split("."):
        now = cast("dict[str, Any]", now).get(part) if isinstance(now, dict) else None
    if isinstance(now, dict):
        keys = cast("list[dict[str, Any]]", cast("dict[str, Any]", now).get("keys", []))
        return [float(key["value"]) for key in keys]
    return [float(now)] if isinstance(now, int | float) else []


def _nulls_taken(node: dict[str, Any]) -> None:
    """Take away what a patch set ``null`` of a Planar Tracker or a Text+, as the engine reads it:
    a frame of the path of the one, a part of the other."""
    params = cast("dict[str, Any]", node.get("params", {}))
    if node["kind"] == "planar_tracker" and isinstance(params.get("path"), dict):
        path = cast("dict[str, Any]", params["path"])
        params["path"] = {frame: one for frame, one in path.items() if one is not None}
    if node["kind"] == "text_plus":
        for part in _TEXT_PLUS_PARTS:
            if part in params and params[part] is None:
                del params[part]


def _patched(params: dict[str, Any], patch: dict[str, Any]) -> dict[str, Any]:
    """The params with a patch put in: objects into objects, other values in place."""
    out = copy.deepcopy(params)
    for key, value in patch.items():
        if isinstance(out.get(key), dict) and isinstance(value, dict):
            out[key] = _patched(out[key], cast("dict[str, Any]", value))
        else:
            out[key] = copy.deepcopy(value)
    return out


def grey_frame(bins: int, value: float = 128 / 255) -> dict[str, Any]:
    """The figures of a frame of 4 x 2 pixels all of one grey, as ``scopes.statistics``."""
    return frame_of([(value, value, value)] * 8, bins)


def frame_of(pixels: list[tuple[float, float, float]], bins: int) -> dict[str, Any]:
    """The figures of pixels of R', G' and B' as ``scopes.statistics`` answers them."""
    names = ("red", "green", "blue", "luma")
    values = [(r, g, b, 0.2126 * r + 0.7152 * g + 0.0722 * b) for r, g, b in pixels]
    counts = {name: [0] * bins for name in names}
    for value in values:
        for name, one in zip(names, value, strict=True):
            counts[name][round(min(max(one, 0.0), 1.0) * (bins - 1))] += 1
    figures: dict[str, Any] = {}
    for at, name in enumerate(names):
        each = [value[at] for value in values]
        figures[name] = {"min": min(each), "max": max(each), "mean": sum(each) / len(each)}
    return {
        "frame": 0,
        "width": len(pixels),
        "height": 1,
        "samples": len(pixels),
        **figures,
        "chroma": {
            "centroid": [0.0, 0.0],
            "spread": [0.0, 0.0],
            "radius": 0.0,
            "saturation": 0.0,
            "hue": 0.0,
        },
        "percentiles": {
            name: {
                "1": figures[name]["min"],
                "50": figures[name]["mean"],
                "99": figures[name]["max"],
            }
            for name in names
        },
        "histogram": {"bins": bins, **counts},
    }


def png_of_grey(width: int, height: int, level: int) -> bytes:
    """An opaque PNG of R'G'B' of 8 bits, every pixel the grey ``level``, as a thumbnail is."""

    def chunk(kind: bytes, body: bytes) -> bytes:
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    row = b"\x00" + bytes([level]) * (width * 3)
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"".join(
        (
            b"\x89PNG\r\n\x1a\n",
            chunk(b"IHDR", header),
            chunk(b"IDAT", zlib.compress(row * height)),
            chunk(b"IEND", b""),
        )
    )


def _same_project(one: dict[str, Any], other: dict[str, Any]) -> bool:
    """Whether two projects are alike but for ``next_id``, which a gesture does not take back."""
    return {**one, "next_id": 0} == {**other, "next_id": 0}


def _duration(timeline: dict[str, Any]) -> int:
    """How many frames a timeline lasts: to the end of the last thing on it."""
    ends = [
        one["start"] + one["length"]
        for track in timeline["tracks"]
        for one in [*track.get("clips", []), *track.get("cues", [])]
    ]
    return max(ends, default=0)


@dataclass
class Step:
    """A step of the history: the project before and after it, what was done, and by whom."""

    before: dict[str, Any]
    after: dict[str, Any]
    done: list[dict[str, Any]]
    made_by: str


def _media(path: Path) -> dict[str, Any]:
    return {
        "id": "",
        "path": str(path),
        "name": path.name,
        "bin": None,
        "mark_in": None,
        "mark_out": None,
        "identity": {"size": path.stat().st_size, "modified_us": 0, "digest": "0" * 64},
        "facts": {
            "duration": {"ticks": 25, "scale": 25},
            "video": {
                "width": 16,
                "height": 16,
                "rate": {"num": 25, "den": 1},
                "codec": "h264",
                "bits": 8,
                "rotation": 0,
            },
        },
    }


class Connection(asyncio.Protocol):
    """One client of the fake engine."""

    def __init__(self, fake: Fake) -> None:
        self.fake = fake
        self.buffer = b""
        self.client: str | None = None
        self.transport: asyncio.Transport | None = None
        self.kept: list[dict[str, Any]] = []

    def connection_made(self, transport: asyncio.BaseTransport) -> None:
        self.transport = cast("asyncio.Transport", transport)
        self.fake.connections.append(self)
        self.fake.log({"event": "connected"})

    def connection_lost(self, exc: Exception | None) -> None:
        self.fake.connections.remove(self)
        self.fake.last_left = time.monotonic()

    def data_received(self, data: bytes) -> None:
        self.buffer += data
        while b"\n" in self.buffer:
            line, _, self.buffer = self.buffer.partition(b"\n")
            if line.strip() and self.transport is not None and not self.transport.is_closing():
                self.read(line.rstrip(b"\r"))

    def read(self, line: bytes) -> None:
        """Answer a line, or refuse it as the engine does, with an id of null."""
        if len(line) > LIMIT:
            return self.refused(-32600, "request line exceeds 16 MiB")
        try:
            request: object = json.loads(line)
        except ValueError:
            return self.refused(-32700, "the line is not JSON")
        if not isinstance(request, dict):
            return self.refused(-32600, "not a request object")
        number = cast("dict[str, Any]", request).get("id")
        if isinstance(number, bool) or not isinstance(number, (int, str)):
            return self.refused(-32600, "a request has an id of a whole number or a string")
        self.handled(cast("dict[str, Any]", request))
        return None

    def refused(self, code: int, message: str) -> None:
        """Answer with an error that names no request."""
        self.send({"jsonrpc": "2.0", "id": None, "error": {"code": code, "message": message}})

    def send(self, message: dict[str, Any]) -> None:
        """Send a message as a line."""
        assert self.transport is not None
        if not self.transport.is_closing():
            self.transport.write(json.dumps(message).encode() + b"\n")

    def handled(self, request: dict[str, Any]) -> None:
        """Answer a request."""
        fake = self.fake
        method, number = str(request.get("method")), request.get("id")
        params = cast("dict[str, Any]", request.get("params") or {})
        fake.asked.append(method)
        assert self.transport is not None
        if method == fake.drop:
            fake.drop = None
            self.transport.close()
            return
        if method == fake.huge:
            fake.huge = None
            self.transport.write(b'{"pad": "' + b"x" * fake.huge_bytes + b'"}\n')
            return
        if method == fake.anonymous:
            fake.anonymous = None
            self.refused(-32600, "the line could not be read")
            return
        answer: dict[str, Any] = {"jsonrpc": "2.0", "id": number}
        try:
            if self.client is None and method != "hello":
                raise RefusedError(-32600, "hello is said first")
            if method in fake.refusals:
                answer["error"] = fake.refusals[method]
            else:
                found = getattr(fake, method.replace(".", "_"), None)
                if found is None:
                    raise RefusedError(-32601, f"there is no method {method}")
                result = found(self, params)
                answer["result"] = fake.results.get(method, result)
        except RefusedError as refused:
            answer["error"] = refused.error
        self.answered(answer)
        if method == "hello" and "result" in answer:
            for line in fake.noise:
                self.transport.write(line + b"\n")
        then = fake.then.pop(method, None)
        if then is not None:
            then()

    def answered(self, answer: dict[str, Any]) -> None:
        if self.fake.hold:
            self.kept.append(answer)
            self.fake.hold -= 1
            if self.fake.hold:
                return
            kept, self.kept = self.kept, []
            for one in reversed(kept):
                self.send(one)
            return
        self.send(answer)


@contextlib.asynccontextmanager
async def served(
    directory: Path, *, lock: bool = True, address: str | None = None
) -> AsyncGenerator[Fake]:
    """Serve a project in the loop of the test, and say in ``engine.lock`` where, if ``lock``.

    It listens at an address of the form of an engine's, unless another is given.
    """
    directory.mkdir(parents=True, exist_ok=True)
    fake = Fake(directory, address or address_of())
    close = await _listening(fake)
    if lock:
        _locked(fake)
    try:
        yield fake
    finally:
        close()
        for one in list(fake.connections):
            cast("asyncio.Transport", one.transport).close()
        await asyncio.sleep(0)


async def _listening(fake: Fake) -> Callable[[], None]:
    loop = asyncio.get_running_loop()
    if sys.platform == "win32":
        proactor = cast("asyncio.ProactorEventLoop", loop)
        servers = await proactor.start_serving_pipe(
            lambda: Connection(fake),  # pyright: ignore[reportArgumentType]
            fake.address,
        )

        def closed() -> None:
            for server in servers:
                server.close()

        return closed
    server = await loop.create_unix_server(lambda: Connection(fake), fake.address)

    def unlinked() -> None:
        server.close()
        with contextlib.suppress(OSError):
            Path(fake.address).unlink()

    return unlinked


def _locked(fake: Fake) -> None:
    started = datetime.now(UTC).isoformat()
    (fake.directory / "engine.lock").write_text(
        json.dumps({"pid": os.getpid(), "address": fake.address, "started": started}),
        encoding="utf-8",
    )


def _say(line: dict[str, Any], *, to: Any = None) -> None:
    out = to or sys.stderr
    out.write(json.dumps(line) + "\n")
    out.flush()


async def _engine(directory: Path, idle_seconds: int) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    fake = Fake(directory, address_of())
    fake.log = _say
    close = await _listening(fake)
    _locked(fake)
    _say({"listening": fake.address, "pid": os.getpid(), "protocol": 2}, to=sys.stdout)
    # Where it would look for libraries, for a test to see.
    looks = ("PATH", "LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH")
    _say({"event": "listening", "environment": {name: os.environ.get(name) for name in looks}})
    while fake.connections or time.monotonic() - fake.last_left < idle_seconds:  # noqa: ASYNC110
        await asyncio.sleep(0.02)
    close()
    (directory / "engine.lock").unlink()


def main(argv: list[str]) -> int:
    """Run as the engine is run, or end as one that cannot start ends."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True, type=Path)
    parser.add_argument("--headless", action="store_true")
    parser.add_argument("--idle-seconds", default="30")
    parser.add_argument("--mode", nargs="+", default=["listen"])
    args = parser.parse_args(argv)
    mode, *rest = args.mode
    directory: Path = args.project
    if not args.idle_seconds.isdigit():
        _say({"error": f"--idle-seconds takes whole seconds, not {args.idle_seconds}"})
        return 2
    if mode == "held":
        _say({"held": {"pid": 1, "address": rest[0] if rest else 7}})
        return 3
    if mode == "held-moved":
        (directory / "engine.lock").write_text(json.dumps({"address": rest[0]}), encoding="utf-8")
        _say({"held": {"pid": 1, "address": address_of()}})
        return 3
    marked = directory / "contended"
    if mode == "contended" or (mode == "contended-once" and not marked.exists()):
        directory.mkdir(parents=True, exist_ok=True)
        marked.touch()
        _say({"contended": True})
        return 3
    if mode == "error":
        _say({"error": "project.json cannot be read"})
        return 2
    if mode == "exit":
        sys.stderr.write("not json\n[1]\n")
        _say({"event": "failed"})
        return 5
    if mode == "libraries":
        sys.stderr.flush()
        os._exit(-1073741515)
    if mode in ("babble", "elsewhere"):
        elsewhere = r"\\.\pipe\elsewhere" if sys.platform == "win32" else "/tmp/elsewhere.sock"
        said = {"listening": elsewhere, "pid": os.getpid(), "protocol": 2}
        print("hello there" if mode == "babble" else json.dumps(said), flush=True)
        time.sleep(60)
        return 0
    if mode == "silent":
        time.sleep(60)
        return 0
    asyncio.run(_engine(directory, int(args.idle_seconds)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
