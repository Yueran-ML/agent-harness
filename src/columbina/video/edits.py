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
from collections.abc import Iterable, Mapping, Sequence
from fractions import Fraction
from typing import Any, Final, Literal

from pydantic import Field

from columbina.data import Data

__all__ = [
    "LEFT_OUT",
    "Alignment",
    "AttributeItem",
    "AudioParamName",
    "ChainOf",
    "ColorSource",
    "CrossfadeLevel",
    "CueItem",
    "Edge",
    "Key",
    "KeyInterp",
    "Keys",
    "LaneThen",
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
    "add_bus",
    "add_cue",
    "add_insert",
    "add_timeline",
    "add_track",
    "add_track_group",
    "add_transition",
    "add_vca",
    "append",
    "blade",
    "clear_lanes",
    "clip_gain_keys",
    "clip_gain_range",
    "close_gap",
    "copy_effects",
    "cues_from",
    "fade_to_playhead",
    "insert",
    "insert_move",
    "lane_draw",
    "lane_points",
    "lane_range",
    "lift",
    "move",
    "move_insert",
    "move_track",
    "nudge_clip_gain",
    "of_color",
    "of_media",
    "of_title",
    "overwrite",
    "paste_attributes",
    "place_on_top",
    "remove_attributes",
    "remove_bin",
    "remove_bus",
    "remove_cue",
    "remove_media",
    "remove_timeline",
    "remove_track",
    "remove_track_group",
    "remove_transition",
    "remove_vca",
    "replace",
    "ripple_delete",
    "ripple_delete_range",
    "ripple_overwrite",
    "roll",
    "set_bin",
    "set_bus",
    "set_clip",
    "set_clip_keys",
    "set_cue",
    "set_insert",
    "set_lane",
    "set_media",
    "set_speed",
    "set_timeline",
    "set_timeline_audio",
    "set_track",
    "set_track_audio",
    "set_track_group",
    "set_transition",
    "set_vca",
    "slide",
    "slip",
    "smart_insert",
    "splice_clip_keys",
    "track",
    "trim",
    "untangled",
]

TrackKind = Literal["video", "audio", "subtitle"]
"""What a track holds: picture, sound or subtitles."""

CrossfadeLevel = Literal[-3, 0, 3]
"""The level of a side of a crossfade in dB: -3 (``t^1.5``), 0 (linear) or +3 (equal power)."""

AudioParamName = Literal["gain_db", "pan"]
"""A setting of the sound of a clip that has keyframes: its volume or its pan."""

KeyInterp = Literal["linear", "ease_in", "ease_out"]
"""How the value goes from a keyframe of the sound of a clip to the next."""

AttributeItem = Literal["volume", "plugins", "equalizer"]
"""What Paste Attributes and Remove Attributes take of the sound of a clip."""

ChainOf = dict[str, str]
"""A chain of inserts, as ``of`` names it: ``{"track": id}``, ``{"bus": id}`` or
``{"clip": id}``; or what a lane is of: ``{"track": id}``, ``{"bus": id}`` or ``{"vca": id}``."""

LaneThen = Literal["keep", "reset", "hold"]
"""What a setting is left at once its lanes are cleared."""

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


class Key(Data):
    """A keyframe of a setting of a clip's picture.

    Attributes:
        frame: The frame of the clip's media whose picture it is at, numbered at the media's
            own rate from 0; of a title, a colour or a still, the frame of the clip from its
            first, 0. The keys of the clip's grade count the same frames.
        value: What the setting is there.
        interp: How it goes on to the next key: ``"linear"``, ``"hold"``, or
            ``{"bezier": {"out": [t, dv], "in": [t, dv]}}``.
    """

    frame: int
    value: float
    interp: Literal["linear", "hold"] | dict[str, Any] = "linear"


class Keys(Data):
    """A setting with keyframes, in the order of their frames, one at least."""

    keys: list[Key]


class Transform(Data):
    """Where the picture of a clip is put on the picture of its timeline.

    Each setting is a number, or :class:`Keys`.

    Attributes:
        x: How far it is moved right, as a share of the width of the timeline.
        y: How far it is moved down, as a share of the height.
        scale: How much larger it is made, about its middle.
        rotation: How far it is turned clockwise, in degrees.
        opacity: How opaque it is: from 0, not seen, to 1.
    """

    x: float | Keys = 0.0
    y: float | Keys = 0.0
    scale: float | Keys = 1.0
    rotation: float | Keys = 0.0
    opacity: float | Keys = 1.0


class MediaSource(Data):
    """Media that an edit puts on a timeline.

    From ``from_`` to ``to``, ``to`` not included: from its mark in, or its start, to its mark
    out, or its end, where they are not given; or for ``length`` frames from ``from_``. A mark out
    is where the range marked ends, the start of the frame after its Out frame, so that the Out
    frame is in it, as DaVinci Resolve's Out point is the last frame of the range.

    Attributes:
        media: The media.
        from_: Where in it the clip begins.
        to: Where in it the clip ends: the start of the frame after its last.
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


def close_gap(
    timeline: str,
    track: str,
    at: int,
    *,
    straddling: Literal["refuse", "stay"] | None = None,
) -> dict[str, Any]:
    """The gap that frame ``at`` of ``track`` is in is closed, on every unlocked track.

    Where a clip of another unlocked track lies across a frame of the gap, the engine refuses
    with ``ripple_blocked`` (``straddling="refuse"``, the default, which is not written). With
    ``straddling="stay"`` such clips stay where they are, as the Cut page closes up its Track 1:
    what starts at the end of the gap or later still moves back on every unlocked track, and the
    edit is refused with ``ripple_blocked`` only where a clip would then run into one that stays.
    """
    written: dict[str, Any] = {"edit": "close_gap", "timeline": timeline, "track": track, "at": at}
    if straddling == "stay":
        written["straddling"] = straddling
    return written


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

    Every clip that moves, the linked ones too, goes as many tracks along its own kind as
    ``to_track`` is from the track of the first: the sound of a picture moved from V1 to V2 goes
    from A1 to A2. No track is made: the engine refuses with ``no_track`` where a clip has no track
    to go to, and with ``locked`` where it would go on a locked one. Where something is in the way
    it refuses with ``no_room``, unless ``overwrite``.
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
    fade_in_level_db: CrossfadeLevel | None = None,
    fade_out_level_db: CrossfadeLevel | None = None,
) -> dict[str, Any]:
    """The settings of a transition that are given.

    ``fade_in_level_db`` and ``fade_out_level_db`` are the levels of the two sides of a crossfade
    only: -3, 0 or 3 dB.
    """
    return _set(
        {"edit": "set_transition", "timeline": timeline, "id": id},
        kind=kind,
        length=length,
        align=align,
        fade_in_level_db=fade_in_level_db,
        fade_out_level_db=fade_out_level_db,
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


def untangled(items: Iterable[CueItem]) -> list[CueItem]:
    """Subtitles in the order in which they are shown, none of them over the next.

    Each ends where the next begins, if it would end later, and one then shown for no frame is
    left out: the engine refuses subtitles of a track that overlap.
    """
    shown = sorted(items, key=lambda one: one.start)
    kept: list[CueItem] = []
    for place, one in enumerate(shown):
        end = one.start + one.length
        if place + 1 < len(shown):
            end = min(end, shown[place + 1].start)
        if end > one.start:
            kept.append(CueItem(start=one.start, length=end - one.start, text=one.text))
    return kept


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


def track(
    kind: TrackKind,
    *,
    id: str = "",
    name: str = "",
    language: str = "",
    enabled: bool = True,
) -> dict[str, Any]:
    """An empty track, as :func:`add_timeline` takes it: the engine names one with no ``id``.

    A timeline shows one subtitle track at a time: of several subtitle tracks, at most one is
    ``enabled``, and the engine refuses a timeline with more as ``subtitle_shown``.
    """
    written: dict[str, Any] = {
        "id": id,
        "kind": kind,
        "name": name,
        "enabled": enabled,
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
    mute: bool | None = None,
    solo: bool | None = None,
    solo_safe: bool | None = None,
    color: str | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """The settings of a track that are given.

    ``mute``, ``solo`` and ``solo_safe`` are of a track of sound only: ``enabled`` ``False`` does
    not read it at all, and ``mute`` silences it.
    """
    written = _set(
        {"op": "set_track", "timeline": timeline, "id": id},
        name=name,
        enabled=enabled,
        locked=locked,
        language=language,
        mute=mute,
        solo=solo,
        solo_safe=solo_safe,
    )
    if color is not LEFT_OUT:
        written["color"] = color
    return written


def move_track(timeline: str, id: str, index: int) -> dict[str, Any]:
    """Move an existing track to its final zero-based index in the complete track list."""
    return {"op": "move_track", "timeline": timeline, "id": id, "index": index}


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
    audio_fade_in: Mapping[str, Any] | _Kept | None = LEFT_OUT,
    audio_fade_out: Mapping[str, Any] | _Kept | None = LEFT_OUT,
    eq: Mapping[str, Any] | None = None,
    effects: Sequence[Mapping[str, Any]] | None = None,
    pitch_correction: bool | None = None,
    pitch: Mapping[str, Any] | None = None,
    channels: Sequence[Mapping[str, Any]] | _Kept | None = LEFT_OUT,
) -> dict[str, Any]:
    """The settings of a clip that are given.

    ``link`` is the group of linked clips that it is in: ``None`` takes it out of its group, and
    an empty id puts it in a new one. A clip of a title or a colour may be given another
    ``source`` of its kind; its length is the clip's, and that of the source is not used. Its
    speed is set with :func:`set_speed`, which keeps the stretch of its source.

    Of its sound (schema 4): ``audio_fade_in`` and ``audio_fade_out`` (an ``AudioFade``
    ``{"length", "curve", "skew"}``, or ``None`` for a straight fade over the frames), ``eq``
    (whole, 4 bands), ``effects`` (whole), ``pitch_correction``, ``pitch``, and ``channels`` (as
    many maps as its track has channels, or ``None`` for the media's channels mixed as libav
    mixes them).
    """
    written = _set(
        {"op": "set_clip", "timeline": timeline, "id": id},
        enabled=enabled,
        transform=transform,
        gain_db=gain_db,
        pan=pan,
        fade_in=fade_in,
        fade_out=fade_out,
        eq=None if eq is None else dict(eq),
        effects=None if effects is None else [dict(one) for one in effects],
        pitch_correction=pitch_correction,
        pitch=None if pitch is None else dict(pitch),
    )
    if source is not None:
        written["source"] = {key: value for key, value in _json(source).items() if key != "length"}
    sound = {"audio_fade_in": audio_fade_in, "audio_fade_out": audio_fade_out, "channels": channels}
    for key, value in sound.items():
        if isinstance(value, Mapping):
            written[key] = dict(value)
        elif value is not LEFT_OUT and value is not None:
            written[key] = [dict(one) for one in value]
        elif value is None:
            written[key] = None
    return _given(written, link=link)


def set_clip_keys(timeline: str, id: str, path: str, param: float | Keys) -> dict[str, Any]:
    """A number, or keys, in place of a setting of a clip that takes keys.

    ``path`` names the setting in the clip: ``transform.x``, ``transform.y``,
    ``transform.scale``, ``transform.rotation`` or ``transform.opacity``.
    """
    written = _json(param) if isinstance(param, Keys) else param
    return {"op": "set_clip_keys", "timeline": timeline, "id": id, "path": path, "param": written}


def splice_clip_keys(
    timeline: str, id: str, path: str, at: int, remove: int, keys: Sequence[Key]
) -> dict[str, Any]:
    """``keys`` in place of ``remove`` keys of a setting of a clip, from its key ``at`` on.

    A key is added, moved, changed or taken away without the others being sent. A setting that
    is a number becomes the keys that are put; a setting keeps one key at least.
    """
    return {
        "op": "splice_clip_keys",
        "timeline": timeline,
        "id": id,
        "path": path,
        "at": at,
        "remove": remove,
        "keys": [_json(key) for key in keys],
    }


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
    """The settings of media that are given: ``None`` is the root for ``bin``, none for a mark.

    ``mark_out`` is where the range marked ends: to mark frame ``n`` as the Out, give the start
    of frame ``n + 1``, and the Out frame is in the range, as in DaVinci Resolve.
    """
    return _given(
        _set({"op": "set_media", "id": id}, name=name, still=still),
        bin=bin,
        mark_in=mark_in,
        mark_out=mark_out,
    )


def remove_media(id: str) -> dict[str, Any]:
    """Media that no clip uses is taken away from the project."""
    return {"op": "remove_media", "id": id}


# The sound of a timeline (schema 4, the Fairlight page)
#
# A patch is merged into the object that it names as the engine merges it: each object in it into
# the object of that name, any other value (a number, a list, ``None``) in place of the one there.
# Times of lanes are times of the timeline, :class:`Time` in any scale (that of its samples, say);
# keyframes of the sound of a clip are times of its media.


def set_track_audio(
    timeline: str, track: str, patch: Mapping[str, Any], *, follow_groups: bool = True
) -> dict[str, Any]:
    """A patch of the sound of a track.

    It names its ``format``, ``channels``, ``order``, ``mute``, ``solo``, ``solo_safe``, ``adc``,
    ``automation_arm`` and ``strip`` (``fader_db`` -100 to 10, where -100 or below is -inf,
    ``pan``, ``eq``, ``dynamics``, ``sends``, ``outputs``, ``inserts``, ``track_fx``). The other
    tracks of an active group that it is in follow, unless ``follow_groups`` is ``False``.
    """
    written: dict[str, Any] = {
        "edit": "set_track_audio",
        "timeline": timeline,
        "track": track,
        "patch": dict(patch),
    }
    if not follow_groups:
        written["follow_groups"] = False
    return written


def add_bus(timeline: str, bus: Mapping[str, Any], *, index: int | None = None) -> dict[str, Any]:
    """A bus of a timeline; one with an empty ``id`` is given the next ``B<n>``.

    A bus that outputs to no bus is a main output.
    """
    return _set({"op": "add_bus", "timeline": timeline, "bus": dict(bus)}, index=index)


def set_bus(timeline: str, id: str, patch: Mapping[str, Any]) -> dict[str, Any]:
    """A patch of a bus; soloed while the solo is exclusive, it takes the solo off the others."""
    return {"edit": "set_bus", "timeline": timeline, "id": id, "patch": dict(patch)}


def remove_bus(timeline: str, id: str) -> dict[str, Any]:
    """A bus that nothing outputs, sends or listens to is taken away."""
    return {"op": "remove_bus", "timeline": timeline, "id": id}


def add_vca(timeline: str, vca: Mapping[str, Any], *, index: int | None = None) -> dict[str, Any]:
    """A VCA of a timeline; one with an empty ``id`` is given the next ``VCA<n>``."""
    return _set({"op": "add_vca", "timeline": timeline, "vca": dict(vca)}, index=index)


def set_vca(timeline: str, id: str, patch: Mapping[str, Any]) -> dict[str, Any]:
    """A patch of a VCA."""
    return {"op": "set_vca", "timeline": timeline, "id": id, "patch": dict(patch)}


def remove_vca(timeline: str, id: str) -> dict[str, Any]:
    """A VCA that nothing is a member of is taken away."""
    return {"op": "remove_vca", "timeline": timeline, "id": id}


def add_track_group(
    timeline: str, group: Mapping[str, Any], *, index: int | None = None
) -> dict[str, Any]:
    """A group of tracks of sound, ``{"id": "", "name", "members", "linked"}``."""
    return _set({"op": "add_track_group", "timeline": timeline, "group": dict(group)}, index=index)


def set_track_group(timeline: str, id: str, patch: Mapping[str, Any]) -> dict[str, Any]:
    """A patch of a group of tracks."""
    return {"op": "set_track_group", "timeline": timeline, "id": id, "patch": dict(patch)}


def remove_track_group(timeline: str, id: str) -> dict[str, Any]:
    """A group of tracks is taken away; its tracks stay."""
    return {"op": "remove_track_group", "timeline": timeline, "id": id}


def set_timeline_audio(timeline: str, patch: Mapping[str, Any]) -> dict[str, Any]:
    """A patch of the settings of the sound of a timeline.

    It names ``follows_edit``, the monitored bus, the meters and the loudness.
    """
    return {"op": "set_timeline_audio", "timeline": timeline, "patch": dict(patch)}


def add_insert(
    timeline: str, of: ChainOf, insert: Mapping[str, Any], *, index: int | None = None
) -> dict[str, Any]:
    """An insert put in a chain at ``index``, or at its end.

    It is written ``{"effect": {"kind": "limiter", ...}}``; one of a strip with an empty id is
    given the next ``fx<n>`` of the strip.
    """
    return _set(
        {"op": "add_insert", "timeline": timeline, "of": dict(of), "insert": dict(insert)},
        index=index,
    )


def set_insert(
    timeline: str, of: ChainOf, slot: int, insert: Mapping[str, Any] | None
) -> dict[str, Any]:
    """The insert at ``slot`` of a chain replaced, or taken away (``None``), with its lanes."""
    written = None if insert is None else dict(insert)
    return {"edit": "set_insert", "timeline": timeline, "of": dict(of), "slot": slot,
            "insert": written}  # fmt: skip


def move_insert(
    timeline: str, of: ChainOf, from_: int, to: int, *, to_of: ChainOf | None = None
) -> dict[str, Any]:
    """The insert at ``from_`` of a chain moved to ``to``, of the same chain or of ``to_of``."""
    written: dict[str, Any] = {
        "edit": "move_insert", "timeline": timeline, "of": dict(of), "from": from_, "to": to,
    }  # fmt: skip
    if to_of is not None:
        written["to_of"] = dict(to_of)
    return written


def copy_effects(timeline: str, from_: ChainOf, to: Sequence[ChainOf]) -> dict[str, Any]:
    """The inserts of one chain put in place of all those of others."""
    return {"edit": "copy_effects", "timeline": timeline, "from": dict(from_),
            "to": [dict(one) for one in to]}  # fmt: skip


def clip_gain_keys(
    timeline: str,
    clip: str,
    action: Literal["add", "remove", "move", "set_interp"],
    at: Time,
    *,
    param: AudioParamName = "gain_db",
    value: float | None = None,
    to: Time | None = None,
    interp: KeyInterp | None = None,
) -> dict[str, Any]:
    """A keyframe of the volume (or the pan) of a clip, named by its time ``at`` in its media.

    It is added (at the value there where ``value`` is not given), taken away, moved ``to``
    another time, or given another way to the next.
    """
    written = {"edit": "clip_gain_keys", "timeline": timeline, "clip": clip, "action": action,
               "at": _json(at)}  # fmt: skip
    if param != "gain_db":
        written["param"] = param
    return _set(written, value=value, to=to, interp=interp)


def clip_gain_range(
    timeline: str, clip: str, from_: Time, to: Time, delta_db: float
) -> dict[str, Any]:
    """The volume of a clip raised or lowered between two times of its media."""
    return {"edit": "clip_gain_range", "timeline": timeline, "clip": clip, "from": _json(from_),
            "to": _json(to), "delta_db": delta_db}  # fmt: skip


def nudge_clip_gain(
    timeline: str, clips: Sequence[str], delta_db: float, *, linked: bool = True
) -> dict[str, Any]:
    """The volume of clips of sound moved by ``delta_db``, keyframes and all."""
    return {"edit": "nudge_clip_gain", "timeline": timeline, "clips": list(clips),
            "delta_db": delta_db, "linked": linked}  # fmt: skip


def paste_attributes(
    timeline: str, from_: str, to: Sequence[str], items: Sequence[AttributeItem]
) -> dict[str, Any]:
    """The volume (keyframes and all), the effects or the equalizer of a clip put on others."""
    return {"edit": "paste_attributes", "timeline": timeline, "from": from_, "to": list(to),
            "items": list(items)}  # fmt: skip


def remove_attributes(
    timeline: str, clips: Sequence[str], items: Sequence[AttributeItem]
) -> dict[str, Any]:
    """The volume, the effects or the equalizer of clips put back as a new clip has them."""
    return {"edit": "remove_attributes", "timeline": timeline, "clips": list(clips),
            "items": list(items)}  # fmt: skip


def fade_to_playhead(timeline: str, clips: Sequence[str], side: Edge, at: int) -> dict[str, Any]:
    """Clips faded in from their start to frame ``at``, or out from it to their end."""
    return {"edit": "fade_to_playhead", "timeline": timeline, "clips": list(clips), "side": side,
            "at": at}  # fmt: skip


def set_lane(
    timeline: str, of: ChainOf, path: str, lane: Mapping[str, Any] | None
) -> dict[str, Any]:
    """The lane of the setting ``path`` put in place, or taken away (``None``).

    A lane is written ``{"points": [{"at", "value"}, ...], "kind"?}``.
    """
    written = None if lane is None else dict(lane)
    return {"op": "set_lane", "timeline": timeline, "of": dict(of), "path": path, "lane": written}


def lane_range(
    timeline: str,
    of: ChainOf,
    path: str,
    from_: Time,
    to: Time,
    action: Literal["delete", "cut", "paste", "shift_value", "thin"],
    *,
    points: Sequence[tuple[Time, float]] = (),
    delta: float | None = None,
    tolerance: float | None = None,
) -> dict[str, Any]:
    """A range of a lane taken away, pasted over, raised or lowered, or thinned.

    ``points`` are pasted at times from ``from_``, ``delta`` raises or lowers the values; the
    values outside the range are kept.
    """
    written: dict[str, Any] = {"edit": "lane_range", "timeline": timeline, "of": dict(of),
                               "path": path, "from": _json(from_), "to": _json(to),
                               "action": action}  # fmt: skip
    if points:
        written["points"] = [_point(time, value) for time, value in points]
    return _set(written, delta=delta, tolerance=tolerance)


def lane_points(
    timeline: str,
    of: ChainOf,
    path: str,
    action: Literal["add", "remove", "move"],
    *,
    points: Sequence[tuple[Time, float | None]] = (),
    at: Sequence[Time] = (),
    by: Time | None = None,
    delta: float | None = None,
    grouped: bool = False,
) -> dict[str, Any]:
    """Points of the lane of the setting ``path`` (``fader_db``, ``pan.x``, ``mute``, ...).

    They are added (a point without a value keeps the curve there), or those at the times ``at``
    are taken away, or moved ``by`` a time and ``delta`` of value.
    """
    written: dict[str, Any] = {"edit": "lane_points", "timeline": timeline, "of": dict(of),
                               "path": path, "action": action}  # fmt: skip
    if action == "add":
        written["points"] = [_point(time, value) for time, value in points]
    else:
        written["at"] = [_json(time) for time in at]
    if grouped:
        written["grouped"] = True
    return _set(written, by=by, delta=delta)


def _point(at: Time, value: float | None) -> dict[str, Any]:
    point: dict[str, Any] = {"at": _json(at)}
    if value is not None:
        point["value"] = value
    return point


def lane_draw(
    timeline: str,
    of: ChainOf,
    path: str,
    from_: Time,
    to: Time,
    points: Sequence[tuple[Time, float]],
    *,
    tolerance: float | None = None,
    grouped: bool = False,
) -> dict[str, Any]:
    """Points drawn in place of a range of a lane, thinned; after ``to`` the lane is as it was.

    ``grouped`` draws the lanes of the same setting of the other tracks of an active group too,
    as the engine of plan T16b does; one before it refuses the field.
    """
    written: dict[str, Any] = {
        "edit": "lane_draw",
        "timeline": timeline,
        "of": dict(of),
        "path": path,
        "from": _json(from_),
        "to": _json(to),
        "points": [_point(time, value) for time, value in points],
    }
    if grouped:
        written["grouped"] = True
    return _set(written, tolerance=tolerance)


def clear_lanes(
    timeline: str,
    of: ChainOf,
    *,
    paths: Sequence[str] | None = None,
    then: LaneThen | None = None,
    at: Time | None = None,
) -> dict[str, Any]:
    """The lanes of ``paths``, or all, taken away, their settings left as ``then`` says."""
    written: dict[str, Any] = {"edit": "clear_lanes", "timeline": timeline, "of": dict(of)}
    if paths is not None:
        written["paths"] = list(paths)
    return _set(written, then=then, at=at)
