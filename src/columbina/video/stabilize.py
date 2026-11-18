"""The stabilizer of a clip, as the agent reads, sets and measures it through the engine.

A clip of a track of picture may have a stabilizer (``engine/PROTOCOL.md``, The stabilizer; the
Stabilization of the Inspector of the Edit and Cut pages): the motion of the whole picture of its
source, measured once by the engine (``track.stabilize``) and kept as its analysis, frame by
frame; and the controls by which the path of the camera is worked out of it and each frame moved
against the shake. Of the controls, the Method alone asks for a new analysis, as the motion is
measured as a homography for Perspective and as a similarity for Similarity and Translation;
Camera Lock, Zoom, the Cropping Ratio, Smooth and Strength are drawn at once from the analysis
that the clip has. The picture is held still after the clip's composition, and before its grade,
its transform and its crop. A title, a colour and a still have no frames of a source, and are not
stabilized whatever they keep.

What is here: :func:`info` tells the stabilizer of a clip; :func:`adjust` sets some of its
controls (``set_stabilize_settings``) and :func:`remove` takes it away (``set_stabilize`` of
``null``), each one step of the history; :func:`analyse` has the engine measure the clip and
write the analysis, one step too, and waits for the end. :func:`run` does what
``video_stabilize`` is asked. What can be told from the project is checked before anything is
sent, and said in words with the name that the engine would refuse it by.

An analysis counts the frames of the clip's source, as its keys do
(:class:`columbina.video.clip_keys.KeyClock`); a model thinks in frames of the timeline, and is
told those.
"""

import contextlib
import math
from collections.abc import Mapping, Sequence
from typing import Any, Literal, cast

from columbina.video import edits
from columbina.video.clip_keys import KeyClock
from columbina.video.edits import Rate
from columbina.video.engine import Engine, EngineError
from columbina.video.grading import GradeError, applied, timeline_of

__all__ = [
    "DEFAULTS",
    "METHODS",
    "SETTINGS",
    "Action",
    "Method",
    "StabilizeError",
    "adjust",
    "analyse",
    "brief",
    "hint_of",
    "info",
    "remove",
    "run",
]

Action = Literal["info", "analyse", "set", "remove"]
"""What ``video_stabilize`` does: tell the stabilizer, measure the clip, set some of the
controls, or take the stabilizer away."""

Method = Literal["perspective", "similarity", "translation"]
"""How much of the motion of the picture is taken to be the camera's: all of a homography, a
shift with a turn and a zoom, or a shift alone."""

METHODS: tuple[Method, ...] = ("perspective", "similarity", "translation")

SETTINGS: tuple[str, ...] = (
    "method",
    "camera_lock",
    "zoom",
    "cropping_ratio",
    "smooth",
    "strength",
)
"""The controls of a stabilizer, as a clip keeps them."""

DEFAULTS: dict[str, Any] = {
    "method": "perspective",
    "camera_lock": False,
    "zoom": True,
    "cropping_ratio": 0.5,
    "smooth": 0.25,
    "strength": 1.0,
}
"""The controls that a clip is given with its first stabilizer, as the Inspector shows them."""

_RANGES: dict[str, tuple[float, float]] = {
    "cropping_ratio": (0.0, 1.0),
    "smooth": (0.0, 1.0),
    "strength": (-1.0, 1.0),
}
"""The bounds of the controls that are numbers, as the engine holds them (``stabilize.<name>``)."""

_SWITCHES = ("camera_lock", "zoom")

_MEASURE_SECONDS = (30.0, 0.25)
"""How long a measure is waited for: a time of its own, and a time for each frame that it reads.
One that takes longer is stopped, and writes nothing."""

_STOP_SECONDS = 5.0
"""How long a measure that was stopped may take to tell how it ended."""

_UNSTABILIZED = (
    "its source as it is, with no stabilizer: analyse measures the clip and gives it one, set "
    "gives it one with the controls given"
)
_UNMEASURED = "it is not measured yet, so nothing is corrected: analyse measures the clip"
_DONE = (
    "the clip is held still by what was measured, one undo step (video_undo takes it back); set "
    "changes camera_lock, zoom, cropping_ratio, smooth and strength at once, without measuring "
    "again; video_look shows frames of it"
)

_ENDED = {
    "stale_revision": (
        "the project was changed while it measured (by another client, an undo or a redo): "
        "nothing was written; analyse again"
    ),
    "stale_gesture": "the project was changed while it measured: nothing was written",
    "disconnected": "the connection to the engine went: nothing was written",
    "target": "its clip was taken away while it measured: nothing was written",
    "cannot_read": "the media of the clip cannot be read: nothing was written",
    "missing_frame": "a frame of the media could not be decoded: nothing was written",
    "stop": "it was stopped where it was started: nothing was written",
}
"""Why a measure ended before its end, in words, by the reason that the engine gives."""

_TAKES: dict[str, tuple[str, ...]] = {
    "info": (),
    "analyse": ("method",),
    "set": SETTINGS,
    "remove": (),
}
"""What each action takes, besides the clip."""


class StabilizeError(Exception):
    """What is wrong with what ``video_stabilize`` was asked, said so that it can be put right."""


# ---------------------------------------------------------------------------------------------
# A clip and its stabilizer as they are read
# ---------------------------------------------------------------------------------------------


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


def _is_map(value: object) -> bool:
    return isinstance(value, Mapping)


def _fixed(value: object) -> float | None:
    """A number as a float, or none where the value is not a finite number."""
    if isinstance(value, int | float) and not isinstance(value, bool) and math.isfinite(value):
        return float(value)
    return None


def _int_of(value: object) -> int | None:
    """A whole number as the engine wrote it, or none where it is not one."""
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _method_of(value: object) -> Method | None:
    """A Method that is known, or none."""
    for one in METHODS:
        if value == one:
            return one
    return None


def _frames_of(analysis: Mapping[str, Any]) -> list[int]:
    """The frames of the source that an analysis measured, in order."""
    frames: list[int] = []
    for key in _object(analysis.get("path")):
        with contextlib.suppress(ValueError):
            frames.append(int(key))
    return sorted(frames)


def brief(stabilizer: object) -> dict[str, Any]:
    """A stabilizer as ``video_info`` tells it: the motion of each frame measured left out.

    Its controls as they are; of its analysis, the Method that it was measured for, how many
    frames of the source it measured, and the first and the last of them (``source_frames``).
    """
    told = _object(stabilizer)
    analysis = _object(told.get("analysis"))
    if analysis:
        frames = _frames_of(analysis)
        measured: dict[str, Any] = {
            "method": analysis.get("method"),
            "frames": len(frames),
            "source_frames": [frames[0], frames[-1]] if frames else None,
        }
        told["analysis"] = measured
    return told


def _timeline(project: Mapping[str, Any], id: str | None) -> dict[str, Any]:
    try:
        return timeline_of(project, id)
    except GradeError as error:
        raise StabilizeError(str(error)) from None


def _picture_clip(timeline: Mapping[str, Any], id: str) -> dict[str, Any]:
    """A clip of a timeline by its id, which is to be one of a track of picture.

    Raises:
        StabilizeError: There is no such clip, or it is one of sound.
    """
    for track in map(_object, _items(timeline.get("tracks"))):
        for clip in map(_object, _items(track.get("clips"))):
            if clip.get("id") == id:
                if track.get("kind") != "video":
                    raise StabilizeError(
                        f"{id} is a clip of sound on {track.get('id')}; a stabilizer is of a "
                        "clip of a picture track (the engine refuses it as wrong_track "
                        "stabilize)"
                    )
                return clip
    raise StabilizeError(
        f"there is no clip {id} on timeline {timeline.get('id')}; video_info lists the clips"
    )


def _media_of(project: Mapping[str, Any], clip: Mapping[str, Any]) -> dict[str, Any] | None:
    named = _object(clip.get("source")).get("media")
    return next(
        (one for one in map(_object, _items(project.get("media"))) if one.get("id") == named),
        None,
    )


def _clock(
    project: Mapping[str, Any], timeline: Mapping[str, Any], clip: Mapping[str, Any]
) -> KeyClock:
    return KeyClock.of(clip, Rate.model_validate(timeline["rate"]), _media_of(project, clip))


def _sourceless(project: Mapping[str, Any], clip: Mapping[str, Any]) -> str | None:
    """What a clip is that has no frames of a source: a title, a colour or a still; else none."""
    kind = _object(clip.get("source")).get("kind")
    if kind == "title":
        return "a title"
    if kind == "color":
        return "a colour"
    video = _object(_object(_object(_media_of(project, clip)).get("facts")).get("video"))
    return "a still" if video.get("still") else None


def _unmeasurable(project: Mapping[str, Any], clip: Mapping[str, Any]) -> str | None:
    """What a clip is that has no moving picture of media to measure; none where it has one."""
    what = _sourceless(project, clip)
    if what is not None:
        return what
    media = _media_of(project, clip)
    if media is None:
        return "of media that the project does not have"
    if not _object(_object(media.get("facts")).get("video")):
        return "of media without a picture"
    return None


def _shown_near(clock: KeyClock, key: int) -> int:
    """The first frame of the timeline that shows the frame ``key`` of the clip's source.

    Where none does (a frame that a trim left out, or that a fast clip passes over), the frame
    of the timeline that shows the frame nearest to it.
    """
    shown = clock.shown(key)
    if shown is not None:
        return shown
    frames = range(clock.start, clock.start + max(clock.length, 1))
    return min(frames, key=lambda at: abs(clock.at(at) - key))


def _showing(clock: KeyClock, frames: Sequence[int]) -> list[int]:
    """The frames of the timeline, in order, at which the clip shows a frame that was measured.

    An analysis holds every frame from its first to its last.
    """
    if not frames:
        return []
    low, high = frames[0], frames[-1]
    after = clock.start + clock.length
    return [at for at in range(clock.start, after) if low <= clock.at(at) <= high]


def _analysis_told(
    stabilizer: Mapping[str, Any], clock: KeyClock, notes: list[str]
) -> dict[str, Any]:
    """The analysis of a stabilizer as it is told, and whether the clip is to be measured again.

    What is to be said of it is put in ``notes``.
    """
    analysis = _object(stabilizer.get("analysis"))
    if not analysis:
        notes.append(_UNMEASURED)
        return {"analysis": None}
    method = _method_of(stabilizer.get("method")) or "perspective"
    measured = str(analysis.get("method"))
    frames = _frames_of(analysis)
    showing = _showing(clock, frames)
    unmeasured = clock.length - len(showing)
    told: dict[str, Any] = {
        "analysis": {
            "method": measured,
            "frames": len(frames),
            "timeline_frames": [showing[0], showing[-1]] if showing else None,
        },
        "analyse_again": measured != method or unmeasured > 0,
    }
    if measured != method:
        notes.append(
            f"the analysis was measured for {measured}, and the method is {method}: analyse "
            "measures the clip again; until then the analysis is drawn by the part of its "
            f"motion that {method} corrects"
        )
    if unmeasured > 0:
        notes.append(
            f"{unmeasured} frames of the clip show frames of its source that were not measured "
            "(it was made longer or slipped since): analyse measures them"
        )
    return told


def _state(project: Mapping[str, Any], timeline: str | None, clip: str) -> dict[str, Any]:
    """The stabilizer of a clip as it is told: its controls and analysis, or that it has none."""
    on = _timeline(project, timeline)
    found = _picture_clip(on, clip)
    told: dict[str, Any] = {"timeline": on["id"], "clip": clip}
    notes: list[str] = []
    what = _sourceless(project, found)
    if what is not None:
        notes.append(
            f"clip {clip} is {what}, which has no frames of a source: it is not stabilized"
        )
    own = found.get("stabilize")
    if not _is_map(own):
        told["stabilizer"] = False
        told["shows"] = _UNSTABILIZED
    else:
        stabilizer = _object(own)
        told["stabilizer"] = True
        told["settings"] = {name: stabilizer.get(name, value) for name, value in DEFAULTS.items()}
        told.update(_analysis_told(stabilizer, _clock(project, on, found), notes))
    if notes:
        told["notes"] = notes
    return told


def _after(project: Mapping[str, Any], timeline: str, clip: str) -> dict[str, Any]:
    """The stabilizer of a clip after a change, as :func:`info` tells it."""
    try:
        return _state(project, timeline, clip)
    except StabilizeError:  # Another client may have taken the clip away meanwhile.
        return {"timeline": timeline, "clip": clip}


# ---------------------------------------------------------------------------------------------
# What the tool does
# ---------------------------------------------------------------------------------------------


def _checked(settings: Mapping[str, Any]) -> dict[str, Any]:
    """The controls given, checked against what the engine takes before anything is sent.

    Raises:
        StabilizeError: A control is not one, or is out of its range; said with the name that
            the engine refuses it by.
    """
    checked: dict[str, Any] = {}
    for name, value in settings.items():
        if name == "method":
            method = _method_of(value)
            if method is None:
                raise StabilizeError(f"method is one of {', '.join(METHODS)}")
            checked[name] = method
        elif name in _SWITCHES:
            if not isinstance(value, bool):
                raise StabilizeError(f"{name} is true or false")
            checked[name] = value
        elif name in _RANGES:
            low, high = _RANGES[name]
            number = _fixed(value)
            if number is None or not low <= number <= high:
                raise StabilizeError(
                    f"{name} is {low:g} to {high:g}, and {value} is not (the engine refuses it "
                    f"as out_of_range stabilize.{name})"
                )
            checked[name] = number
        else:
            raise StabilizeError(f"a stabilizer has no {name}; it has {', '.join(SETTINGS)}")
    return checked


async def info(engine: Engine, timeline: str | None, clip: str) -> dict[str, Any]:
    """Tell the stabilizer of a clip.

    Returns:
        The clip's timeline and id, whether it has a stabilizer (``stabilizer``), and of one
        its controls (``settings``), its analysis (the Method that it was measured for, how
        many frames, and the first and the last frame of the timeline that show one measured),
        whether the clip is to be measured again (``analyse_again``: another Method, or frames
        shown that were not measured), and what is to be said of it (``notes``).

    Raises:
        StabilizeError: There is no such timeline or clip, or the clip is one of sound.
    """
    _, project = await engine.project()
    return _state(project, timeline, clip)


async def adjust(
    engine: Engine, timeline: str | None, clip: str, settings: Mapping[str, Any]
) -> dict[str, Any]:
    """Set some of the controls of the stabilizer of a clip, one step of the history.

    Those given that would change are sent (``set_stabilize_settings``), and the others and the
    analysis are kept; a clip that has no stabilizer is given one with :data:`DEFAULTS`, and
    those given in their place. A new Method asks for a new analysis, which the result says.

    Returns:
        The revision made, how many operations it was, and the stabilizer after the change as
        :func:`info` tells it.

    Raises:
        StabilizeError: What is asked is wrong, or would change nothing, said before anything
            is sent.
        EngineError: The engine refused the change.
    """
    patch = _checked(settings)
    if not patch:
        raise StabilizeError(f"set needs one of {', '.join(SETTINGS)} at least")
    wrote: list[str] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = _timeline(project, timeline)
        found = _picture_clip(on, clip)
        wrote[:] = [str(on["id"])]
        what = _sourceless(project, found)
        if what is not None:
            raise StabilizeError(
                f"clip {clip} is {what}, which has no frames of a source: only a clip of moving "
                "media is stabilized"
            )
        now = _object(found.get("stabilize"))
        # A clip without a stabilizer is given one: everything given is a change then.
        changed = {name: value for name, value in patch.items() if now.get(name) != value}
        if not changed:
            raise StabilizeError(
                f"every value given is what the stabilizer of clip {clip} has already: nothing "
                "changes"
            )
        return [edits.set_stabilize_settings(wrote[0], clip, changed)]

    done, project = await applied(engine, build)
    told: dict[str, Any] = {"revision": done.revision, "operations": len(done.done)}
    told.update(_after(project, wrote[0], clip))
    return told


async def remove(engine: Engine, timeline: str | None, clip: str) -> dict[str, Any]:
    """Take the stabilizer of a clip away, its analysis with it, one step of the history.

    Returns:
        The revision made, how many operations it was, and the clip after it as :func:`info`
        tells it.

    Raises:
        StabilizeError: The clip has no stabilizer, said before anything is sent.
        EngineError: The engine refused the change.
    """
    wrote: list[str] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = _timeline(project, timeline)
        found = _picture_clip(on, clip)
        wrote[:] = [str(on["id"])]
        if not _is_map(found.get("stabilize")):
            raise StabilizeError(
                f"clip {clip} has no stabilizer to take away: it shows its source as it is"
            )
        return [edits.set_stabilize(wrote[0], clip, None)]

    done, project = await applied(engine, build)
    told: dict[str, Any] = {"revision": done.revision, "operations": len(done.done)}
    told.update(_after(project, wrote[0], clip))
    return told


def _ended_told(end: Mapping[str, Any], clock: KeyClock, late: float | None) -> dict[str, Any]:
    """How a measure ended, as ``track.state`` tells it, at frames of the timeline.

    ``late`` is how long it was waited for, where it was stopped for taking longer.
    """
    phase = str(end.get("phase"))
    reason = "timeout" if late is not None else end.get("reason")
    told: dict[str, Any] = {"phase": phase}
    if reason is not None:
        told["reason"] = reason
    message = end.get("message")
    if isinstance(message, str):
        told["message"] = message
    told["frames"] = _int_of(end.get("frames"))
    lost = _int_of(end.get("lost")) or 0
    told["lost"] = lost
    span = [one for one in map(_int_of, _items(end.get("analysed"))) if one is not None]
    told["analysed"] = sorted(_shown_near(clock, one) for one in span) or None
    notes: list[str] = []
    if late is not None:
        notes.append(
            f"it took longer than {late:g} s and was stopped, and a measure that is stopped "
            "writes nothing: analyse again"
        )
    elif phase != "done":
        notes.append(_ENDED.get(str(reason), "it ended before its end: nothing was written"))
    if lost:
        notes.append(
            f"the picture was lost at {lost} frames (one colour, a cut): each holds the motion "
            "of the frame before it"
        )
    if phase == "done" and span:
        notes.append(_DONE)
    if notes:
        told["notes"] = notes
    return told


async def analyse(
    engine: Engine, timeline: str | None, clip: str, *, method: Method | None = None
) -> dict[str, Any]:
    """Have the engine measure the motion of the picture of a clip, and wait for the end.

    The engine reads every frame of the source from the first that the clip shows to the last,
    and writes what it measured at the end as the analysis of the clip's stabilizer, with
    ``method``, one step of the history; the clip's Method, or Perspective, where it is left
    out. A clip without a stabilizer is given one with :data:`DEFAULTS` then. A measure that is
    stopped or fails writes nothing. What can be told from the project is checked before
    anything is sent.

    The measure is waited for as long as :data:`_MEASURE_SECONDS` gives for the frames that it
    reads; one that takes longer is stopped.

    Returns:
        The Method measured for, how it ended (``phase``: ``done``, ``stopped`` or ``failed``,
        with a ``reason`` but where it is done, ``timeout`` where it took too long), how many
        frames it read (``frames``) and at how many the picture was lost (``lost``), the first
        and the last frame of the timeline that show a frame of the analysis written
        (``analysed``), and what may be done next (``notes``).

    Raises:
        StabilizeError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the measure, or the connection to it was lost.
    """
    revision, project = await engine.project()
    session = engine.session
    on = _timeline(project, timeline)
    found = _picture_clip(on, clip)
    what = _unmeasurable(project, found)
    if what is not None:
        raise StabilizeError(
            f"clip {clip} is {what}: it has no frames of a moving picture to measure (the engine "
            "refuses it as untrackable)"
        )
    own = _object(found.get("stabilize"))
    measuring = method or _method_of(own.get("method")) or "perspective"
    asked: dict[str, Any] = {
        "timeline": on["id"],
        "clip": clip,
        "revision": revision,
        "method": measuring,
    }
    tracking = await engine.track(asked, session=session, method="track.stabilize")
    started = tracking.started
    first, last = _int_of(started.get("first")), _int_of(started.get("last"))
    frames = 0 if first is None or last is None else max(last - first + 1, 0)
    seconds = _MEASURE_SECONDS[0] + _MEASURE_SECONDS[1] * frames
    end, late = await tracking.waited(seconds, stop_seconds=_STOP_SECONDS)
    told: dict[str, Any] = {
        "timeline": on["id"],
        "clip": clip,
        "method": started.get("method", measuring),
    }
    if end is None:
        told.update(phase="stopped", reason="timeout")
        told["notes"] = [
            f"it took longer than {seconds:g} s and was stopped, and the engine told nothing "
            "more: info tells whether the analysis was written"
        ]
        return told
    told.update(_ended_told(end, _clock(project, on, found), seconds if late else None))
    return told


async def run(
    engine: Engine, timeline: str | None, clip: str, action: Action, given: Mapping[str, Any]
) -> dict[str, Any]:
    """Do what ``video_stabilize`` is asked, one action a call.

    ``given`` holds the controls that the call names, ``None`` for one that it leaves out:
    ``set`` sets them, and ``analyse`` takes ``method`` alone.

    Raises:
        StabilizeError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused, or the connection to it was lost.
    """
    named = {name: value for name, value in given.items() if value is not None}
    takes = _TAKES[action]
    other = [name for name in named if name not in takes]
    if other:
        verb = "is" if len(other) == 1 else "are"
        whole = ", ".join(takes) or "the clip alone"
        raise StabilizeError(f"{', '.join(other)} {verb} not for {action}, which takes {whole}")
    if action == "analyse":
        method = _checked(named).get("method")
        return await analyse(engine, timeline, clip, method=_method_of(method))
    if action == "set":
        return await adjust(engine, timeline, clip, named)
    if action == "remove":
        return await remove(engine, timeline, clip)
    return await info(engine, timeline, clip)


# ---------------------------------------------------------------------------------------------
# What a refusal of the engine means
# ---------------------------------------------------------------------------------------------

_HINTS = {
    "busy": (
        "another track or measure is running (one at a time): wait for it to end, or stop it "
        "where it was started"
    ),
    "stale_revision": "the project changed meanwhile: call again",
    "untrackable": (
        "a clip of sound, a title, a colour, a still or media that is not there has no frames "
        "to measure"
    ),
    "target": "video_info lists the timelines and their clips",
    "missing": "video_info lists the timelines and their clips",
    "wrong_track": "a stabilizer is of a clip of a picture track",
}
"""What a model can do about a refusal of the engine, by its reason."""


def hint_of(error: EngineError) -> str | None:
    """What a model can do about a refusal of ``video_stabilize``, if more is to be said."""
    data = error.data or {}
    reason, what = data.get("reason"), data.get("what")
    if reason == "out_of_range" and isinstance(what, str):
        name = what.removeprefix("stabilize.")
        if name in _RANGES:
            low, high = _RANGES[name]
            return f"{name} is {low:g} to {high:g}"
        return None
    return _HINTS.get(str(reason))
