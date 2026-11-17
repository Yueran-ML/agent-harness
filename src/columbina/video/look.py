"""Frames of a timeline as pictures, for a model to look at what it changed.

``timeline.thumbnail`` (``engine/PROTOCOL.md``, Thumbnails of the Color page) draws a frame of a
timeline off screen as an export draws it: from the original media, graded, through the output
transform, with every layer, title and composition, and no subtitles; kept to 8 bits, opaque,
``height`` lines high (no higher than the timeline) and as wide as the timeline's aspect makes
it. With ``clip`` it draws that clip's layer alone, fitted to the frame, by the clip's own grades
(its group's pre-clip grade, its own and its group's post-clip grade) and not the timeline's. It
changes nothing and needs no desktop.

:func:`look` asks it for one to four frames and gives them back as pictures, in the order asked,
beside a receipt that tells of each its frame, its time, the clip on top there and its size.
Everything asked is checked before the first frame is drawn. A model that takes no pictures is
told so in their place by whoever sends it the result (the adapters of :mod:`columbina.llm`), as
it is of a picture that ``read`` gives back. The pictures of one call come to no more than a
model takes (:data:`columbina.pictures.MOST_BYTES`): a frame whose picture would go past it is
left out, and said to be.

What one node of a composition shows alone is not drawn here: ``video_grade_scopes`` with
``fusion`` measures it.
"""

import base64
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, cast

from columbina import pictures
from columbina.llm import Picture
from columbina.video.editing import receipt
from columbina.video.edits import Rate
from columbina.video.engine import Engine, EngineError
from columbina.video.grading import GradeError, timeline_of

__all__ = ["HIGHEST", "LOWEST", "MOST_FRAMES", "LookError", "Looked", "hint_of", "look"]

MOST_FRAMES = 4
"""How many frames one call looks at, at most."""

LOWEST = 90
"""The fewest lines of a picture: fewer show too little to judge a change by."""

HIGHEST = 270
"""The most lines of a picture: the most that the engine draws a thumbnail with."""

_EXPORTED = (
    "each frame as an export draws it: graded, through the output transform, with every layer, "
    "title and composition; no subtitles"
)


class LookError(Exception):
    """What is wrong with what ``video_look`` was asked, said so that it can be put right."""


@dataclass(frozen=True, slots=True)
class Looked:
    """What ``video_look`` gives back.

    Attributes:
        text: What the model reads: the receipt of the pictures.
        pictures: The frames, as PNG, in the order that the receipt tells them.
    """

    text: str
    pictures: tuple[Picture, ...] = ()


async def look(
    engine: Engine,
    timeline: str | None,
    *,
    frames: Sequence[int] = (),
    clip: str | None = None,
    alone: bool = False,
    height: int = HIGHEST,
) -> Looked:
    """Draw frames of a timeline off screen, and give them back as pictures.

    The frames are ``frames``, in their order, or the middle frame of ``clip`` where none are
    given; with ``clip``, each must be a frame of that clip. Each picture is the frame as an
    export draws it, or, with ``alone``, the layer of ``clip`` alone, by its own grades. Nothing
    is changed.

    Raises:
        LookError: What was asked is not there, or not as it may be asked.
        EngineError: The engine refused a frame, or could not draw it.
    """
    _checked(frames, clip, alone, height)
    _, project = await engine.project()
    try:
        on = timeline_of(project, timeline)
    except GradeError as error:
        raise LookError(str(error)) from None
    id = str(on.get("id"))
    length = await engine.duration(id)
    asked = list(frames)
    for frame in asked:
        if not 0 <= frame < length:
            raise LookError(_outside(id, frame, length))
    if clip is not None:
        placed = _picture_clip(on, clip)
        start, end = int(placed["start"]), int(placed["start"]) + int(placed["length"])
        if not asked:
            asked = [start + (end - start) // 2]
        for frame in asked:
            if not start <= frame < end:
                raise LookError(
                    f"frame {frame} is not of clip {clip}, which lies on frames {start} to "
                    f"{end - 1}"
                )
    rate = Rate.model_validate(on["rate"])
    kept: list[Picture] = []
    shown: list[dict[str, Any]] = []
    left: list[int] = []
    total = 0
    for frame in asked:
        width, high, png = await engine.thumbnail(
            id, frame, clip=clip if alone else None, height=height
        )
        if total + len(png) > pictures.MOST_BYTES:
            left.append(frame)
            continue
        total += len(png)
        kept.append(Picture(media_type="image/png", data=base64.b64encode(png).decode("ascii")))
        shown.append(
            {
                "frame": frame,
                "seconds": round(frame * rate.den / rate.num, 3),
                "on_top": _on_top(on, frame),
                "width": width,
                "height": high,
            }
        )
    if not kept:
        raise LookError(
            f"the picture of frame {asked[0]} is larger than a model takes "
            f"({pictures.MOST_BYTES // 1_000_000} MB): ask a lower height"
        )
    told: dict[str, Any] = {
        "timeline": id,
        "shows": _alone(clip) if alone and clip is not None else _EXPORTED,
        "pictures": shown,
        "note": "the pictures are shown beside this text, in this order",
    }
    if left:
        told["left_out"] = {
            "frames": left,
            "why": (
                "with them the pictures would be larger than a model takes in one result: look "
                "at them in another call, or at a lower height"
            ),
        }
    return Looked(receipt(told), tuple(kept))


def hint_of(error: EngineError) -> str | None:
    """What a model can do about a refusal of a frame, where there is more to say."""
    data = error.data or {}
    hints = {
        "timeline": "video_info lists the timelines",
        "clip": (
            "a clip is seen alone only at its own frames and while it is on; video_info tells "
            "where each clip lies"
        ),
        "height": f"height is from {LOWEST} to {HIGHEST} lines",
        "draw": "try again; video_grade_scopes measures the frame in numbers",
    }
    return hints.get(str(data.get("reason")))


def _checked(frames: Sequence[int], clip: str | None, alone: bool, height: int) -> None:
    """Refuse what is asked wrongly before anything is asked of the engine.

    Raises:
        LookError: Too many frames, one twice, none and no clip, alone without a clip, or a
            height out of its range.
    """
    if len(frames) > MOST_FRAMES:
        raise LookError(f"look at {MOST_FRAMES} frames at most in one call, not {len(frames)}")
    if not LOWEST <= height <= HIGHEST:
        raise LookError(f"height is from {LOWEST} to {HIGHEST} lines, not {height}")
    if alone and clip is None:
        raise LookError("alone shows the layer of one clip: give clip too")
    if not frames and clip is None:
        raise LookError("give frames, or clip to look at its middle frame")
    for at, frame in enumerate(frames):
        if frame in frames[:at]:
            raise LookError(f"frame {frame} is named twice")


def _outside(timeline: str, frame: int, length: int) -> str:
    if length == 0:
        return f"timeline {timeline} is empty: it has no frame to look at"
    return f"frame {frame} is outside timeline {timeline}, whose frames are 0 to {length - 1}"


def _alone(clip: str) -> str:
    return (
        f"clip {clip} alone, fitted to the frame and opaque, by its own grades (its group's "
        "pre-clip grade, its own, its group's post-clip grade); without the timeline grade and "
        "the other layers"
    )


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


def _picture_clip(timeline: Mapping[str, Any], id: str) -> dict[str, Any]:
    """A clip of a picture track of the timeline.

    Raises:
        LookError: There is no such clip, or it is a clip of sound.
    """
    for track in map(_object, _items(timeline.get("tracks"))):
        for clip in map(_object, _items(track.get("clips"))):
            if clip.get("id") == id:
                if track.get("kind") != "video":
                    raise LookError(
                        f"{id} is a clip of sound on {track.get('id')}; only clips of picture "
                        "tracks are seen"
                    )
                return clip
    raise LookError(
        f"there is no clip {id} on timeline {timeline.get('id')}; video_info lists the clips"
    )


def _on_top(timeline: Mapping[str, Any], frame: int) -> str | None:
    """The clip seen on top at a frame: of the highest picture track that is on and shows one.

    The tracks of picture are stacked in their order, the first at the bottom; a track or a
    clip that is off shows nothing.
    """
    top: str | None = None
    for track in map(_object, _items(timeline.get("tracks"))):
        if track.get("kind") != "video" or track.get("enabled") is False:
            continue
        for clip in map(_object, _items(track.get("clips"))):
            start, length = clip.get("start"), clip.get("length")
            if (
                clip.get("enabled") is not False
                and isinstance(start, int)
                and isinstance(length, int)
                and start <= frame < start + length
            ):
                top = str(clip.get("id"))
    return top
