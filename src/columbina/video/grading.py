"""The grades of the Color page, as the agent reads and changes them through the engine.

A grade is a tree of nodes (``engine/PROTOCOL.md``, Grades): correctors, and mixers that bring
their pictures together. A clip has one, its group a pre-clip and a post-clip one, and the
timeline one over all of them, in that order (Reference Manual 20, p. 3330). The engine keeps
them in the project and changes them only by ``transaction.apply``, so a change that a tool
makes is one step of the history that the person in the desktop undoes too.

What is here:

- **Summaries**: a grade as a model is to read it, its nodes in order with the settings that
  change the picture and none of those that do not (a corrector has some ninety).
- **Checks**: a patch of the settings of a node, checked in DaVinci's units before it is sent,
  so that what is wrong with it is told in words that a model can act on.
- **Writing**: the targets, edits and operations of grades as the engine reads them.
- :class:`Grader`, which does what the tools of grades ask, each as one transaction: and the
  figures of frames (off screen, so a headless engine has them), colour management, and Shot
  Match and Auto Color (:mod:`columbina.video.shotmatch` works them out).
- :func:`reference`: every setting in full, for a model that asks.
"""

import asyncio
import contextlib
import copy
import difflib
import math
import os
import secrets
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, cast

from columbina.video import shotmatch
from columbina.video.engine import Applied, Engine, EngineError, NodeSide, Statistics

__all__ = [
    "COMPOSITES",
    "CORRECTOR",
    "EMPTY_GRADE",
    "GradeError",
    "Grader",
    "brief",
    "checked_patch",
    "expanded",
    "grade_at",
    "hint_of",
    "in_force",
    "luts_of",
    "missing_luts",
    "new_corrector",
    "node_of",
    "reference",
    "summary",
    "target_of",
    "timeline_space",
]

Level = Literal["clip", "pre_clip", "post_clip", "timeline"]
NodeAction = Literal[
    "append", "add_serial", "add_serial_before", "add_parallel", "add_layer", "add_outside",
    "remove", "reset", "extract",
]  # fmt: skip
CopyAction = Literal["copy", "append", "paste_node", "paste_value"]
# Where the keys of a grade copied to another go (Apply Grades Using, pp. 3264, 3287).
KeyAlign = Literal["source_timecode", "start_frame"]
GroupAction = Literal["create", "add", "remove", "rename", "delete", "grade", "collapse"]
KeyAction = Literal["put", "remove", "clear"]

EMPTY_GRADE: dict[str, Any] = {
    "nodes": [],
    "links": [{"from": "source", "to": "output", "kind": "rgb", "input": 0}],
}
"""The grade of a new clip or timeline: its source linked straight to its output."""

_LONGEST_LABEL = 100
_LONGEST_LUT = 4096
_MOST_INPUTS = 256
_MOST_PRESERVED = 10
_LARGEST_FRAME = 2**53 - 1
_ATTEMPTS = 3
"""How many times a change is worked out anew when the project changed under it meanwhile."""


class GradeError(Exception):
    """What is wrong with what a tool of grades was asked, said so that it can be put right."""


# ---------------------------------------------------------------------------------------------
# The settings of nodes, in DaVinci's units
# ---------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Number:
    """A setting that is a number, or keyframes of numbers.

    Attributes:
        name: What DaVinci calls it.
        default: The value that changes nothing, which a new node has.
        low: The lowest value.
        high: The highest value.
    """

    name: str
    default: float
    low: float
    high: float

    def told(self) -> str:
        """Its range and its neutral value, in words."""
        low, high, neutral = _number(self.low), _number(self.high), _number(self.default)
        return f"{self.name}: {low} to {high}, {neutral} changes nothing"


@dataclass(frozen=True)
class Flag:
    """A setting that is on or off."""

    name: str
    default: bool


@dataclass(frozen=True)
class Points:
    """A curve, as points ``[in, out]``: ``in`` from 0 to 1, rising; ``out`` from low to high."""

    name: str
    low: float
    high: float
    least: int
    default: tuple[tuple[float, float], ...]


@dataclass(frozen=True)
class Lut:
    """The LUT of a corrector: a path or a name in the library, or ``null`` for none."""

    name: str


@dataclass(frozen=True)
class Whole:
    """A group of settings that a node may have or not: given whole where it has none."""

    name: str
    fields: "Branch"


@dataclass(frozen=True)
class Count:
    """A whole number of inputs."""

    name: str
    default: int
    low: int
    high: int


@dataclass(frozen=True)
class Choice:
    """One of some names."""

    name: str
    default: str
    choices: tuple[str, ...]


@dataclass(frozen=True)
class Items:
    """A list of groups of settings, given whole: the inputs of a key mixer."""

    name: str
    item: "Branch"
    least: int
    most: int
    count: int


Spec = Number | Flag | Points | Lut | Whole | Count | Choice | Items | dict[str, Any]
Branch = dict[str, Spec]

_CHANNELS = (("y", "Y"), ("r", "red"), ("g", "green"), ("b", "blue"))


def _wheel(name: str, default: float, low: float, high: float) -> Branch:
    return {key: Number(f"{name} {said}", default, low, high) for key, said in _CHANNELS}


def _rgb(name: str, default: float, low: float, high: float) -> Branch:
    return {key: Number(f"{name} {said}", default, low, high) for key, said in _CHANNELS[1:]}


def _curve(name: str) -> Branch:
    return {
        "points": Points(f"Custom curve {name}", 0.0, 1.0, 2, ((0.0, 0.0), (1.0, 1.0))),
        "intensity": Number(f"Custom curve {name} intensity", 100.0, 0.0, 100.0),
    }


def _key_input(matte: bool, name: str) -> Branch:
    return {
        "invert": Flag(f"{name} invert", False),
        "matte": Flag(f"{name} matte (off is Mask mode)", matte),
        "gain": Number(f"{name} gain", 1.0, 0.0, 2.0),
        "offset": Number(f"{name} offset", 0.0, -1.0, 1.0),
    }


_KEY_OUTPUT: Branch = {
    "invert": Flag("Key Output invert", False),
    "gain": Number("Key Output gain (how strongly the node works)", 1.0, 0.0, 2.0),
    "offset": Number("Key Output offset", 0.0, -1.0, 1.0),
}


def _range(name: str) -> Branch:
    """A range of a qualifier (pp. 3173-3175): on, Low, High, Low Soft, High Soft, 0..100."""
    return {
        "on": Flag(f"{name} on (off: it does not limit the key)", True),
        "low": Number(f"{name} Low", 0.0, 0.0, 100.0),
        "high": Number(f"{name} High", 100.0, 0.0, 100.0),
        "low_soft": Number(f"{name} Low Soft", 0.0, 0.0, 100.0),
        "high_soft": Number(f"{name} High Soft", 0.0, 0.0, 100.0),
    }


QUALIFIER: Branch = {
    "mode": Choice("Qualifier mode", "hsl", ("hsl", "rgb", "lum")),
    "invert": Flag("Qualifier invert", False),
    "hue": {
        "on": Flag("Hue on (off: it does not limit the key)", True),
        "center": Number("Hue Center (red 0, green 33.3, blue 66.7)", 50.0, 0.0, 100.0),
        "width": Number("Hue Width (100 every hue)", 100.0, 0.0, 100.0),
        "soft": Number("Hue Soft", 0.0, 0.0, 100.0),
        "symmetry": Number("Hue Symmetry", 50.0, 0.0, 100.0),
    },
    "sat": _range("Sat"),
    "lum": _range("Lum"),
    "red": _range("Red"),
    "green": _range("Green"),
    "blue": _range("Blue"),
    "finesse": {
        "pre_filter": Number("Pre-Filter", 0.0, 0.0, 100.0),
        "clean_black": Number("Clean Black", 0.0, 0.0, 100.0),
        "black_clip": Number("Black Clip", 0.0, 0.0, 100.0),
        "clean_white": Number("Clean White", 0.0, 0.0, 100.0),
        "white_clip": Number("White Clip", 100.0, 0.0, 100.0),
        "blur_radius": Number("Blur Radius", 0.0, 0.0, 2000.0),
        "in_out_ratio": Number("In/Out Ratio", 0.0, -100.0, 100.0),
        "morph": Choice("Morph Operation", "shrink", ("shrink", "grow", "opening", "closing")),
        "morph_radius": Number("Morph Radius", 0.0, 0.0, 100.0),
    },
}
"""The Qualifier palette (pp. 3157-3182), as PROTOCOL.md's Grades gives it."""

COMPOSITES = (
    "normal", "add", "color", "color_burn", "color_dodge", "darken", "darker_color",
    "difference", "divide", "exclusion", "hard_mix", "hardlight", "hue", "lighten",
    "lighter_color", "linear_burn", "linear_dodge", "linear_light", "luminosity", "multiply",
    "overlay", "pin_light", "saturation", "screen", "softlight", "subtract", "vivid_light",
)  # fmt: skip
"""The composite modes of a layer mixer (pp. 1055-1058), as the engine names them."""

CORRECTOR: Branch = {
    "primaries": {
        "lift": _wheel("Lift", 0.0, -1.0, 1.0),
        "gamma": _wheel("Gamma", 0.0, -1.0, 1.0),
        "gain": _wheel("Gain", 1.0, 0.0, 16.0),
        "offset": _rgb("Offset", 25.0, -75.0, 125.0),
        "contrast": Number("Contrast", 1.0, 0.0, 2.0),
        "pivot": Number("Pivot", 0.435, 0.0, 1.0),
        "saturation": Number("Saturation (Sat)", 50.0, 0.0, 100.0),
        "hue": Number("Hue", 50.0, 0.0, 100.0),
        "lum_mix": Number("Lum Mix", 100.0, 0.0, 100.0),
        "temperature": Number("Temp", 0.0, -4000.0, 4000.0),
        "tint": Number("Tint", 0.0, -100.0, 100.0),
        "color_boost": Number("Color Boost", 0.0, -100.0, 100.0),
        "midtone_detail": Number("Midtone Detail", 0.0, -100.0, 100.0),
        "shadows": Number("Shadows", 0.0, -100.0, 100.0),
        "highlights": Number("Highlights", 0.0, -100.0, 100.0),
    },
    "log": {
        "shadow": _rgb("Log Shadow", 0.0, -1.0, 1.0),
        "midtone": _rgb("Log Midtone", 0.0, -1.0, 1.0),
        "highlight": _rgb("Log Highlight", 0.0, -1.0, 1.0),
        "low_range": Number("Low Range", 0.333, 0.0, 1.0),
        "high_range": Number("High Range", 0.55, 0.0, 1.0),
    },
    "curves": {
        "ganged": Flag("Custom curves ganged", True),
        "y": _curve("Y"),
        "r": _curve("R"),
        "g": _curve("G"),
        "b": _curve("B"),
    },
    "hsl": {
        name: Points(said, -1.0, 1.0, 0, ())
        for name, said in (
            ("hue_vs_hue", "Hue vs Hue"),
            ("hue_vs_sat", "Hue vs Sat"),
            ("hue_vs_lum", "Hue vs Lum"),
            ("lum_vs_sat", "Lum vs Sat"),
            ("sat_vs_sat", "Sat vs Sat"),
            ("sat_vs_lum", "Sat vs Lum"),
        )
    },
    "lut": Lut("LUT"),
    "soft_clip": {
        "low": _rgb("Soft Clip Low", 0.0, 0.0, 100.0),
        "high": _rgb("Soft Clip High", 100.0, 0.0, 100.0),
        "low_soft": _rgb("Soft Clip Low Soft", 0.0, 0.0, 100.0),
        "high_soft": _rgb("Soft Clip High Soft", 0.0, 0.0, 100.0),
    },
    "blur": {
        "mode": Choice("Blur palette mode", "blur", ("blur", "sharpen", "mist")),
        "radius": _rgb("Blur Radius (0.5 nothing, above blurs, below sharpens)", 0.5, 0.0, 1.0),
        "ratio": _rgb("Blur H/V Ratio", 0.5, 0.0, 1.0),
        "scaling": _rgb("Blur Scaling (Sharpen and Mist)", 0.25, 0.0, 1.0),
        "coring_softness": Number("Blur Coring Softness (Sharpen)", 0.0, 0.0, 1.0),
        "level": Number("Blur Level (Sharpen)", 0.0, 0.0, 1.0),
        "mix": Number("Blur Mix (Mist)", 100.0, 0.0, 100.0),
        "linked": {
            "radius": Flag("Blur Radius ganged", True),
            "ratio": Flag("Blur H/V Ratio ganged", True),
            "scaling": Flag("Blur Scaling ganged", True),
        },
    },
    "key": {
        "input": _key_input(True, "Key Input"),
        "output": _KEY_OUTPUT,
        "qualifier": _key_input(True, "Key Qualifier"),
        "input_blur": {
            "radius": Number("Key Input Blur Radius", 0.0, 0.0, 2000.0),
            "ratio": Number("Key Input Blur H/V Ratio", 0.5, 0.0, 1.0),
        },
    },
    "shot_match": Whole(
        "Shot Match",
        {
            "gamma": _rgb("Shot Match gamma", 1.0, 0.1, 10.0),
            "gain": _rgb("Shot Match gain", 1.0, 0.0, 16.0),
            "offset": _rgb("Shot Match offset", 0.0, -1.0, 1.0),
            "contrast": Number("Shot Match contrast", 1.0, 0.0, 4.0),
            "pivot": Number("Shot Match pivot", 0.435, 0.0, 1.0),
        },
    ),
    "qualifier": Whole("Qualifier", QUALIFIER),
}
"""The settings of a corrector, with the units, ranges and neutral values of PROTOCOL.md."""

_MIXERS: dict[str, Branch] = {
    "parallel_mixer": {"inputs": Count("inputs", 2, 1, _MOST_INPUTS)},
    "layer_mixer": {
        "inputs": Count("inputs", 2, 1, _MOST_INPUTS),
        "composite": Choice("Composite Mode", "normal", COMPOSITES),
    },
    "key_mixer": {
        "inputs": Items("inputs", _key_input(False, "Key Mixer input"), 1, _MOST_INPUTS, 2),
        "output": _KEY_OUTPUT,
    },
}
_SPECS: dict[str, Branch] = {"corrector": CORRECTOR, **_MIXERS}


def _default(spec: Spec) -> Any:
    if isinstance(spec, dict):
        return {key: _default(one) for key, one in spec.items()}
    if isinstance(spec, Number | Flag | Count | Choice):
        return spec.default
    if isinstance(spec, Points):
        return [list(point) for point in spec.default]
    if isinstance(spec, Items):
        return [_default(spec.item) for _ in range(spec.count)]
    return None


def new_corrector() -> dict[str, Any]:
    """The settings of a new corrector, as the engine writes them."""
    params: dict[str, Any] = _default(CORRECTOR)
    return params


def _number(value: float) -> str:
    return f"{value:g}"


# ---------------------------------------------------------------------------------------------
# Checking a patch
# ---------------------------------------------------------------------------------------------


def expanded(settings: Mapping[str, Any]) -> dict[str, Any]:
    """The settings with each path written with dots, ``primaries.saturation``, made objects.

    Raises:
        GradeError: A path is empty somewhere, or a setting is given twice.
    """
    out: dict[str, Any] = {}
    for key, value in settings.items():
        parts = key.split(".")
        if not all(parts):
            raise GradeError(f"{key!r} is not a path of a setting, such as primaries.saturation")
        place = out
        for at, part in enumerate(parts[:-1]):
            further: object = place.setdefault(part, {})
            if not isinstance(further, dict):
                raise GradeError(f"{'.'.join(parts[: at + 1])} is given twice")
            place = cast("dict[str, Any]", further)
        given = expanded(_object(value)) if _is_branch(value) else value
        last = parts[-1]
        if last in place:
            if not (_is_map(place[last]) and _is_map(given)):
                raise GradeError(f"{key} is given twice")
            place[last] = _merged(_object(place[last]), _object(given), key)
        else:
            place[last] = given
    return out


def _is_branch(value: object) -> bool:
    """Whether a value is an object of settings, not keyframes."""
    return _is_map(value) and "keys" not in _object(value)


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _is_map(value: object) -> bool:
    return isinstance(value, Mapping)


def _is_list(value: object) -> bool:
    return isinstance(value, list)


def _merged(old: dict[str, Any], new: dict[str, Any], where: str) -> dict[str, Any]:
    out = dict(old)
    for key, value in new.items():
        if key in out:
            if not (_is_map(out[key]) and _is_map(value)):
                raise GradeError(f"{where}.{key} is given twice")
            out[key] = _merged(_object(out[key]), _object(value), f"{where}.{key}")
        else:
            out[key] = value
    return out


def checked_patch(settings: Mapping[str, Any], node: Mapping[str, Any]) -> dict[str, Any]:
    """The patch of ``set_node`` for a node, checked against the settings of its kind.

    ``settings`` may write a path with dots, ``{"primaries.saturation": 0}``, or as objects,
    ``{"primaries": {"saturation": 0}}``. A number setting takes a number or keyframes
    ``{"keys": [{"frame": 0, "value": 40, "interp": "linear"}, ...]}``, ``interp`` being
    ``linear`` where it is left out. A shot match given where the node has none is made whole
    with neutral values.

    Raises:
        GradeError: A setting that the node does not have, or a value out of its range, said
            with the setting's name in DaVinci and its range.
    """
    kind = str(node.get("kind"))
    spec = _SPECS.get(kind)
    if spec is None:
        raise GradeError(f"node {node.get('id')} is a {kind}, which this tool cannot set")
    if not settings:
        raise GradeError(
            "settings is empty: give the settings to change, such as primaries.saturation"
        )
    patch = expanded(settings)
    return _object(_checked(spec, patch, "", _object(node.get("params"))))


def _checked(spec: Spec, given: Any, path: str, now: Any) -> Any:
    if isinstance(spec, dict):
        return _checked_branch(spec, given, path, now)
    if isinstance(spec, Number):
        return _checked_number(spec, given, path)
    if isinstance(spec, Flag):
        if not isinstance(given, bool):
            raise GradeError(f"{path} ({spec.name}) is true or false")
        return given
    if isinstance(spec, Points):
        return _checked_points(spec, given, path)
    if isinstance(spec, Lut):
        if given is None:
            return None
        if not isinstance(given, str) or not 1 <= len(given) <= _LONGEST_LUT or "\0" in given:
            raise GradeError(
                f"{path} is the name of a .cube in the LUT library or the project folder, or an "
                "absolute path, or null for none; video_grade_lut sets it by name and lists them"
            )
        return given
    if isinstance(spec, Whole):
        if given is None:
            return None
        if not _is_map(given):
            raise GradeError(
                f"{path} ({spec.name}) is an object of {_listed(spec.fields)}, or null"
            )
        if now is None:
            # Where the node has none, the engine takes it whole: what is not given is neutral.
            whole = _merged(_default(spec.fields), {}, path)
            return _checked_branch(spec.fields, _filled(whole, _object(given), path), path, whole)
        return _checked_branch(spec.fields, given, path, now)
    if isinstance(spec, Count):
        if type(given) is not int or not spec.low <= given <= spec.high:
            raise GradeError(
                f"{path} ({spec.name}) is a whole number from {spec.low} to {spec.high}"
            )
        return given
    if isinstance(spec, Choice):
        if given not in spec.choices:
            raise GradeError(f"{path} ({spec.name}) is one of {', '.join(spec.choices)}")
        return given
    return _checked_items(spec, given, path)


def _filled(whole: dict[str, Any], given: dict[str, Any], path: str) -> dict[str, Any]:
    """``given`` laid over ``whole``, objects into objects."""
    out = dict(whole)
    for key, value in given.items():
        if _is_map(out.get(key)) and _is_map(value):
            out[key] = _filled(_object(out[key]), _object(value), f"{path}.{key}")
        else:
            out[key] = value
    return out


def _checked_branch(spec: Branch, given: Any, path: str, now: Any) -> dict[str, Any]:
    where = path or "the node"
    if not _is_map(given):
        first = next(iter(spec))
        raise GradeError(
            f"{where} holds the settings {_listed(spec)}: give an object such as "
            f'{{"{first}": ...}}, or a path with dots such as {path}.{first}'
        )
    out: dict[str, Any] = {}
    for key, value in _object(given).items():
        if key not in spec:
            close = difflib.get_close_matches(key, list(spec), n=1)
            maybe = f" (did you mean {close[0]}?)" if close else ""
            raise GradeError(f"{where} has no setting {key!r}{maybe}; it has {_listed(spec)}")
        out[key] = _checked(
            spec[key], value, f"{path}.{key}" if path else key, _object(now).get(key)
        )
    return out


def _listed(spec: Branch) -> str:
    return ", ".join(spec)


def _real(value: object) -> bool:
    return (
        isinstance(value, int | float)
        and not isinstance(value, bool)
        and math.isfinite(float(value))
    )


def _checked_number(spec: Number, given: Any, path: str) -> Any:
    if _real(given):
        if not spec.low <= float(given) <= spec.high:
            raise GradeError(f"{path} is {_number(float(given))}, out of range ({spec.told()})")
        return given
    if _is_map(given):
        return _checked_keys(spec, _object(given), path)
    raise GradeError(
        f"{path} ({spec.told()}) is a number, or keyframes "
        '{"keys": [{"frame": 0, "value": ..., "interp": "linear"}, ...]}'
    )


def _checked_keys(spec: Number, given: dict[str, Any], path: str) -> dict[str, Any]:
    keys = given.get("keys")
    if set(given) != {"keys"} or not _is_list(keys) or not keys:
        raise GradeError(
            f'{path} keyframes are {{"keys": [...]}} with one key at least, each '
            '{"frame": 0, "value": ..., "interp": "linear" | "hold" | '
            '{"bezier": {"out": [t, dv], "in": [t, dv]}}}'
        )
    out: list[dict[str, Any]] = []
    last: int | None = None
    for at, key in enumerate(_items(keys)):
        here = f"{path} key {at}"
        if not _is_map(key) or not {"frame", "value"} <= set(_object(key)):
            raise GradeError(f'{here} is {{"frame": n, "value": v, "interp": ...}}')
        key = _object(key)
        unknown = set(key) - {"frame", "value", "interp"}
        if unknown:
            raise GradeError(
                f"{here} has {', '.join(sorted(unknown))}; a key has frame, value and interp"
            )
        frame, value = key["frame"], key["value"]
        if type(frame) is not int or abs(frame) > _LARGEST_FRAME:
            raise GradeError(
                f"{here}: frame is a whole number of frames: of the clip's media (at its own "
                "rate, from 0) for a clip of media, of the clip from its first (0) for a title, "
                "a colour or a still, of the timeline for its grade"
            )
        if last is not None and frame <= last:
            raise GradeError(f"{here}: the frames of keys rise, each key at its own frame")
        last = frame
        if not _real(value) or not spec.low <= float(value) <= spec.high:
            raise GradeError(f"{here}: value is out of range ({spec.told()})")
        out.append(
            {"frame": frame, "value": value, "interp": _interp(key.get("interp", "linear"), here)}
        )
    return {"keys": out}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


def _interp(given: Any, where: str) -> Any:
    if given in ("linear", "hold"):
        return given
    bezier = _object(given).get("bezier") if _is_map(given) else None
    if _is_map(bezier) and set(_object(bezier)) == {"out", "in"}:
        handles = _object(bezier)
        if all(_handle(handles[side]) for side in ("out", "in")):
            return {"bezier": {"out": list(handles["out"]), "in": list(handles["in"])}}
    raise GradeError(
        f'{where}: interp is "linear", "hold" or {{"bezier": {{"out": [t, dv], "in": [t, dv]}}}}, '
        "t from 0 to 1 a share of the frames to the next key, dv a change of value"
    )


def _handle(value: object) -> bool:
    pair = _items(value)
    return len(pair) == 2 and all(_real(one) for one in pair) and 0 <= float(pair[0]) <= 1


def _checked_points(spec: Points, given: Any, path: str) -> list[list[float]]:
    told = (
        f"{path} ({spec.name}) is a list of points [in, out], in from 0 to 1 rising, out from "
        f"{_number(spec.low)} to {_number(spec.high)}"
        + (f", {spec.least} points at least" if spec.least else ", [] for a flat curve")
    )
    points = _items(given) if _is_list(given) else None
    if points is None or len(points) < spec.least:
        raise GradeError(told)
    out: list[list[float]] = []
    for point in points:
        pair = _items(point)
        if len(pair) != 2 or not all(_real(one) for one in pair):
            raise GradeError(told)
        x, y = float(pair[0]), float(pair[1])
        if not (0 <= x <= 1 and spec.low <= y <= spec.high) or (out and x <= out[-1][0]):
            raise GradeError(told)
        out.append([pair[0], pair[1]])
    return out


def _checked_items(spec: Items, given: Any, path: str) -> list[dict[str, Any]]:
    items = _items(given) if _is_list(given) else None
    if items is None or not spec.least <= len(items) <= spec.most:
        raise GradeError(
            f"{path} is a list of {spec.least} to {spec.most} inputs, each an object of "
            f"{_listed(spec.item)}; it replaces the list whole"
        )
    whole = _default(spec.item)
    return [
        _checked_branch(
            spec.item, _filled(whole, _object(one), f"{path}.{at}"), f"{path}.{at}", whole
        )
        for at, one in enumerate(items)
    ]


# ---------------------------------------------------------------------------------------------
# Summaries
# ---------------------------------------------------------------------------------------------


def _changed(spec: Spec, value: Any, path: str, out: dict[str, Any]) -> None:
    """Put into ``out`` each setting under ``path`` that is not as a new node has it."""
    if isinstance(spec, dict):
        if _is_map(value):
            for key, one in spec.items():
                _changed(one, _object(value).get(key), f"{path}.{key}" if path else key, out)
        return
    if isinstance(spec, Lut | Whole) or value is None:
        return
    if isinstance(spec, Items):
        if value != _default(spec):
            out[path] = value
        return
    if value != _default(spec):
        out[path] = value


def _links_into(grade: Mapping[str, Any]) -> dict[tuple[str, str, int], str]:
    into: dict[tuple[str, str, int], str] = {}
    for link in _items(grade.get("links")):
        one = _object(link)
        into[(str(one.get("to")), str(one.get("kind")), int(one.get("input", 0)))] = str(
            one.get("from")
        )
    return into


def _nodes(grade: Mapping[str, Any]) -> list[dict[str, Any]]:
    return [_object(one) for one in _items(grade.get("nodes"))]


def _inputs(node: Mapping[str, Any]) -> tuple[str, int]:
    """The kind of the inputs that a node takes its picture or key at, and how many it has."""
    kind, params = node.get("kind"), _object(node.get("params"))
    if kind == "key_mixer":
        return "key", len(_items(params.get("inputs")))
    if kind in ("parallel_mixer", "layer_mixer"):
        return "rgb", int(params.get("inputs", 2))
    return "rgb", 1


def _incomplete(grade: Mapping[str, Any]) -> str | None:
    """Why a grade changes nothing for being linked only in part, as ``Grade::is_complete``."""
    into = _links_into(grade)
    if ("output", "rgb", 0) not in into:
        return "the output is not linked, so the grade changes nothing"
    outs = {
        (str(_object(one).get("from")), str(_object(one).get("kind")))
        for one in _items(grade.get("links"))
    }
    for number, node in enumerate(_nodes(grade), 1):
        kind, count = _inputs(node)
        name = f"node {number} ({node.get('id')})"
        if any((str(node.get("id")), kind, at) not in into for at in range(count)):
            return f"{name} has an input that is not linked, so the grade changes nothing"
        if (str(node.get("id")), kind) not in outs:
            return f"the output of {name} goes nowhere, so the grade changes nothing"
    return None


def _node_summary(
    number: int,
    node: Mapping[str, Any],
    into: Mapping[tuple[str, str, int], str] | None,
    missing: Mapping[str, str],
) -> dict[str, Any]:
    id, kind, params = str(node.get("id")), str(node.get("kind")), _object(node.get("params"))
    item: dict[str, Any] = {
        "number": number,
        "id": id,
        "kind": kind,
        "label": str(node.get("label", "")),
        "enabled": bool(node.get("enabled", True)),
    }
    if kind == "corrector":
        if into is not None:
            item["input"] = into.get((id, "rgb", 0))
            if (id, "key", 0) in into:
                item["key_input"] = into[(id, "key", 0)]
        settings: dict[str, Any] = {}
        _changed(CORRECTOR, params, "", settings)
        item["settings"] = settings
        lut = params.get("lut")
        if isinstance(lut, str):
            item["lut"] = lut
            if lut in missing:
                item["lut_missing"] = missing[lut]
        if params.get("shot_match") is not None:
            item["shot_match"] = True
        qualifier = params.get("qualifier")
        if _is_map(qualifier):
            item["qualifier"] = _object(qualifier).get("mode", "hsl")
            _changed(QUALIFIER, qualifier, "qualifier", settings)
    elif kind in ("parallel_mixer", "layer_mixer"):
        count = int(params.get("inputs", 2))
        item["inputs"] = (
            [into.get((id, "rgb", at)) for at in range(count)] if into is not None else count
        )
        if kind == "layer_mixer":
            item["composite"] = params.get("composite", "normal")
    elif kind == "key_mixer":
        inputs: list[dict[str, Any]] = []
        for at, one in enumerate(_items(params.get("inputs"))):
            told: dict[str, Any] = {"from": into.get((id, "key", at))} if into is not None else {}
            _changed(_key_input(False, ""), one, "", told)
            inputs.append(told)
        item["inputs"] = inputs
        settings = {}
        _changed(_KEY_OUTPUT, params.get("output"), "output", settings)
        item["settings"] = settings
    return item


def summary(grade: Mapping[str, Any], missing: Mapping[str, str] | None = None) -> dict[str, Any]:
    """A grade as a model is to read it.

    Its nodes in order, each with its number (as DaVinci numbers them), id, kind, label and
    whether it is on; for a corrector, only the settings that change the picture, by their paths
    with dots in DaVinci's units, its LUT and whether that is missing, and whether Shot Match
    adjusted it; for a mixer, what goes into each of its inputs. ``input`` and ``inputs`` name
    the node (or ``source``) that each input is linked from, and ``output`` the node that the
    output takes its picture from. ``complete`` is false, with ``why``, for a grade that is
    linked only in part, which changes nothing.
    """
    missing = missing or {}
    into = _links_into(grade)
    nodes = [_node_summary(at, node, into, missing) for at, node in enumerate(_nodes(grade), 1)]
    out: dict[str, Any] = {"nodes": nodes, "output": into.get(("output", "rgb", 0))}
    why = _incomplete(grade)
    out["complete"] = why is None
    if why is not None:
        out["why"] = why
    lost = sorted({lut for lut in luts_of(grade) if lut in missing})
    if lost:
        out["missing_luts"] = [{"name": lut, "message": missing[lut]} for lut in lost]
    return out


def brief(
    grade: Mapping[str, Any] | None, missing: Mapping[str, str] | None = None
) -> dict[str, Any] | None:
    """A grade in short, for a listing of clips: its nodes and what they change, or ``None``.

    As :func:`summary`, without the links; a grade without nodes, which changes nothing, is
    ``None``.
    """
    if grade is None or not _nodes(grade):
        return None
    missing = missing or {}
    out: dict[str, Any] = {
        "nodes": [
            _node_summary(at, node, None, missing) for at, node in enumerate(_nodes(grade), 1)
        ]
    }
    why = _incomplete(grade)
    if why is not None:
        out["complete"] = False
        out["why"] = why
    return out


def luts_of(grade: Mapping[str, Any] | None) -> set[str]:
    """The LUTs that the correctors of a grade name."""
    if grade is None:
        return set()
    found: set[str] = set()
    for node in _nodes(grade):
        lut = _object(node.get("params")).get("lut")
        if node.get("kind") == "corrector" and isinstance(lut, str) and lut:
            found.add(lut)
    return found


def _same_name(name: str) -> str:
    """A name of a LUT as the system compares it: without case on Windows."""
    name = name.replace("\\", "/")
    return name.casefold() if os.name == "nt" else name


async def missing_luts(
    engine: Engine, names: Iterable[str], readable: Callable[[str], Path | None]
) -> dict[str, str]:
    """The LUTs among ``names`` that the engine would not find, each with why.

    A name is looked for as the engine looks for it (``engine/PROTOCOL.md``, Grades in the
    picture): an absolute path there, if the session may read it (``readable`` gives ``None``
    where it may not, and it is then not told of); another in the folder of the project and the
    library, as ``luts.list`` lists them. Only ``.cube`` files are read. A LUT that is there but
    does not parse is not found out here: the engine tells of it as it draws the picture.
    """
    wanted = {name for name in names if name}
    if not wanted:
        return {}
    found: dict[str, str] = {}
    listed: set[str] | None = None
    if any(not Path(name).is_absolute() for name in wanted):
        try:
            library = await engine.luts()
        except EngineError as error:
            if error.code != -32601:  # An engine from before luts.list cannot tell.
                raise
        else:
            if not library.truncated:
                listed = {_same_name(lut.name) for lut in library.luts}
    for name in sorted(wanted):
        if not name.lower().endswith(".cube"):
            found[name] = f"{name} is not a .cube file; only .cube LUTs are read"
        elif Path(name).is_absolute():
            path = readable(name)
            if path is not None and not await asyncio.to_thread(path.is_file):
                found[name] = f"there is no LUT file {name}"
        elif listed is not None and _same_name(name) not in listed:
            found[name] = f"there is no LUT {name} in the folder of the project or in the library"
    return found


# ---------------------------------------------------------------------------------------------
# Targets, grades and nodes of a project
# ---------------------------------------------------------------------------------------------


def _timeline_of(project: Mapping[str, Any], id: str | None) -> dict[str, Any]:
    timelines = [_object(one) for one in _items(project.get("timelines"))]
    if id is not None:
        for timeline in timelines:
            if timeline.get("id") == id:
                return timeline
        raise GradeError(f"there is no timeline {id}; video_info lists the timelines")
    if len(timelines) == 1:
        return timelines[0]
    if not timelines:
        raise GradeError("there is no timeline; create one with video_edit and add_timeline")
    raise GradeError("name the timeline: this project has more than one")


def _clips(timeline: Mapping[str, Any]) -> Iterable[tuple[dict[str, Any], dict[str, Any]]]:
    for track in _items(timeline.get("tracks")):
        for clip in _items(_object(track).get("clips")):
            yield _object(track), _object(clip)


def _groups(project: Mapping[str, Any]) -> list[dict[str, Any]]:
    return [_object(one) for one in _items(project.get("groups"))]


def _picture_clip(timeline: Mapping[str, Any], id: str) -> tuple[dict[str, Any], dict[str, Any]]:
    for track, clip in _clips(timeline):
        if clip.get("id") == id:
            if track.get("kind") != "video":
                raise GradeError(
                    f"{id} is a clip of sound on {track.get('id')}; grades work on clips of "
                    "picture tracks"
                )
            return track, clip
    raise GradeError(
        f"there is no clip {id} on timeline {timeline.get('id')}; video_info lists the clips"
    )


def target_of(
    given: str | Mapping[str, Any], project: Mapping[str, Any], timeline: str | None
) -> dict[str, Any]:
    """The ``grade`` target of the engine that a tool was given, checked against the project.

    ``given`` is a clip id, or ``{"level", "clip"?, "group"?, "timeline"?}``: ``clip`` (the
    default level) names its clip, ``pre_clip`` and ``post_clip`` their group, ``timeline`` the
    grade over the whole timeline. A timeline that is not given is ``timeline``, or the only one.

    Raises:
        GradeError: What it names is not there, or not all of it is given.
    """
    said: dict[str, Any] = (
        {"level": "clip", "clip": given}
        if isinstance(given, str)
        else {key: value for key, value in given.items() if value is not None}
    )
    level = said.get("level", "clip")
    if level == "clip":
        if not isinstance(said.get("clip"), str):
            raise GradeError('a clip grade needs its clip: {"level": "clip", "clip": "c4"}')
        if "group" in said:
            raise GradeError("group is for the pre_clip and post_clip levels")
        on = _timeline_of(project, said.get("timeline", timeline))
        _picture_clip(on, str(said["clip"]))
        return {"level": "clip", "timeline": on["id"], "clip": said["clip"]}
    if level in ("pre_clip", "post_clip"):
        group = said.get("group")
        if not isinstance(group, str):
            raise GradeError(
                f'a {level} grade needs its group: {{"level": "{level}", "group": "k1"}}'
            )
        if "clip" in said:
            raise GradeError("clip is for the clip level")
        if not any(one.get("id") == group for one in _groups(project)):
            raise GradeError(f"there is no group {group}; video_grade_info lists the groups")
        return {"level": level, "group": group}
    if level == "timeline":
        if "clip" in said or "group" in said:
            raise GradeError("the timeline level names only its timeline")
        on = _timeline_of(project, said.get("timeline", timeline))
        return {"level": "timeline", "timeline": on["id"]}
    raise GradeError(f"level is clip, pre_clip, post_clip or timeline, not {level!r}")


def grade_at(project: Mapping[str, Any], target: Mapping[str, Any]) -> dict[str, Any]:
    """The grade that a checked target names; the empty grade where none is written."""
    level = target["level"]
    if level in ("pre_clip", "post_clip"):
        group = next((one for one in _groups(project) if one.get("id") == target["group"]), None)
        if group is None:
            raise GradeError(f"there is no group {target['group']}; video_grade_info lists them")
        return _object(group.get("pre" if level == "pre_clip" else "post")) or EMPTY_GRADE
    on = _timeline_of(project, target["timeline"])
    if level == "timeline":
        return _object(on.get("grade")) or EMPTY_GRADE
    _, clip = _picture_clip(on, target["clip"])
    return _object(clip.get("grade")) or EMPTY_GRADE


def _named(target: Mapping[str, Any]) -> str:
    level = target["level"]
    if level == "clip":
        return f"the grade of clip {target['clip']}"
    if level == "timeline":
        return f"the grade of timeline {target['timeline']}"
    return f"the {level.replace('_', '-')} grade of group {target['group']}"


def node_of(
    grade: Mapping[str, Any], target: Mapping[str, Any], node: str | int, *, ends: bool = False
) -> str:
    """The id of a node of a grade, named by its id or by its number as DaVinci numbers them.

    ``source`` and ``output`` are taken where ``ends`` is true.

    Raises:
        GradeError: There is no such node.
    """
    nodes = _nodes(grade)
    if isinstance(node, str) and node.isdigit():
        node = int(node)
    if isinstance(node, int):
        if 1 <= node <= len(nodes):
            return str(nodes[node - 1]["id"])
        raise GradeError(
            f"{_named(target)} has {len(nodes)} nodes, numbered from 1; there is no node {node}"
        )
    if ends and node in ("source", "output"):
        return node
    if any(one.get("id") == node for one in nodes):
        return node
    ids = ", ".join(f"{at} {one.get('id')}" for at, one in enumerate(nodes, 1)) or "none"
    raise GradeError(f"{_named(target)} has no node {node}; its nodes are {ids}")


def _node(grade: Mapping[str, Any], id: str) -> dict[str, Any]:
    return next(one for one in _nodes(grade) if one.get("id") == id)


def _label(label: str | None) -> str | None:
    if label is not None and len(label) > _LONGEST_LABEL:
        raise GradeError(f"a label is at most {_LONGEST_LABEL} characters")
    return label


def _set_node(
    target: Mapping[str, Any],
    id: str,
    *,
    params: Mapping[str, Any] | None = None,
    label: str | None = None,
    enabled: bool | None = None,
) -> dict[str, Any]:
    op: dict[str, Any] = {"op": "set_node", "grade": dict(target), "id": id}
    if label is not None:
        op["label"] = label
    if enabled is not None:
        op["enabled"] = enabled
    if params is not None:
        op["params"] = dict(params)
    return op


def _keyed_setting(node: Mapping[str, Any], path: str) -> tuple[Number, Any]:
    """The setting at ``path`` of a node, which takes keys, and what it is now.

    The path is written as the engine writes it: with dots, and an item of a list by its
    number from 0, ``inputs.0.gain``.

    Raises:
        GradeError: The node has no such setting, or it is not a number.
    """
    kind = str(node.get("kind"))
    known = _SPECS.get(kind)
    if known is None:
        raise GradeError(f"node {node.get('id')} is a {kind}, which this tool cannot set")
    spec: Spec = known
    parts = path.split(".")
    if not all(parts):
        raise GradeError(f"{path!r} is not a path of a setting, such as primaries.gain.r")
    now: Any = node.get("params")
    where = ""
    for part in parts:
        if isinstance(spec, Whole):
            if now is None:
                raise GradeError(
                    f"the node has no {where} ({spec.name}): give it with video_grade_set first"
                )
            spec = spec.fields
        if isinstance(spec, Items):
            items = _items(now)
            if not part.isdecimal() or int(part) >= len(items):
                raise GradeError(
                    f"{where} has {len(items)} items, named by their numbers from 0: "
                    f"{where}.0.{next(iter(spec.item))}"
                )
            spec, now = spec.item, items[int(part)]
        elif isinstance(spec, dict):
            if part not in spec:
                close = difflib.get_close_matches(part, list(spec), n=1)
                maybe = f" (did you mean {close[0]}?)" if close else ""
                raise GradeError(
                    f"{where or 'the node'} has no setting {part!r}{maybe}; it has {_listed(spec)}"
                )
            spec, now = spec[part], _object(now).get(part)
        else:
            raise GradeError(f"{where} ({spec.name}) holds no setting {part!r}")
        where = f"{where}.{part}" if where else part
    if isinstance(spec, Number):
        return spec, now
    if isinstance(spec, dict):
        raise GradeError(f"{path} holds the settings {_listed(spec)}: name one that is a number")
    raise GradeError(
        f"{path} ({spec.name}) takes no keys: only a setting that is a number does; "
        "video_grade_set sets it"
    )


def _key_op(
    target: Mapping[str, Any],
    id: str,
    path: str,
    setting: tuple[Number, Any],
    action: KeyAction,
    *,
    frame: int | None,
    value: float | None,
    interp: Any,
) -> dict[str, Any]:
    """The operation that puts, takes away or clears the keys of a setting.

    Raises:
        GradeError: What the action needs is not given, or there is no such key.
    """
    spec, now = setting
    keys = _items(_object(now).get("keys")) if _is_map(now) else []
    op: dict[str, Any] = {"grade": dict(target), "id": id, "path": path}
    at_frames = ", ".join(str(key["frame"]) for key in keys)
    if action == "clear":
        if not keys:
            raise GradeError(f"{path} has no keys to clear")
        if value is None:
            left = keys[0]["value"]
        elif _real(value):
            left = _checked_number(spec, value, path)
        else:
            raise GradeError(f"{path}: value is the number that the setting is left at")
        return {"op": "set_keys", **op, "param": left}
    if type(frame) is not int:
        raise GradeError(f"{action} needs frame, the frame that the key is at")
    at = next((n for n, key in enumerate(keys) if key["frame"] >= frame), len(keys))
    there = at < len(keys) and keys[at]["frame"] == frame
    if action == "remove":
        if not there:
            has = f"its keys are at {at_frames}" if keys else "it has no keys"
            raise GradeError(f"{path} has no key at frame {frame}: {has}")
        if len(keys) == 1:
            # A setting keeps one key at least: without its last it is the number of that key.
            return {"op": "set_keys", **op, "param": keys[0]["value"]}
        return {"op": "splice_keys", **op, "at": at, "remove": 1, "keys": []}
    if value is None:
        raise GradeError(f"put needs value, what {path} is at the key ({spec.told()})")
    given: dict[str, Any] = {"frame": frame, "value": value}
    if interp is not None:
        given["interp"] = interp
    elif there:
        given["interp"] = keys[at]["interp"]
    (key,) = _checked_keys(spec, {"keys": [given]}, path)["keys"]
    return {"op": "splice_keys", **op, "at": at, "remove": int(there), "keys": [key]}


def _picture_clips(
    project: Mapping[str, Any], timeline: str | None, clips: Sequence[str]
) -> tuple[str, list[str]]:
    on = _timeline_of(project, timeline)
    for clip in clips:
        _picture_clip(on, clip)
    return str(on["id"]), list(clips)


# ---------------------------------------------------------------------------------------------
# What the tools do
# ---------------------------------------------------------------------------------------------


Build = Callable[[Mapping[str, Any]], list[dict[str, Any]]]
"""What works a change out from the project as it is: the operations and edits to send."""


class Grader:
    """The tools of grades, each change one transaction of the engine, one step of its history.

    Args:
        engine: The engine of the project.
        readable: The path that a LUT named by an absolute path is at, where the session may
            read it, or ``None``.
        lut_path: The same, raising what the session's rules say where it may not.
    """

    def __init__(
        self,
        engine: Engine,
        readable: Callable[[str], Path | None],
        lut_path: Callable[[str], Path],
    ) -> None:
        self.engine = engine
        self._readable = readable
        self._lut_path = lut_path

    async def _applied(self, build: Build) -> tuple[Applied, dict[str, Any]]:
        """Send what ``build`` works out, at the revision that it was worked out from.

        Ids and node numbers are found in the project as it was read, and a node that is added
        is given ``n`` and the ``next_id`` of the project, as the engine gives it. Where another
        client changed the project meanwhile, the change is worked out again.
        """
        for attempt in range(_ATTEMPTS):
            revision, project = await self.engine.project()
            session = self.engine.session
            ops = build(project)
            try:
                done = await self.engine.apply(ops, revision=revision, session=session)
            except EngineError as error:
                if (error.data or {}).get("reason") == "stale_revision" and attempt + 1 < _ATTEMPTS:
                    continue
                raise
            return done, (await self.engine.project())[1]
        raise AssertionError("unreachable")  # pragma: no cover

    async def _missing(self, grades: Iterable[Mapping[str, Any] | None]) -> dict[str, str]:
        names: set[str] = set()
        for grade in grades:
            names |= luts_of(grade)
        return await missing_luts(self.engine, names, self._readable)

    async def _receipt(
        self, done: Applied, project: Mapping[str, Any], targets: Sequence[Mapping[str, Any]]
    ) -> dict[str, Any]:
        grades = [grade_at(project, target) for target in targets]
        missing = await self._missing(grades)
        told: dict[str, Any] = {"revision": done.revision, "operations": len(done.done)}
        if done.names:
            told["names"] = done.names
        told["grades"] = [
            {"grade": dict(target), **summary(grade, missing)}
            for target, grade in zip(targets, grades, strict=True)
        ]
        return told

    async def info(
        self, timeline: str | None, given: str | Mapping[str, Any] | None
    ) -> dict[str, Any]:
        """Summarise one grade, or every grade of a timeline where none is named."""
        _, project = await self.engine.project()
        if given is not None:
            target = target_of(given, project, timeline)
            grade = grade_at(project, target)
            told: dict[str, Any] = {"grade": target, **summary(grade, await self._missing([grade]))}
            if target["level"] == "clip":
                _, clip = _picture_clip(_timeline_of(project, target["timeline"]), target["clip"])
                if clip.get("group") is not None:
                    told["group"] = clip["group"]
            return told
        on = _timeline_of(project, timeline)
        clips = [(track, clip) for track, clip in _clips(on) if track.get("kind") == "video"]
        used = {clip.get("group") for _, clip in clips if clip.get("group") is not None}
        groups = [one for one in _groups(project) if one.get("id") in used]
        missing = await self._missing(
            [
                _object(on.get("grade")),
                *(_object(clip.get("grade")) for _, clip in clips),
                *(_object(one.get(level)) for one in groups for level in ("pre", "post")),
            ]
        )
        return {
            "timeline": on["id"],
            "grade": summary(_object(on.get("grade")) or EMPTY_GRADE, missing),
            "clips": [
                {
                    "clip": clip["id"],
                    "track": track["id"],
                    "start": clip.get("start"),
                    **({"group": clip["group"]} if clip.get("group") is not None else {}),
                    "grade": brief(_object(clip.get("grade")), missing),
                }
                for track, clip in clips
                if brief(_object(clip.get("grade"))) is not None or clip.get("group") is not None
            ],
            "groups": [_group_told(one, project, missing) for one in groups],
        }

    async def set(
        self,
        timeline: str | None,
        given: str | Mapping[str, Any],
        node: str | int,
        settings: Mapping[str, Any] | None,
        label: str | None,
        enabled: bool | None,
    ) -> dict[str, Any]:
        """Patch the settings of a node, its label or whether it is on."""
        if not settings and label is None and enabled is None:
            raise GradeError("give settings, label or enabled to change")
        _label(label)
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            target = target_of(given, project, timeline)
            grade = grade_at(project, target)
            id = node_of(grade, target, node)
            patch = checked_patch(settings, _node(grade, id)) if settings else None
            targets[:] = [target]
            return [_set_node(target, id, params=patch, label=label, enabled=enabled)]

        done, project = await self._applied(build)
        return await self._receipt(done, project, targets)

    async def keys(
        self,
        timeline: str | None,
        given: str | Mapping[str, Any],
        node: str | int,
        path: str,
        action: KeyAction,
        *,
        frame: int | None = None,
        value: float | None = None,
        interp: Any = None,
    ) -> dict[str, Any]:
        """Put a key of one setting of a node, take one away, or clear them all.

        The other keys of the setting are not sent: a key put at a frame that has one takes its
        place, and keeps its interpolation where none is given.
        """
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            target = target_of(given, project, timeline)
            grade = grade_at(project, target)
            id = node_of(grade, target, node)
            setting = _keyed_setting(_node(grade, id), path)
            targets[:] = [target]
            return [
                _key_op(target, id, path, setting, action, frame=frame, value=value, interp=interp)
            ]

        done, project = await self._applied(build)
        return await self._receipt(done, project, targets)

    async def node(
        self,
        timeline: str | None,
        given: str | Mapping[str, Any],
        action: NodeAction,
        node: str | int | None,
        label: str | None,
        settings: Mapping[str, Any] | None,
    ) -> dict[str, Any]:
        """Add a corrector as a key of the Color page adds one, or remove, reset or extract one.

        A corrector that is added may be given its label and settings in the same transaction.
        """
        adding = action in (
            "append",
            "add_serial",
            "add_serial_before",
            "add_parallel",
            "add_layer",
            "add_outside",
        )
        if not adding and (label is not None or settings):
            raise GradeError(f"label and settings are for a node that is added, not for {action}")
        if action != "append" and node is None:
            raise GradeError(f"{action} needs the node it is done to: its id or number")
        _label(label)
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            target = target_of(given, project, timeline)
            grade = grade_at(project, target)
            targets[:] = [target]
            ops: list[dict[str, Any]] = []
            if action == "append":
                ops.append({"edit": "append_node", "grade": target})
            elif action == "remove":
                ops.append(
                    {
                        "op": "remove_node",
                        "grade": target,
                        "id": node_of(grade, target, _given(node)),
                    }
                )
            elif action in ("reset", "extract"):
                ops.append(
                    {
                        "edit": f"{action}_node",
                        "grade": target,
                        "node": node_of(grade, target, _given(node)),
                    }
                )
            else:
                ops.append(
                    {
                        "edit": action,
                        "grade": target,
                        "node": node_of(grade, target, _given(node), ends=True),
                    }
                )
            if adding and (label is not None or settings):
                # The corrector that the edit adds is the first node it adds, and the engine
                # gives it n and the next_id of the project: no node has an id from there on.
                params = new_corrector()
                if action == "add_outside":
                    params["key"]["input"]["invert"] = True
                new = {"id": f"n{project['next_id']}", "kind": "corrector", "params": params}
                patch = checked_patch(settings, new) if settings else None
                ops.append(_set_node(target, str(new["id"]), params=patch, label=label))
            return ops

        done, project = await self._applied(build)
        told = await self._receipt(done, project, targets)
        added = [
            str(_object(op.get("node")).get("id"))
            for op in done.done
            if op.get("op") == "add_node" and _object(op.get("node")).get("kind") == "corrector"
        ]
        if adding and added:
            told["added"] = added[0]
        return told

    async def copy(
        self,
        timeline: str | None,
        action: CopyAction,
        source: str | Mapping[str, Any],
        to: Sequence[str | Mapping[str, Any]],
        *,
        preserve_nodes: int | None,
        from_node: str | int | None,
        to_node: str | int | None,
        param: str | None,
        keyframes: KeyAlign | None = None,
    ) -> dict[str, Any]:
        """Copy a grade, append its nodes, or paste a node or one of its settings.

        ``keyframes`` says where the keys of a grade copied or appended go (``copy_grade`` and
        ``append_node_graph``, PROTOCOL "Keys copied to another clip"): ``source_timecode``, the
        engine's default, or ``start_frame``.
        """
        if not to:
            raise GradeError("to names the grades to copy into, one at least")
        if keyframes is not None and action not in ("copy", "append"):
            raise GradeError("keyframes is for copy and append")
        if preserve_nodes is not None and action != "copy":
            raise GradeError("preserve_nodes is for copy")
        if preserve_nodes is not None and not 0 <= preserve_nodes <= _MOST_PRESERVED:
            raise GradeError(f"preserve_nodes is from 0 to {_MOST_PRESERVED}")
        pasting = action in ("paste_node", "paste_value")
        if pasting and (len(to) != 1 or from_node is None or to_node is None):
            raise GradeError(f"{action} takes from_node, one grade in to, and to_node")
        if action == "paste_value" and not param:
            raise GradeError(
                "paste_value takes param, the path of the setting, such as primaries.saturation"
            )
        if param is not None:
            _setting(param)
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            origin = target_of(source, project, timeline)
            into = [target_of(one, project, timeline) for one in to]
            targets[:] = into
            if action == "copy":
                edit: dict[str, Any] = {"edit": "copy_grade", "from": origin, "to": into}
                if preserve_nodes:
                    edit["preserve_nodes"] = preserve_nodes
                if keyframes is not None:
                    edit["keyframes"] = keyframes
                return [edit]
            if action == "append":
                edit = {"edit": "append_node_graph", "from": origin, "to": into}
                if keyframes is not None:
                    edit["keyframes"] = keyframes
                return [edit]
            edit = {
                "edit": action,
                "from": origin,
                "from_node": node_of(grade_at(project, origin), origin, _given(from_node)),
                "to": into[0],
                "to_node": node_of(grade_at(project, into[0]), into[0], _given(to_node)),
            }
            if action == "paste_value":
                edit["param"] = param
            return [edit]

        done, project = await self._applied(build)
        return await self._receipt(done, project, targets)

    async def lut(
        self,
        timeline: str | None,
        action: Literal["set", "clear", "list", "export"],
        given: str | Mapping[str, Any] | None,
        node: str | int | None,
        lut: str | None,
        folder: str | None,
        *,
        size: Literal[17, 33, 65] = 33,
        path: str | None = None,
    ) -> dict[str, Any]:
        """List the LUTs that a node may name, set or clear the LUT of a node, or export one.

        A LUT set where no node is named goes on a corrector appended for it. ``export`` bakes
        the grade (or its node) into a ``.cube`` at ``path``, as Generate LUT does.
        """
        if action == "export":
            return await self._export_lut(timeline, given, node, size, path)
        if action == "list":
            library = await self.engine.luts(folder)
            return {
                "roots": [
                    {"place": root.place, "path": root.path, "exists": root.exists}
                    for root in library.roots
                ],
                "luts": [
                    {
                        "name": one.name,
                        **({"kind": one.kind} if one.kind is not None else {}),
                        **({"shadowed": True} if one.shadowed else {}),
                        **({"error": one.message} if one.status == "error" else {}),
                    }
                    for one in library.luts
                ],
                "truncated": library.truncated,
            }
        if given is None:
            raise GradeError(f"{action} needs the grade of the node")
        if action == "clear" and node is None:
            raise GradeError("clear needs the node whose LUT is taken away")
        if action == "set" and not lut:
            raise GradeError("set needs lut, a name from action list, or an absolute path")
        name = await self._lut_named(lut) if action == "set" and lut else None
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            target = target_of(given, project, timeline)
            grade = grade_at(project, target)
            targets[:] = [target]
            if node is None:
                id = f"n{project['next_id']}"
                return [
                    {"edit": "append_node", "grade": target},
                    _set_node(target, id, params={"lut": name}),
                ]
            id = node_of(grade, target, node)
            if _node(grade, id).get("kind") != "corrector":
                raise GradeError(
                    f"node {node} is a {_node(grade, id).get('kind')}; a LUT is on a corrector"
                )
            return [_set_node(target, id, params={"lut": name})]

        done, project = await self._applied(build)
        return await self._receipt(done, project, targets)

    async def _export_lut(
        self,
        timeline: str | None,
        given: str | Mapping[str, Any] | None,
        node: str | int | None,
        size: Literal[17, 33, 65],
        path: str | None,
    ) -> dict[str, Any]:
        """Bake a grade, or a node of it, into a ``.cube`` at ``path``; say what was baked."""
        if given is None:
            raise GradeError("export needs the grade to bake")
        if not path:
            raise GradeError(
                "export needs path: a .cube name in the project folder, or an absolute path"
            )
        _, project = await self.engine.project()
        target = target_of(given, project, timeline)
        id = None if node is None else node_of(grade_at(project, target), target, node)
        return await self.engine.export_lut(target, path, size=size, node=id)

    async def _lut_named(self, given: str) -> str:
        """The name of a LUT to set: as the library spells it, or a whole path that may be read.

        Raises:
            GradeError: There is no such LUT.
        """
        if Path(given).is_absolute():
            path = self._lut_path(given)
            if path.suffix.lower() != ".cube" or not await asyncio.to_thread(path.is_file):
                raise GradeError(f"there is no .cube file {given}")
            return str(path)
        if not given.lower().endswith(".cube"):
            raise GradeError(f"{given} is not a .cube file; only .cube LUTs are read")
        try:
            library = await self.engine.luts()
        except EngineError as error:
            if error.code == -32601:  # An engine from before luts.list: it is told as missing.
                return given
            raise
        named = [one for one in library.luts if not one.shadowed]
        names = [one.name for one in named]
        for one in named:
            if _same_name(one.name) == _same_name(given):
                if one.status == "error":
                    raise GradeError(f"the LUT {one.name} cannot be used: {one.message}")
                return one.name
        if library.truncated:
            return given
        close = difflib.get_close_matches(given, names, n=5, cutoff=0.4)
        maybe = f"; the closest are {', '.join(close)}" if close else ""
        raise GradeError(
            f"there is no LUT {given} in the folder of the project or in the library{maybe}; "
            "video_grade_lut with action list lists them"
        )

    async def group(
        self,
        timeline: str | None,
        action: GroupAction,
        *,
        group: str | None,
        name: str | None,
        clips: Sequence[str],
        level: Literal["pre", "post"] | None,
        source: str | Mapping[str, Any] | None,
    ) -> dict[str, Any]:
        """Make, fill, empty, rename, delete or collapse a group, or set its pre or post grade."""
        if action in ("create", "rename") and not name:
            raise GradeError(f"{action} needs the name of the group")
        if name is not None and len(name) > _LONGEST_LABEL:
            raise GradeError(f"a name of a group is at most {_LONGEST_LABEL} characters")
        if action in ("add", "rename", "delete", "grade") and group is None:
            raise GradeError(f"{action} needs the group, by its id")
        if action in ("add", "remove") and not clips:
            raise GradeError(f"{action} needs the clips")
        if action == "collapse" and not clips and group is None:
            raise GradeError("collapse needs the clips, or the group whose clips are collapsed")
        if action == "grade" and level is None:
            raise GradeError("grade needs level: pre or post")
        if source is not None and action not in ("create", "grade"):
            raise GradeError("from is for create and grade")
        if action == "create" and source is not None and level is None:
            raise GradeError("from needs level: pre or post")
        made: list[str] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            known = {str(one.get("id")) for one in _groups(project)}
            if group is not None and action != "create" and group not in known:
                raise GradeError(f"there is no group {group}; video_grade_info lists the groups")
            if action == "create":
                on, chosen = _picture_clips(project, timeline, clips)
                ops: list[dict[str, Any]] = [
                    {
                        "op": "add_group",
                        "group": {
                            "id": "$group",
                            "name": name,
                            "pre": copy.deepcopy(EMPTY_GRADE),
                            "post": copy.deepcopy(EMPTY_GRADE),
                        },
                    }
                ]
                ops += [
                    {"op": "set_clip_group", "timeline": on, "clip": clip, "group": "$group"}
                    for clip in chosen
                ]
                if source is not None and level is not None:
                    ops.append(_group_grade(project, timeline, source, "$group", level))
                return ops
            if action in ("add", "remove"):
                on, chosen = _picture_clips(project, timeline, clips)
                into = group if action == "add" else None
                return [
                    {"op": "set_clip_group", "timeline": on, "clip": clip, "group": into}
                    for clip in chosen
                ]
            if action == "rename":
                return [{"op": "set_group", "id": group, "name": name}]
            if action == "delete":
                ops = [
                    {
                        "op": "set_clip_group",
                        "timeline": one["id"],
                        "clip": clip["id"],
                        "group": None,
                    }
                    for one in map(_object, _items(project.get("timelines")))
                    for _, clip in _clips(one)
                    if clip.get("group") == group
                ]
                return [*ops, {"op": "remove_group", "id": group}]
            if action == "grade":
                assert group is not None
                assert level is not None
                if source is None:
                    target = {"level": f"{level}_clip", "group": group}
                    return [
                        {"op": "set_grade", "grade": target, "value": copy.deepcopy(EMPTY_GRADE)}
                    ]
                return [_group_grade(project, timeline, source, group, level)]
            on = _timeline_of(project, timeline)
            chosen = list(clips) or [
                str(clip["id"]) for _, clip in _clips(on) if clip.get("group") == group
            ]
            if not chosen:
                raise GradeError(f"no clip of timeline {on['id']} is in group {group}")
            for clip in chosen:
                _picture_clip(on, clip)
            made[:] = chosen
            return [{"edit": "collapse_group_grades", "timeline": on["id"], "clips": chosen}]

        done, project = await self._applied(build)
        told: dict[str, Any] = {"revision": done.revision, "operations": len(done.done)}
        if done.names:
            told["names"] = done.names
        id = done.names.get("$group", group)
        found = next((one for one in _groups(project) if one.get("id") == id), None)
        if found is not None:
            missing = await self._missing([_object(found.get("pre")), _object(found.get("post"))])
            told["group"] = _group_told(found, project, missing)
        if made:
            told["collapsed"] = made
        return told

    # -----------------------------------------------------------------------------------------
    # Scopes, colour management, Shot Match and Auto Color
    # -----------------------------------------------------------------------------------------

    async def scopes(
        self,
        timeline: str | None,
        *,
        frame: int | None,
        clip: str | None,
        given: str | Mapping[str, Any] | None,
        node: str | int | None,
        at: NodeSide | None,
        histogram: int | None,
    ) -> dict[str, Any]:
        """The figures of a frame drawn off screen, or of the picture at a node there.

        The frame is ``frame``, or the middle of ``clip``; a node is of ``given`` (the grade of
        ``clip`` where that is left out). Nothing is changed, and no monitor is needed.
        """
        if node is None and (given is not None or at is not None):
            raise GradeError("grade and at are of a node: give node too")
        if node is not None and given is None and clip is None:
            raise GradeError("node needs the grade it is of, or the clip whose grade it is")
        _, project = await self.engine.project()
        on = _timeline_of(project, timeline)
        target: dict[str, Any] | None = None
        id: str | None = None
        if node is not None:
            target = target_of(given if given is not None else str(clip), project, timeline)
            id = node_of(grade_at(project, target), target, node)
            if clip is None and target["level"] == "clip":
                clip = str(target["clip"])
        if frame is None:
            if clip is None:
                raise GradeError("give frame, or clip to measure the middle of the clip")
            _, placed = _picture_clip(on, clip)
            frame = _middle(placed)
        elif clip is not None:
            _picture_clip(on, clip)
        figures = await self.engine.statistics(
            timeline=str(on["id"]),
            frame=frame,
            grade=target,
            node=id,
            at=at,
            clip=clip if target is not None and target["level"] != "clip" else None,
            bins=_BINS,
        )
        told: dict[str, Any] = {
            "timeline": on["id"],
            "frame": figures.frame,
            "width": figures.width,
            "height": figures.height,
            "samples": figures.samples,
            "measured": (
                {"grade": target, "node": id, "at": at or "output"}
                if target is not None
                else "the frame as an export draws it"
            ),
        }
        for name in ("red", "green", "blue", "luma"):
            channel = getattr(figures, name)
            told[name] = {
                "min": round(channel.min, 4),
                "max": round(channel.max, 4),
                "mean": round(channel.mean, 4),
                "percentiles": {
                    str(level): round(shotmatch.percentile(figures.histogram[name], level / 100), 4)
                    for level in shotmatch.LEVELS
                },
            }
        chroma = figures.chroma
        told["chroma"] = {
            "centroid": [round(one, 4) for one in chroma.centroid],
            "spread": [round(one, 4) for one in chroma.spread],
            "saturation": round(chroma.saturation, 4),
            "hue": round(chroma.hue, 2),
        }
        told["mid_grey"] = round(_figures(figures).grey, 4)
        if histogram:
            told["histogram"] = {
                "bins": histogram,
                **{
                    name: _coarse(figures.histogram[name], histogram)
                    for name in ("red", "green", "blue", "luma")
                },
            }
        return told

    async def color_settings(self, settings: Mapping[str, Any] | None) -> dict[str, Any]:
        """The project's colour management, or a patch of it set as one transaction."""
        if settings is None:
            revision, project = await self.engine.project()
            color = _object(project.get("color"))
            return {"revision": revision, "color": color, "in_force": in_force(color)}
        if not settings:
            raise GradeError("settings is empty: give the fields to change, such as science")
        done, project = await self._applied(
            lambda _: [{"op": "set_color_settings", "settings": dict(settings)}]
        )
        color = _object(project.get("color"))
        return {"revision": done.revision, "color": color, "in_force": in_force(color)}

    async def media_color(
        self,
        media: Sequence[str],
        *,
        input: str | None,
        levels: Literal["auto", "video", "full"] | None,
    ) -> dict[str, Any]:
        """The colour of media: listed, or its input space and data levels set by hand.

        ``input`` is a space, ``bypass``, or ``default`` (the project's, Project Default).
        """
        changing = input is not None or levels is not None
        if changing and not media:
            raise GradeError("name the media to set: their ids, as video_info lists them")

        def chosen(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            items = [_object(one) for one in _items(project.get("media"))]
            if not media:
                return [one for one in items if _object(one.get("facts")).get("video")]
            known = {str(one.get("id")): one for one in items}
            missing = [one for one in media if one not in known]
            if missing:
                raise GradeError(
                    f"there is no media {', '.join(missing)}; video_info lists the media"
                )
            return [known[one] for one in media]

        revision: int | None = None
        if changing:

            def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
                ops: list[dict[str, Any]] = []
                for one in chosen(project):
                    op: dict[str, Any] = {"op": "set_media_color", "id": one["id"]}
                    if input is not None:
                        op["input"] = None if input == "default" else input
                    if levels is not None:
                        op["levels"] = levels
                    ops.append(op)
                return ops

            done, project = await self._applied(build)
            revision = done.revision
        else:
            revision, project = await self.engine.project()
        told: list[dict[str, Any]] = []
        for one in chosen(project):
            item: dict[str, Any] = {
                "media": one.get("id"),
                "name": one.get("name"),
                "color_override": {"input": None, "levels": "auto"}
                | _object(one.get("color_override")),
            }
            facts = _object(_object(one.get("facts")).get("video")).get("color")
            if facts is not None:
                item["file"] = facts
            told.append(item)
        return {"revision": revision, "media": told, "in_force": in_force(project.get("color"))}

    async def shot_match(
        self,
        timeline: str | None,
        clips: Sequence[str],
        target: str | Mapping[str, Any] | None,
        *,
        frame: int | None,
        node: str | int | None,
        auto: bool,
        reset: bool,
    ) -> dict[str, Any]:
        """Match clips to a target clip or still, balance them by Auto Color, or reset a match.

        A match goes into the ``shot_match`` of a corrector of each clip, Auto Color into its
        Lift and Gain; a node is ``node`` of each clip's grade, else its last corrector, else one
        appended. Each call is one step of the history, whatever it adds and sets.
        """
        if not clips:
            raise GradeError("clips names the clips, one at least")
        if len(set(clips)) != len(clips):
            raise GradeError("a clip is named twice in clips")
        if auto and reset:
            raise GradeError("auto and reset are two things: give one")
        if (auto or reset) and (target is not None or frame is not None):
            raise GradeError(
                f"{'auto' if auto else 'reset'} works on each clip alone: no target or frame"
            )
        if not (auto or reset) and target is None:
            raise GradeError(
                'target names the clip (its id) or the still ({"still": "s1"}) to match to'
            )
        if reset:
            return await self._unmatched(timeline, clips, node)
        for attempt in range(_ATTEMPTS):
            told = await self._matched(timeline, clips, target, frame, node, auto)
            if told is not None:
                return told
            if attempt + 1 == _ATTEMPTS:
                break
        raise GradeError("the project kept changing while the clips were measured; call again")

    async def _unmatched(
        self, timeline: str | None, clips: Sequence[str], node: str | int | None
    ) -> dict[str, Any]:
        targets: list[dict[str, Any]] = []

        def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
            ops: list[dict[str, Any]] = []
            targets.clear()
            for clip in clips:
                target = target_of(clip, project, timeline)
                grade = grade_at(project, target)
                ids = (
                    [node_of(grade, target, node)]
                    if node is not None
                    else [
                        str(one["id"])
                        for one in _nodes(grade)
                        if _object(one.get("params")).get("shot_match") is not None
                    ]
                )
                for id in ids:
                    if _object(_node(grade, id).get("params")).get("shot_match") is None:
                        raise GradeError(f"node {id} of clip {clip} has no shot match")
                    ops.append(_set_node(target, id, params={"shot_match": None}))
                targets.append(target)
            if not ops:
                raise GradeError("none of the clips has a shot match")
            return ops

        done, project = await self._applied(build)
        return await self._receipt(done, project, targets)

    async def _matched(
        self,
        timeline: str | None,
        clips: Sequence[str],
        given: str | Mapping[str, Any] | None,
        frame: int | None,
        node: str | int | None,
        auto: bool,
    ) -> dict[str, Any] | None:
        """One attempt at a match or Auto Color; ``None`` where the project changed under it."""
        revision, project = await self.engine.project()
        session = self.engine.session
        on = _timeline_of(project, timeline)
        timeline_id = str(on["id"])
        still = _still_of(given)
        source: str | None = None if given is None or still is not None else _clip_of(given)
        if source is not None:
            _picture_clip(on, source)
        elif frame is not None:
            raise GradeError("frame is of a target clip; a still is matched as it was grabbed")
        plans: list[_Plan] = []
        skipped: list[dict[str, Any]] = []
        for clip in clips:
            if clip == source:
                skipped.append({"clip": clip, "why": "it is the target"})
                continue
            target = target_of(clip, project, timeline_id)
            grade = grade_at(project, target)
            if node is not None:
                id: str | None = node_of(grade, target, node)
                kind = _node(grade, str(id)).get("kind")
                if kind != "corrector":
                    raise GradeError(
                        f"node {node} of clip {clip} is a {kind}; a match goes on a corrector"
                    )
            else:
                id = _last_corrector(grade)
            _, placed = _picture_clip(on, clip)
            plans.append(_Plan(clip, target, id, _middle(placed), _graded(grade)))
        if not plans:
            raise GradeError("there is no clip to match but the target")
        probe: _Plan | None = None
        if source is not None:
            target = target_of(source, project, timeline_id)
            _, placed = _picture_clip(on, source)
            at = _middle(placed) if frame is None else frame
            if not int(placed["start"]) <= at < int(placed["start"]) + int(placed["length"]):
                raise GradeError(
                    f"frame {at} is not of clip {source}, which is on frames "
                    f"{placed['start']} to {int(placed['start']) + int(placed['length']) - 1}"
                )
            probe = _Plan(source, target, _output_of(grade_at(project, target)), at, False)
        adding = [plan for plan in plans if plan.node is None]
        if probe is not None and probe.node is None:
            adding.append(probe)
        gesture: dict[str, Any] | None = None
        at_revision = revision
        if adding:
            # A grade without a node is given one, to measure the picture that comes out of it
            # (the target's only for that, taken away again), in a gesture: one step to undo.
            gesture = {"id": f"shot-match-{secrets.token_hex(6)}"}
            edits = [{"edit": "append_node", "grade": plan.target} for plan in adding]
            try:
                first = await self.engine.apply(
                    edits, revision=revision, session=session, gesture=gesture
                )
            except EngineError as error:
                if _stale(error):
                    return None
                raise
            added = [
                str(_object(op.get("node")).get("id"))
                for op in first.done
                if op.get("op") == "add_node" and _object(op.get("node")).get("kind") == "corrector"
            ]
            for plan, id in zip(adding, added, strict=True):
                plan.node = id
                plan.added = True
            at_revision = first.revision
        originals = [
            {"op": "set_grade", "grade": plan.target, "value": grade_at(project, plan.target)}
            for plan in adding
        ]
        ended = None if gesture is None else {**gesture, "end": True}
        try:
            ops, told = await self._worked(timeline_id, plans, probe, still, auto)
        except Exception:
            if ended is not None:
                # Back to the project as it was: a gesture that ends so leaves no step.
                with contextlib.suppress(EngineError):
                    await self.engine.apply(
                        originals, revision=at_revision, session=session, gesture=ended
                    )
            raise
        if skipped:
            told["skipped"] = skipped + told.get("skipped", [])
        if probe is not None and probe.added:
            ops.append(originals[-1])
        if not ops:
            if ended is not None:
                await self.engine.apply(
                    originals, revision=at_revision, session=session, gesture=ended
                )
            return {"operations": 0, **told}
        try:
            done = await self.engine.apply(
                ops, revision=at_revision, session=session, gesture=ended
            )
        except EngineError as error:
            if not _stale(error):
                raise
            if probe is not None and probe.added:
                await self._probe_taken_away(probe, originals[-1])
            return None
        told = {"revision": done.revision, "operations": len(done.done), **told}
        return told

    async def _probe_taken_away(self, probe: "_Plan", original: Mapping[str, Any]) -> None:
        """Take away the node that a target was given to be measured, as a step of its own.

        That is where another change stopped the match and left it.
        """
        revision, project = await self.engine.project()
        grade = grade_at(project, probe.target)
        nodes = _nodes(grade)
        if len(nodes) == 1 and nodes[0].get("id") == probe.node:
            try:
                await self.engine.apply([original], revision=revision, session=self.engine.session)
            except EngineError as error:
                if not _stale(error):
                    raise

    async def _worked(
        self,
        timeline: str,
        plans: Sequence["_Plan"],
        probe: "_Plan | None",
        still: str | None,
        auto: bool,
    ) -> tuple[list[dict[str, Any]], dict[str, Any]]:
        """Measure the clips (and the target) and work out the operations that match them."""
        _, project = await self.engine.project()
        told: dict[str, Any] = {}
        space = timeline_space(project.get("color"))
        if space != _MADE_FOR:
            told["note"] = (
                f"Shot Match and Auto Color are made for {_MADE_FOR} (p. 3047), and they measure "
                f"in the timeline's space, which is {space or 'not known'}"
            )
        wanted: shotmatch.Figures | None = None
        if probe is not None:
            wanted = await self._figures(timeline, probe, project, replacing=False)
            told["target"] = {"clip": probe.clip, "frame": probe.frame, **wanted.told()}
        elif still is not None:
            wanted = await self._still_figures(still)
            told["target"] = {"still": still, **wanted.told()}
        ops: list[dict[str, Any]] = []
        results: list[dict[str, Any]] = []
        skipped: list[dict[str, Any]] = []
        for plan in plans:
            assert plan.node is not None
            grade = grade_at(project, plan.target)
            params = _object(_node(grade, plan.node).get("params"))
            figures = await self._figures(timeline, plan, project)
            item: dict[str, Any] = {
                "clip": plan.clip,
                "node": _number_of(grade, plan.node),
                "id": plan.node,
                "frame": plan.frame,
            }
            if plan.added:
                item["added"] = True
            if auto:
                found = _auto_of(figures, params)
                if isinstance(found, str):
                    skipped.append({"clip": plan.clip, "why": found})
                    continue
                lift, gain = found
                patch = {
                    "primaries": {
                        "lift": dict(zip("rgb", map(_rounded, lift), strict=True)),
                        "gain": dict(zip("rgb", map(_rounded, gain), strict=True)),
                    }
                }
                ops.append(_set_node(plan.target, plan.node, params=patch))
                results.append({**item, "settings": _flat(patch)})
                continue
            assert wanted is not None
            solved = shotmatch.solve(figures, wanted)
            match = solved.match
            values = {
                "gamma": dict(zip("rgb", map(_rounded, match.gamma), strict=True)),
                "gain": dict(zip("rgb", map(_rounded, match.gain), strict=True)),
                "offset": dict(zip("rgb", map(_rounded, match.offset), strict=True)),
                "contrast": _rounded(match.contrast),
                "pivot": _rounded(match.pivot),
            }
            ops.append(_set_node(plan.target, plan.node, params={"shot_match": values}))
            foretold = match.mapped(figures)
            item.update(
                shot_match=values,
                before=round(solved.before, 4),
                after=round(solved.after, 4),
                mid_grey=[round(figures.grey, 4), round(foretold.grey, 4)],
                chroma=[
                    [round(one, 4) for one in figures.chroma],
                    [round(one, 4) for one in foretold.chroma],
                ],
            )
            if solved.limited:
                item["limited"] = list(solved.limited)
            if plan.graded:
                item["graded"] = (
                    "the clip was graded already: Shot Match is meant for clips that are not "
                    "(p. 3048)"
                )
            results.append(item)
        told["auto" if auto else "matched"] = results
        if skipped:
            told["skipped"] = skipped
        return ops, told

    async def _figures(
        self,
        timeline: str,
        plan: "_Plan",
        project: Mapping[str, Any],
        *,
        replacing: bool = True,
    ) -> shotmatch.Figures:
        """The node's figures, undoing its old match only when that match will be replaced."""
        assert plan.node is not None
        measured = await self.engine.statistics(
            timeline=timeline, frame=plan.frame, grade=plan.target, node=plan.node, bins=_BINS
        )
        figures = _figures(measured)
        held = _object(_node(grade_at(project, plan.target), plan.node).get("params")).get(
            "shot_match"
        )
        if held is None or not replacing:
            return figures
        try:
            return shotmatch.ShotMatch.of(_object(held)).unmapped(figures)
        except ValueError:
            raise GradeError(
                f"the shot match of node {plan.node} of clip {plan.clip} has keyframes; reset it "
                "first (video_shot_match with reset)"
            ) from None

    async def _still_figures(self, still: str) -> shotmatch.Figures:
        try:
            _, _, png = await self.engine.still_thumbnail(still, _STILL_HEIGHT)
        except EngineError as error:
            if (error.data or {}).get("reason") == "missing":
                raise GradeError(f"there is no still {still} in the Gallery") from None
            raise
        try:
            return await asyncio.to_thread(shotmatch.figures_of_png, png)
        except ValueError as error:
            raise GradeError(f"the picture of still {still} cannot be read: {error}") from None


def _given(node: str | int | None) -> str | int:
    assert node is not None
    return node


def _spec_at(path: str) -> Spec | None:
    """The setting of a corrector at a path with dots, or ``None`` where there is none."""
    spec: Spec = CORRECTOR
    for part in path.split("."):
        if isinstance(spec, Whole):
            spec = spec.fields
        if not isinstance(spec, dict) or part not in spec:
            return None
        found: Spec = spec[part]
        spec = found
    return spec


def _setting(param: str) -> None:
    """Check that a path names one setting of a corrector, for ``paste_value``."""
    if _spec_at(param) is None:
        raise GradeError(
            f"{param} is not a setting of a corrector; video_grade_info shows them by such "
            "paths, as primaries.saturation or primaries.lift"
        )


def _group_grade(
    project: Mapping[str, Any],
    timeline: str | None,
    source: str | Mapping[str, Any],
    group: str,
    level: Literal["pre", "post"],
) -> dict[str, Any]:
    origin = target_of(source, project, timeline)
    return {
        "edit": "copy_grade",
        "from": origin,
        "to": [{"level": f"{level}_clip", "group": group}],
    }


def _group_told(
    group: Mapping[str, Any], project: Mapping[str, Any], missing: Mapping[str, str]
) -> dict[str, Any]:
    members = [
        {"timeline": timeline["id"], "clip": clip["id"]}
        for timeline in map(_object, _items(project.get("timelines")))
        for _, clip in _clips(timeline)
        if clip.get("group") == group.get("id")
    ]
    return {
        "id": group.get("id"),
        "name": group.get("name"),
        "clips": members,
        "pre": brief(_object(group.get("pre")), missing),
        "post": brief(_object(group.get("post")), missing),
    }


def hint_of(error: EngineError) -> str | None:
    """What a model can do about a refusal of a change of grades, where there is more to say."""
    data = error.data or {}
    reason, what = data.get("reason"), data.get("what")
    if reason == "out_of_range" and isinstance(what, str):
        spec = _spec_at(what)
        return f"{what} is {spec.told()}" if isinstance(spec, Number) else None
    hints = {
        "missing": "video_grade_info lists the nodes of a grade and the groups",
        "wrong_node": (
            "video_grade_info shows the kind of each node: add_parallel, add_layer and "
            "add_serial_before go beside or before a corrector (add_serial_before also before "
            "output), add_outside after a corrector, add_serial after a node or source"
        ),
        "no_port": "video_grade_info shows how the nodes are linked",
        "input_taken": "video_grade_info shows how the nodes are linked",
        "loop": "video_grade_info shows how the nodes are linked",
        "grouped": "take its clips out first, or use action delete, which does",
        "ungrouped": "only a clip in a group can be collapsed",
        "params": "video_grade_info shows the kind of the node and its settings",
        "unknown_space": "video_grade_info with reference true lists the spaces",
        "inapplicable": (
            "the setting is not of the science or preset in force, before or after the change; "
            "video_color_settings shows what is in force"
        ),
        "color_settings": "video_grade_info with reference true lists the colour settings",
        "camera_raw": "camera raw keeps its maker's space: only bypass or default",
        "no_stream": "media without a picture keeps the defaults",
        "empty": "every value given is the value there already",
    }
    return hints.get(str(reason))


# ---------------------------------------------------------------------------------------------
# Colour management, as far as the tools tell it
# ---------------------------------------------------------------------------------------------

_MADE_FOR = "Rec.709 Gamma 2.4"
"""The space that Shot Match and Auto Color are made for (p. 3047)."""

_PRESETS = {
    "sdr_rec709": "Rec.709 Gamma 2.4",
    "sdr_p3_broadcast": "P3-D65 Gamma 2.4",
    "sdr_p3_cinema": "DCI-P3 Gamma 2.6",
    "sdr_rec2020": "Rec.2020 Gamma 2.4",
    "davinci_wide_gamut": "DaVinci Wide Gamut/DaVinci Intermediate",
    "hdr_p3_broadcast": "P3-D65 ST2084",
    "hdr_rec2020": "Rec.2100 ST2084",
}
"""The timeline space of each preset of RCM (PROTOCOL.md, The project's color; the engine's
``Preset::settings``)."""


def in_force(color: object) -> dict[str, Any]:
    """What a project's ``color`` puts in force: whether it is managed, and its spaces.

    A preset of Resolve Color Management (or Automatic) decides the timeline space and keeps
    the input at Rec.709 Gamma 2.4; Custom takes its own fields; ACES works in ACEScct or ACEScc
    from its IDT to its ODT. Fields left out are read as their defaults.
    """
    given = _object(color)
    science = given.get("science", "yrgb")
    if science == "yrgb":
        return {"science": "yrgb", "managed": False}
    if science in ("acescct", "acescc"):
        return {
            "science": science,
            "managed": True,
            "timeline": "ACEScct" if science == "acescct" else "ACEScc",
            "input": given.get("idt", _MADE_FOR),
            "output": given.get("odt", "Rec.709"),
            "aces_version": given.get("aces_version", "2.0"),
        }
    if given.get("automatic"):
        preset = "hdr_rec2020" if given.get("processing_mode") == "hdr" else "sdr_rec709"
    else:
        preset = str(given.get("preset", "sdr_rec709"))
    custom = preset == "custom"
    return {
        "science": science,
        "managed": True,
        "preset": preset,
        "timeline": given.get("timeline", _MADE_FOR) if custom else _PRESETS.get(preset),
        "input": given.get("input", _MADE_FOR) if custom else _MADE_FOR,
        "output": given.get("output", _MADE_FOR),
    }


def timeline_space(color: object) -> str | None:
    """The space that grades work in: Rec.709 Gamma 2.4 for a project not colour managed."""
    told = in_force(color)
    if not told["managed"]:
        return _MADE_FOR
    space = told.get("timeline")
    return space if isinstance(space, str) else None


# ---------------------------------------------------------------------------------------------
# Shot Match and Auto Color
# ---------------------------------------------------------------------------------------------

_BINS = 1024
"""The levels of the histograms measured: their percentiles are within a thousandth."""

_STILL_HEIGHT = 270
"""The thumbnail of a still that its figures are read from: the largest the engine gives."""


@dataclass
class _Plan:
    """A clip to be measured: its grade, the node (``None`` until it is added), the frame."""

    clip: str
    target: dict[str, Any]
    node: str | None
    frame: int
    graded: bool
    added: bool = False


def _middle(clip: Mapping[str, Any]) -> int:
    return int(clip["start"]) + int(clip["length"]) // 2


def _last_corrector(grade: Mapping[str, Any]) -> str | None:
    """The corrector nearest the output, else the last by number; ``None`` where there is none.

    It is found by following the picture back from the output through the first input of each
    node.
    """
    into = _links_into(grade)
    kinds = {str(one.get("id")): one.get("kind") for one in _nodes(grade)}
    at = into.get(("output", "rgb", 0))
    seen: set[str] = set()
    while at is not None and at in kinds and at not in seen:
        seen.add(at)
        if kinds[at] == "corrector":
            return at
        at = into.get((at, "rgb", 0))
    correctors = [id for id, kind in kinds.items() if kind == "corrector"]
    return correctors[-1] if correctors else None


def _output_of(grade: Mapping[str, Any]) -> str | None:
    """The node whose picture is a grade's output; ``None`` where the source is."""
    at = _links_into(grade).get(("output", "rgb", 0))
    return at if at is not None and any(one.get("id") == at for one in _nodes(grade)) else None


def _graded(grade: Mapping[str, Any]) -> bool:
    """Whether a grade changes the picture by any setting of a corrector or a LUT."""
    for node in _nodes(grade):
        params = _object(node.get("params"))
        if node.get("kind") == "corrector":
            settings: dict[str, Any] = {}
            _changed(CORRECTOR, params, "", settings)
            if settings or params.get("lut"):
                return True
    return False


def _number_of(grade: Mapping[str, Any], id: str) -> int:
    return next(at for at, one in enumerate(_nodes(grade), 1) if one.get("id") == id)


def _still_of(given: str | Mapping[str, Any] | None) -> str | None:
    if isinstance(given, Mapping):
        still = given.get("still")
        if set(given) != {"still"} or not isinstance(still, str):
            raise GradeError('target is a clip id, or {"still": "s1"} for a still of the Gallery')
        return still
    return None


def _clip_of(given: str | Mapping[str, Any]) -> str:
    if not isinstance(given, str):
        raise GradeError('target is a clip id, or {"still": "s1"} for a still of the Gallery')
    return given


def _stale(error: EngineError) -> bool:
    return (error.data or {}).get("reason") == "stale_revision"


def _figures(measured: Statistics) -> shotmatch.Figures:
    means = (measured.red.mean, measured.green.mean, measured.blue.mean)
    return shotmatch.figures(measured.histogram, means)


def _coarse(counts: Sequence[int], bins: int) -> list[float]:
    """A histogram in fewer levels, each the percent of the pixels in it."""
    total = sum(counts) or 1
    out = [0] * bins
    for at, count in enumerate(counts):
        out[min(at * bins // len(counts), bins - 1)] += count
    return [round(100 * one / total, 2) for one in out]


def _rounded(value: float) -> float:
    return round(value, 6)


def _flat(patch: Mapping[str, Any], path: str = "") -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, value in patch.items():
        where = f"{path}.{key}" if path else key
        if _is_map(value):
            out.update(_flat(_object(value), where))
        else:
            out[where] = value
    return out


def _auto_of(
    figures: shotmatch.Figures, params: Mapping[str, Any]
) -> tuple[tuple[float, float, float], tuple[float, float, float]] | str:
    """Auto Color of a node from the figures of its output: its Lift and Gain, or why not."""
    primaries = _object(params.get("primaries"))
    now: list[list[float]] = []
    for name, neutral in (("lift", 0.0), ("gain", 1.0)):
        wheel = _object(primaries.get(name))
        values = [wheel.get(channel, neutral) for channel in "rgb"]
        if not all(_real(one) for one in values):
            return f"the {name.capitalize()} of its node has keyframes, which Auto Color keeps"
        now.append([float(cast("float", one)) for one in values])
    found = shotmatch.auto_color(
        [one[0] for one in figures.levels], [one[-1] for one in figures.levels], now[0], now[1]
    )
    if found is None:
        return "the frame has too little range to balance by"
    return found


# ---------------------------------------------------------------------------------------------
# The reference that the short descriptions of the tools leave out
# ---------------------------------------------------------------------------------------------

_KEYFRAMES = (
    'A number may be keyframes {"keys": [{"frame": 0, "value": 40, "interp": "linear"}, ...]}: '
    "frames whole and rising, one key at least; interp linear (where left out), hold, or "
    '{"bezier": {"out": [t, dv], "in": [t, dv]}}, t from 0 to 1 a share of the frames to the next '
    "key and dv a change of value. A key's frame: of a clip of media, the frame of its media (at "
    "the media's own rate, from 0) whose picture is shown, so keys stay on their pictures through "
    "trims, slips and speed changes; of a title, a colour or a still, the clip's frame from its "
    "first (0); of a group's grade, of the clip it is on; of the timeline's grade, the timeline's "
    "frame."
)
_GAMUTS = ("Rec.709", "Rec.601 (PAL)", "Rec.601 (NTSC)", "Rec.2020", "DCI-P3", "P3-D65")
_GAMMAS = ("Gamma 2.4", "Gamma 2.2", "Gamma 2.6", "Gamma 2.8", "sRGB", "Linear", "ST2084", "HLG")
_CAMERAS = (
    "Rec.709 (Scene)", "Apple Log", "Apple Log 2", "DJI D-Gamut/D-Log", "Sony S-Gamut3/S-Log3",
    "Sony S-Gamut3.Cine/S-Log3", "Panasonic V-Gamut/V-Log", "Canon Cinema Gamut/Canon Log 2",
    "Canon Cinema Gamut/Canon Log 3", "ARRI Wide Gamut 3/LogC3", "ARRI Wide Gamut 4/LogC4",
    "FujiFilm F-Gamut/F-Log2", "Blackmagic Design Wide Gamut Gen 5/Film Gen 5",
    "RED Wide Gamut RGB/Log3G10", "DaVinci Wide Gamut/DaVinci Intermediate",
    "DaVinci Wide Gamut/Linear", "ACEScct", "ACEScc", "ACEScg", "ACES (AP0) Linear",
)  # fmt: skip
_COLOR = {
    "science": "yrgb (DaVinci YRGB, not managed: the default), yrgb_managed (RCM), acescct, acescc",
    "automatic": "RCM: Automatic color management, true or false",
    "processing_mode": "RCM with automatic: sdr or hdr",
    "preset": (
        "RCM without automatic: sdr_rec709, sdr_p3_broadcast, sdr_p3_cinema, sdr_rec2020, "
        "davinci_wide_gamut, hdr_p3_broadcast, hdr_rec2020, custom; a preset sets the timeline "
        "space, luminance, DRTs and gamut limit, and not output: send both"
    ),
    "output": "RCM: a space, or bypass",
    "input": "Custom: a space or bypass, for media that says none",
    "timeline": "Custom: a space, the one grades work in",
    "working_luminance": (
        "Custom: sdr_100, hdr_500, hdr_1000, hdr_2000, hdr_4000, hdr_er_1000_2000, custom "
        "(custom_luminance, nits 1-10000)"
    ),
    "input_drt, output_drt": "Custom: none, simple, luminance, davinci, saturation_preserving",
    "input_sat_rolloff, output_sat_rolloff": "{start, limit} nits, start < limit",
    "limit_gamut": "Custom: null, rec709, rec601-pal, rec601-ntsc, rec2020, dci-p3, p3-d65",
    "hdr_203_nits, inverse_drt, white_point_adaptation, color_space_aware_tools": "Custom: flags",
    "resize_in": "Custom: timeline, log, linear, linear_mapped, gamma, gamma_mapped",
    "graphics_white": "RCM: nits 1-10000",
    "aces_version, idt, odt": (
        "ACES: 1.3 or 2.0, an IDT space, an ODT of that version (Rec.709, sRGB, P3-D65, "
        "Rec.2020, ...): change version and odt together"
    ),
    "gamut_compress": "ACES: flag",
    "node_luts_in": "ACES: timeline or ap0_linear",
    "lut_interpolation": "trilinear or tetrahedral",
    "input_lut, output_lut, viewer_lut": '{"1d": name or null, "3d": name or null}',
    "s_curve_contrast": "flag: Contrast as an S-curve",
}


def _reference_of(spec: Spec, path: str, out: dict[str, str]) -> None:
    if isinstance(spec, dict):
        for key, one in spec.items():
            _reference_of(one, f"{path}.{key}" if path else key, out)
    elif isinstance(spec, Number):
        out[path] = spec.told()
    elif isinstance(spec, Flag):
        out[path] = f"{spec.name}: true or false, {str(spec.default).lower()} for a new node"
    elif isinstance(spec, Points):
        least = f", {spec.least} points at least" if spec.least else ", [] is flat"
        out[path] = (
            f"{spec.name}: points [[in, out], ...], in 0 to 1 rising, out {_number(spec.low)} "
            f"to {_number(spec.high)}{least}"
        )
    elif isinstance(spec, Lut):
        out[path] = "a .cube by its name in the LUT library or project folder, a path, or null"
    elif isinstance(spec, Whole):
        out[path] = (
            f"{spec.name}: null, or an object of {_listed(spec.fields)}, made whole with "
            "neutral values where the node has none; invisible on the palettes"
        )
        _reference_of(spec.fields, path, out)
    elif isinstance(spec, Count):
        out[path] = f"{spec.name}: {spec.low} to {spec.high}"
    elif isinstance(spec, Choice):
        out[path] = f"{spec.name}: {', '.join(spec.choices)}"
    else:
        out[path] = (
            f"{spec.least} to {spec.most} inputs, each {{{_listed(spec.item)}}}; the list is "
            "replaced whole"
        )


def reference() -> dict[str, Any]:
    """What the tools of grades take, in full.

    Each setting of a node with its DaVinci name, range and neutral value, keyframes, the
    settings of colour management and the spaces.
    """
    correctors: dict[str, str] = {}
    _reference_of(CORRECTOR, "", correctors)
    mixers: dict[str, dict[str, str]] = {}
    for kind, spec in _MIXERS.items():
        mixers[kind] = {}
        _reference_of(spec, "", mixers[kind])
    return {
        "paths": (
            'Settings are written by paths with dots, as {"primaries.lift.b": 0.05}, or as '
            "objects; a patch merges objects and replaces other values."
        ),
        "corrector": correctors,
        "mixers": mixers,
        "keyframes": _KEYFRAMES,
        "shot_match": (
            "Each channel S(gain * x^gamma + offset), then an S-curve of contrast about pivot "
            "keeping 0 and 1; after Soft Clip, the last step of its node, within its key. "
            "video_shot_match works it out; set to null to reset."
        ),
        "color_settings": _COLOR,
        "spaces": (
            f"'<gamut> <gamma>' for each gamut of {', '.join(_GAMUTS)} and gamma of "
            f"{', '.join(_GAMMAS)} (Rec.709 sRGB is 'sRGB'; Rec.2020 ST2084 and HLG are "
            f"'Rec.2100 ST2084' and 'Rec.2100 HLG'), and {', '.join(_CAMERAS)}"
        ),
    }
