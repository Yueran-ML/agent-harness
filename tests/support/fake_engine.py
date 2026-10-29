"""An engine of video for the tests: in the process of a test, or as a program of its own.

It speaks protocol 2 of ``engine/PROTOCOL.md`` over a named pipe of Windows or a socket of
Unix, one message a line: ``hello``, ``project.get``, ``transaction.apply`` (with names of the
form ``$name``, and edits beside operations), ``history.undo``, ``history.redo``,
``media.import``, ``timeline.duration``, ``render.start`` and ``render.cancel``. Every change is
told to every connection that said hello before it is answered, as the engine does. A test makes
it misbehave through the attributes of :class:`Fake`.

Its project is of schema 2. Of the rules of the engine it has few: an edit is answered with
operations only, as the engine answers it, but they are made up simply. An edit that puts a
source on a timeline becomes one ``insert_clip`` and nothing moves; ``lift`` and
``ripple_delete`` take their clips away; ``ripple_delete_range`` cuts the frames out of every
track that is not locked, as the engine does for clips that play forward; ``add_track`` and
``remove_track`` do what they say; any other edit is answered with a ``set_timeline`` that changes
nothing. An edit that names a locked track, or a clip on one, is refused with ``locked``. A
render writes a few bytes where it is told to, and tells its progress to every connection.

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
import contextlib
import copy
import json
import os
import sys
import time
from collections.abc import AsyncGenerator, Callable
from dataclasses import dataclass
from datetime import UTC, datetime
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


_KINDS = {"add_media": "media", "add_timeline": "timeline", "add_track": "track", "add_bin": "bin"}
_LETTERS = {"media": "m", "timeline": "t", "clip": "c", "bin": "b"}
_TRACK_LETTERS = {"video": "V", "audio": "A", "subtitle": "S"}
_PLACING = {
    "insert",
    "overwrite",
    "replace",
    "smart_insert",
    "ripple_overwrite",
    "place_on_top",
    "append",
}
_EDITS = _PLACING | {
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
        jobs: The state of each render, by its job.
        rendered: What each ``render.start`` was asked, in order.
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
        self.rendered: list[dict[str, Any]] = []
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

    def project_get(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return {
            "revision": self.revision,
            "next_id": self.project["next_id"],
            "project": copy.deepcopy(self.project),
        }

    def transaction_apply(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        if params.get("revision", self.revision) != self.revision:
            raise RefusedError(-32001, "the project changed", {"reason": "stale_revision"})
        ops = cast("list[dict[str, Any]]", params.get("ops") or [])
        if not ops:
            message = "the transaction has no operations"
            raise RefusedError(-32001, message, {"reason": "empty", "message": message})
        before = copy.deepcopy(self.project)
        try:
            done, names, clamped = self._applied(copy.deepcopy(ops))
        except RefusedError:
            self.project = before
            raise
        self.edits.extend(copy.deepcopy(op) for op in ops if "edit" in op)
        self.changed(str(asking.client), done, before)
        return {
            "revision": self.revision,
            "done": done,
            "next_id": self.project["next_id"],
            "names": names,
            "clamped": clamped,
        }

    def history_undo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
        return self._stepped(asking, self.steps, self.undone, back=True)

    def history_redo(self, asking: "Connection", params: dict[str, Any]) -> dict[str, Any]:
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
        path = Path(str(params.get("path")))
        if not path.is_absolute():
            message = f"render.start takes an absolute path, and {path} is not one"
            raise RefusedError(-32602, message, {"path": str(path), "reason": "relative"})
        if path.exists() and not params.get("replace"):
            message = f"{path} is there already"
            raise RefusedError(-32002, message, {"path": str(path), "reason": "exists"})
        job = f"r{len(self.rendered)}"
        frames = self.render_frames if self.render_frames is not None else _duration(timeline)
        self.jobs[job] = "running"
        asyncio.get_running_loop().call_soon(self._progress, job, path, frames, 0)
        return {"job": job}

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
        if name not in _EDITS:
            message = f"there is no edit {name}"
            raise RefusedError(-32602, message, {"index": index, "reason": "unknown_edit"})
        edit = {key: self._named(value, names, index) for key, value in edit.items()}
        timeline = self._timeline(str(edit.get("timeline")), index)
        self._unlocked(timeline, edit, index)
        on = timeline["id"]
        if name in _PLACING:
            return [self._placed(timeline, edit, index)]
        if name in ("lift", "ripple_delete"):
            return [{"op": "remove_clip", "timeline": on, "id": one} for one in edit["clips"]]
        if name == "ripple_delete_range":
            return self._cut_out(timeline, int(edit["from"]), int(edit["to"]), edit.get("tracks"))
        if name == "add_track":
            kind = str(edit["kind"])
            track: dict[str, Any] = {
                "id": "",
                "kind": kind,
                "name": edit.get("name", ""),
                "enabled": True,
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
        clip = _clip(given, int(at), length)
        return {"op": "insert_clip", "timeline": timeline["id"], "track": track["id"], "clip": clip}

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
        elif kind == "track":
            places = self._timeline(str(op["timeline"]), index)["tracks"]
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
            track.update({k: v for k, v in op.items() if k not in ("op", "id", "timeline")})
        elif name in ("remove_clip", "move_clip", "trim_clip", "set_clip"):
            self._clip_changed(op, index)

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
                else:
                    clip.update({k: v for k, v in op.items() if k not in ("op", "id", "timeline")})
                for one in timeline["tracks"]:
                    one.get("clips", []).sort(key=_start)
                return
        message = f"there is no clip {op['id']}"
        data = {"index": index, "reason": "missing", "what": "clip", "id": op["id"]}
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
