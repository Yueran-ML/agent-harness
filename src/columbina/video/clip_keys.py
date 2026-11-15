"""The keys of a clip's picture, its transform, its crop and a title's style, one at a time.

``video_clip_keys`` puts a key of one setting of a clip, takes one away, or clears them all, as
``video_grade_keys`` does of a setting of a node: :func:`columbina.video.grading.key_op` works
both out, and here the operations are the engine's ``set_clip_keys`` and ``splice_clip_keys``,
so the other keys of the setting are not sent and the change is one step of the history.

A model thinks in frames of the timeline, and the engine keeps a key at a frame of the clip's
source (``engine/PROTOCOL.md``, Frames of keys): of a clip of media, the frame of its media
whose picture is shown, numbered at the media's own rate from 0; of a title, a colour or a
still, the frame of the clip from its first. So the tool is given a frame of the timeline,
finds the frame of the key as the engine finds it (:class:`KeyClock`, which counts as
``KeyClock`` of ``engine/crates/model/src/keyframes.rs`` counts and must be kept the same), and
tells both, with the frame of the timeline that each key of the setting is shown at.

The settings of a title's style, ``title.size``, ``title.x`` and ``title.y``, are of a clip whose
source is a title only: the engine refuses them of any other clip as it refuses a path that it
does not know, and so the tool says so before anything is sent.
"""

import difflib
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, cast

from columbina.video import edits
from columbina.video.edits import Rate, Speed, Time
from columbina.video.engine import Engine
from columbina.video.grading import (
    GradeError,
    KeyAction,
    KeyOps,
    Number,
    applied,
    key_op,
    timeline_of,
)
from columbina.video.retime import Course
from columbina.video.span import media_frame, placed, ticks_of_scaled

__all__ = ["SETTINGS", "KeyClock", "KeysError", "keys", "setting_of"]


class KeysError(Exception):
    """What is wrong with what ``video_clip_keys`` was asked, said so that it can be put right."""


@dataclass(frozen=True)
class _Styled(Number):
    """A setting of a title's style, which has no value that changes nothing.

    Its ``default`` is what the engine's catalog gives it, the value that a new title has.
    """

    def told(self) -> str:
        """Its range and what it is in a new title, in words."""
        return f"{self.name}: {self.low:g} to {self.high:g}, {self.default:g} in a new title"


SETTINGS: dict[str, Number] = {
    "transform.x": Number("Position X, right, in widths of the timeline's picture", 0, -16, 16),
    "transform.y": Number("Position Y, down, in heights of the timeline's picture", 0, -16, 16),
    "transform.scale": Number("Zoom, a factor", 1, 0.01, 16),
    "transform.rotation": Number("Rotation Angle, in degrees clockwise", 0, -3600, 3600),
    "transform.opacity": Number("Opacity", 1, 0, 1),
    "crop.left": Number("Crop Left, a share of the width of the clip's picture", 0, 0, 1),
    "crop.right": Number("Crop Right, a share of the width of the clip's picture", 0, 0, 1),
    "crop.top": Number("Crop Top, a share of the height of the clip's picture", 0, 0, 1),
    "crop.bottom": Number("Crop Bottom, a share of the height of the clip's picture", 0, 0, 1),
    "crop.softness": Number("Crop Softness, a share of the height of the clip's picture", 0, 0, 1),
    "title.size": _Styled("Size of a title, the height of its letters in pixels", 72, 1, 4096),
    "title.x": _Styled("Position X of a title's anchor, in widths of the picture", 0.5, -16, 16),
    "title.y": _Styled("Position Y of a title's anchor, in heights of the picture", 0.5, -16, 16),
}
"""The settings of a clip that take keys, by their paths in the clip, with the bounds that the
engine's catalog gives them (``clip`` of ``engine/crates/model/tests/data/catalog.json``). Those
of ``title`` are of the style of a clip whose source is a title, and of no other clip."""

_PATHS = (
    "transform.x, y, scale, rotation, opacity and crop.left, right, top, bottom, softness, "
    "and of a title title.size, x, y"
)
_AS_DAVINCI = {"zoom": "scale", "angle": "rotation"}
"""What the Inspector calls a setting, where a model may name it so."""


def setting_of(path: str) -> Number:
    """The setting of a clip at ``path``, which takes keys.

    Raises:
        KeysError: A clip has no such setting that takes keys; the nearest is suggested.
    """
    spec = SETTINGS.get(path)
    if spec is not None:
        return spec
    # A name alone, under the wrong one of the three, misspelt or as DaVinci has it, is that
    # name. A name that two of them have, x or y, is that of the one that the path names, or
    # of the transform, which every clip of picture has.
    named = path.split(".", 1)[0].lower()
    names: dict[str, str] = {}
    for one in SETTINGS:
        holder, name = one.split(".")
        if name not in names or holder == named:
            names[name] = one
    last = path.rsplit(".", 1)[-1].lower()
    last = _AS_DAVINCI.get(last, last)
    close = [last] if last in names else difflib.get_close_matches(last, list(names), n=1)
    maybe = f" (did you mean {names[close[0]]}?)" if close else ""
    raise KeysError(f"a clip has no setting {path!r} that takes keys{maybe}; it has {_PATHS}")


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


# ---------------------------------------------------------------------------------------------
# The frames of keys
# ---------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Picture:
    """How a clip reads the picture of its media.

    ``course`` is that of a clip with speed points, which it is read by, and not by its speed.
    """

    begins: int
    speed: Speed
    reverse: bool
    rate: Rate
    duration: Time
    course: Course | None = None


@dataclass(frozen=True)
class KeyClock:
    """How the frame of the keys of a clip is found from a frame of its timeline.

    A clip of media with a picture that is not a still counts the frames of its media, the
    number of the picture that it shows, through its speed and its direction, or along its
    speed points; any other counts its own frames from its first, 0.

    Attributes:
        start: The first frame of the clip on its timeline.
        length: How many frames it lasts.
        timeline: The rate of its timeline.
        picture: How it reads the picture of its media, where it has one.
    """

    start: int
    length: int
    timeline: Rate
    picture: _Picture | None

    @classmethod
    def of(cls, clip: Mapping[str, Any], rate: Rate, media: Mapping[str, Any] | None) -> "KeyClock":
        """The clock of a clip as the engine writes it, on a timeline of ``rate``.

        ``media`` is the media that the clip names as the project has it, ``None`` for a clip
        of a title or a colour, or of media that the project does not have.

        Raises:
            ValueError: It is not a clip of the engine.
        """
        read = placed(clip)
        video = _object(_object(_object(media).get("facts")).get("video"))
        picture = None
        if read is not None and video and not video.get("still"):
            picture = _Picture(
                begins=read.begins.ticks,
                speed=read.speed,
                reverse=read.reverse,
                rate=Rate.model_validate(video["rate"]),
                duration=Time.model_validate(_object(media)["facts"]["duration"]),
                course=read.course,
            )
        return cls(int(clip["start"]), int(clip["length"]), rate, picture)

    @property
    def counts(self) -> str:
        """What its frames are frames of, in words."""
        return "the clip's media" if self.picture is not None else "the clip"

    def at(self, frame: int) -> int:
        """The frame of the keys at ``frame`` of the timeline, as ``KeyClock::at`` finds it.

        The place in the media that the clip shows there (``span::source_tick``, in the scale
        of the media's duration), then the frame of the media's rate that the place is in
        (``span::media_frame``); or the frame of the clip from its first. ``frame`` may be
        before the clip's first or after its last, as a transition shows it.
        """
        k = frame - self.start
        picture = self.picture
        if picture is None:
            return k
        scale = picture.duration.scale
        if picture.course is not None:
            # Along its speed points: one picture through a freeze, and earlier ones frame by
            # frame where a segment plays backward.
            tick = picture.course.tick(picture.begins, k, self.timeline, scale)
            return media_frame(tick, picture.rate, picture.duration)
        shown = self.length - 1 - k if picture.reverse else k
        tick = picture.begins + ticks_of_scaled(shown, picture.speed, self.timeline, scale)
        return media_frame(tick, picture.rate, picture.duration)

    def shown(self, key: int) -> int | None:
        """The first frame of the timeline at which the clip is at the frame ``key`` of its keys.

        ``None`` where it is at it on none of its frames: a picture that a trim left out, or one
        that a clip faster than its media passes over.
        """
        first, after = self.start, self.start + self.length
        if after <= first:
            return None
        course = None if self.picture is None else self.picture.course
        if course is None:
            return self._shown_in(first, after, key)
        # A clip with speed points may rise in one segment and fall in the next: the segments
        # are asked in their order, and the first that is at the key has the first frame.
        for segment in course.segments:
            found = self._shown_in(first + segment.start, first + segment.end, key)
            if found is not None:
                return found
        return None

    def _shown_in(self, first: int, after: int, key: int) -> int | None:
        """:meth:`shown` among the frames from ``first`` to before ``after``, which go one way."""
        # The frames of keys only rise along them, or only fall where they play backward.
        rising = self.at(after - 1) >= self.at(first)
        low, high = first, after
        while low < high:
            middle = (low + high) // 2
            there = self.at(middle)
            if (there >= key) if rising else (there <= key):
                high = middle
            else:
                low = middle + 1
        return low if low < after and self.at(low) == key else None


# ---------------------------------------------------------------------------------------------
# The clip and its setting in a project
# ---------------------------------------------------------------------------------------------


def _found(timeline: Mapping[str, Any], id: str) -> tuple[dict[str, Any], dict[str, Any]] | None:
    """A clip of a timeline and the track that it is on."""
    for track in map(_object, _items(timeline.get("tracks"))):
        for clip in map(_object, _items(track.get("clips"))):
            if clip.get("id") == id:
                return track, clip
    return None


def _picture_clip(timeline: Mapping[str, Any], id: str) -> dict[str, Any]:
    found = _found(timeline, id)
    if found is None:
        raise KeysError(
            f"there is no clip {id} on timeline {timeline.get('id')}; video_info lists the clips"
        )
    track, clip = found
    if track.get("kind") != "video":
        raise KeysError(
            f"{id} is a clip of sound on {track.get('id')}; a transform and a crop are of the "
            "clips of picture tracks"
        )
    return clip


def _clock(
    project: Mapping[str, Any], timeline: Mapping[str, Any], clip: Mapping[str, Any]
) -> KeyClock:
    named = _object(clip.get("source")).get("media")
    media = next(
        (one for one in map(_object, _items(project.get("media"))) if one.get("id") == named),
        None,
    )
    return KeyClock.of(clip, Rate.model_validate(timeline["rate"]), media)


def _is_title(clip: Mapping[str, Any]) -> bool:
    return _object(clip.get("source")).get("kind") == "title"


def _now(clip: Mapping[str, Any], path: str, spec: Number) -> Any:
    """What the setting at ``path`` of a clip is: a number, or keys."""
    of, name = path.split(".")
    if of == "title":
        # The style of a title is in its source, and not in the clip beside its transform.
        return _object(_object(clip.get("source")).get("style")).get(name, spec.default)
    return _object(clip.get(of)).get(name, spec.default)


def _keys(now: Any) -> list[dict[str, Any]]:
    return [_object(key) for key in _items(_object(now).get("keys"))]


def _told(now: Any, clock: KeyClock) -> Any:
    """A setting as it is told: each of its keys with the frame of the timeline that shows it."""
    if not isinstance(now, Mapping):
        return now
    return {"keys": [{"timeline_frame": clock.shown(key["frame"]), **key} for key in _keys(now)]}


def _where(keys: Sequence[Mapping[str, Any]], clock: KeyClock) -> str:
    """Where the keys of a setting are, for whoever named a frame that has none."""
    if not keys:
        return "it has no keys"
    places = [(clock.shown(key["frame"]), key["frame"]) for key in keys]
    seen = ", ".join(str(shown) for shown, _ in places if shown is not None)
    unseen = ", ".join(str(frame) for shown, frame in places if shown is None)
    said = [f"its keys are at frames {seen} of the timeline"] if seen else []
    if unseen:
        said.append(
            f"its keys at frames {unseen} of {clock.counts} are shown at no frame of the clip "
            "(video_edit's splice_clip_keys takes one away by its place among the keys)"
        )
    return "; ".join(said)


def _clip_ops(timeline: str, clip: str, path: str) -> KeyOps:
    """The operations that write the keys of the setting at ``path`` of a clip."""
    return KeyOps(
        number=lambda param: edits.set_clip_keys(timeline, clip, path, param),
        splice=lambda at, remove, keys: edits.splice_clip_keys(
            timeline, clip, path, at, remove, [edits.Key.model_validate(key) for key in keys]
        ),
    )


# ---------------------------------------------------------------------------------------------
# What the tool does
# ---------------------------------------------------------------------------------------------


async def keys(
    engine: Engine,
    timeline: str | None,
    clip: str,
    path: str,
    action: KeyAction,
    *,
    frame: int | None = None,
    value: float | None = None,
    interp: Any = None,
) -> dict[str, Any]:
    """Put a key of one setting of a clip's picture, take one away, or clear them all.

    ``path`` is one of :data:`SETTINGS`; ``title.size``, ``title.x`` and ``title.y`` are of a
    clip whose source is a title. ``frame`` is a frame of the timeline, within the clip; the key
    is at the frame of the clip's keys that the clip is at there (:class:`KeyClock`). The other
    keys of the setting are not sent: a key put at a frame that has one takes its place, and
    keeps its interpolation where none is given; the last key taken away leaves its value as a
    number; ``clear`` leaves ``value``, or the value of the first key.

    Returns:
        The revision made; ``key``, the key that was put or taken away, by the frame of the
        timeline that was given and its own frame; what its own frame counts (``key_frames``);
        and the setting as it is now, each of its keys with the first frame of the timeline
        that shows it (``timeline_frame``, ``None`` where none does).

    Raises:
        KeysError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the change.
    """
    spec = setting_of(path)
    if action != "clear" and frame is None:
        raise KeysError(f"{action} needs frame, the frame of the timeline that the key is at")
    if action == "put" and value is None:
        raise KeysError(f"put needs value, what {path} is at the key ({spec.told()})")
    if action != "remove" and value is not None and not spec.low <= value <= spec.high:
        raise KeysError(f"{path} is {value:g}, out of range ({spec.told()})")
    wrote: list[tuple[str, int | None]] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = timeline_of(project, timeline)
        found = _picture_clip(on, clip)
        if path.startswith("title.") and not _is_title(found):
            raise KeysError(
                f"{clip} is not a title; {path} is a setting of the style of a title, and the "
                "transform and the crop are of every clip of picture"
            )
        clock = _clock(project, on, found)
        now = _now(found, path, spec)
        at = None
        if action != "clear" and frame is not None:
            last = clock.start + clock.length - 1
            if not clock.start <= frame <= last:
                raise KeysError(
                    f"frame {frame} is not in clip {clip}, which is at frames {clock.start} to "
                    f"{last} of timeline {on['id']}; frame is a frame of the timeline"
                )
            at = clock.at(frame)
            if action == "remove" and all(key.get("frame") != at for key in _keys(now)):
                raise KeysError(
                    f"{path} of clip {clip} has no key at frame {frame} of the timeline (frame "
                    f"{at} of {clock.counts}): {_where(_keys(now), clock)}"
                )
        wrote[:] = [(str(on["id"]), at)]
        ops = _clip_ops(str(on["id"]), clip, path)
        return [key_op(ops, path, (spec, now), action, frame=at, value=value, interp=interp)]

    try:
        done, project = await applied(engine, build)
    except GradeError as error:
        raise KeysError(str(error)) from None
    ((on, at),) = wrote
    told: dict[str, Any] = {
        "revision": done.revision,
        "operations": len(done.done),
        "timeline": on,
        "clip": clip,
        "path": path,
    }
    if at is not None:
        told["key"] = {"timeline_frame": frame, "frame": at}
    timelines = map(_object, _items(project.get("timelines")))
    after = _object(next((one for one in timelines if one.get("id") == on), None))
    found = _found(after, clip)
    if found is not None:  # Another client may have taken the clip away meanwhile.
        clock = _clock(project, after, found[1])
        told["key_frames"] = f"frames of {clock.counts}, from 0"
        told["setting"] = _told(_now(found[1], path, spec), clock)
    return told
