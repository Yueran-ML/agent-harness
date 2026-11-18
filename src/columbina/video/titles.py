"""The style of a title as the agent gives it: its parts filled, its runs found, all checked.

The style of a title (``engine/PROTOCOL.md``, a stored source) has, besides its font, size,
colour, anchor and alignment, four parts that it may be without: a ``stroke`` about its letters,
``{color, width}``; a ``shadow`` under them, ``{color, x, y, blur}``; a ``background`` under the
text, ``{color, padding, radius}``; and ``spans``, runs of its text each drawn its own way,
``{from, to, color?, scale?, bold?, italic?, font?}``. Each of the three parts is given
whole, its numbers in pixels of the timeline. The runs are by the indices of the characters of
the text, Unicode scalars, as a ``str`` of Python counts them: in order, none over another, 256
at the most, and none past the end of the text. The engine refuses what is not so as
``out_of_range``, with ``what`` the name of the part, or ``spans``.

``video_edit`` takes the edits of the engine as a model writes them, and what is here goes over
them before they are sent:

- The source of a title in any edit, an insert or a ``set_clip`` alike, may give a part in part:
  it is filled with what a new one has (:data:`PARTS`), and one given ``null`` is left out. Its
  runs may be named by what they say, ``{"text": "word"}``, and are found in its text. What the
  engine would refuse is said in words before anything is sent (:func:`sourced`).
- ``{"edit": "set_title", "clip", "text"?, "style"?, "spans"?}``, which the engine does not
  know, changes a title that is on the timeline: it is worked out into the ``set_clip`` that puts
  the title's source whole, from the title as the project has it. Its style is patched, the runs
  given are laid over those that it has (:func:`layered`), and where its text changes, those are
  moved with it first (:func:`moved`), so that none is left past its end (:func:`titled`).
"""

import difflib
import itertools
import json
import math
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Final, cast

from columbina.video.clip_keys import SETTINGS

__all__ = [
    "MOST_SPANS",
    "PARTS",
    "SET_TITLE",
    "SPAN_STYLE",
    "Run",
    "TitleError",
    "is_set_title",
    "layered",
    "moved",
    "part_of",
    "run_of",
    "runs_of",
    "sourced",
    "titled",
]

SET_TITLE: Final = "set_title"
"""The edit of ``video_edit`` that changes a title on the timeline, worked out here."""

MOST_SPANS: Final = 256
"""How many runs of its text a title has at the most."""

SPAN_STYLE: tuple[str, ...] = ("color", "scale", "bold", "italic", "font")
"""What a run of the text of a title may have of its own, in the order that it is written."""

PARTS: dict[str, dict[str, Any]] = {
    "stroke": {"color": "#000000ff", "width": 4},
    "shadow": {"color": "#00000080", "x": 4, "y": 4, "blur": 4},
    "background": {"color": "#00000099", "padding": 16, "radius": 8},
}
"""Each part of the style of a title as a new one is: a black stroke 4 pixels wide; a shadow
black at half its opacity, 4 pixels right of the letters and 4 down, blurred over 4; a box
black at 60% of its opacity, 16 pixels beyond the text, with corners rounded over 8. Its
numbers are those that the engine's catalog gives a new part (:data:`SETTINGS`), and each is
in the range that the catalog gives it."""

_SCALE = (0.1, 10.0)
"""The bounds of the scale of a run, as the engine holds them."""

_HEX = re.compile(r"#[0-9a-fA-F]{6}(?:[0-9a-fA-F]{2})?")

_RUN_FIELDS = frozenset({"text", "occurrence", "from", "to", "clear", *SPAN_STYLE})

_SET_TITLE_FIELDS = frozenset({"edit", "timeline", "clip", "text", "style", "spans"})

_STYLE_FIELDS = (
    "font", "size", "color", "x", "y", "align", "stroke", "shadow", "background", "spans",
)  # fmt: skip
"""The settings of the style of a title."""

_KEPT: Final = object()
"""What ``set_title`` gives where it leaves the runs of a title as they are."""

_Codes = Sequence[tuple[str, int, int, int, int]]
"""How one text becomes another, as :meth:`difflib.SequenceMatcher.get_opcodes` tells it."""


class TitleError(ValueError):
    """What is wrong with a title given to ``video_edit``, said so that it can be put right.

    A value that is wrong, which ``columbina video edit`` tells as it tells any other.
    """


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


def _number(value: object) -> float | None:
    """A number as a float, or none where the value is not a finite number."""
    if isinstance(value, int | float) and not isinstance(value, bool) and math.isfinite(value):
        return float(value)
    return None


def _int_of(value: object) -> int | None:
    """A whole number, or none where the value is not one."""
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _refused(what: str) -> str:
    return f"the engine refuses it as out_of_range {what}"


def _nothing(value: object) -> bool:
    """Whether a setting of a run is nothing of its own: none, false or an empty font."""
    return value is None or value is False or value == ""


# ---------------------------------------------------------------------------------------------
# The parts of a style
# ---------------------------------------------------------------------------------------------


def _color(value: object, where: str) -> str:
    """A colour as it is sent, ``#rrggbbaa`` in lower case: one of six digits is opaque."""
    if isinstance(value, str) and _HEX.fullmatch(value):
        return (value if len(value) == 9 else value + "ff").lower()
    raise TitleError(
        f"{where} is a colour #rrggbbaa, alpha last, as #000000ff for black; {value!r} is not one"
    )


def _param(value: object, path: str, part: str) -> object:
    """A number of a part of a title, or its keys, checked against the range of ``path``."""
    spec = SETTINGS[path]
    bounds = f"{spec.low:g} to {spec.high:g} px"
    if isinstance(value, Mapping):
        given = cast("Mapping[str, Any]", value)
        keys = _items(given.get("keys"))
        numbers = [_number(_object(key).get("value")) for key in keys]
        values = [one for one in numbers if one is not None]
        if set(given) != {"keys"} or not keys or len(values) != len(keys):
            raise TitleError(f'{path} is a number, or {{"keys": [{{frame, value, interp}}, ...]}}')
        wrong = next((one for one in values if not spec.low <= one <= spec.high), None)
        if wrong is not None:
            raise TitleError(f"a key of {path} is {wrong:g}, out of {bounds} ({_refused(part)})")
        return dict(given)
    number = _number(value)
    if number is None:
        raise TitleError(f'{path} is a number, {bounds}, or {{"keys": [{{frame, value}}, ...]}}')
    if not spec.low <= number <= spec.high:
        raise TitleError(f"{path} is {bounds}, and {value} is not ({_refused(part)})")
    return value


def part_of(name: str, given: object, now: object = None) -> dict[str, Any]:
    """A stroke, a shadow or a background of a title as it is sent: whole, and checked.

    What ``given`` names is laid over the part that the title has (``now``), or over a new one
    (:data:`PARTS`) where it has none: ``{}`` is the part as it is, or a new one. A colour of
    six digits is opaque.

    Raises:
        TitleError: It is not an object of the settings of the part, or a setting is not one;
            said with the refusal of the engine.
    """
    new = PARTS[name]
    if not isinstance(given, Mapping):
        raise TitleError(
            f"{name} is an object {{{', '.join(new)}}}, what it leaves out as the title has it "
            f"or as a new {name} has it ({json.dumps(new)}), or null for none; {given!r} is not"
        )
    asked = cast("Mapping[str, Any]", given)
    other = sorted(str(key) for key in asked if key not in new)
    if other:
        raise TitleError(f"a {name} has no {', '.join(other)}; it has {', '.join(new)}")
    had = {key: value for key, value in _object(now).items() if key in new}
    whole: dict[str, Any] = {**new, **had, **asked}
    whole["color"] = _color(whole["color"], f"{name}.color")
    for setting in new:
        if setting != "color":
            whole[setting] = _param(whole[setting], f"title.{name}.{setting}", name)
    return whole


# ---------------------------------------------------------------------------------------------
# The runs of a text
# ---------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Run:
    """A run of the text of a title that a model asked to style.

    Attributes:
        start: Its first character, counted from 0.
        end: The character after its last.
        style: What it is given of :data:`SPAN_STYLE`; ``None`` where what the text has of it
            there is taken away.
        clear: Whether all that the text has there is taken away first.
    """

    start: int
    end: int
    style: dict[str, Any]
    clear: bool = False


def run_of(text: str, asked: Mapping[str, Any], where: str = "a span") -> tuple[int, int]:
    """Where a run is in ``text``: the indices of its first character and of the one after it.

    It is named by what it says, ``{"text", "occurrence"?}``, ``occurrence`` being which time it
    is found from 1 (the first where it is left out; one time may begin within the time before);
    or by the indices of its characters, ``{"from", "to"}``.

    Raises:
        TitleError: It is named both ways or neither, what it says is not in the text as many
            times, or its indices are not within the text.
    """
    said = asked.get("text")
    if said is not None:
        if "from" in asked or "to" in asked:
            raise TitleError(f"{where} is named by text or by from and to, not by both")
        if not isinstance(said, str) or not said:
            raise TitleError(f"{where}: text is some of the title's text, as it is written there")
        times = _int_of(asked.get("occurrence", 1))
        if times is None or times < 1:
            raise TitleError(f"{where}: occurrence is which time the text is found, 1 the first")
        found = [at for at in range(len(text)) if text.startswith(said, at)]
        if len(found) < times:
            count = {0: "nowhere", 1: "once"}.get(len(found), f"{len(found)} times")
            also = f", and occurrence is {times}" if found else ""
            raise TitleError(f"{where}: {said!r} is found {count} in the text {text!r}{also}")
        start = found[times - 1]
        return start, start + len(said)
    start, end = _int_of(asked.get("from")), _int_of(asked.get("to"))
    if start is None or end is None:
        raise TitleError(
            f'{where} is named by what it says, {{"text": "word", "occurrence"?: 2}}, or by the '
            'indices of its characters, {"from": 0, "to": 4}'
        )
    if not 0 <= start < end <= len(text):
        raise TitleError(
            f"{where} is from {start} to {end}: a span is within the text, from 0 to its "
            f"{len(text)} characters at the most, and ends after it begins ({_refused('spans')})"
        )
    return start, end


def _run_style(given: Mapping[str, Any], where: str) -> dict[str, Any]:
    """What a run is given of :data:`SPAN_STYLE`, checked: ``None`` for what is taken away."""
    style: dict[str, Any] = {}
    for key in SPAN_STYLE:
        if key not in given:
            continue
        value = given[key]
        if _nothing(value):
            style[key] = None
        elif key == "color":
            style[key] = _color(value, f"{where}.color")
        elif key == "scale":
            number = _number(value)
            low, high = _SCALE
            if number is None or not low <= number <= high:
                raise TitleError(
                    f"{where}.scale is {low:g} to {high:g}, the size of its letters to the "
                    f"title's, and {value!r} is not ({_refused('spans')})"
                )
            style[key] = value
        elif key in ("bold", "italic"):
            if value is not True:
                raise TitleError(f"{where}.{key} is true or false")
            style[key] = True
        elif not isinstance(value, str):
            raise TitleError(f"{where}.font is the family of a font, a string")
        else:
            style[key] = value
    return style


def runs_of(text: str, asked: object) -> list[Run]:
    """The runs of ``text`` that a model asked to style, in the order that they were given.

    Each is ``{"text", "occurrence"?}`` or ``{"from", "to"}`` (:func:`run_of`), with any of
    :data:`SPAN_STYLE`: ``null``, ``false`` or an empty font takes that away from the run, and
    ``"clear": true`` takes away all that it had first.

    Raises:
        TitleError: What is asked is not a list of runs, or a run is wrong; said with the refusal
            of the engine.
    """
    if not isinstance(asked, list):
        raise TitleError(
            'spans is a list of runs, as [{"text": "word", "bold": true}], or null for none'
        )
    runs: list[Run] = []
    for number, one in enumerate(cast("list[Any]", asked)):
        where = f"spans[{number}]"
        if not isinstance(one, Mapping):
            raise TitleError(f'{where} is an object, as {{"text": "word", "color": "#ff0000ff"}}')
        given = cast("Mapping[str, Any]", one)
        other = sorted(str(key) for key in given if key not in _RUN_FIELDS)
        if other:
            raise TitleError(
                f"{where} has no {', '.join(other)}; a span has text and occurrence, or from "
                f"and to, and {', '.join(SPAN_STYLE)}, and clear"
            )
        clear = given.get("clear", False)
        if not isinstance(clear, bool):
            raise TitleError(f"{where}.clear is true or false")
        start, end = run_of(text, given, where)
        runs.append(Run(start, end, _run_style(given, where), clear))
    return runs


def _style_of(span: Mapping[str, Any]) -> dict[str, Any]:
    """What a span has of :data:`SPAN_STYLE` that is something of its own."""
    return {key: span[key] for key in SPAN_STYLE if key in span and not _nothing(span[key])}


def layered(spans: Sequence[Mapping[str, Any]], runs: Sequence[Run]) -> list[dict[str, Any]]:
    """The spans of a title once ``runs`` are laid over those that it has, one after another.

    Over its part of the text a run puts what it is given and takes away what it gives ``None``,
    the rest being kept: as text that is chosen and made bold keeps its colour. Text with nothing
    of its own is in no span, and two spans that meet with the same style are one; so the spans
    are in order and none is over another, as the engine takes them.

    Raises:
        TitleError: There would be more than :data:`MOST_SPANS`.
    """
    had = [(int(one["from"]), int(one["to"]), _style_of(one)) for one in spans]
    cuts = sorted(
        {edge for begins, ends, _ in had for edge in (begins, ends)}
        | {edge for run in runs for edge in (run.start, run.end)}
    )
    pieces: list[dict[str, Any]] = []
    for start, end in itertools.pairwise(cuts):
        style: dict[str, Any] = {}
        for begins, ends, own in had:
            if begins <= start and end <= ends:
                style.update(own)
        for run in runs:
            if not run.start <= start < end <= run.end:
                continue
            if run.clear:
                style.clear()
            for key, value in run.style.items():
                if value is None:
                    style.pop(key, None)
                else:
                    style[key] = value
        if not style:
            continue
        last = pieces[-1] if pieces else None
        if last is not None and last["to"] == start and _style_of(last) == style:
            last["to"] = end
        else:
            pieces.append({"from": start, "to": end, **_style_of(style)})
    if len(pieces) > MOST_SPANS:
        raise TitleError(
            f"a title has {MOST_SPANS} spans at the most, and this would have {len(pieces)} "
            f"({_refused('spans')})"
        )
    return pieces


def _first(codes: _Codes, at: int) -> int:
    """Where the character ``at`` of the old text is in the new, as the first of a span.

    Text written in place of it begins the span; where it was taken away, the span begins
    where it was.
    """
    for tag, i1, i2, j1, _ in codes:
        if i1 <= at < i2:
            return j1 + (at - i1) if tag == "equal" else j1
    return codes[-1][4] if codes else 0


def _after(codes: _Codes, at: int) -> int:
    """Where the character after ``at`` of the old text is in the new, as the end of a span.

    Text written in place of ``at`` ends the span; where it was taken away, the span ends
    where it was.
    """
    for tag, i1, i2, j1, j2 in codes:
        if i1 <= at < i2:
            return j1 + (at - i1) + 1 if tag == "equal" else j2
    return codes[-1][4] if codes else 0


def moved(old: str, new: str, spans: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """The spans of a title whose text ``old`` becomes ``new``, as the engine needs them.

    The two texts are compared character by character (:class:`difflib.SequenceMatcher`), and
    each span goes with the characters that it had: text written within it widens it, text
    written at either end of it does not, text taken away from it narrows it, and text written in
    place of some of it is in it. A span whose characters are all taken away is no more, so none
    is left past the end of the new text, which the engine refuses. Text written in place of the
    end of one span and the start of the next is in the first.
    """
    if old == new:
        return [dict(one) for one in spans]
    codes = difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes()
    kept: list[dict[str, Any]] = []
    reached = 0
    for one in spans:
        start = max(_first(codes, int(one["from"])), reached)
        end = _after(codes, int(one["to"]) - 1)
        if end > start:
            kept.append({**one, "from": start, "to": end})
            reached = end
    return kept


# ---------------------------------------------------------------------------------------------
# The edits of video_edit
# ---------------------------------------------------------------------------------------------


def _spans_now(style: Mapping[str, Any]) -> list[dict[str, Any]]:
    """The spans that a title has, as the engine wrote them."""
    spans = [_object(one) for one in _items(style.get("spans"))]
    return [
        one
        for one in spans
        if _int_of(one.get("from")) is not None and _int_of(one.get("to")) is not None
    ]


def _whole(style: Mapping[str, Any], text: str) -> dict[str, Any]:
    """The style of a title given whole, as it is sent: its parts filled, its runs found."""
    written = dict(style)
    for name in PARTS:
        if name not in written:
            continue
        if written[name] is None:
            del written[name]
        else:
            written[name] = part_of(name, written[name])
    if "spans" in written:
        asked = written.pop("spans")
        spans: list[dict[str, Any]] = [] if asked is None else layered([], runs_of(text, asked))
        if spans:
            written["spans"] = spans
    return written


def sourced(item: Mapping[str, Any]) -> dict[str, Any]:
    """An edit of ``video_edit`` with the style of the title that it puts, if any, as it is sent.

    The ``source`` of an edit, or of ``set_clip``, that is a title is given whole: each part
    given in part is filled (:func:`part_of`), one given ``null`` is left out, and its runs are
    found in its text and laid in order (:func:`runs_of`, :func:`layered`). The rest of it, and
    any other edit, are as they were given.

    Raises:
        TitleError: A part or a run is wrong; said with the refusal of the engine.
    """
    written = dict(item)
    source = written.get("source")
    if not isinstance(source, Mapping):
        return written
    given = dict(cast("Mapping[str, Any]", source))
    style = given.get("style")
    if given.get("kind") != "title" or not isinstance(style, Mapping):
        return written
    text = given.get("text")
    styled = cast("Mapping[str, Any]", style)
    given["style"] = _whole(styled, text if isinstance(text, str) else "")
    written["source"] = given
    return written


def is_set_title(item: Mapping[str, Any]) -> bool:
    """Whether an edit of ``video_edit`` is ``set_title``, which is worked out here."""
    return item.get("edit") == SET_TITLE


def _title(timeline: Mapping[str, Any], clip: str) -> dict[str, Any]:
    """The source of the title ``clip`` of a timeline."""
    for track in map(_object, _items(timeline.get("tracks"))):
        for one in map(_object, _items(track.get("clips"))):
            if one.get("id") != clip:
                continue
            source = _object(one.get("source"))
            if source.get("kind") != "title":
                raise TitleError(
                    f"{clip} is not a title: set_title changes the text and the style of a title "
                    "(video_info tells the source of each clip)"
                )
            return source
    raise TitleError(
        f"there is no clip {clip} on timeline {timeline.get('id')}; video_info lists the clips"
    )


def titled(item: Mapping[str, Any], timeline: Mapping[str, Any]) -> dict[str, Any]:
    """The ``set_clip`` that a ``set_title`` of a title of ``timeline`` is worked out into.

    ``clip`` is the title. ``text`` is its new text. ``style`` holds what changes of its style:
    each setting given takes the place of the title's; a part given in part is laid over the
    part that the title has, or over a new one, and one given ``null`` is taken away. ``spans``
    are runs of its text laid over the spans that it has (:func:`runs_of`, :func:`layered`), or
    ``null`` for none; they may be given in ``style`` instead. Where the text changes, the spans
    that the title has are moved with it first (:func:`moved`).

    Raises:
        TitleError: What is asked is wrong, or the clip is not a title of the timeline; said
            before anything is sent.
    """
    other = sorted(str(key) for key in item if key not in _SET_TITLE_FIELDS)
    if other:
        raise TitleError(
            f"set_title has no {', '.join(other)}; it takes clip, and text, style or spans"
        )
    clip = item.get("clip")
    if not isinstance(clip, str) or not clip:
        raise TitleError("set_title needs clip, the id of a title on the timeline")
    if clip.startswith("$"):
        raise TitleError(
            f"set_title changes a title that is on the timeline before the batch, and {clip} is "
            "put in it: give that title its style in its source, or change it in the next call"
        )
    if not any(key in item for key in ("text", "style", "spans")):
        raise TitleError("set_title needs text, style or spans: what is to change")
    source = _title(timeline, clip)
    said = source.get("text")
    old = said if isinstance(said, str) else ""
    text = item.get("text", old)
    if not isinstance(text, str):
        raise TitleError("text is the new text of the title, a string")
    patch = item.get("style", {})
    if not isinstance(patch, Mapping):
        raise TitleError('style holds the settings that change, as {"size": 96}')
    changes = cast("Mapping[str, Any]", patch)
    other = sorted(str(key) for key in changes if key not in _STYLE_FIELDS)
    if other:
        raise TitleError(
            f"a title's style has no {', '.join(other)}; it has {', '.join(_STYLE_FIELDS)} "
            "(bold, italic, or a colour, a scale or a font of some of its text: spans)"
        )
    if "spans" in item and "spans" in changes:
        raise TitleError("spans are given once: in set_title, or in its style")
    now = _object(source.get("style"))
    style = dict(now)
    for key, value in changes.items():
        if key == "spans":
            continue
        if key not in PARTS:
            style[key] = value
        elif value is None:
            style.pop(key, None)
        else:
            style[key] = part_of(key, value, now.get(key))
    spans: list[dict[str, Any]] = moved(old, text, _spans_now(now))
    asked = item["spans"] if "spans" in item else changes.get("spans", _KEPT)
    if asked is None:
        spans = []
    elif asked is not _KEPT:
        spans = layered(spans, runs_of(text, asked))
    style.pop("spans", None)
    if spans:
        style["spans"] = spans
    return {
        "op": "set_clip",
        "timeline": timeline.get("id"),
        "id": clip,
        "source": {"kind": "title", "text": text, "style": style},
    }
