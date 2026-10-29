"""Edits and operations of a project of the engine, written as the engine reads them.

**Edits** are what an editor does to a timeline, named as DaVinci Resolve names them: insert,
overwrite, replace, place on top, append, ripple delete, blade, the four trims, transitions,
speed and subtitles. The engine works an edit out into operations at the moment it is applied,
by its own rules: which tracks a ripple moves, what is locked, how far a trim may go. None of
those rules is worked out here; ``engine/crates/model/src/edit`` of the repository has them, and
``engine/PROTOCOL.md`` says them. A function here only writes the object of JSON that
``transaction.apply`` takes for one edit, ``{"edit": "ripple_delete", ...}``, so that what is
sent has the form that the engine reads.

**Operations** are the changes that the history of the project is made of; most are written by
the engine as it works edits out, and a client writes few of them itself: a timeline or a bin
added, the settings of a timeline, a track, a clip or media set. Those are here too, as
``{"op": "set_clip", ...}``.

**Values.** Times in media are :class:`Time`, ticks of the scale of the duration of the media.
Times on a timeline are whole frames. A field that the engine may be given as ``null`` and may
also be left out, such as the track of an insert, is left out where it is :data:`LEFT_OUT`.

An object written here may be given to :meth:`columbina.video.engine.Engine.apply` beside others,
as one transaction.
"""

import enum
from collections.abc import Sequence
from fractions import Fraction
from typing import Any, Final, Literal

from pydantic import Field

from columbina.data import Data

__all__ = [
    "LEFT_OUT",
    "Alignment",
    "ColorSource",
    "CueItem",
    "Edge",
    "MediaSource",
    "Rate",
    "Source",
    "Speed",
    "TitleSource",
    "TitleStyle",
    "TrackKind",
    "Transform",
    "TransitionKind",
    "add_bin",
    "add_cue",
    "add_timeline",
    "add_track",
    "add_transition",
    "append",
    "blade",
    "close_gap",
    "cues_from",
    "insert",
    "insert_move",
    "lift",
    "move",
    "of_color",
    "of_media",
    "of_title",
    "overwrite",
    "place_on_top",
    "remove_bin",
    "remove_cue",
    "remove_media",
    "remove_timeline",
    "remove_track",
    "remove_transition",
    "replace",
    "ripple_delete",
    "ripple_delete_range",
    "ripple_overwrite",
    "roll",
    "set_bin",
    "set_clip",
    "set_cue",
    "set_media",
    "set_speed",
    "set_timeline",
    "set_track",
    "set_transition",
    "slide",
    "slip",
    "smart_insert",
    "track",
    "trim",
]

TrackKind = Literal["video", "audio", "subtitle"]
"""What a track holds: picture, sound or subtitles."""

TransitionKind = Literal["dissolve", "dip_black", "dip_white", "crossfade"]
"""How one clip gives way to another: the first three on tracks of picture, ``crossfade`` on
tracks of sound."""

Alignment = Literal["start", "center", "end"]
"""Where a transition lies about its cut."""

Edge = Literal["in", "out"]
"""An edge of a clip: its start or its end."""


class _LeftOut(enum.Enum):
    LEFT_OUT = "left out"


LEFT_OUT: Final = _LeftOut.LEFT_OUT
"""A field that is not written at all, as distinct from one written as ``null``. The track of an
edit that is left out is the first track of its kind, and one that is ``None`` is not used; a
setting of an operation that is left out stays as it is."""

_Kept = Literal[_LeftOut.LEFT_OUT]


class Time(Data):
    """A place in media: ``ticks`` of ``1 / scale`` of a second, in the scale of its duration.

    Attributes:
        ticks: How many ticks.
        scale: How many ticks make a second.
    """

    ticks: int
    scale: int = Field(gt=0)

    @classmethod
    def at(cls, us: int, scale: int) -> "Time":
        """The place ``us`` microseconds in, in ticks of ``scale``, rounded down."""
        return cls(ticks=us * scale // 1_000_000, scale=scale)


class Rate(Data):
    """A rate of frames: ``num / den`` frames a second, as ``30000/1001``.

    Attributes:
        num: The frames.
        den: The seconds that they take.
    """

    num: int = Field(gt=0)
    den: int = Field(gt=0)


class Speed(Data):
    """How fast a clip plays its source: ``num / den`` times as fast as it was recorded.

    Attributes:
        num: The numerator.
        den: The denominator.
    """

    num: int = Field(gt=0)
    den: int = Field(gt=0)

    @classmethod
    def of(cls, speed: Fraction | int) -> "Speed":
        """The speed of a fraction, as ``Fraction(1, 2)`` for half the speed."""
        speed = Fraction(speed)
        return cls(num=speed.numerator, den=speed.denominator)


class TitleStyle(Data):
    """How the text of a title is drawn.

    Attributes:
        font: The family of the font, or empty for the one that the engine chooses.
        size: The height of its letters in pixels of the timeline.
        color: The colour of its letters, as ``#rrggbbaa``.
        x: Where its anchor is across the picture: 0 at the left edge, 1 at the right.
        y: Where its anchor is down the picture: 0 at the top edge, 1 at the bottom.
        align: Which side of the text the anchor is on.
    """

    font: str = ""
    size: int = Field(default=72, ge=1)
    color: str = Field(default="#ffffffff", pattern=r"^#[0-9a-f]{8}$")
    x: float = 0.5
    y: float = 0.5
    align: Literal["left", "center", "right"] = "center"


class Transform(Data):
    """Where the picture of a clip is put on the picture of its timeline.

    Attributes:
        x: How far it is moved right, as a share of the width of the timeline.
        y: How far it is moved down, as a share of the height.
        scale: How much larger it is made, about its middle.
        rotation: How far it is turned clockwise, in degrees.
        opacity: How opaque it is: from 0, not seen, to 1.
    """

    x: float = 0.0
    y: float = 0.0
    scale: float = 1.0
    rotation: float = 0.0
    opacity: float = 1.0


class MediaSource(Data):
    """Media that an edit puts on a timeline.

    From ``from_`` to ``to``: from its mark in, or its start, to its mark out, or its end, where
    they are not given; or for ``length`` frames from ``from_``.

    Attributes:
        media: The media.
        from_: Where in it the clip begins.
        to: Where in it the clip ends.
        length: How many frames the clip lasts, if not as far as ``to``.
    """

    kind: Literal["media"] = "media"
    media: str
    from_: Time | None = Field(default=None, alias="from")
    to: Time | None = None
    length: int | None = None


class TitleSource(Data):
    """A title that an edit puts on a track of picture.

    Attributes:
        text: What it says.
        style: How it is drawn.
        length: How many frames it lasts.
    """

    kind: Literal["title"] = "title"
    text: str
    style: TitleStyle = TitleStyle()
    length: int


class ColorSource(Data):
    """A colour that an edit puts on a track of picture, filling it.

    Attributes:
        color: The colour, as ``#rrggbbaa``.
        length: How many frames it lasts.
    """

    kind: Literal["color"] = "color"
    color: str = Field(pattern=r"^#[0-9a-f]{8}$")
    length: int


Source = MediaSource | TitleSource | ColorSource
"""What an edit puts on a timeline."""


class CueItem(Data):
    """A subtitle of :func:`cues_from`.

    Attributes:
        start: The frame that it is first shown at.
        length: How many frames it is shown for.
        text: What it says.
    """

    start: int
    length: int
    text: str


def of_media(
    media: str, *, start: Time | None = None, end: Time | None = None, length: int | None = None
) -> MediaSource:
    """Media to put on a timeline: from ``start`` to ``end``, or for ``length`` frames."""
    return MediaSource.model_validate({"media": media, "from": start, "to": end, "length": length})


def of_title(text: str, length: int, style: TitleStyle | None = None) -> TitleSource:
    """A title to put on a track of picture, for ``length`` frames."""
    return TitleSource(text=text, length=length, style=style or TitleStyle())


def of_color(color: str, length: int) -> ColorSource:
    """A colour, ``#rrggbbaa``, to put on a track of picture, for ``length`` frames."""
    return ColorSource(color=color, length=length)


def _json(value: Data) -> dict[str, Any]:
    return value.model_dump(mode="json", by_alias=True, exclude_none=True)


def _given(written: dict[str, Any], **fields: object) -> dict[str, Any]:
    """Add the fields that are not left out, and those that are ``None`` as ``null``."""
    for key, value in fields.items():
        if value is LEFT_OUT:
            continue
        written[key] = _json(value) if isinstance(value, Data) else value
    return written


def _set(written: dict[str, Any], **fields: object) -> dict[str, Any]:
    """Add the settings that are given: those that are ``None`` are left as they are."""
    return _given(written, **{key: value for key, value in fields.items() if value is not None})


# Putting a source on a timeline


def _placed(
    edit: str,
    timeline: str,
    source: Source,
    at: int | None,
    video_track: str | _Kept | None,
    audio_track: str | _Kept | None,
) -> dict[str, Any]:
    written: dict[str, Any] = {"edit": edit, "timeline": timeline, "source": _json(source)}
    if at is not None:
        written["at"] = at
    return _given(written, video_track=video_track, audio_track=audio_track)


def insert(
    timeline: str,
    source: Source,
    at: int,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Insert: every track that is not locked is cut at ``at``, and what follows moves on.

    The picture of media goes on ``video_track`` and its sound on ``audio_track``, linked: a
    track that is left out is the first of its kind, and one that is ``None`` is not used.
    """
    return _placed("insert", timeline, source, at, video_track, audio_track)


def overwrite(
    timeline: str,
    source: Source,
    at: int,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Overwrite: the source goes over what is on its tracks from ``at``; nothing moves."""
    return _placed("overwrite", timeline, source, at, video_track, audio_track)


def replace(
    timeline: str,
    source: Source,
    at: int,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Replace: the clip at ``at`` shows the source instead, where it is and for as long."""
    return _placed("replace", timeline, source, at, video_track, audio_track)


def smart_insert(
    timeline: str,
    source: Source,
    at: int,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Smart insert: an insert at the cut nearest to ``at``."""
    return _placed("smart_insert", timeline, source, at, video_track, audio_track)


def ripple_overwrite(
    timeline: str,
    source: Source,
    at: int,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Ripple overwrite: the source takes the place of the clip at ``at``, for as long as it lasts.

    What follows moves by the difference.
    """
    return _placed("ripple_overwrite", timeline, source, at, video_track, audio_track)


def place_on_top(timeline: str, source: Source, at: int) -> dict[str, Any]:
    """Place on top: on the first free track above everything at ``at``, or on a new one."""
    return _placed("place_on_top", timeline, source, at, LEFT_OUT, LEFT_OUT)


def append(
    timeline: str,
    source: Source,
    *,
    video_track: str | _Kept | None = LEFT_OUT,
    audio_track: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """Append at end: after the last clip of its tracks."""
    return _placed("append", timeline, source, None, video_track, audio_track)


# Taking away, cutting and moving


def lift(timeline: str, clips: Sequence[str], *, linked: bool = True) -> dict[str, Any]:
    """Lift: the clips are taken away and leave gaps; with those linked to them, if ``linked``."""
    return {"edit": "lift", "timeline": timeline, "clips": list(clips), "linked": linked}


def ripple_delete(timeline: str, clips: Sequence[str], *, linked: bool = True) -> dict[str, Any]:
    """Ripple delete: the clips are taken away and the gaps closed, on every unlocked track."""
    return {"edit": "ripple_delete", "timeline": timeline, "clips": list(clips), "linked": linked}


def ripple_delete_range(
    timeline: str, start: int, end: int, *, tracks: Sequence[str] | None = None
) -> dict[str, Any]:
    """Frames ``start`` to ``end``, the end not included, are cut out and the gap closed.

    On the tracks given, or on every track that is not locked: what lies over the frames on
    other tracks is cut with them, so it stays in step.
    """
    written: dict[str, Any] = {
        "edit": "ripple_delete_range",
        "timeline": timeline,
        "from": start,
        "to": end,
    }
    if tracks is not None:
        written["tracks"] = list(tracks)
    return written


def close_gap(timeline: str, track: str, at: int) -> dict[str, Any]:
    """The gap that frame ``at`` of ``track`` is in is closed, on every unlocked track."""
    return {"edit": "close_gap", "timeline": timeline, "track": track, "at": at}


def blade(
    timeline: str, at: int, *, tracks: Sequence[str] | None = None, linked: bool = True
) -> dict[str, Any]:
    """Blade: the clips across ``at`` are cut in two, on the tracks given or every unlocked one."""
    written: dict[str, Any] = {"edit": "blade", "timeline": timeline, "at": at}
    if tracks is not None:
        written["tracks"] = list(tracks)
    written["linked"] = linked
    return written


def move(
    timeline: str,
    clips: Sequence[str],
    delta: int,
    *,
    to_track: str | None = None,
    overwrite: bool = False,
    linked: bool = True,
) -> dict[str, Any]:
    """The clips move ``delta`` frames later, and to ``to_track`` with the first of them.

    Where something is in the way the engine refuses with ``no_room``, unless ``overwrite``.
    """
    written: dict[str, Any] = {
        "edit": "move",
        "timeline": timeline,
        "clips": list(clips),
        "delta": delta,
    }
    if to_track is not None:
        written["to_track"] = to_track
    written.update(overwrite=overwrite, linked=linked)
    return written


def insert_move(
    timeline: str, clips: Sequence[str], at: int, *, linked: bool = True
) -> dict[str, Any]:
    """The clips are taken away, the gaps closed, and they are inserted at ``at``."""
    return {
        "edit": "insert_move",
        "timeline": timeline,
        "clips": list(clips),
        "at": at,
        "linked": linked,
    }


# Trims


def trim(
    timeline: str,
    clip: str,
    edge: Edge,
    delta: int,
    *,
    ripple: bool = False,
    linked: bool = True,
) -> dict[str, Any]:
    """An edge of a clip moves ``delta`` frames later; with ``ripple``, what follows moves too."""
    return {
        "edit": "trim",
        "timeline": timeline,
        "clip": clip,
        "edge": edge,
        "delta": delta,
        "ripple": ripple,
        "linked": linked,
    }


def roll(
    timeline: str, left: str, right: str, delta: int, *, linked: bool = True
) -> dict[str, Any]:
    """Roll: the cut between two clips that meet moves ``delta`` frames later."""
    return {
        "edit": "roll",
        "timeline": timeline,
        "left": left,
        "right": right,
        "delta": delta,
        "linked": linked,
    }


def slip(timeline: str, clip: str, delta: int, *, linked: bool = True) -> dict[str, Any]:
    """Slip: a clip shows its source ``delta`` frames later, and stays where it is."""
    return {"edit": "slip", "timeline": timeline, "clip": clip, "delta": delta, "linked": linked}


def slide(timeline: str, clip: str, delta: int, *, linked: bool = True) -> dict[str, Any]:
    """Slide: a clip moves ``delta`` frames between its neighbours, which give and take them."""
    return {"edit": "slide", "timeline": timeline, "clip": clip, "delta": delta, "linked": linked}


# Transitions and speed


def add_transition(
    timeline: str,
    left: str,
    right: str,
    length: int,
    *,
    kind: TransitionKind | None = None,
    align: Alignment = "center",
) -> dict[str, Any]:
    """A transition at the cut between two clips that meet.

    Without a ``kind``, a dissolve on a track of picture and a crossfade on one of sound.
    """
    written: dict[str, Any] = {
        "edit": "add_transition",
        "timeline": timeline,
        "left": left,
        "right": right,
    }
    if kind is not None:
        written["kind"] = kind
    written.update(length=length, align=align)
    return written


def set_transition(
    timeline: str,
    id: str,
    *,
    kind: TransitionKind | None = None,
    length: int | None = None,
    align: Alignment | None = None,
) -> dict[str, Any]:
    """The settings of a transition that are given."""
    return _set(
        {"edit": "set_transition", "timeline": timeline, "id": id},
        kind=kind,
        length=length,
        align=align,
    )


def remove_transition(timeline: str, id: str) -> dict[str, Any]:
    """A transition is taken away."""
    return {"edit": "remove_transition", "timeline": timeline, "id": id}


def set_speed(
    timeline: str,
    clip: str,
    speed: Speed,
    *,
    reverse: bool | None = None,
    ripple: bool = False,
    linked: bool = True,
) -> dict[str, Any]:
    """A clip plays the same stretch of its source at another speed.

    So it lasts more or fewer frames; with ``ripple`` what follows moves with its end.
    ``reverse`` is left as it is where it is ``None``.
    """
    written: dict[str, Any] = {
        "edit": "set_speed",
        "timeline": timeline,
        "clip": clip,
        "speed": _json(speed),
    }
    if reverse is not None:
        written["reverse"] = reverse
    written.update(ripple=ripple, linked=linked)
    return written


# Subtitles and tracks


def add_cue(timeline: str, track: str, start: int, length: int, text: str) -> dict[str, Any]:
    """A subtitle on a track of subtitles, shown from frame ``start`` for ``length`` frames."""
    return {
        "edit": "add_cue",
        "timeline": timeline,
        "track": track,
        "start": start,
        "length": length,
        "text": text,
    }


def set_cue(
    timeline: str,
    id: str,
    *,
    start: int | None = None,
    length: int | None = None,
    text: str | None = None,
) -> dict[str, Any]:
    """The settings of a subtitle that are given."""
    return _set(
        {"edit": "set_cue", "timeline": timeline, "id": id}, start=start, length=length, text=text
    )


def remove_cue(timeline: str, id: str) -> dict[str, Any]:
    """A subtitle is taken away."""
    return {"edit": "remove_cue", "timeline": timeline, "id": id}


def cues_from(timeline: str, track: str, items: Sequence[CueItem]) -> dict[str, Any]:
    """These subtitles go on a track of subtitles, in place of all that are on it."""
    return {
        "edit": "cues_from",
        "timeline": timeline,
        "track": track,
        "items": [_json(item) for item in items],
    }


def add_track(
    timeline: str,
    kind: TrackKind,
    *,
    index: int | None = None,
    name: str | None = None,
    language: str | None = None,
) -> dict[str, Any]:
    """An empty track: after the last of its kind, unless ``index`` places it among them."""
    return _set(
        {"edit": "add_track", "timeline": timeline, "kind": kind},
        index=index,
        name=name,
        language=language,
    )


def remove_track(timeline: str, track: str) -> dict[str, Any]:
    """A track is taken away, with everything on it."""
    return {"edit": "remove_track", "timeline": timeline, "track": track}


# Operations


def track(kind: TrackKind, *, id: str = "", name: str = "", language: str = "") -> dict[str, Any]:
    """An empty track, as :func:`add_timeline` takes it: the engine names one with no ``id``."""
    written: dict[str, Any] = {
        "id": id,
        "kind": kind,
        "name": name,
        "enabled": True,
        "locked": False,
    }
    if kind == "subtitle":
        written.update(language=language, cues=[])
    else:
        written.update(clips=[], transitions=[])
    return written


def add_timeline(
    name: str,
    rate: Rate,
    *,
    id: str = "",
    width: int = 1920,
    height: int = 1080,
    sample_rate: int = 48000,
    tracks: Sequence[dict[str, Any]] = (),
) -> dict[str, Any]:
    """A timeline, with the tracks of :func:`track`. Its ``id`` may be a ``$name``."""
    return {
        "op": "add_timeline",
        "timeline": {
            "id": id,
            "name": name,
            "rate": _json(rate),
            "width": width,
            "height": height,
            "sample_rate": sample_rate,
            "tracks": list(tracks),
        },
    }


def remove_timeline(id: str) -> dict[str, Any]:
    """A timeline is taken away, with everything on it."""
    return {"op": "remove_timeline", "id": id}


def set_timeline(
    id: str,
    *,
    name: str | None = None,
    rate: Rate | None = None,
    width: int | None = None,
    height: int | None = None,
    sample_rate: int | None = None,
) -> dict[str, Any]:
    """The settings of a timeline that are given."""
    return _set(
        {"op": "set_timeline", "id": id},
        name=name,
        rate=rate,
        width=width,
        height=height,
        sample_rate=sample_rate,
    )


def set_track(
    timeline: str,
    id: str,
    *,
    name: str | None = None,
    enabled: bool | None = None,
    locked: bool | None = None,
    language: str | None = None,
) -> dict[str, Any]:
    """The settings of a track that are given."""
    return _set(
        {"op": "set_track", "timeline": timeline, "id": id},
        name=name,
        enabled=enabled,
        locked=locked,
        language=language,
    )


def set_clip(
    timeline: str,
    id: str,
    *,
    enabled: bool | None = None,
    link: str | _Kept | None = LEFT_OUT,
    transform: Transform | None = None,
    gain_db: float | None = None,
    pan: float | None = None,
    fade_in: int | None = None,
    fade_out: int | None = None,
    source: TitleSource | ColorSource | None = None,
) -> dict[str, Any]:
    """The settings of a clip that are given.

    ``link`` is the group of linked clips that it is in: ``None`` takes it out of its group, and
    an empty id puts it in a new one. A clip of a title or a colour may be given another
    ``source`` of its kind; its length is the clip's, and that of the source is not used. Its
    speed is set with :func:`set_speed`, which keeps the stretch of its source.
    """
    written = _set(
        {"op": "set_clip", "timeline": timeline, "id": id},
        enabled=enabled,
        transform=transform,
        gain_db=gain_db,
        pan=pan,
        fade_in=fade_in,
        fade_out=fade_out,
    )
    if source is not None:
        written["source"] = {key: value for key, value in _json(source).items() if key != "length"}
    return _given(written, link=link)


def add_bin(name: str, *, id: str = "", parent: str | None = None) -> dict[str, Any]:
    """A bin of the media pool, in ``parent`` or at the root. Its ``id`` may be a ``$name``."""
    return {"op": "add_bin", "bin": {"id": id, "name": name, "parent": parent}}


def set_bin(
    id: str, *, name: str | None = None, parent: str | _Kept | None = LEFT_OUT
) -> dict[str, Any]:
    """The settings of a bin that are given: ``parent`` ``None`` puts it at the root."""
    return _given(_set({"op": "set_bin", "id": id}, name=name), parent=parent)


def remove_bin(id: str) -> dict[str, Any]:
    """A bin that holds nothing is taken away."""
    return {"op": "remove_bin", "id": id}


def set_media(
    id: str,
    *,
    name: str | None = None,
    still: bool | None = None,
    bin: str | _Kept | None = LEFT_OUT,
    mark_in: Time | _Kept | None = LEFT_OUT,
    mark_out: Time | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """The settings of media that are given: ``None`` is the root for ``bin``, none for a mark."""
    return _given(
        _set({"op": "set_media", "id": id}, name=name, still=still),
        bin=bin,
        mark_in=mark_in,
        mark_out=mark_out,
    )


def remove_media(id: str) -> dict[str, Any]:
    """Media that no clip uses is taken away from the project."""
    return {"op": "remove_media", "id": id}
