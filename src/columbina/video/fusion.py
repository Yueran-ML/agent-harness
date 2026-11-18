"""The compositions of the Fusion page, as the agent reads and changes them through the engine.

A clip of a track of picture may have a composition (``engine/PROTOCOL.md``, Compositions): a
graph of nodes that makes its picture, taken for the source of the clip, before its grade and
its transform. Pictures go from a ``media_in``, the clip's own source, through the nodes to the
one ``media_out``, which the timeline shows. A clip without a composition shows its source; the
engine gives it none of its own accord, so the first change of such a clip sends, in the same
transaction, the composition that the Fusion page gives a clip at first
(:func:`columbina.video.edits.first_composition`).

What is here:

- **The kinds of node** (:data:`KINDS`): what each is for, its inputs, and its settings in
  Fusion's units with the bounds of the engine's catalog
  (``engine/crates/model/tests/data/catalog.json``, the sections ``fusion_*``), which a test
  holds them to. A node is added whole, every setting written (:func:`new_params`).
- **A tracker**: its patterns are a list that is sent whole, which a model may change one
  pattern at a time (``trackers.N.<field>``, ``trackers.+``, ``trackers.-``), every path kept;
  the paths are left to tracking, which fills them. The center of a merge, a transform or a
  mask may follow it (``center.follow``), and a tracker that one follows is not taken away.
- **Reading** (:func:`summary`): a composition as a model is to read it.
- **Checks**: a patch of the settings of a node and the links of a change, checked before they
  are sent, so that what is wrong is told in words that a model can act on, with the name that
  the engine would refuse it by.
- **A planar tracker and a Text+**: the region of a planar tracker is a list set whole, and its
  path, a homography at each frame tracked, is left to tracking; a Text+ may be without its
  outline, its border and its follower (:attr:`Kind.parts`), and one of them is given whole
  where it has it not (:func:`new_part`).
- **What the tools do**: :func:`info`, :func:`node` and :func:`keys`, each change one
  transaction of the engine and one step of its history; :func:`track`, which has the
  engine follow the patterns of a tracker, or the region of a planar tracker, through the
  frames of its clip and write their paths, one step of the history too, and waits for the
  end; and :func:`animate`, which puts a ready-made animation on a Text+, one step.

A model thinks in frames of the timeline, and the engine keeps a key at a frame of the clip's
source (PROTOCOL, Frames of keys): :func:`keys` is given a frame of the timeline and finds the
frame of the key as ``video_clip_keys`` does (:class:`columbina.video.clip_keys.KeyClock`), and
so is :func:`track` for the frame that a track starts from, and tells the frames of the paths
at frames of the timeline.
"""

import copy
import difflib
import math
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Literal, cast

from columbina.video import edits
from columbina.video.clip_keys import KeyClock
from columbina.video.edits import FusionKind, Rate
from columbina.video.engine import Applied, Engine, EngineError, Tracking
from columbina.video.grading import (
    COMPOSITES,
    Build,
    Choice,
    Flag,
    GradeError,
    KeyAction,
    KeyOps,
    Number,
    applied,
    expanded,
    key_op,
    timeline_of,
)

__all__ = [
    "FOLLOWS",
    "KINDS",
    "MASKS",
    "PATTERN",
    "POINT",
    "PRESETS",
    "Follows",
    "FusionError",
    "Homographies",
    "Kind",
    "NodeAction",
    "PatternList",
    "Places",
    "Plain",
    "PointList",
    "Preset",
    "Setting",
    "TrackDirection",
    "Words",
    "animate",
    "hint_of",
    "info",
    "keys",
    "new_params",
    "new_part",
    "new_pattern",
    "node",
    "reference",
    "summary",
    "track",
    "track_hint_of",
]

NodeAction = Literal["add", "remove", "connect", "disconnect", "set", "reset"]

TrackDirection = Literal["forward", "reverse", "forward_and_back", "one_forward", "one_reverse"]
"""Which way a track goes on the timeline from its frame: to the end of the clip, back to its
start, both, or one frame either way."""

Preset = Literal["typewriter", "fade_in", "rise", "pop", "fade_out"]
"""A ready-made animation of a Text+ (:func:`animate`)."""

PRESETS: tuple[Preset, ...] = ("typewriter", "fade_in", "rise", "pop", "fade_out")
"""The animations of a Text+ that :func:`animate` puts, in the order that they are told."""

_MOST_NODES = 256
_MOST_POINTS = 128
_MOST_TRACKERS = 16
_LARGEST_FRAME = 2**53 - 1
"""How far from 0 the frames of the paths of a tracker, and its reference, may be."""
_LONGEST_LABEL = 100
_SCREEN_APART = 0.0001
"""How far one channel of the screen colour of a keyer is above its other two, at least."""


class FusionError(Exception):
    """What is wrong with what a Fusion tool was asked, said so that it can be put right."""


# ---------------------------------------------------------------------------------------------
# The kinds of node and their settings, in Fusion's units
# ---------------------------------------------------------------------------------------------

_UNITS = {
    "widths": "widths of the picture",
    "heights": "heights of the picture",
    "degrees_counterclockwise": "degrees counter-clockwise",
    "pixels1080": "pixels of a picture 1080 high",
    "pixels": "pixels of the picture",
}
"""The units of the engine's catalog, in words; a ``number`` has none."""


@dataclass(frozen=True)
class Setting(Number):
    """A setting of a node that is a number, or keyframes of numbers.

    Its ``default`` is the value that a new node has, which need not be one that changes nothing.

    Attributes:
        unit: What it counts, as the engine's catalog names it: ``number``, ``widths``,
            ``heights``, ``degrees_counterclockwise``, ``pixels1080`` or ``pixels``.
    """

    unit: str = "number"

    def told(self) -> str:
        """Its range, what it is in a new node and its unit, in words."""
        unit = _UNITS.get(self.unit)
        counted = f", in {unit}" if unit else ""
        return f"{self.name}: {self.low:g} to {self.high:g}, {self.default:g} at first{counted}"


@dataclass(frozen=True)
class PointList:
    """The points of a polygon: a list that is set whole, of ``most`` points at most."""

    name: str
    most: int


@dataclass(frozen=True)
class Plain(Number):
    """A setting of a tracker that is a number and takes no keys.

    Attributes:
        unit: What it counts, as :class:`Setting` names it; or ``frame``, a frame of the
            timeline within the clip, which is sent as the frame of the clip's keys there.
        whole: Whether it is a whole number.
    """

    unit: str = "number"
    whole: bool = False

    def told(self) -> str:
        """Its range, what it is in a new node and its unit, in words."""
        if self.unit == "frame":
            return f"{self.name}: a frame of the timeline within the clip; no keys"
        unit = _UNITS.get(self.unit)
        counted = f", in {unit}" if unit else ""
        whole = "a whole number " if self.whole else ""
        return (
            f"{self.name}: {whole}{self.low:g} to {self.high:g}, {self.default:g} at first"
            f"{counted}; no keys"
        )


@dataclass(frozen=True)
class PatternList:
    """The patterns of a tracker: a list that is set whole, of ``most`` patterns at most."""

    name: str
    most: int


@dataclass(frozen=True)
class Follows:
    """What the center of a node follows of a tracker, ``{node, output, tracker}``, or none.

    It is written only where the center follows one: a new node has none.
    """

    name: str


@dataclass(frozen=True)
class Words:
    """A setting that is words, which takes no keys: the text of a Text+, or its font.

    Attributes:
        most: How long it may be: characters (Unicode scalars), or bytes of its UTF-8 where
            ``in_bytes``.
    """

    name: str
    default: str
    most: int
    in_bytes: bool = False


@dataclass(frozen=True)
class Places:
    """The region of a Planar Tracker: places ``{x, y}`` from the lower left, set whole.

    Attributes:
        least: How few places it has at least.
        most: How many it has at most.
        default: The places of a new node.
    """

    name: str
    least: int
    most: int
    default: tuple[tuple[float, float], ...]


@dataclass(frozen=True)
class Homographies:
    """The path of a Planar Tracker: a homography at each frame tracked, filled by tracking.

    It is ``{"<frame>": [h11, h12, h13, h21, h22, h23, h31, h32]}`` at frames of the clip's
    keys, ``{}`` in a new node; a frame given ``null`` is taken away.
    """

    name: str


FusionSpec = (
    Setting
    | Plain
    | Flag
    | Choice
    | PointList
    | PatternList
    | Follows
    | Words
    | Places
    | Homographies
)


@dataclass(frozen=True)
class Kind:
    """A kind of node of a composition.

    Attributes:
        told: What it is for, in a line.
        inputs: Its inputs, the first first.
        output: Whether it has an output: every kind but ``media_out``.
        settings: Its settings by their paths in its ``params``, in the order of the catalog.
        hands_on: The input whose picture it hands on as it is when it is passed through, which
            is the one that it is joined across by when it is taken away; ``None`` where it
            makes its picture of nothing.
        parts: The groups of its settings that a node may be without (the outline of a Text+),
            each by the first name of the paths of its settings: a new node has none of them,
            and one is given whole where the node has it not (:func:`new_part`).
    """

    told: str
    inputs: tuple[str, ...]
    output: bool
    settings: dict[str, FusionSpec]
    hands_on: str | None
    parts: tuple[str, ...] = ()


_DEGREES = "degrees_counterclockwise"
_PIXELS = "pixels1080"


def _center(low: float, high: float, of: str) -> dict[str, FusionSpec]:
    return {
        "center.x": Setting(f"Center X, {of} from the left edge", 0.5, low, high, "widths"),
        "center.y": Setting(f"Center Y, {of} from the lower edge, up", 0.5, low, high, "heights"),
        "center.follow": Follows("Center follows a tracker, the place that it publishes"),
    }


def _mask(shape: dict[str, FusionSpec]) -> dict[str, FusionSpec]:
    """The controls that every mask has, then its place, then those of its shape."""
    return {
        "level": Setting("Level, what the mask is multiplied by", 1, 0, 1),
        "soft_edge": Setting("Soft Edge, how wide the ramp at the edge is", 0, 0, 1, "widths"),
        "border_width": Setting(
            "Border Width, added to the shape, or taken from it below 0", 0, -1, 1, "widths"
        ),
        "invert": Flag("Invert: the mask is where the shape is not", False),
        "solid": Flag("Solid: the shape is filled; false draws its border alone", True),
        "paint_mode": Choice(
            "Paint Mode, how it is combined with a mask linked into its own mask input",
            "merge",
            ("merge", "add", "subtract"),
        ),
        **_center(-10, 10, "the middle of the shape,"),
        **shape,
    }


def _colour(
    path: str, name: str, first: tuple[float, float, float], high: float
) -> dict[str, FusionSpec]:
    """The red, the green and the blue of a colour, each a number from 0 to ``high``."""
    channels = zip(("r", "g", "b"), ("red", "green", "blue"), first, strict=True)
    return {
        f"{path}.{letter}": Setting(f"{name}, {word}", value, 0, high)
        for letter, word, value in channels
    }


_MASK_ANGLE = Setting("Angle, about the center", 0, -36000, 36000, _DEGREES)

POINT: dict[str, Setting] = {
    "x": Setting("X of a point, to the right of the center", 0, -16, 16, "widths"),
    "y": Setting("Y of a point, up from the center", 0, -16, 16, "widths"),
}
"""The two numbers of a point of a polygon, ``points.N.x`` and ``points.N.y``, which take keys."""

MASKS = ("rectangle", "ellipse", "polygon")
"""The kinds that make a mask: white where the shape is, premultiplied by the mask."""

FOLLOWS = ("tracker", "steady_position", "unsteady_position")
"""What a center may follow of a tracker: where one of its patterns is, with its offset; or the
middle of the picture less the shift of its motion, or with it."""

PATTERN: dict[str, Plain] = {
    "pattern.x": Plain("Pattern X, where its middle was put", 0.5, -15.5, 16.5, "widths"),
    "pattern.y": Plain("Pattern Y, up", 0.5, -15.5, 16.5, "heights"),
    "pattern.width": Plain("Pattern Width", 0.05, 0, 1, "widths"),
    "pattern.height": Plain("Pattern Height", 0.05, 0, 1, "heights"),
    "search.width": Plain("Search Width, of the area it is looked for in", 0.125, 0, 1, "widths"),
    "search.height": Plain("Search Height", 0.125, 0, 1, "heights"),
    "offset.x": Plain("X Offset, right, of the place that it publishes", 0, -16, 16, "widths"),
    "offset.y": Plain("Y Offset, up", 0, -16, 16, "heights"),
}
"""The numbers of a pattern of a tracker, by their paths in the pattern; none takes keys."""

_ONE_PATTERN = (
    "trackers.N.<field> changes the pattern at N (from 0), trackers.+ adds one given in part, "
    "and trackers.- takes the one at N away"
)
_PATTERN_FIELDS = f"a pattern has name, enabled, {', '.join(PATTERN)} and path"

_PLACE = Plain("a place, across or up from the lower left corner", 0.5, -15.5, 16.5)
"""A number of a place of a Planar Tracker: of a place of its region, or of a corner."""

_REGION = ((0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75))
"""The region of a new Planar Tracker: the box from 0.25 to 0.75 across and up."""

_CORNERS = {
    "tl": ("upper left", 0.25, 0.75),
    "tr": ("upper right", 0.75, 0.75),
    "br": ("lower right", 0.75, 0.25),
    "bl": ("lower left", 0.25, 0.25),
}
"""The corners of the Corner Pin of a Planar Tracker, with where they are in a new one."""


def _corners() -> dict[str, FusionSpec]:
    """The numbers of the corners of a Planar Tracker, which take no keys."""
    specs: dict[str, FusionSpec] = {}
    for corner, (said, x, y) in _CORNERS.items():
        specs[f"corners.{corner}.x"] = Plain(
            f"Corner Pin, where the {said} corner of the foreground goes", x, -15.5, 16.5, "widths"
        )
        specs[f"corners.{corner}.y"] = Plain(
            f"Corner Pin, the {said} corner, up", y, -15.5, 16.5, "heights"
        )
    return specs


def _rgba(path: str, name: str, first: tuple[float, float, float, float]) -> dict[str, FusionSpec]:
    """The red, the green, the blue and the alpha of a colour of a Text+, each from 0 to 1."""
    channels = zip(("r", "g", "b", "a"), ("red", "green", "blue", "alpha"), first, strict=True)
    return {
        f"{path}.{letter}": Setting(f"{name}, {word}", value, 0, 1)
        for letter, word, value in channels
    }


_TEXT = Words("Styled Text, what is written; \\n begins a line", "", 4096)
_FONT = Words("Font, a family; empty, the engine's own", "", 256, in_bytes=True)
_TEXT_SIZE = Setting("Size of the letters", 0.08, 0, 1, "heights")
_OFFSET = (-10, 10)
_SCALE = (0, 100)

KINDS: dict[str, Kind] = {
    "media_in": Kind(
        told="the picture of the clip's own source, as a clip without a composition shows it",
        inputs=(),
        output=True,
        settings={},
        hands_on=None,
    ),
    "media_out": Kind(
        told=(
            "the picture that goes back to the timeline; a composition has exactly one, which is "
            "neither added nor removed; where its input is not linked the clip is transparent"
        ),
        inputs=("input",),
        output=False,
        settings={},
        hands_on=None,
    ),
    "merge": Kind(
        told=(
            "the foreground laid over the background, placed by center, size and angle "
            "(picture-in-picture, a keyed shot over a new backdrop); the result has the size of "
            "the background"
        ),
        inputs=("background", "foreground", "mask"),
        output=True,
        settings={
            "apply": Choice("Apply Mode, how the foreground is blended", "normal", COMPOSITES),
            "operator": Choice("Operator", "over", ("over",)),
            "subtractive_additive": Setting(
                "Subtractive - Additive, read in the normal mode alone", 1, 0, 1
            ),
            "alpha_gain": Setting("Alpha Gain", 1, 0, 1),
            "burn_in": Setting("Burn In", 0, 0, 1),
            "blend": Setting("Blend, how much of the foreground is laid over", 1, 0, 1),
            **_center(-15.5, 16.5, "where the middle of the foreground is in the background,"),
            "size": Setting("Size, a factor of the foreground about its middle", 1, 0, 16),
            "angle": Setting("Angle of the foreground", 0, -3600, 3600, _DEGREES),
        },
        hands_on="background",
    ),
    "transform": Kind(
        told="its input moved, made larger or smaller, turned and flipped (reframing a shot)",
        inputs=("input", "mask"),
        output=True,
        settings={
            **_center(-15.5, 16.5, "where the middle of the picture is put,"),
            "size": Setting("Size, a factor about the middle", 1, 0, 16),
            "angle": Setting("Angle", 0, -3600, 3600, _DEGREES),
            "flip_horizontal": Flag("Flip Horizontal", False),
            "flip_vertical": Flag("Flip Vertical", False),
        },
        hands_on="input",
    ),
    "background": Kind(
        told="one colour over the whole picture (a backdrop); opaque black at first",
        inputs=("mask",),
        output=True,
        settings={
            **_colour("color", "Color", (0, 0, 0), 1),
            "color.a": Setting("Color, alpha, which it is not premultiplied by", 1, 0, 1),
        },
        hands_on=None,
    ),
    "rectangle": Kind(
        told="the mask of a rectangle",
        inputs=("mask",),
        output=True,
        settings=_mask(
            {
                "width": Setting("Width", 0.5, 0, 10, "widths"),
                "height": Setting("Height", 0.5, 0, 10, "heights"),
                "corner_radius": Setting(
                    "Corner Radius, from sharp corners to short sides of half circles", 0, 0, 1
                ),
                "angle": _MASK_ANGLE,
            }
        ),
        hands_on="mask",
    ),
    "ellipse": Kind(
        told="the mask of an ellipse (a face, a plate, a vignette)",
        inputs=("mask",),
        output=True,
        settings=_mask(
            {
                "width": Setting("Width", 0.5, 0, 10, "widths"),
                "height": Setting(
                    "Height, in widths too: equal to width, a circle", 0.5, 0, 10, "widths"
                ),
                "angle": _MASK_ANGLE,
            }
        ),
        hands_on="mask",
    ),
    "polygon": Kind(
        told="the mask of a polygon of straight sides through its points (a garbage matte)",
        inputs=("mask",),
        output=True,
        settings=_mask(
            {
                "size": Setting("Size, what its points are multiplied by", 1, 0, 10),
                "angle": _MASK_ANGLE,
                "points": PointList("Points, each counted from the center", _MOST_POINTS),
                "fill": Choice(
                    "Fill Method, alternate leaving a hole where the outline lies over itself",
                    "alternate",
                    ("alternate", "non_zero_winding"),
                ),
            }
        ),
        hands_on="mask",
    ),
    "blur": Kind(
        told="its input blurred; with a mask, within the mask alone",
        inputs=("input", "mask"),
        output=True,
        settings={
            "filter": Choice("Filter", "gaussian", ("gaussian", "box")),
            "lock_xy": Flag("Lock X/Y: size.x blurs across and down, size.y is not read", True),
            "size.x": Setting("Blur Size across", 5, 0, 500, _PIXELS),
            "size.y": Setting("Blur Size down, read where lock_xy is false", 5, 0, 500, _PIXELS),
            "blend": Setting("Blend, from the picture as it came to the blurred one", 1, 0, 1),
        },
        hands_on="input",
    ),
    "mosaic": Kind(
        told="its input laid out in cells of one colour each (pixelated); with a mask, within it",
        inputs=("input", "mask"),
        output=True,
        settings={
            "frequency": Setting("Pixel Frequency, how many cells lie across", 32, 1, 16384),
            "aspect": Setting("Cell Aspect, how many times as wide as high", 1, 0.0625, 16),
            "offset.x": Setting("Offset of the grid to the right", 0, -16384, 16384, "pixels"),
            "offset.y": Setting("Offset of the grid down", 0, -16384, 16384, "pixels"),
            "whole_cells_only": Flag(
                "Whole Cells Only: a cell is shown only where the mask covers all of it", False
            ),
        },
        hands_on="input",
    ),
    "delta_keyer": Kind(
        told=(
            "its input keyed: the green or blue screen that it was shot against made "
            "transparent; garbage_matte forces transparent and solid_matte solid where a mask "
            "linked into them is"
        ),
        inputs=("input", "garbage_matte", "solid_matte", "mask"),
        output=True,
        settings={
            **_colour("screen", "Background Color, the colour of the screen", (0, 1, 0), 65504),
            "gain": Setting("Gain: more makes more of the screen colour transparent", 1, 0, 10),
            "balance": Setting("Balance", 0.5, 0, 1),
            "pre_blur": Setting("Pre-Blur, of the picture before the key", 0, 0, 100, _PIXELS),
            "low": Setting("Lower Threshold: a matte at or below it is transparent", 0, 0, 1),
            "high": Setting("Upper Threshold: a matte at or above it is solid", 1, 0, 1),
            "erode_dilate": Setting(
                "Erode/Dilate: the matte shrunk below 0, grown above", 0, -100, 100, _PIXELS
            ),
            "matte_blur": Setting("Matte Blur", 0, 0, 100, _PIXELS),
            "clean_foreground": Setting("Clean Foreground: holes in the solid filled", 0, 0, 1),
            "clean_background": Setting(
                "Clean Background: specks of what is transparent taken away", 0, 0, 1
            ),
            "replace": Choice(
                "Replace Mode, what fills what the key took away and the matte gave back",
                "soft_color",
                ("none", "source", "hard_color", "soft_color"),
            ),
            **_colour("replace_color", "Replace Color", (0.5, 0.5, 0.5), 65504),
            "garbage_invert": Flag("Invert of the Garbage Matte", False),
            "spill_method": Choice(
                "Spill Method, from the weakest (medium for green screens, well_done for blue)",
                "medium",
                ("none", "rare", "medium", "well_done", "burnt"),
            ),
            "spill_suppression": Setting("Spill Suppression", 1, 0, 1),
            "view": Choice(
                "View Mode, what comes out of the node (matte to judge the key)",
                "final_result",
                ("pre_matte", "matte", "status", "final_result"),
            ),
        },
        hands_on="input",
    ),
    "tracker": Kind(
        told=(
            "patterns followed through the clip, their paths filled by tracking "
            "(video_fusion_track): with operation match_move, its input held still (merge "
            "bg_only) or its foreground moved with them; the center of another node may follow it"
        ),
        inputs=("input", "foreground", "mask"),
        output=True,
        settings={
            "operation": Choice(
                "Operation: match_move moves pictures by the paths; none hands input on",
                "none",
                ("none", "match_move"),
            ),
            "merge": Choice(
                "Merge, what match_move draws: bg_only the input held still (stabilized), "
                "fg_only the foreground moved with the paths, fg_over_bg that over the input, "
                "bg_over_fg the input over it",
                "bg_only",
                ("bg_only", "fg_only", "fg_over_bg", "bg_over_fg"),
            ),
            "position": Flag("Position: the shift is corrected", True),
            "rotation": Flag("Rotation: the turn is corrected (two patterns or more)", True),
            "scaling": Flag("Scaling: the size is corrected (two patterns or more)", True),
            "pivot": Choice(
                "Pivot Type, what the turn and the size are about",
                "tracker_average",
                ("tracker_average", "selected_tracker", "manual"),
            ),
            "pivot_tracker": Plain(
                "Pivot Tracker, the pattern of selected_tracker, from 0", 0, 0, 15, whole=True
            ),
            "pivot_point.x": Plain("Pivot X, read with manual", 0.5, -15.5, 16.5, "widths"),
            "pivot_point.y": Plain("Pivot Y, up", 0.5, -15.5, 16.5, "heights"),
            "reference": Plain(
                "Reference, the frame that the picture is held to, where nothing moves",
                0,
                -_LARGEST_FRAME,
                _LARGEST_FRAME,
                "frame",
                whole=True,
            ),
            "trackers": PatternList("Trackers, its patterns", _MOST_TRACKERS),
        },
        hands_on="input",
    ),
    "planar_tracker": Kind(
        told=(
            "a region of the picture followed as a plane, its path filled by tracking "
            "(video_fusion_track): operation steady holds the plane still, corner_pin lays the "
            "foreground on it (a screen or a sign replaced)"
        ),
        inputs=("input", "foreground", "mask"),
        output=True,
        settings={
            "operation": Choice(
                "Operation: steady holds the plane still, corner_pin lays the foreground on its "
                "corners; none hands input on",
                "none",
                ("none", "steady", "corner_pin"),
            ),
            "motion": Choice(
                "Motion Type, what a track keeps of the motion of the plane (read as it tracks)",
                "perspective",
                (
                    "translation",
                    "translation_rotation",
                    "translation_rotation_scale",
                    "perspective",
                ),
            ),
            "reference": Plain(
                "Reference Frame, where the region was drawn and nothing moves",
                0,
                -_LARGEST_FRAME,
                _LARGEST_FRAME,
                "frame",
                whole=True,
            ),
            "region": Places(
                "Region, the polygon of the plane at the reference frame", 3, 64, _REGION
            ),
            **_corners(),
            "path": Homographies(
                "Path, the homography of the plane from the reference frame at each frame tracked"
            ),
        },
        hands_on="input",
    ),
    "text_plus": Kind(
        told=(
            "text drawn over nothing (a title, a caption, a lower third), laid over the picture "
            "by a merge; write_on, transform and follower animate it, character by character"
        ),
        inputs=("mask",),
        output=True,
        settings={
            "text": _TEXT,
            "font": _FONT,
            "size": _TEXT_SIZE,
            "center.x": Setting(
                "Center X, the middle of the text from the left edge", 0.5, -15.5, 16.5, "widths"
            ),
            "center.y": Setting("Center Y, from the lower edge, up", 0.5, -15.5, 16.5, "heights"),
            "center.follow": Follows("Center follows a tracker, the place that it publishes"),
            "justify": Choice("Horizontal Justification", "center", ("left", "center", "right")),
            "vertical": Choice("Vertical Justification", "center", ("top", "center", "bottom")),
            "line_spacing": Setting("Line Spacing, a factor", 1, 0, 10),
            "tracking": Setting("Tracking, room between the letters", 0, -1, 10),
            "angle": Setting("Angle of the text", 0, -3600, 3600, _DEGREES),
            "write_on.start": Setting(
                "Write On, start: the share of the text not yet shown", 0, 0, 1
            ),
            "write_on.end": Setting("Write On, end: the share of the text shown", 1, 0, 1),
            "transform.select": Choice(
                "Transform Select, what it moves each of: characters, words or lines",
                "characters",
                ("characters", "words", "lines"),
            ),
            "transform.offset.x": Setting("Transform Offset X", 0, *_OFFSET, "heights"),
            "transform.offset.y": Setting("Transform Offset Y, up", 0, *_OFFSET, "heights"),
            "transform.angle": Setting("Transform Angle", 0, -3600, 3600, _DEGREES),
            "transform.scale": Setting("Transform Size, a factor", 1, *_SCALE),
            **_rgba("fill.color", "Fill Color", (1, 1, 1, 1)),
            "outline.thickness": Setting("Outline Thickness", 0.02, 0, 0.5),
            **_rgba("outline.color", "Outline Color", (0, 0, 0, 1)),
            "border.extend": Setting(
                "Border Extend, how far beyond the text", 0.02, 0, 1, "heights"
            ),
            "border.round": Setting("Border Round, of its corners", 0, 0, 1),
            **_rgba("border.color", "Border Color", (0, 0, 0, 0.6)),
            "follower.order": Choice(
                "Follower Order, which character goes first",
                "left_to_right",
                ("left_to_right", "right_to_left", "inside_out", "outside_in"),
            ),
            "follower.delay": Plain(
                "Follower Delay, in frames: how long each character waits", 2, 0, 10000
            ),
            "follower.delay_type": Choice(
                "Follower Delay Type: each waits delay after the one before, or first_to_last "
                "spreads delay over them all",
                "each",
                ("each", "first_to_last"),
            ),
            "follower.opacity": Setting("Follower Opacity of each character", 1, 0, 1),
            "follower.offset.x": Setting("Follower Offset X", 0, *_OFFSET, "heights"),
            "follower.offset.y": Setting("Follower Offset Y, up", 0, *_OFFSET, "heights"),
            "follower.angle": Setting("Follower Angle", 0, -3600, 3600, _DEGREES),
            "follower.scale": Setting("Follower Size, a factor", 1, *_SCALE),
        },
        hands_on=None,
        parts=("outline", "border", "follower"),
    ),
}
"""The kinds of node, as PROTOCOL.md's Compositions gives them: a ``kind`` that is not here is
one that the engine cannot read."""

_COORDINATES = (
    "A place {x, y} is a share of the width and of the height of the picture from its LOWER "
    "LEFT corner, x to the right and y up: the middle is 0.5, 0.5. Widths, soft edges and the "
    "points of a polygon are in widths of the picture; a size is a factor; an angle is in "
    "degrees counter-clockwise. (A clip's own transform counts from the middle, y down and "
    "clockwise: the two are not the same.) The pictures of a composition have the size of the "
    "clip's source."
)
_LINKS = (
    "A link goes from the output of a node into one input of another: an output may feed many "
    "inputs, an input takes one link, and links make no loop. An input that is not linked gives "
    "a transparent picture; a mask that is not linked is no mask. A mask input reads the alpha "
    "of what is linked into it: where it is 1 the node does its work, where it is 0 the picture "
    "of the node's first input passes as it came. pass_through hands the first input on as it is."
)
_KEYS = (
    "A number takes keys: video_fusion_keys puts one at a frame of the timeline by the path "
    "<node>.<setting>, as n4.center.x or n9.points.2.x. A word, a true or false and anything of "
    "a tracker or a planar_tracker take none. A number of a part of a text_plus that it has not "
    "(outline, border, follower) takes none until the part is added."
)
_TRACKING = (
    "Each pattern of a tracker has a path, where its middle was found at each frame that was "
    "tracked, filled by video_fusion_track (or by tracking on the Fusion page), which follows "
    "each pattern from where it was put, pattern.x and pattern.y, or from where its path has it "
    "at the frame that it starts from. Between the reference frame and a frame "
    "the patterns move by a motion: of one pattern a shift, of two or more a turn and a size "
    "too, each corrected where position, rotation or scaling is on. operation match_move draws "
    "it: merge bg_only holds the input still (stabilizing), and fg_only, fg_over_bg and "
    "bg_over_fg move the foreground with the patterns. The center of a merge, a transform or a "
    "mask may follow a tracker by center.follow: output tracker is where its pattern tracker "
    "(from 0) is, with its offset; steady_position and unsteady_position follow its motion. "
    "While it follows, center.x and center.y are read only where nothing is published; null "
    "lets go, and a tracker that a center follows is not removed. The region of a "
    "planar_tracker is followed as a plane by video_fusion_track too, from its reference frame "
    "where the clip shows it: its path is the homography of the plane from the reference frame "
    "at each frame tracked, which operation steady takes back (the plane held still) and "
    "corner_pin lays the corners of the foreground along (its corners, at the reference frame)."
)
_PARTS = (
    '{parts} are not there at first: {{"{first}": {{}}}} adds one as a new one is, a setting '
    'of one adds it with the rest as a new one has them, and {{"{first}": null}} takes it away'
)
"""How the parts of a kind are added and taken away, for :func:`reference`."""
_NONE = (
    "the clip has no composition and shows its source; the first action of video_fusion_node "
    "gives it MediaIn1 -> MediaOut1.input, which shows the same"
)


def _a(kind: object) -> str:
    """A kind with its article, as it is said in a sentence: ``a blur``, ``an ellipse``."""
    word = str(kind)
    return f"an {word}" if word.startswith(("a", "e", "i", "o", "u")) else f"a {word}"


def _put(params: dict[str, Any], path: str, value: Any) -> None:
    """Put a value at a path with dots, making the objects on the way."""
    *heads, last = path.split(".")
    place = params
    for head in heads:
        place = cast("dict[str, Any]", place.setdefault(head, {}))
    place[last] = value


def new_params(kind: str) -> dict[str, Any]:
    """The settings of a new node of a kind, every one written, as the engine takes a node whole.

    Raises:
        KeyError: There is no such kind.
    """
    known = KINDS[kind]
    params: dict[str, Any] = {}
    for path, spec in known.settings.items():
        if isinstance(spec, Follows):
            continue  # Written only where the center follows a tracker.
        if path.split(".")[0] in known.parts:
            continue  # A part that a new node has not.
        _put(params, path, _first_value(spec))
    return params


def new_part(kind: str, part: str) -> dict[str, Any]:
    """A part of a node that it may be without, whole, as a new one is.

    The ``outline`` of a ``text_plus`` is black and 0.02 thick at first.

    Raises:
        KeyError: The kind has no such part.
    """
    known = KINDS[kind]
    if part not in known.parts:
        raise KeyError(part)
    whole: dict[str, Any] = {}
    for path, spec in known.settings.items():
        head, _, rest = path.partition(".")
        if head == part:
            _put(whole, rest, _first_value(spec))
    return whole


def _first_value(spec: FusionSpec) -> Any:
    """What a setting is in a new node."""
    if isinstance(spec, PointList | PatternList):
        return []
    if isinstance(spec, Places):
        return [{"x": x, "y": y} for x, y in spec.default]
    if isinstance(spec, Homographies):
        return {}
    if isinstance(spec, Follows):
        return None
    return spec.default


def new_pattern() -> dict[str, Any]:
    """A pattern of a tracker as one is at first: unnamed, on, in the middle and not tracked."""
    pattern: dict[str, Any] = {"name": "", "enabled": True}
    for path, spec in PATTERN.items():
        _put(pattern, path, spec.default)
    pattern["path"] = {}
    return pattern


def _spec_told(spec: FusionSpec) -> str:
    if isinstance(spec, Setting | Plain):
        return spec.told()
    if isinstance(spec, Flag):
        return f"{spec.name}: true or false, {str(spec.default).lower()} at first; no keys"
    if isinstance(spec, Choice):
        return f"{spec.name}: one of {', '.join(spec.choices)}; {spec.default} at first; no keys"
    if isinstance(spec, Follows):
        return (
            f"{spec.name}: {{node, output, tracker}}, node the id of a tracker, output one of "
            f"{', '.join(FOLLOWS)}, tracker its pattern (from 0), read with output tracker; null "
            "lets go; none at first; while it follows, center.x and center.y are read only where "
            "nothing is published; no keys"
        )
    if isinstance(spec, PatternList):
        fields = "; ".join(one.told() for one in PATTERN.values())
        return (
            f"{spec.name}: a list of {spec.most} patterns at most, set whole ([] at first), each "
            '{"name", "enabled", "pattern": {x, y, width, height}, "search": {width, height}, '
            '"offset": {x, y}, "path"} given in part, what is left out as in a new pattern '
            f'(name "", enabled true); {fields}; path, {{"<frame>": [x, y]}} at frames of the '
            "clip's keys, is filled by tracking on the Fusion page or by video_fusion_track ({} "
            "clears it); "
            f"{_ONE_PATTERN}"
        )
    if isinstance(spec, Words):
        counted = "bytes of UTF-8" if spec.in_bytes else "characters"
        return f'{spec.name}: words, {spec.most} {counted} at most, "" at first; no keys'
    if isinstance(spec, Places):
        return (
            f"{spec.name}: a list of {spec.least} to {spec.most} places {{x, y}} (or [x, y]), "
            f"each {_PLACE.low:g} to {_PLACE.high:g} from the lower left, set whole (the box from "
            "0.25 to 0.75 at first); no keys"
        )
    if isinstance(spec, Homographies):
        return (
            f'{spec.name}: {{"<frame>": [h11, h12, h13, h21, h22, h23, h31, h32]}} at frames of '
            "the clip's keys, filled by video_fusion_track ({} clears it); no keys"
        )
    return (
        f"{spec.name}: a list of {{x, y}}, {spec.most} at most, set whole ([] at first, and "
        f"fewer than three are a mask of nothing); {POINT['x'].told()}; {POINT['y'].told()}; "
        "each number takes keys by points.N.x and points.N.y, N from 0"
    )


def reference() -> dict[str, Any]:
    """Every kind of node in full: what it is for, its inputs and each of its settings.

    A setting is told by its path with its name, its range, its value in a new node and its
    unit; with them, how places are counted, what a link does, which settings take keys, and
    what a tracker does and how a center follows one. A kind whose node may be without some of
    its settings tells how they are added and taken away (``parts``).
    """
    kinds: dict[str, Any] = {}
    for name, kind in KINDS.items():
        kinds[name] = {
            "for": kind.told,
            "inputs": list(kind.inputs),
            "output": kind.output,
            "settings": {path: _spec_told(spec) for path, spec in kind.settings.items()},
        }
        if kind.parts:
            kinds[name]["parts"] = _PARTS.format(parts=", ".join(kind.parts), first=kind.parts[0])
    return {
        "kinds": kinds,
        "coordinates": _COORDINATES,
        "links": _LINKS,
        "keys": _KEYS,
        "tracking": _TRACKING,
    }


# ---------------------------------------------------------------------------------------------
# A composition as it is read
# ---------------------------------------------------------------------------------------------


def _object(value: object) -> dict[str, Any]:
    return dict(cast("Mapping[str, Any]", value)) if isinstance(value, Mapping) else {}


def _items(values: object) -> list[Any]:
    return list(cast("list[Any]", values)) if isinstance(values, list) else []


def _is_map(value: object) -> bool:
    return isinstance(value, Mapping)


def _is_list(value: object) -> bool:
    return isinstance(value, list)


def _real(value: object) -> bool:
    return (
        isinstance(value, int | float)
        and not isinstance(value, bool)
        and math.isfinite(float(value))
    )


def _fixed(value: object) -> float | None:
    """A number without keys as a float, or none where the value is not one."""
    if isinstance(value, int | float) and not isinstance(value, bool) and math.isfinite(value):
        return float(value)
    return None


def _nodes(composition: Mapping[str, Any]) -> list[dict[str, Any]]:
    return [_object(one) for one in _items(composition.get("nodes"))]


def _links(composition: Mapping[str, Any]) -> list[dict[str, Any]]:
    return [_object(one) for one in _items(composition.get("links"))]


def _into(composition: Mapping[str, Any]) -> dict[tuple[str, str], str]:
    """The node that each input that is linked takes its picture from, by its node and name."""
    return {
        (str(link.get("to")), str(link.get("input"))): str(link.get("from"))
        for link in _links(composition)
    }


def _feeding(composition: Mapping[str, Any], id: str) -> set[str]:
    """Every node that the picture of a node depends on, through any of its inputs."""
    links = _links(composition)
    found: set[str] = set()
    waiting = [id]
    while waiting:
        to = waiting.pop()
        for link in links:
            source = str(link.get("from"))
            if link.get("to") == to and source not in found:
                found.add(source)
                waiting.append(source)
    return found


def _at(params: object, path: str) -> Any:
    """What is at a path with dots: an item of a list by its place from 0, ``points.2.x``."""
    now: Any = params
    for part in path.split("."):
        if _is_list(now):
            items = _items(now)
            now = items[int(part)] if part.isdecimal() and int(part) < len(items) else None
        else:
            now = _object(now).get(part)
    return now


def _keys_of(now: Any) -> list[dict[str, Any]]:
    return [_object(key) for key in _items(_object(now).get("keys"))]


def _keys_told(now: Any, clock: KeyClock) -> Any:
    """A setting as it is told: each of its keys with the frame of the timeline that shows it."""
    if not _is_map(now):
        return now
    shown = [{"timeline_frame": clock.shown(int(key["frame"])), **key} for key in _keys_of(now)]
    return {"keys": shown}


def _points_keyed(params: Mapping[str, Any]) -> bool:
    """Whether a number of a point of a polygon has keys."""
    points = [_object(point) for point in _items(params.get("points"))]
    return any(_is_map(point.get(axis)) for point in points for axis in POINT)


def _outputs(node: Mapping[str, Any]) -> bool:
    """Whether a node has an output: every kind but the ``media_out``."""
    known = KINDS.get(str(node.get("kind")))
    return known is None or known.output


def _tracked(params: Mapping[str, Any]) -> bool:
    """Whether a pattern of a tracker has a path."""
    return any(_object(_object(one).get("path")) for one in _items(params.get("trackers")))


def _path_of(value: object) -> list[tuple[int, float, float]]:
    """The places of the path of a pattern, ``(frame, x, y)``, by their frames in order."""
    places: list[tuple[int, float, float]] = []
    for frame, place in _object(value).items():
        pair = [_fixed(one) for one in _items(place)]
        if len(pair) != 2 or not frame.removeprefix("-").isdecimal():
            continue
        x, y = pair
        if x is not None and y is not None:
            places.append((int(frame), x, y))
    places.sort()
    return places


def _place_at(places: Sequence[tuple[int, float, float]], frame: int) -> tuple[float, float]:
    """Where a pattern is at a frame of its keys, as the engine reads its path.

    On the line between the frames of the path about the frame; before its first frame where
    it was at that one, and after its last where it was at that one.
    """
    before = [one for one in places if one[0] <= frame]
    after = [one for one in places if one[0] >= frame]
    if not before:
        return places[0][1], places[0][2]
    if not after:
        return places[-1][1], places[-1][2]
    (start, x0, y0), (end, x1, y1) = before[-1], after[0]
    if end == start:
        return x0, y0
    share = (frame - start) / (end - start)
    return x0 + (x1 - x0) * share, y0 + (y1 - y0) * share


def _pattern_told(place: int, pattern: Mapping[str, Any], clock: KeyClock) -> dict[str, Any]:
    """A pattern of a tracker as it is told: its settings, and what its path is.

    Of the path, how many frames it has, its first and its last with the frames of the
    timeline that show them, and where the pattern is at the middle of the clip; not the path
    itself, which may have a place at every frame.
    """
    told: dict[str, Any] = {
        "index": place,
        "name": pattern.get("name", ""),
        "enabled": pattern.get("enabled", True),
    }
    for group in ("pattern", "search", "offset"):
        told[group] = _object(pattern.get(group))
    places = _path_of(pattern.get("path"))
    if not places:
        told["path"] = "none: not tracked"
        return told
    middle = clock.start + clock.length // 2
    x, y = _place_at(places, clock.at(middle))
    first, last = places[0][0], places[-1][0]
    told["path"] = {
        "frames": len(places),
        "first": {"timeline_frame": clock.shown(first), "frame": first},
        "last": {"timeline_frame": clock.shown(last), "frame": last},
        "at_middle": {"timeline_frame": middle, "x": round(x, 4), "y": round(y, 4)},
    }
    return told


_LONGEST_TOLD = 160
"""How many characters of the text of a Text+ are told."""


def _homographies_told(value: object, clock: KeyClock) -> Any:
    """The path of a Planar Tracker as it is told, which is not the path itself.

    How many frames it has, and its first and its last with the frames of the timeline that
    show them; or that it has none.
    """
    frames = sorted(int(one) for one in _object(value) if one.removeprefix("-").isdecimal())
    if not frames:
        return "none: not tracked"
    first, last = frames[0], frames[-1]
    return {
        "frames": len(frames),
        "first": {"timeline_frame": clock.shown(first), "frame": first},
        "last": {"timeline_frame": clock.shown(last), "frame": last},
    }


def _as_new(spec: FusionSpec, now: object) -> bool:
    """Whether a setting is as a new node has it, a number without keys."""
    if isinstance(spec, Setting | Plain | Flag | Choice | Words):
        return not _is_map(now) and now == spec.default
    return False


def _settings_told(kind: Kind, params: Mapping[str, Any], clock: KeyClock) -> dict[str, Any]:
    """Every setting of a node by its path, with what it is now.

    Of a kind that has parts (a Text+), only what is not as a new node has it, and the text,
    are told, and a part that the node has not is told as ``None``: so is a text kept short.
    """
    told: dict[str, Any] = {}
    for path, spec in kind.settings.items():
        now = _at(params, path)
        head = path.split(".")[0]
        if head in kind.parts:
            if not _is_map(params.get(head)):
                told[head] = None
                continue
        elif kind.parts and path != "text" and _as_new(spec, now):
            continue
        if isinstance(spec, Follows) and not _is_map(now):
            continue  # A center that follows nothing is told as its x and y alone.
        if isinstance(spec, Words):
            words = now if isinstance(now, str) else ""
            cut = len(words) > _LONGEST_TOLD
            told[path] = f"{words[:_LONGEST_TOLD]}... ({len(words)} characters)" if cut else words
        elif isinstance(spec, Places):
            told[path] = {"places": len(_items(now))}
        elif isinstance(spec, Homographies):
            told[path] = _homographies_told(now, clock)
        elif isinstance(spec, PointList):
            told[path] = [
                {axis: _keys_told(_object(point).get(axis), clock) for axis in POINT}
                for point in _items(now)
            ]
        elif isinstance(spec, PatternList):
            told[path] = [
                _pattern_told(place, _object(one), clock) for place, one in enumerate(_items(now))
            ]
        elif isinstance(spec, Follows):
            told[path] = _object(now)
        elif isinstance(spec, Plain) and spec.unit == "frame":
            frame = _fixed(now)
            shown = None if frame is None else clock.shown(int(frame))
            told[path] = {"timeline_frame": shown, "frame": now}
        else:
            told[path] = _keys_told(now, clock)
    return told


def _notes(kind: str, params: Mapping[str, Any]) -> list[str]:
    """What is to be said of a node beside its settings."""
    notes: list[str] = []
    if kind == "polygon" and len(_items(params.get("points"))) < 3:
        notes.append("fewer than three points: a mask of nothing")
    if kind == "tracker" and not _tracked(params):
        notes.append(
            "no pattern has a path yet: video_fusion_track fills them, and until then nothing moves"
        )
    if kind == "planar_tracker" and not _object(params.get("path")):
        notes.append(
            "not tracked yet: video_fusion_track follows its region, and until then nothing moves"
        )
    if kind == "text_plus":
        notes.append(
            "settings as a new node has them are left out (settings_of tells them); a part that "
            'is null is not there: {"outline": {}} adds one'
        )
    follow = _object(_at(params, "center.follow"))
    if follow:
        notes.append(
            f"its center follows {follow.get('node')}: center.x and center.y are read only where "
            "nothing is published"
        )
    return notes


def summary(composition: Mapping[str, Any], clock: KeyClock) -> dict[str, Any]:
    """A composition as a model is to read it.

    ``nodes``: each with its id, kind, label and whether it is passed through; ``inputs``, the
    node that each of its linked inputs takes its picture from; ``free``, its inputs that
    nothing is linked into; and ``settings``, every setting by its path with what it is now, one
    that has keys as ``{"keys": [...]}``, each key with the first frame of the timeline that
    shows it (``timeline_frame``, by ``clock``; ``None`` where none does) beside its own.
    ``links``: each as ``from -> to.input``. ``output``: the node that the ``media_out`` reads,
    ``None`` where the clip is transparent. ``unread``: the nodes whose output nothing reads;
    ``unused``: those that the picture does not depend on. ``settings_of``: for each kind that
    the composition has, each setting with its name, range, value in a new node and unit.
    """
    nodes = _nodes(composition)
    into = _into(composition)
    read = set(into.values())
    out = next((str(one.get("id")) for one in nodes if one.get("kind") == "media_out"), None)
    used = _feeding(composition, out) if out is not None else set[str]()
    told: list[dict[str, Any]] = []
    kinds: dict[str, dict[str, str]] = {}
    for one in nodes:
        id, kind = str(one.get("id")), str(one.get("kind"))
        item: dict[str, Any] = {
            "id": id,
            "kind": kind,
            "label": str(one.get("label", "")),
            "pass_through": bool(one.get("pass_through", False)),
        }
        known = KINDS.get(kind)
        if known is not None:
            linked = {name: into[(id, name)] for name in known.inputs if (id, name) in into}
            if linked:
                item["inputs"] = linked
            free = [name for name in known.inputs if (id, name) not in into]
            if free:
                item["free"] = free
            if known.settings:
                params = _object(one.get("params"))
                item["settings"] = _settings_told(known, params, clock)
                kinds[kind] = {path: _spec_told(spec) for path, spec in known.settings.items()}
                notes = _notes(kind, params)
                if notes:
                    item["note"] = "; ".join(notes)
        told.append(item)
    outputs = [str(one.get("id")) for one in nodes if _outputs(one)]
    return {
        "nodes": told,
        "links": [
            f"{link.get('from')} -> {link.get('to')}.{link.get('input')}"
            for link in _links(composition)
        ],
        "output": None if out is None else into.get((out, "input")),
        "unread": [id for id in outputs if id not in read],
        "unused": [id for id in outputs if id not in used],
        "settings_of": kinds,
    }


# ---------------------------------------------------------------------------------------------
# The clip and its composition in a project
# ---------------------------------------------------------------------------------------------


def _timeline(project: Mapping[str, Any], id: str | None) -> dict[str, Any]:
    try:
        return timeline_of(project, id)
    except GradeError as error:
        raise FusionError(str(error)) from None


def _picture_clip(timeline: Mapping[str, Any], id: str) -> dict[str, Any]:
    for track in map(_object, _items(timeline.get("tracks"))):
        for clip in map(_object, _items(track.get("clips"))):
            if clip.get("id") == id:
                if track.get("kind") != "video":
                    raise FusionError(
                        f"{id} is a clip of sound on {track.get('id')}; a composition is of a "
                        "clip of a picture track (the engine refuses it as wrong_track fusion)"
                    )
                return clip
    raise FusionError(
        f"there is no clip {id} on timeline {timeline.get('id')}; video_info lists the clips"
    )


def _clock(
    project: Mapping[str, Any], timeline: Mapping[str, Any], clip: Mapping[str, Any]
) -> KeyClock:
    named = _object(clip.get("source")).get("media")
    media = next(
        (one for one in map(_object, _items(project.get("media"))) if one.get("id") == named),
        None,
    )
    return KeyClock.of(clip, Rate.model_validate(timeline["rate"]), media)


def _node_of(composition: Mapping[str, Any], id: str) -> dict[str, Any]:
    """A node of a composition by its id.

    Raises:
        FusionError: There is no such node; the ones that there are are told.
    """
    nodes = _nodes(composition)
    for one in nodes:
        if one.get("id") == id:
            return one
    ids = ", ".join(f"{one.get('id')} ({one.get('kind')})" for one in nodes) or "none"
    raise FusionError(f"the composition has no node {id}; its nodes are {ids}")


def _kind_of(node: Mapping[str, Any]) -> Kind:
    kind = KINDS.get(str(node.get("kind")))
    if kind is None:
        raise FusionError(
            f"node {node.get('id')} is {_a(node.get('kind'))}, a kind that this tool does not know"
        )
    return kind


def _state(project: Mapping[str, Any], timeline: str | None, clip: str) -> dict[str, Any]:
    """The composition of a clip as it is told: as :func:`summary`, or that it has none."""
    on = _timeline(project, timeline)
    found = _picture_clip(on, clip)
    told: dict[str, Any] = {"timeline": on["id"], "clip": clip}
    own = found.get("fusion")
    if not _is_map(own):
        told["composition"] = False
        told["shows"] = _NONE
        return told
    clock = _clock(project, on, found)
    told["composition"] = True
    told.update(summary(_object(own), clock))
    told["key_frames"] = f"frames of {clock.counts}, from 0"
    return told


# ---------------------------------------------------------------------------------------------
# Checking the settings of a node
# ---------------------------------------------------------------------------------------------


def _point_of(kind: str, path: str) -> tuple[int, str] | None:
    """The place of a point of a polygon and which of its numbers a path names, if any."""
    parts = path.split(".")
    if kind != "polygon" or len(parts) != 3 or parts[0] != "points":
        return None
    if not parts[1].isdecimal() or parts[2] not in POINT:
        return None
    return int(parts[1]), parts[2]


def _unknown(kind: str, path: str) -> FusionError:
    """What is said of a path that names no setting of a kind, with the nearest that it has."""
    paths = list(KINDS[kind].settings)
    if not paths:
        return FusionError(f"{_a(kind)} has no settings")
    last = path.rsplit(".", 1)[-1]
    close = (
        [one for one in paths if one.startswith(f"{path}.")]
        or [one for one in paths if one.rsplit(".", 1)[-1] == last]
        or difflib.get_close_matches(path, paths, n=3)
    )
    maybe = f" (did you mean {', '.join(close)}?)" if close else ""
    has = ", ".join(paths)
    if kind == "polygon":
        has += "; a number of a point is points.N.x or points.N.y, N from 0"
    if kind == "tracker":
        has += f"; {_ONE_PATTERN}"
    parts = KINDS[kind].parts
    if parts:
        has += f"; {', '.join(parts)} as a whole: {{}} adds one, null takes it away"
    return FusionError(f"{_a(kind)} has no setting {path!r}{maybe}; it has {has}")


def _number(spec: Setting | Plain, path: str, value: Any, what: str) -> Any:
    """A number of a setting, within its range; one that is whole as a whole number.

    Raises:
        FusionError: It is no number, or out of the range, said with the bounds and with the
            name that the engine refuses it by.
    """
    if not _real(value):
        if _is_map(value) and isinstance(spec, Setting):
            raise FusionError(
                f"{path} takes a number here ({spec.told()}); video_fusion_keys puts its keys, "
                "at frames of the timeline"
            )
        raise FusionError(f"{path} is a number ({spec.told()})")
    if isinstance(spec, Plain) and spec.whole:
        if not float(value).is_integer():
            raise FusionError(f"{path} is a whole number ({spec.told()})")
        value = int(value)
    if not spec.low <= float(value) <= spec.high:
        raise FusionError(
            f"{path} is {float(value):g}, out of range ({spec.told()}); the engine refuses it "
            f"as out_of_range {what}"
        )
    return value


def _points(spec: PointList, value: Any) -> list[dict[str, Any]]:
    """The points of a polygon as the engine takes them, from ``{x, y}`` or ``[x, y]`` each."""
    told = (
        f'points is a list of {{"x": ..., "y": ...}} (or [x, y]), {spec.most} at most (the '
        "engine refuses more as out_of_range polygon.points); it is set whole"
    )
    if not _is_list(value) or len(_items(value)) > spec.most:
        raise FusionError(told)
    points: list[dict[str, Any]] = []
    for place, point in enumerate(_items(value)):
        pair = _items(point)
        named = _object(point)
        if _is_list(point) and len(pair) == 2:
            x, y = pair
        elif _is_map(point) and set(named) == {"x", "y"}:
            x, y = named["x"], named["y"]
        else:
            raise FusionError(told)
        points.append(
            {
                "x": _number(POINT["x"], f"points.{place}.x", x, "polygon.points.x"),
                "y": _number(POINT["y"], f"points.{place}.y", y, "polygon.points.y"),
            }
        )
    return points


def _words(spec: Words, path: str, value: Any, what: str) -> str:
    """Words of a setting, no longer than it takes.

    Raises:
        FusionError: They are no words, or too long, said with the name that the engine
            refuses them by.
    """
    if not isinstance(value, str):
        raise FusionError(f"{path} is words, a string ({_spec_told(spec)})")
    length = len(value.encode("utf-8")) if spec.in_bytes else len(value)
    if length > spec.most:
        counted = "bytes of UTF-8" if spec.in_bytes else "characters"
        raise FusionError(
            f"{path} is {length} {counted} long, and {spec.most} at most (the engine refuses it "
            f"as out_of_range {what})"
        )
    return value


def _places(spec: Places, path: str, value: Any, what: str) -> list[dict[str, Any]]:
    """The places of a region as the engine takes them, from ``{x, y}`` or ``[x, y]`` each.

    Raises:
        FusionError: It is no list of places, has too few or too many, or a place is out of
            range; said with the name that the engine refuses it by.
    """
    told = (
        f'{path} is a list of {spec.least} to {spec.most} places {{"x": ..., "y": ...}} (or '
        f"[x, y]) from the lower left, set whole (the engine refuses others as out_of_range "
        f"{what})"
    )
    given = _items(value)
    if not _is_list(value) or not spec.least <= len(given) <= spec.most:
        raise FusionError(told)
    places: list[dict[str, Any]] = []
    for place, point in enumerate(given):
        pair = _items(point)
        named = _object(point)
        if _is_list(point) and len(pair) == 2:
            x, y = pair
        elif _is_map(point) and set(named) == {"x", "y"}:
            x, y = named["x"], named["y"]
        else:
            raise FusionError(told)
        places.append(
            {
                "x": _number(_PLACE, f"{path}.{place}.x", x, what),
                "y": _number(_PLACE, f"{path}.{place}.y", y, what),
            }
        )
    return places


def _part_given(part: str, value: Any) -> dict[str, Any] | None:
    """A part of a node given whole: ``{}`` to add it, or ``None`` to take it away.

    ``true`` adds it too, and ``false`` takes it away.

    Raises:
        FusionError: It is given as something else.
    """
    if value is None or value is False:
        return None
    if value is True or (_is_map(value) and not _object(value)):
        return {}
    raise FusionError(
        f'{part} is given by its settings, {{"{part}": {{...}}}}; {{}} or true adds one as a new '
        "one is, and null or false takes it away"
    )


def _parts_put(kind: str, now: Mapping[str, Any], patch: dict[str, Any]) -> None:
    """Put each part of a node that a patch adds whole into the patch, as the engine takes it.

    A setting of a part that the node has stays a path of the patch, which the engine puts into
    the part. A part that it has not is put whole, what is not given as a new one has it
    (:func:`new_part`); one that is added (``{}``) and that it has is left as it is; one that is
    taken away (``None``) is taken away where it has it. (A part is not given both ways at once:
    :func:`columbina.video.grading.expanded` refuses that as given twice.)
    """
    for part in KINDS[kind].parts:
        fields = {path: patch.pop(path) for path in list(patch) if path.startswith(f"{part}.")}
        had = _is_map(now.get(part))
        if part in patch and patch[part] is None:
            if not had:
                del patch[part]  # It has none to take away.
            continue
        if part in patch:
            del patch[part]  # Added: as it is, or as a new one, with the fields given.
            if not had:
                patch[part] = new_part(kind, part)
        elif fields and not had:
            patch[part] = new_part(kind, part)
        if part in patch:
            for path, value in fields.items():
                _put(patch[part], path.removeprefix(f"{part}."), value)
        else:
            patch.update(fields)


def _whole(path: str) -> bool:
    """Whether the object at a path of a patch is one value: a follow, a new pattern, a path."""
    return path in ("center.follow", "trackers.+") or path.rsplit(".", 1)[-1] == "path"


def _flattened(given: Mapping[str, Any], path: str, out: dict[str, Any]) -> None:
    """Put into ``out`` each value of a patch by its path with dots."""
    for key, value in given.items():
        here = f"{path}.{key}" if path else key
        inner = _object(value)
        if _is_map(value) and inner and "keys" not in inner and not _whole(here):
            _flattened(inner, here, out)
        else:
            out[here] = value


def _pattern_put(pattern: dict[str, Any], field: str, value: Any, where: str) -> None:
    """Put one field of a pattern of a tracker, checked as the engine checks it.

    Raises:
        FusionError: The pattern has no such field, or the value is not one that it takes. A
            path is filled by tracking: ``{}`` alone is taken, which clears it.
    """
    path = f"{where}.{field}"
    spec = PATTERN.get(field)
    if spec is not None:
        what = f"tracker.trackers.{field.split('.')[0]}"
        _put(pattern, field, _number(spec, path, value, what))
    elif field == "name":
        if not isinstance(value, str) or len(value) > _LONGEST_LABEL:
            raise FusionError(
                f"{path} is a name of {_LONGEST_LABEL} characters at most (the engine refuses a "
                "longer one as out_of_range tracker.trackers.name)"
            )
        pattern["name"] = value
    elif field == "enabled":
        if not isinstance(value, bool):
            raise FusionError(f"{path} is true or false: false leaves it out of the motion")
        pattern["enabled"] = value
    elif field == "path":
        if not _is_map(value) or _object(value):
            raise FusionError(
                f"{path} is filled by tracking on the Fusion page: {{}} clears it, and nothing "
                "else is taken; video_fusion_track tracks it"
            )
        pattern["path"] = {}
    else:
        raise FusionError(f"{path!r} names nothing of a pattern: {_PATTERN_FIELDS}")


def _pattern(given: Any, where: str) -> dict[str, Any]:
    """A new pattern of a tracker, given in part: what is left out is as in a new pattern.

    Raises:
        FusionError: It is no object, or a field of it is wrong.
    """
    if not _is_map(given):
        raise FusionError(
            f'{where} is a pattern, given in part, as {{"name": "eye", "pattern": {{"x": 0.4, '
            f'"y": 0.6}}}}; {_PATTERN_FIELDS}'
        )
    try:
        fields = expanded(_object(given))
    except GradeError as error:
        raise FusionError(str(error)) from None
    flat: dict[str, Any] = {}
    _flattened(fields, "", flat)
    pattern = new_pattern()
    for field, value in flat.items():
        _pattern_put(pattern, field, value, where)
    return pattern


def _patterns(spec: PatternList, value: Any) -> list[dict[str, Any]]:
    """The patterns of a tracker as the engine takes them, each given in part and filled in.

    Raises:
        FusionError: It is no list, has too many, or a pattern of it is wrong.
    """
    if not _is_list(value):
        raise FusionError(
            f"trackers is a list of patterns, {spec.most} at most, each given in part: "
            '[{"name": "eye", "pattern": {"x": 0.4, "y": 0.6}}]'
        )
    given = _items(value)
    if len(given) > spec.most:
        raise FusionError(
            f"a tracker has {spec.most} patterns at most (the engine refuses more as "
            "out_of_range tracker.trackers)"
        )
    return [_pattern(one, f"trackers.{place}") for place, one in enumerate(given)]


def _edited(
    now: Sequence[Any], changes: Mapping[str, Any], named: str
) -> tuple[list[dict[str, Any]], int | None]:
    """The patterns of a tracker with changes of single patterns put in, the others as they are.

    ``trackers.N.<field>`` changes a field of the pattern at N (from 0), ``trackers.-`` takes
    the pattern at the place that it is given away, and ``trackers.+`` adds a pattern given in
    part at the end, in that order. Every path that is not cleared stays as it is.

    Returns:
        The patterns, and the place of the one taken away, if one is.

    Raises:
        FusionError: A pattern that is named is not there, or what is given is wrong.
    """
    patterns = [_object(copy.deepcopy(one)) for one in now]
    count = len(patterns)
    for path, value in changes.items():
        if path in ("trackers.+", "trackers.-"):
            continue
        place, _, field = path.removeprefix("trackers.").partition(".")
        if not place.isdecimal() or not field:
            raise FusionError(f"{path!r} names no pattern: {_ONE_PATTERN}")
        if int(place) >= count:
            raise FusionError(
                f"{named} has {count} patterns, numbered from 0: there is no {path}; {_ONE_PATTERN}"
            )
        _pattern_put(patterns[int(place)], field, value, f"trackers.{place}")
    removed: int | None = None
    if "trackers.-" in changes:
        taken = changes["trackers.-"]
        if not _real(taken) or not float(taken).is_integer() or not 0 <= taken < count:
            has = f"0 to {count - 1}" if count else "it has none"
            raise FusionError(
                f"trackers.- is the place of the pattern of {named} to take away, from 0: {has}"
            )
        removed = int(taken)
        del patterns[removed]
    if "trackers.+" in changes:
        patterns.append(_pattern(changes["trackers.+"], "trackers.+"))
    if len(patterns) > _MOST_TRACKERS:
        raise FusionError(
            f"a tracker has {_MOST_TRACKERS} patterns at most (the engine refuses more as "
            "out_of_range tracker.trackers)"
        )
    return patterns, removed


_FOLLOW_FORM = (
    'center.follow is {"node": the id of a tracker, "output": "tracker", "tracker": N}, N the '
    "place of its pattern from 0 (output steady_position or unsteady_position follows its "
    "motion instead), or null to let go"
)


def _follow_given(value: Any) -> dict[str, Any] | None:
    """What a center is to follow as it is given, its fields checked; ``None`` lets go.

    Raises:
        FusionError: It is not written as what a center follows, or names an output or a
            pattern that a tracker does not publish.
    """
    if value is None:
        return None
    given = _object(value)
    if not given or not set(given) <= {"node", "output", "tracker"}:
        raise FusionError(_FOLLOW_FORM)
    if not isinstance(given.get("node", ""), str):
        raise FusionError(_FOLLOW_FORM)
    if "output" in given and given["output"] not in FOLLOWS:
        raise FusionError(f"the output of center.follow is one of {', '.join(FOLLOWS)}")
    if "tracker" in given:
        place = given["tracker"]
        if not _real(place) or not float(place).is_integer() or not 0 <= place < _MOST_TRACKERS:
            raise FusionError(
                "the tracker of center.follow is the place of a pattern, a whole number from 0 "
                f"to {_MOST_TRACKERS - 1} (the engine refuses others as out_of_range "
                "center.follow.tracker)"
            )
        given["tracker"] = int(place)
    return given


def _checked(
    kind: str, named: str, now: Mapping[str, Any], settings: Mapping[str, Any]
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    """A patch of the settings of a node of ``kind``, checked against the settings of the kind.

    ``settings`` may write a path with dots, ``{"center.x": 0.3}``, or as objects,
    ``{"center": {"x": 0.3}}``. ``now`` is the settings that the node has, and ``named`` what it
    is called in what is said.

    Returns:
        The values of the settings by their paths; the numbers of points of a polygon that are
        named one by one, by their paths (``points.2.x``), which a patch of the node cannot
        carry, since the list of its points is put whole; and the changes of single patterns
        of a tracker by their paths (``trackers.0.name``, ``trackers.+``, ``trackers.-``),
        which are put into the list of its patterns (:func:`_edited`), put whole too.

    Raises:
        FusionError: A setting that the kind has not, with the nearest that it has; or a value
            that it does not take, with its range and the name that the engine refuses it by.
    """
    known = KINDS[kind].settings
    if not known:
        raise FusionError(f"{_a(kind)} has no settings")
    for key in settings:
        if not all(key.split(".")):
            raise FusionError(f"{key!r} is not a path of a setting, such as center.x")
    try:
        given = expanded(settings)
    except GradeError as error:
        raise FusionError(str(error)) from None
    flat: dict[str, Any] = {}
    _flattened(given, "", flat)
    patch: dict[str, Any] = {}
    points: dict[str, Any] = {}
    patterns: dict[str, Any] = {}
    for path, value in flat.items():
        spec = known.get(path)
        if spec is None:
            if kind == "tracker" and path.startswith("trackers."):
                patterns[path] = value
                continue
            if path in KINDS[kind].parts:
                patch[path] = _part_given(path, value)
                continue
            point = _point_of(kind, path)
            if point is None:
                raise _unknown(kind, path)
            count = len(_items(now.get("points")))
            if point[0] >= count:
                raise FusionError(
                    f"{named} has {count} points, numbered from 0: there is no {path}; points "
                    "sets the list whole"
                )
            points[path] = _number(POINT[point[1]], path, value, f"polygon.points.{point[1]}")
        elif isinstance(spec, Setting):
            patch[path] = _number(spec, path, value, f"{kind}.{path}")
        elif isinstance(spec, Plain):
            # The engine names a place of a tracker whole, tracker.pivot_point, and the corners
            # of a planar tracker, planar_tracker.corners.
            whole = kind in ("tracker", "planar_tracker")
            what = f"{kind}.{path.split('.')[0]}" if whole else f"{kind}.{path}"
            patch[path] = _number(spec, path, value, what)
        elif isinstance(spec, Words):
            patch[path] = _words(spec, path, value, f"{kind}.{path}")
        elif isinstance(spec, Places):
            patch[path] = _places(spec, path, value, f"{kind}.{path}")
        elif isinstance(spec, Homographies):
            if not _is_map(value) or _object(value):
                raise FusionError(
                    f"{path} is filled by tracking (video_fusion_track): {{}} clears it, and "
                    "nothing else is taken"
                )
            patch[path] = {}
        elif isinstance(spec, Flag):
            if not isinstance(value, bool):
                raise FusionError(f"{path} ({spec.name}) is true or false")
            patch[path] = value
        elif isinstance(spec, Choice):
            if value not in spec.choices:
                raise FusionError(f"{path} ({spec.name}) is one of {', '.join(spec.choices)}")
            patch[path] = value
        elif isinstance(spec, PointList):
            patch[path] = _points(spec, value)
        elif isinstance(spec, PatternList):
            patch[path] = _patterns(spec, value)
        else:
            patch[path] = _follow_given(value)
    return patch, points, patterns


def _whole_checked(kind: str, params: Mapping[str, Any]) -> None:
    """What the engine checks of the settings of a node together, of numbers without keys.

    Raises:
        FusionError: The thresholds of a keyer have crossed, or its screen colour has no
            channel above its other two.
    """
    if kind != "delta_keyer":
        return
    low, high = _fixed(params.get("low")), _fixed(params.get("high"))
    if low is not None and high is not None and low > high:
        raise FusionError(
            f"low ({low:g}) is above high ({high:g}): the lower threshold is at "
            "most the upper one (the engine refuses it as out_of_range delta_keyer.low)"
        )
    screen = _object(params.get("screen"))
    red, green, blue = (_fixed(screen.get(name)) for name in ("r", "g", "b"))
    if red is not None and green is not None and blue is not None:
        ordered = sorted((red, green, blue))
        if ordered[2] - ordered[1] < _SCREEN_APART:
            raise FusionError(
                f"screen is r {red:g}, g {green:g}, b {blue:g}: one channel is to be above the "
                f"other two by {_SCREEN_APART:g}, as the green of a green screen is (the engine "
                "refuses it as out_of_range delta_keyer.screen)"
            )


# ---------------------------------------------------------------------------------------------
# The operations of a change
# ---------------------------------------------------------------------------------------------

At = tuple[str, str]
"""The clip whose composition a change is of: the id of its timeline, and its own."""


def _input_checked(node: Mapping[str, Any], name: str) -> None:
    inputs = _kind_of(node).inputs
    if name not in inputs:
        has = ", ".join(inputs) or "no inputs"
        raise FusionError(
            f"node {node.get('id')} is {_a(node.get('kind'))}, which has no input {name!r}: it "
            f"has {has} (the engine refuses it as no_input)"
        )


def _output_checked(node: Mapping[str, Any]) -> None:
    if not _kind_of(node).output:
        raise FusionError(
            f"node {node.get('id')} is the media_out, which has no output: it is the end of the "
            "composition (the engine refuses it as no_output)"
        )


def _given_links(
    composition: Mapping[str, Any], kind: str, links: Sequence[Mapping[str, Any]]
) -> list[dict[str, str]]:
    """The links of a node that is added, as they are given, checked."""
    form = (
        'links is a list of {"from": id, "to": id, "input": name}, the new node being "" at '
        "one end of each (the engine refuses others as out_of_range links)"
    )
    wired: list[dict[str, str]] = []
    ends: set[tuple[str, str]] = set()
    for link in links:
        one = _object(link)
        words = all(isinstance(end, str) for end in one.values())
        if set(one) != {"from", "to", "input"} or not words:
            raise FusionError(form)
        source, to, name = str(one["from"]), str(one["to"]), str(one["input"])
        if (source == "") == (to == ""):
            raise FusionError(form)
        if source == "":
            _input_checked(_node_of(composition, to), name)
        else:
            _output_checked(_node_of(composition, source))
            if name not in KINDS[kind].inputs:
                has = ", ".join(KINDS[kind].inputs) or "no inputs"
                raise FusionError(
                    f"{_a(kind)} has no input {name!r}: it has {has} (the engine refuses it "
                    "as no_input)"
                )
        if (to, name) in ends:
            where = f"{to or 'the new node'}.{name}"
            raise FusionError(
                f"two links go into {where}: an input takes one (the engine refuses it as "
                "input_linked)"
            )
        ends.add((to, name))
        wired.append(edits.fusion_link(source, to, name))
    return wired


def _after(composition: Mapping[str, Any], kind: str, after: str) -> list[dict[str, str]]:
    """The links that lay a new node in after a node: into its first input, and on from it.

    The output of ``after`` goes into the input of a picture that the new node has first, and
    the output of the new node into every input that ``after`` fed, each in the place of the
    link that was there, as the node editor of the Fusion page does it.
    """
    before = _node_of(composition, after)
    _output_checked(before)
    first = KINDS[kind].hands_on
    if kind in MASKS:
        raise FusionError(
            f"{_a(kind)} is a mask, which goes into no chain of pictures: give mask_of, the "
            "node that it limits, or links"
        )
    if first is None:
        raise FusionError(
            f"{_a(kind)} has no input of a picture to come after {after} by: it is added "
            'unlinked, or with links, as [{"from": "", "to": id, "input": name}]'
        )
    fed = [link for link in _links(composition) if link.get("from") == after]
    return [
        edits.fusion_link(after, "", first),
        *(edits.fusion_link("", str(link.get("to")), str(link.get("input"))) for link in fed),
    ]


def _masking(composition: Mapping[str, Any], kind: str, mask_of: str) -> list[dict[str, str]]:
    """The link of a new mask into the ``mask`` of the node that it is to limit."""
    if kind not in MASKS:
        raise FusionError(
            f"mask_of is for a mask ({', '.join(MASKS)}): {_a(kind)} is linked by after or links"
        )
    _input_checked(_node_of(composition, mask_of), "mask")
    taken = _into(composition).get((mask_of, "mask"))
    if taken is not None:
        raise FusionError(
            f"the mask of {mask_of} is linked from {taken}: disconnect it first; or, where "
            f"{taken} is a mask, add this one with mask_of {taken}, and the two are combined by "
            f"the paint_mode of {taken}"
        )
    return [edits.fusion_link("", mask_of, "mask")]


def _followers(composition: Mapping[str, Any], id: str) -> list[tuple[str, dict[str, Any]]]:
    """The nodes whose center follows the node ``id``, each by its id with what it follows."""
    follows = [
        (str(one.get("id")), _object(_at(one.get("params"), "center.follow")))
        for one in _nodes(composition)
    ]
    return [(follower, follow) for follower, follow in follows if follow.get("node") == id]


def _follow_checked(
    composition: Mapping[str, Any],
    given: Mapping[str, Any],
    now: object,
    named: str,
    said: list[str],
) -> dict[str, Any]:
    """What a center is to follow, whole: what it follows with what is given put in, checked.

    A pattern that the tracker has not is followed, as the engine allows: the center is then
    its own x and y until the pattern is there and tracked, which is put in ``said``.

    Raises:
        FusionError: There is no node or no output, or the node is not a tracker of the
            composition.
    """
    merged = {**_object(now), **given}
    if "node" not in merged or "output" not in merged:
        raise FusionError(_FOLLOW_FORM)
    id = str(merged["node"])
    target = _node_of(composition, id)
    if target.get("kind") != "tracker":
        raise FusionError(
            f"node {id} is {_a(target.get('kind'))}: a center follows a tracker (the engine "
            "refuses it as wrong_node)"
        )
    follow: dict[str, Any] = {
        "node": id,
        "output": merged["output"],
        "tracker": merged.get("tracker", 0),
    }
    count = len(_items(_object(target.get("params")).get("trackers")))
    if follow["output"] == "tracker" and follow["tracker"] >= count:
        said.append(
            f"{named} follows pattern {follow['tracker']} of {id}, which has {count}: its "
            "center is its own x and y until that pattern is there and tracked"
        )
    return follow


def _repointed(
    at: At, composition: Mapping[str, Any], tracker: str, removed: int
) -> list[dict[str, Any]]:
    """The operations that keep the centers that follow a tracker on the patterns they follow.

    The pattern at ``removed`` is taken away, and each after it comes one place nearer.

    Raises:
        FusionError: A center follows the pattern that is taken away.
    """
    ops: list[dict[str, Any]] = []
    for follower, follow in _followers(composition, tracker):
        place = follow.get("tracker", 0)
        if follow.get("output") != "tracker" or not isinstance(place, int) or place < removed:
            continue
        if place == removed:
            raise FusionError(
                f"the center of {follower} follows pattern {removed} of {tracker}, which is not "
                f'taken away while it does: set {{"center.follow": null}} on {follower} first, '
                "or have it follow another pattern"
            )
        moved = {"center": {"follow": {"tracker": place - 1}}}
        ops.append(edits.set_fusion_node(at[0], at[1], follower, params=moved))
    return ops


def _framed(patch: dict[str, Any], clock: KeyClock) -> None:
    """Put the frame of the clip's keys in place of the frame of the timeline of a reference.

    Raises:
        FusionError: The frame is not one of the clip.
    """
    frame = patch.get("reference")
    if frame is None:
        return
    last = clock.start + clock.length - 1
    if not clock.start <= frame <= last:
        raise FusionError(
            f"reference is {frame}, not a frame of the clip, which is at frames {clock.start} "
            f"to {last} of the timeline: reference is the frame of the timeline that the "
            "picture is held to"
        )
    patch["reference"] = clock.at(frame)


def _added(
    at: At,
    composition: Mapping[str, Any],
    kind: FusionKind,
    settings: Mapping[str, Any] | None,
    label: str | None,
    pass_through: bool | None,
    *,
    after: str | None,
    mask_of: str | None,
    links: Sequence[Mapping[str, Any]] | None,
    clock: KeyClock,
    said: list[str],
) -> list[dict[str, Any]]:
    """The operation that adds a node of a kind, every setting written, with its links.

    The patterns of a tracker are given whole or one by one, each in part; its reference at a
    frame of the timeline, which ``clock`` makes a frame of the clip's keys; what a center
    follows, checked against the composition. What is to be said of it is put in ``said``.
    """
    if kind == "media_out":
        raise FusionError(
            "a composition has exactly one media_out, and it has it: none is added (the engine "
            "refuses it as one_media_out)"
        )
    if len(_nodes(composition)) >= _MOST_NODES:
        raise FusionError(
            f"a composition has {_MOST_NODES} nodes at most (the engine refuses more as "
            "out_of_range fusion_nodes)"
        )
    params = new_params(kind)
    if settings:
        patch, _, patterns = _checked(kind, "the new node", params, settings)
        _parts_put(kind, params, patch)
        if patterns:
            patch["trackers"] = _edited([], patterns, "the new node")[0]
        _framed(patch, clock)
        follow = patch.pop("center.follow", None)
        if follow is not None:
            patch["center.follow"] = _follow_checked(
                composition, follow, None, "the new node", said
            )
        for path, value in patch.items():
            _put(params, path, value)
    _whole_checked(kind, params)
    if links is not None:
        wired = _given_links(composition, kind, links)
    elif after is not None:
        wired = _after(composition, kind, after)
    elif mask_of is not None:
        wired = _masking(composition, kind, mask_of)
    else:
        wired = []
    made = edits.fusion_node(kind, params, label=label or "", pass_through=bool(pass_through))
    return [edits.add_fusion_node(at[0], at[1], made, links=wired or None)]


def _removed(
    at: At, composition: Mapping[str, Any], id: str, *, bridge: bool
) -> list[dict[str, Any]]:
    """The operation that takes a node away, joining what fed it to what it fed where asked.

    With ``bridge``, each input that the node fed is linked from the node that fed the input
    whose picture the node hands on (:attr:`Kind.hands_on`), where that one is linked; without
    it, or where it is not, what the node fed is left without a picture. A tracker that a
    center follows is not taken away.
    """
    target = _node_of(composition, id)
    if target.get("kind") == "media_out":
        raise FusionError(
            "the media_out is not taken away: a composition has exactly one (the engine refuses "
            "it as one_media_out); action reset takes the whole composition away"
        )
    followers = [follower for follower, _ in _followers(composition, id)]
    if followers:
        who = ", ".join(followers)
        raise FusionError(
            f"the center of {who} follows {id}, which is not taken away while it does (the "
            f'engine refuses it as in_use): set {{"center.follow": null}} on {who} first'
        )
    known = KINDS.get(str(target.get("kind")))
    first = None if known is None else known.hands_on
    up = _into(composition).get((id, first)) if bridge and first is not None else None
    wired = (
        [
            edits.fusion_link(up, str(link.get("to")), str(link.get("input")))
            for link in _links(composition)
            if link.get("from") == id
        ]
        if up is not None
        else []
    )
    return [edits.remove_fusion_node(at[0], at[1], id, links=wired or None)]


def _connected(
    at: At, composition: Mapping[str, Any], source: str, to: str, name: str
) -> list[dict[str, Any]]:
    """The operation that links the output of a node into an input of another."""
    _output_checked(_node_of(composition, source))
    _input_checked(_node_of(composition, to), name)
    if source == to or to in _feeding(composition, source):
        raise FusionError(
            f"{source} takes its picture through {to}: the link would close a loop (the engine "
            "refuses it as loop)"
        )
    if _into(composition).get((to, name)) == source:
        raise FusionError(
            f"{source} is linked into {to}.{name} already: nothing changes (the engine refuses "
            "it as empty)"
        )
    return [edits.connect_fusion(at[0], at[1], edits.fusion_link(source, to, name))]


def _disconnected(
    at: At, composition: Mapping[str, Any], to: str, name: str
) -> list[dict[str, Any]]:
    """The operation that takes the link into an input of a node away."""
    _input_checked(_node_of(composition, to), name)
    into = _into(composition)
    if (to, name) not in into:
        linked = ", ".join(f"{one} from {into[(node, one)]}" for node, one in into if node == to)
        raise FusionError(
            f"nothing is linked into {to}.{name}: {to} has {linked or 'no links into it'} (the "
            "engine refuses it as missing link)"
        )
    return [edits.disconnect_fusion(at[0], at[1], to, name)]


def _planar_changed(
    id: str, now: Mapping[str, Any], changed: dict[str, Any], said: list[str]
) -> None:
    """Make what changes of a Planar Tracker as the engine takes it, and say what it means.

    Its path is put into its path frame by frame: it is cleared by each frame that it has given
    ``null``. A path that is kept was tracked with the region, the motion and the reference as
    they were.
    """
    path = _object(now.get("path"))
    if "path" in changed:
        changed["path"] = dict.fromkeys(path)
        said.append(f"the path of node {id} is cleared: video_fusion_track tracks it again")
    elif path and {"region", "motion", "reference"} & set(changed):
        said.append(
            f"the path of node {id} was tracked with the region, motion and reference as they "
            "were: video_fusion_track tracks it again, or video_undo takes this back"
        )


def _set(
    at: At,
    composition: Mapping[str, Any],
    id: str,
    settings: Mapping[str, Any] | None,
    label: str | None,
    pass_through: bool | None,
    *,
    clock: KeyClock,
    said: list[str],
) -> list[dict[str, Any]]:
    """The operations that patch the settings of a node, its label or its Pass Through.

    Only what would change is sent. A number of a point of a polygon that is named by itself
    is put by ``set_fusion_keys``, the list of the points being a list that a patch puts whole.
    The patterns of a tracker that are changed one by one are put into the list of its
    patterns, which is sent whole, every other pattern and every path as it was; where one is
    taken away, the centers that follow a later one are kept on it in the same transaction.
    The reference of a tracker is given at a frame of the timeline, which ``clock`` makes a
    frame of the clip's keys. What is to be said of the change is put in ``said``.

    Raises:
        FusionError: A setting is wrong; one that has keys would lose them; a whole list of
            patterns would lose their paths; or nothing would change, which the engine refuses.
    """
    target = _node_of(composition, id)
    kind = str(target.get("kind"))
    now = _object(target.get("params"))
    patch: dict[str, Any] = {}
    points: dict[str, Any] = {}
    kept: list[dict[str, Any]] = []
    if settings:
        known = _kind_of(target).settings
        patch, points, patterns = _checked(kind, f"node {id}", now, settings)
        _parts_put(kind, now, patch)
        keyed = [
            path
            for path in patch
            if isinstance(known.get(path), Setting) and _is_map(_at(now, path))
        ]
        keyed.extend(path for path in points if _is_map(_at(now, path)))
        if "points" in patch and _points_keyed(now):
            keyed.append("points")
        if keyed:
            raise FusionError(
                f"{', '.join(keyed)} of node {id} {'has' if len(keyed) == 1 else 'have'} keys, "
                "which a number would take away: video_fusion_keys puts a key at a frame of the "
                "timeline, or clears the keys to a number"
            )
        if "trackers" in patch and _tracked(now):
            raise FusionError(
                f"patterns of node {id} have paths, which a whole list of trackers would take "
                f"away: {_ONE_PATTERN}"
            )
        if patterns:
            trackers = _items(now.get("trackers"))
            patch["trackers"], removed = _edited(trackers, patterns, f"node {id}")
            if removed is not None:
                kept = _repointed(at, composition, id, removed)
                pivot = now.get("pivot_tracker")
                if "pivot_tracker" not in patch and isinstance(pivot, int) and pivot > removed:
                    patch["pivot_tracker"] = pivot - 1
        _framed(patch, clock)
        follow = patch.get("center.follow")
        if follow is not None:
            patch["center.follow"] = _follow_checked(
                composition, follow, _at(now, "center.follow"), f"node {id}", said
            )
        merged = copy.deepcopy(now)
        for path, value in patch.items():
            _put(merged, path, value)
        _whole_checked(kind, merged)
        following = _object(_at(merged, "center.follow")).get("node")
        if following is not None and {"center.x", "center.y"} & set(patch):
            said.append(
                f"the center of node {id} follows {following}: its x and y are kept, and read "
                "only where nothing is published"
            )
    changed = {path: value for path, value in patch.items() if _at(now, path) != value}
    if kind == "planar_tracker":
        _planar_changed(id, now, changed, said)
    moved = {f"{id}.{path}": value for path, value in points.items() if _at(now, path) != value}
    named = label if label is not None and label != str(target.get("label", "")) else None
    passes = bool(target.get("pass_through", False))
    passed = pass_through if pass_through is not None and pass_through != passes else None
    if not changed and not moved and named is None and passed is None:
        raise FusionError(
            f"every value given is what node {id} has already: nothing changes (the engine "
            "refuses it as empty)"
        )
    ops: list[dict[str, Any]] = []
    if changed or named is not None or passed is not None:
        nested: dict[str, Any] = {}
        for path, value in changed.items():
            _put(nested, path, value)
        ops.append(
            edits.set_fusion_node(
                at[0], at[1], id, label=named, pass_through=passed, params=nested or None
            )
        )
    ops.extend(edits.set_fusion_keys(at[0], at[1], path, value) for path, value in moved.items())
    ops.extend(kept)
    return ops


_TAKES: dict[str, tuple[str, ...]] = {
    "add": ("kind", "settings", "label", "pass_through", "after", "mask_of", "links"),
    "remove": ("node",),
    "connect": ("from", "node", "input"),
    "disconnect": ("node", "input"),
    "set": ("node", "settings", "label", "pass_through"),
    "reset": (),
}
"""What each action of ``video_fusion_node`` takes, besides the clip."""

_NEEDS: dict[str, tuple[str, ...]] = {
    "add": ("kind",),
    "remove": ("node",),
    "connect": ("from", "node", "input"),
    "disconnect": ("node", "input"),
    "set": ("node",),
    "reset": (),
}
"""What each action cannot do without."""


def _asked(action: str, given: Sequence[str]) -> None:
    """Check that an action is given what it needs, and nothing that is for another.

    Raises:
        FusionError: Something is missing, or is not for the action.
    """
    takes = ", ".join(_TAKES[action]) or "the clip alone"
    missing = [name for name in _NEEDS[action] if name not in given]
    if missing:
        raise FusionError(f"{action} needs {', '.join(missing)}; it takes {takes}")
    other = [name for name in given if name not in _TAKES[action]]
    if other:
        verb = "is" if len(other) == 1 else "are"
        raise FusionError(f"{', '.join(other)} {verb} not for {action}, which takes {takes}")
    if action == "add" and len({"after", "mask_of", "links"} & set(given)) > 1:
        raise FusionError("give one of after, mask_of and links: each says how the node is linked")
    if action == "set" and not {"settings", "label", "pass_through"} & set(given):
        raise FusionError("give settings, label or pass_through to change")


# ---------------------------------------------------------------------------------------------
# What the tools do
# ---------------------------------------------------------------------------------------------


async def _sent(engine: Engine, build: Build) -> tuple[Applied, dict[str, Any]]:
    """Send what ``build`` works out, as :func:`columbina.video.grading.applied` sends it."""
    try:
        return await applied(engine, build)
    except GradeError as error:
        raise FusionError(str(error)) from None


async def info(engine: Engine, timeline: str | None, clip: str | None) -> dict[str, Any]:
    """Tell the composition of a clip, or which clips of a timeline have one.

    Returns:
        Of a clip: its timeline and id, whether it has a composition (``composition``), and the
        composition as :func:`summary` tells it, with what the frames of its keys count
        (``key_frames``); of a clip without one, that it shows its source (``shows``). Without
        ``clip``: the clips of the timeline that have a composition, each with its track, its
        first frame and the kinds of its nodes.

    Raises:
        FusionError: There is no such timeline or clip, or the clip is one of sound.
    """
    _, project = await engine.project()
    if clip is not None:
        return _state(project, timeline, clip)
    on = _timeline(project, timeline)
    listed: list[dict[str, Any]] = []
    for track in map(_object, _items(on.get("tracks"))):
        for one in map(_object, _items(track.get("clips"))):
            own = one.get("fusion")
            if track.get("kind") == "video" and _is_map(own):
                kinds = [str(each.get("kind")) for each in _nodes(_object(own))]
                listed.append(
                    {
                        "clip": one.get("id"),
                        "track": track.get("id"),
                        "start": one.get("start"),
                        "kinds": kinds,
                    }
                )
    return {"timeline": on["id"], "compositions": listed}


async def node(
    engine: Engine,
    timeline: str | None,
    clip: str,
    action: NodeAction,
    *,
    node: str | None = None,
    kind: FusionKind | None = None,
    settings: Mapping[str, Any] | None = None,
    label: str | None = None,
    pass_through: bool | None = None,
    after: str | None = None,
    mask_of: str | None = None,
    links: Sequence[Mapping[str, Any]] | None = None,
    source: str | None = None,
    input: str | None = None,
    bridge: bool = True,
) -> dict[str, Any]:
    """Change the graph of the composition of a clip, one action a call and a transaction.

    ``add`` a node of ``kind``, every setting written and ``settings`` laid over those of a new
    node: after the node ``after``, laid into the chain there; a mask into the ``mask`` of the
    node ``mask_of``; with ``links`` as they are given, the new node being ``""`` in them; or
    linked to nothing. ``remove`` the node ``node``, what it fed joined to what fed it with
    ``bridge``. ``connect`` the output of ``source`` into ``input`` of ``node``, in the place of
    a link that the input has; ``disconnect`` the link into ``input`` of ``node``. ``set`` a
    patch of the settings of ``node``, its ``label`` or its ``pass_through``. ``reset`` takes
    the composition away, and the clip shows its source again.

    A clip that has no composition is given the first one in the same transaction, by every
    action but ``reset``.

    Returns:
        The revision made, how many operations it was, the id of a node that was added
        (``added``), what is to be said of the change (``notes``, where there is something),
        and the composition after it as :func:`info` tells it.

    Raises:
        FusionError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the change.
    """
    given = {
        "node": node,
        "kind": kind,
        "settings": settings or None,
        "label": label,
        "pass_through": pass_through,
        "after": after,
        "mask_of": mask_of,
        "links": links,
        "from": source,
        "input": input,
    }
    _asked(action, [name for name, value in given.items() if value is not None])
    if label is not None and len(label) > _LONGEST_LABEL:
        raise FusionError(
            f"a label is at most {_LONGEST_LABEL} characters (the engine refuses a longer one "
            "as out_of_range label)"
        )
    wrote: list[str] = []
    said: list[str] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = _timeline(project, timeline)
        found = _picture_clip(on, clip)
        at = (str(on["id"]), clip)
        wrote[:] = [at[0]]
        said.clear()
        own = found.get("fusion")
        if action == "reset":
            if not _is_map(own):
                raise FusionError(
                    f"clip {clip} has no composition to take away: it shows its source (the "
                    "engine refuses it as empty)"
                )
            return [edits.set_fusion(at[0], at[1], None)]
        composition = _object(own) if _is_map(own) else edits.first_composition()
        clock = _clock(project, on, found)
        if action == "add":
            assert kind is not None
            ops = _added(
                at,
                composition,
                kind,
                settings,
                label,
                pass_through,
                after=after,
                mask_of=mask_of,
                links=links,
                clock=clock,
                said=said,
            )
        elif action == "remove":
            assert node is not None
            ops = _removed(at, composition, node, bridge=bridge)
        elif action == "connect":
            assert source is not None
            assert node is not None
            assert input is not None
            ops = _connected(at, composition, source, node, input)
        elif action == "disconnect":
            assert node is not None
            assert input is not None
            ops = _disconnected(at, composition, node, input)
        else:
            assert node is not None
            ops = _set(at, composition, node, settings, label, pass_through, clock=clock, said=said)
        # The engine gives a clip no composition of its own accord: the first one goes before
        # the first change of a clip that has none, as the Fusion page sends it.
        return ops if _is_map(own) else [edits.set_fusion(at[0], at[1], composition), *ops]

    done, project = await _sent(engine, build)
    told: dict[str, Any] = {"revision": done.revision, "operations": len(done.done)}
    added = [
        str(_object(op.get("node")).get("id"))
        for op in done.done
        if op.get("op") == "add_fusion_node"
    ]
    if added:
        told["added"] = added[0]
    if said:
        told["notes"] = list(said)
    try:
        told.update(_state(project, wrote[0], clip))
    except FusionError:  # Another client may have taken the clip away meanwhile.
        told.update(timeline=wrote[0], clip=clip)
    return told


def _keyed(node: Mapping[str, Any], setting: str) -> tuple[Setting, Any, str]:
    """The setting of a node at a path, which takes keys.

    Returns:
        What the setting takes, what it is now, and the name that the engine refuses a value
        out of its range by.

    Raises:
        FusionError: The node has no such setting, or it is one that takes no keys.
    """
    kind = str(node.get("kind"))
    of = _kind_of(node)
    known = of.settings
    if kind in ("tracker", "planar_tracker"):
        paths = "paths" if kind == "tracker" else "path"
        raise FusionError(
            f"{setting} of {_a(kind)} takes no keys: nothing of {_a(kind)} does, its {paths}, "
            "filled by tracking on the Fusion page, being what moves; video_fusion_node with "
            "action set sets its settings"
        )
    params = _object(node.get("params"))
    spec = known.get(setting)
    part = setting.split(".")[0]
    if isinstance(spec, Setting) and part in of.parts and not _is_map(params.get(part)):
        raise FusionError(
            f"node {node.get('id')} has no {part}: video_fusion_node with action set and "
            f'settings {{"{part}": {{}}}} adds one as a new one is, and then its keys are put'
        )
    if isinstance(spec, Setting):
        return spec, _at(params, setting), f"{kind}.{setting}"
    if isinstance(spec, PointList):
        raise FusionError(
            "points is a list, which takes no keys: name a number of a point, points.0.x "
            "(from 0); video_fusion_node with action set puts the list whole"
        )
    if spec is not None:
        raise FusionError(
            f"{setting} ({spec.name}) takes no keys: only a setting that is a number does; "
            "video_fusion_node with action set sets it"
        )
    point = _point_of(kind, setting)
    if point is None:
        raise _unknown(kind, setting)
    place, axis = point
    points = _items(params.get("points"))
    if place >= len(points):
        raise FusionError(
            f"node {node.get('id')} has {len(points)} points, numbered from 0: there is no "
            f"{setting}"
        )
    return POINT[axis], _object(points[place]).get(axis), f"polygon.points.{axis}"


def _where(found: Sequence[Mapping[str, Any]], clock: KeyClock) -> str:
    """Where the keys of a setting are, for whoever named a frame that has none."""
    if not found:
        return "it has no keys"
    places = [(clock.shown(int(key["frame"])), key["frame"]) for key in found]
    seen = ", ".join(str(shown) for shown, _ in places if shown is not None)
    unseen = ", ".join(str(frame) for shown, frame in places if shown is None)
    said = [f"its keys are at frames {seen} of the timeline"] if seen else []
    if unseen:
        said.append(
            f"its keys at frames {unseen} of {clock.counts} are shown at no frame of the clip "
            "(video_edit's splice_fusion_keys takes one away by its place among the keys)"
        )
    return "; ".join(said)


def _key_ops(timeline: str, clip: str, path: str) -> KeyOps:
    """The operations that write the keys of the setting at ``path`` of a node of a clip."""
    return KeyOps(
        number=lambda param: edits.set_fusion_keys(timeline, clip, path, param),
        splice=lambda at, remove, put: edits.splice_fusion_keys(
            timeline, clip, path, at, remove, [edits.Key.model_validate(key) for key in put]
        ),
    )


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
    """Put a key of one number of a node of a composition, take one away, or clear them all.

    ``path`` is the id of the node, a dot, and the path of the setting, ``n4.center.x``; a
    number of a point of a polygon is ``n9.points.2.x``. ``frame`` is a frame of the timeline,
    within the clip; the key is at the frame of the clip's keys that the clip is at there
    (:class:`columbina.video.clip_keys.KeyClock`). The other keys of the setting are not sent:
    a key put at a frame that has one takes its place, and keeps its interpolation where none
    is given; the last key taken away leaves its value as a number; ``clear`` leaves ``value``,
    or the value of the first key.

    Returns:
        The revision made; ``key``, the key that was put or taken away, by the frame of the
        timeline that was given and its own frame; what its own frame counts (``key_frames``);
        and the setting as it is now, each of its keys with the first frame of the timeline
        that shows it (``timeline_frame``, ``None`` where none does).

    Raises:
        FusionError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the change.
    """
    id, _, setting = path.partition(".")
    if not id or not setting:
        raise FusionError(
            f"{path!r} is not the path of a setting: the id of the node, a dot, and the "
            "setting, as n4.center.x; video_fusion_info shows them"
        )
    if action != "clear" and frame is None:
        raise FusionError(f"{action} needs frame, the frame of the timeline that the key is at")
    wrote: list[tuple[str, int | None]] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = _timeline(project, timeline)
        found = _picture_clip(on, clip)
        own = found.get("fusion")
        if not _is_map(own):
            raise FusionError(
                f"clip {clip} has no composition, and so no node {id}: video_fusion_node adds "
                "nodes (the engine refuses it as missing fusion)"
            )
        spec, now, what = _keyed(_node_of(_object(own), id), setting)
        if action == "put" and value is None:
            raise FusionError(f"put needs value, what {path} is at the key ({spec.told()})")
        if action != "remove" and value is not None and not spec.low <= value <= spec.high:
            raise FusionError(
                f"{path} is {value:g}, out of range ({spec.told()}); the engine refuses it as "
                f"out_of_range {what}"
            )
        clock = _clock(project, on, found)
        at = None
        if action != "clear" and frame is not None:
            last = clock.start + clock.length - 1
            if not clock.start <= frame <= last:
                raise FusionError(
                    f"frame {frame} is not in clip {clip}, which is at frames {clock.start} to "
                    f"{last} of timeline {on['id']}; frame is a frame of the timeline"
                )
            at = clock.at(frame)
            if action == "remove" and all(key.get("frame") != at for key in _keys_of(now)):
                raise FusionError(
                    f"{path} of clip {clip} has no key at frame {frame} of the timeline (frame "
                    f"{at} of {clock.counts}): {_where(_keys_of(now), clock)}"
                )
        wrote[:] = [(str(on["id"]), at)]
        ops = _key_ops(str(on["id"]), clip, path)
        return [key_op(ops, path, (spec, now), action, frame=at, value=value, interp=interp)]

    done, project = await _sent(engine, build)
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
    try:
        after = _timeline(project, on)
        found = _picture_clip(after, clip)
        _, now, _ = _keyed(_node_of(_object(found.get("fusion")), id), setting)
    except FusionError:  # Another client may have changed the clip meanwhile.
        return told
    clock = _clock(project, after, found)
    told["key_frames"] = f"frames of {clock.counts}, from 0"
    told["setting"] = _keys_told(now, clock)
    return told


# ---------------------------------------------------------------------------------------------
# Tracking
# ---------------------------------------------------------------------------------------------

_TRACK_SECONDS = (30.0, 0.25)
"""How long a track is waited for: a time of its own, and a time for each frame that it goes
through. One that takes longer is stopped, and what it tracked is kept."""

_TRACK_STOP_SECONDS = 5.0
"""How long a track that was stopped may take to tell how it ended."""

_MOST_LOST = 4
"""How many of the frames at which a pattern was lost are told."""

_BACKWARD: tuple[TrackDirection, ...] = ("reverse", "one_reverse")
"""The directions that start from the last frame of the clip where no frame is given."""

_ENDED = {
    "stale_revision": (
        "the project was changed while it tracked (by another client, an undo or a redo): what "
        "it wrote before stays, as its own undo step"
    ),
    "stale_gesture": "the project was changed while it tracked: what it wrote before stays",
    "disconnected": "the connection to the engine went",
    "target": "its clip or its tracker was taken away while it tracked",
    "tracker": "a pattern that it followed was taken away while it tracked",
    "cannot_read": "the media of the clip cannot be read",
    "missing_frame": "a frame of the media could not be decoded: what was found before it is kept",
    "no_features": (
        "the region holds nothing that can be followed at the frame that it started from, and "
        "nothing was written: draw it over a flat surface with texture (settings region)"
    ),
}
"""Why a track ended before its end, in words, by the reason that the engine gives."""

_PLANE_LOST = (
    "the plane was lost at some frames, which were not written, and the track went on: a region "
    "on a flat surface with texture, away from what passes over it, holds better"
)
_PLANE_NOTHING = (
    "nothing was written: the path had each frame as it was found, or none was found beyond the "
    "frame that it started from"
)
_PLANE_NEXT = (
    "video_undo takes the whole track back. To hold the plane still: settings operation steady. "
    "To lay a picture on it: link the picture into its foreground, operation corner_pin, and "
    "corners where the picture's corners go at the reference frame"
)

_LOST = (
    "a pattern lost is followed no further that way: a larger search area "
    "(trackers.N.search.width, height) or a pattern on a feature with more texture may hold it; "
    "track again from a frame before"
)
_NOTHING = (
    "no pattern was found beyond the frame that it started from, and nothing was written: put "
    "the pattern within the picture, on a feature with texture (trackers.N.pattern.x, y)"
)
_NEXT = (
    "video_undo takes the whole track back. To stabilize: settings operation match_move, merge "
    'bg_only. To have a mask or a merge follow pattern N: its {"center.follow": {"node": the '
    'tracker, "output": "tracker", "tracker": N}}'
)


def _int_of(value: object) -> int | None:
    """A whole number as the engine wrote it, or none where it is not one."""
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _untrackable(project: Mapping[str, Any], clip: Mapping[str, Any]) -> str | None:
    """What a clip is that has no moving picture of media to track; none where it has one."""
    source = _object(clip.get("source"))
    kind = source.get("kind")
    if kind in ("title", "color"):
        return "a title" if kind == "title" else "a colour"
    named = source.get("media")
    media = next(
        (one for one in map(_object, _items(project.get("media"))) if one.get("id") == named),
        None,
    )
    if media is None:
        return "of media that the project does not have"
    video = _object(_object(media.get("facts")).get("video"))
    if not video:
        return "of media without a picture"
    return "a still" if video.get("still") else None


def _followed_checked(
    node: str, patterns: Sequence[Mapping[str, Any]], named: Sequence[int] | None
) -> None:
    """Check that a track of a tracker has patterns to follow, as the engine checks it.

    Those that are named are each to be there and enabled; where none are named, one at least
    is to be enabled.

    Raises:
        FusionError: A pattern that is named is not there or is off, or there is none to follow.
    """
    if not patterns:
        raise FusionError(
            f'node {node} has no pattern to follow: video_fusion_node set {{"trackers.+": '
            '{"pattern": {"x": 0.4, "y": 0.6}}} adds one, a place from the lower left of the '
            "picture (the engine refuses it as no_trackers)"
        )
    enabled = [at for at, one in enumerate(patterns) if one.get("enabled", True) is not False]
    on = ", ".join(map(str, enabled)) or "none"
    if named is None:
        if not enabled:
            raise FusionError(
                f"every pattern of node {node} is off: trackers.N.enabled true turns one on "
                "(the engine refuses it as no_trackers)"
            )
        return
    for at in named:
        if at >= len(patterns):
            raise FusionError(
                f"node {node} has {len(patterns)} patterns, numbered from 0: there is no pattern "
                f"{at} (enabled: {on}; the engine refuses it as tracker)"
            )
        if at not in enabled:
            raise FusionError(
                f"pattern {at} of node {node} is off: trackers.{at}.enabled true turns it on "
                f"(enabled: {on}; the engine refuses it as tracker)"
            )


def _steps(direction: str, frame: int, first: int, last: int) -> int:
    """How many frames a track goes through, the way that it goes in the source from ``frame``."""
    if direction == "forward":
        return max(last - frame, 0)
    if direction == "reverse":
        return max(frame - first, 0)
    if direction == "forward_and_back":
        return max(last - first, 0)
    return 1


def _shown_near(clock: KeyClock, key: int) -> int:
    """The first frame of the timeline that shows the frame ``key`` of the clip's keys.

    Where none does (a frame that a clip faster than its media passes over), the frame of the
    timeline that shows the frame nearest to it.
    """
    shown = clock.shown(key)
    if shown is not None:
        return shown
    frames = range(clock.start, clock.start + max(clock.length, 1))
    return min(frames, key=lambda at: abs(clock.at(at) - key))


def _patterns_told(
    end: Mapping[str, Any], patterns: Sequence[Mapping[str, Any]], clock: KeyClock
) -> list[dict[str, Any]]:
    """Each pattern that a track followed, as ``track.state`` tells it, at frames of the timeline.

    Its place, its name, at how many frames it was found (``frames_found``, the one that it
    started from not counted), and, where it was lost, the first frames of the timeline that
    show the frames at which it was (``lost_at``, one each way at most).
    """
    told: list[dict[str, Any]] = []
    for one in map(_object, _items(end.get("trackers"))):
        at = _int_of(one.get("tracker"))
        if at is None:
            continue
        item: dict[str, Any] = {
            "index": at,
            "name": patterns[at].get("name", "") if 0 <= at < len(patterns) else "",
            "frames_found": _int_of(one.get("frames")),
        }
        lost = [frame for frame in map(_int_of, _items(one.get("lost"))) if frame is not None]
        if lost:
            item["lost_at"] = [_shown_near(clock, frame) for frame in lost[:_MOST_LOST]]
        told.append(item)
    return told


def _end_told(
    end: Mapping[str, Any], clock: KeyClock, late: float | None
) -> tuple[dict[str, Any], list[str]]:
    """What any track tells of how it ended, at frames of the timeline, and why it ended early.

    Its ``phase``, its ``reason`` (``timeout`` where it was stopped for taking longer than
    ``late`` seconds), its ``message``, the first and the last frame of the timeline that show
    a frame written (``tracked``) and how many frames it went through (``frames``).
    """
    phase = str(end.get("phase"))
    reason = "timeout" if late is not None else end.get("reason")
    told: dict[str, Any] = {"phase": phase}
    if reason is not None:
        told["reason"] = reason
    message = end.get("message")
    if isinstance(message, str):
        told["message"] = message
    span = [one for one in map(_int_of, _items(end.get("tracked"))) if one is not None]
    told["tracked"] = sorted(_shown_near(clock, one) for one in span) or None
    told["frames"] = _int_of(end.get("frames"))
    notes: list[str] = []
    reached = _int_of(end.get("frame"))
    if late is not None:
        there = "" if reached is None else f" at frame {_shown_near(clock, reached)}"
        notes.append(
            f"it took longer than {late:g} s and was stopped{there}; what it tracked is kept: "
            "track again from there to go on"
        )
    elif phase != "done" and str(reason) in _ENDED:
        notes.append(_ENDED[str(reason)])
    return told, notes


def _ended_told(
    end: Mapping[str, Any],
    patterns: Sequence[Mapping[str, Any]],
    clock: KeyClock,
    late: float | None,
) -> dict[str, Any]:
    """How a track of a tracker ended, as ``track.state`` tells it, at frames of the timeline.

    ``late`` is how long it was waited for, where it was stopped for taking longer.
    """
    told, notes = _end_told(end, clock, late)
    followed = _patterns_told(end, patterns, clock)
    told["patterns"] = followed
    if any("lost_at" in item for item in followed):
        notes.append(_LOST)
    if any((item["frames_found"] or 0) > 0 for item in followed):
        notes.append(_NEXT)
    elif told["phase"] == "done":
        notes.append(_NOTHING)
    if notes:
        told["notes"] = notes
    return told


def _plane_told(end: Mapping[str, Any], clock: KeyClock, late: float | None) -> dict[str, Any]:
    """How a track of a planar tracker ended, as ``track.state`` tells it.

    As any track's end, with the frames at which the plane was lost (``lost``), which were not
    written.
    """
    told, notes = _end_told(end, clock, late)
    told["lost"] = _int_of(end.get("lost")) or 0
    if told["lost"]:
        notes.append(_PLANE_LOST)
    if told["tracked"] is not None:
        notes.append(_PLANE_NEXT)
    elif told["phase"] == "done":
        notes.append(_PLANE_NOTHING)
    if notes:
        told["notes"] = notes
    return told


def _region_checked(node: str, params: Mapping[str, Any]) -> None:
    """Check that a planar tracker has a region to follow, as far as the project tells it.

    Raises:
        FusionError: Its region has fewer than three places.
    """
    count = len(_items(params.get("region")))
    if count < 3:
        raise FusionError(
            f"the region of node {node} has {count} places, and a plane is followed within three "
            "at least: video_fusion_node set region (the engine refuses it as region)"
        )


def _planar_start(params: Mapping[str, Any], clock: KeyClock) -> int | None:
    """The frame of the timeline that a track of a planar tracker starts from by default.

    The first that shows its reference frame, where the region was drawn; ``None`` where the
    clip shows it nowhere.
    """
    reference = _int_of(params.get("reference"))
    return None if reference is None else clock.shown(reference)


async def _waited(tracking: Tracking, seconds: float) -> tuple[dict[str, Any] | None, bool]:
    """How a track ended, waited for ``seconds`` at most, and whether it took longer.

    One that takes longer is stopped between frames, keeping what it tracked, and how it ended
    then is waited for a while more; one whose wait is given up on, as when the call is
    cancelled, is stopped too (:meth:`columbina.video.engine.Tracking.waited`).

    Returns:
        What ``track.state`` told, or ``None`` where a track that was stopped told nothing in
        time; and whether it was stopped for taking longer than ``seconds``.
    """
    return await tracking.waited(seconds, stop_seconds=_TRACK_STOP_SECONDS)


async def track(
    engine: Engine,
    timeline: str | None,
    clip: str,
    node: str,
    *,
    trackers: Sequence[int] | None = None,
    frame: int | None = None,
    direction: TrackDirection = "forward",
) -> dict[str, Any]:
    """Track a tracker or a planar tracker of a clip's composition through the frames of the clip.

    Of a tracker, the engine follows each pattern from where it is at ``frame``, a frame of the
    timeline within the clip (its first by default, its last for a track that goes in reverse),
    the way that ``direction`` goes on the timeline, and writes the places that it finds into
    the paths of the patterns as it goes, one step of the history in all (PROTOCOL, The
    patterns of a Tracker). ``trackers`` names the patterns to follow by their places from 0:
    every one that is enabled, where it is left out. Of a planar tracker, it follows the region
    as a plane, and writes the homography of the plane at each frame into its path (The region
    of a Planar Tracker); a track starts by default at the frame that shows its reference frame,
    where the region was drawn, if the clip shows it. The frame is sent as the frame of the
    clip's keys that the clip is at there (:class:`columbina.video.clip_keys.KeyClock`). What
    can be told from the project is checked before anything is sent.

    The track is waited for as long as :data:`_TRACK_SECONDS` gives for the frames that it goes
    through; one that takes longer is stopped, and what it tracked is kept.

    Returns:
        The frame that it started from (``from``), by the frame of the timeline and its own;
        how it ended (``phase``: ``done``, ``stopped`` or ``failed``, with a ``reason`` but
        where it is done, ``timeout`` where it took too long); the first and the last frame of
        the timeline that show a frame written into a path (``tracked``); how many frames it
        went through (``frames``); of a tracker, each pattern followed with its name, at how
        many frames it was found and the frames of the timeline at which it was lost; of a
        planar tracker, at how many frames the plane was lost (``lost``); and what may be done
        next (``notes``).

    Raises:
        FusionError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the track, or the connection to it was lost.
    """
    if trackers is not None and (not trackers or min(trackers) < 0):
        raise FusionError(
            "trackers is a list of the places of patterns, from 0: leave it out to follow every "
            "pattern that is enabled"
        )
    revision, project = await engine.project()
    session = engine.session
    on = _timeline(project, timeline)
    found = _picture_clip(on, clip)
    own = found.get("fusion")
    if not _is_map(own):
        raise FusionError(
            f"clip {clip} has no composition, and so no tracker: video_fusion_node adds one "
            "(action add, kind tracker or planar_tracker, after MediaIn1)"
        )
    target = _node_of(_object(own), node)
    kind = str(target.get("kind"))
    if kind not in ("tracker", "planar_tracker"):
        raise FusionError(
            f"node {node} is {_a(kind)}: video_fusion_track follows the patterns of a tracker, "
            "or the region of a planar_tracker (the engine refuses it as wrong_node)"
        )
    what = _untrackable(project, found)
    if what is not None:
        raise FusionError(
            f"clip {clip} is {what}: a title, a colour or a still has no frames to track (the "
            "engine refuses it as untrackable)"
        )
    params = _object(target.get("params"))
    planar = kind == "planar_tracker"
    patterns: list[dict[str, Any]] = []
    if planar:
        if trackers is not None:
            raise FusionError(
                f"node {node} is a planar_tracker, which follows its region whole: trackers names "
                "the patterns of a tracker; leave it out"
            )
        _region_checked(node, params)
    else:
        patterns = [_object(one) for one in _items(params.get("trackers"))]
        _followed_checked(node, patterns, trackers)
    clock = _clock(project, on, found)
    last = clock.start + clock.length - 1
    if frame is None:
        start = _planar_start(params, clock) if planar else None
        if start is None:
            start = last if direction in _BACKWARD else clock.start
        frame = start
    if not clock.start <= frame <= last:
        raise FusionError(
            f"frame {frame} is not in clip {clip}, which is at frames {clock.start} to {last} "
            f"of timeline {on['id']}; frame is a frame of the timeline"
        )
    key = clock.at(frame)
    aim: dict[str, Any] = {"timeline": on["id"], "clip": clip, "node": node}
    if trackers is not None:
        aim["trackers"] = sorted(set(trackers))
    asked: dict[str, Any] = {
        "planar" if planar else "fusion": aim,
        "revision": revision,
        "frame": key,
        "direction": direction,
    }
    try:
        tracking = await engine.track(asked, session=session)
    except EngineError as error:
        if (error.data or {}).get("reason") != "frame":
            raise
        # The engine counts the frames of the clip's media: the clip is told as the model
        # counts it.
        raise EngineError(
            f"{error}; clip {clip} is at frames {clock.start} to {last} of the timeline, and "
            f"frame {frame} of it shows frame {key} of {clock.counts}",
            code=error.code,
            data=error.data,
        ) from None
    started = tracking.started
    first, final = _int_of(started.get("first")), _int_of(started.get("last"))
    way = str(started.get("direction", direction))
    steps = _steps(way, key, key if first is None else first, key if final is None else final)
    seconds = _TRACK_SECONDS[0] + _TRACK_SECONDS[1] * steps
    end, late = await _waited(tracking, seconds)
    told: dict[str, Any] = {
        "timeline": on["id"],
        "clip": clip,
        "node": node,
        "from": {"timeline_frame": frame, "frame": key},
        "direction": direction,
        "key_frames": f"frames of {clock.counts}, from 0",
    }
    if end is None:
        told.update(phase="stopped", reason="timeout")
        told["notes"] = [
            f"it took longer than {seconds:g} s and was stopped, and the engine told nothing "
            "more: video_fusion_info shows the frames of the paths"
        ]
        return told
    waited = seconds if late else None
    told.update(
        _plane_told(end, clock, waited) if planar else _ended_told(end, patterns, clock, waited)
    )
    return told


# ---------------------------------------------------------------------------------------------
# Animations of a Text+
# ---------------------------------------------------------------------------------------------

_RISE = -0.05
"""How far below its place a character of ``rise`` begins, in heights of the picture."""

_OVERSHOOT = 1.15
"""How large a character of ``pop`` grows before it settles at its own size."""

_ORDERS: dict[Preset, str] = {
    "fade_in": "left_to_right",
    "rise": "left_to_right",
    "pop": "inside_out",
    "fade_out": "left_to_right",
}
"""The order in which the characters go, of each preset that moves them by the follower."""

_MOST_DELAY = 10000
"""The longest delay of a follower, in frames."""

_HEX = re.compile(r"#([0-9a-fA-F]{6})([0-9a-fA-F]{2})?")

_ANIMATED = (
    "video_look shows frames of it; video_fusion_keys moves or changes one of its keys by the "
    "path <node>.<setting>, and video_undo takes the whole of it back"
)


@dataclass(frozen=True)
class _Animation:
    """What a preset puts on a Text+, worked out for its clip.

    Attributes:
        first: Its first frame, counted from the first frame of the clip, 0.
        frames: How many frames it lasts, to its last key.
        keys: The keys of each setting that it animates by its path, at frames of the clip's
            keys, in order.
        follower: The settings of the follower that it moves the characters by, or ``None``.
    """

    first: int
    frames: int
    keys: dict[str, list[dict[str, Any]]]
    follower: dict[str, Any] | None


def _clocked(clock: KeyClock, points: Sequence[tuple[int, float]]) -> list[dict[str, Any]]:
    """The keys at frames of the clip, from its first, put at the frames of the clip's keys.

    Raises:
        FusionError: Two fall on one frame of the keys, as on a clip slower than its media.
    """
    placed = sorted((clock.at(clock.start + offset), value) for offset, value in points)
    frames = [frame for frame, _ in placed]
    if len(set(frames)) < len(frames):
        raise FusionError(
            "two keys of the animation fall on one frame of the clip's keys, as on a clip slower "
            "than its media, which shows one picture for several frames: give a longer length"
        )
    return [{"frame": frame, "value": value, "interp": "linear"} for frame, value in placed]


def _animation(
    preset: Preset, clip: str, clock: KeyClock, rate: Rate, at: int | None, length: int | None
) -> _Animation:
    """The keys and the follower of a preset on a clip, from ``at`` for ``length`` frames.

    ``at`` is counted from the first frame of the clip; left out, the animation begins there, or
    ends at the last frame of the clip for ``fade_out``. ``length`` is a second of the timeline
    where it is left out, or as much of it as the clip has. A preset that moves the characters
    one by one (all but ``typewriter``) spreads their starts over the length, first to last,
    each character moving for half of it: so the last ends at the end, however long the text.

    Raises:
        FusionError: The animation does not fit in the clip, or is too short for the preset.
    """
    last = clock.length - 1
    frames = length if length is not None else min(max(2, round(rate.num / rate.den)), last)
    if frames < 1:
        raise FusionError(f"clip {clip} has {clock.length} frame: an animation needs 2 at least")
    first = at if at is not None else (last - frames if preset == "fade_out" else 0)
    if first < 0 or first + frames > last:
        raise FusionError(
            f"an animation of {frames} frames from frame {first} of clip {clip} does not fit in "
            f"it: the clip has frames 0 to {last}, and at + length is {last} at most"
        )
    each = max(1, frames // 2)
    settle = first + each
    if preset == "pop" and each < 2:
        raise FusionError("pop grows past its size and settles back: length is 4 frames at least")
    points: dict[str, list[tuple[int, float]]]
    if preset == "typewriter":
        points = {"write_on.end": [(first, 0.0), (first + frames, 1.0)]}
    elif preset == "fade_in":
        points = {"follower.opacity": [(first, 0.0), (settle, 1.0)]}
    elif preset == "rise":
        points = {
            "follower.opacity": [(first, 0.0), (settle, 1.0)],
            "follower.offset.y": [(first, _RISE), (settle, 0.0)],
        }
    elif preset == "pop":
        top = first + max(1, round(each * 0.7))
        points = {"follower.scale": [(first, 0.0), (top, _OVERSHOOT), (settle, 1.0)]}
    else:
        points = {"follower.opacity": [(first, 1.0), (settle, 0.0)]}
    keys = {path: _clocked(clock, one) for path, one in points.items()}
    follower: dict[str, Any] | None = None
    if preset != "typewriter":
        # The last character starts this many frames of the keys after the first.
        delay = abs(clock.at(clock.start + first + frames) - clock.at(clock.start + settle))
        if delay > _MOST_DELAY:
            raise FusionError(
                f"the characters start over {_MOST_DELAY} frames at most: give a shorter length"
            )
        follower = {"order": _ORDERS[preset], "delay": delay, "delay_type": "first_to_last"}
    return _Animation(first, frames, keys, follower)


def _rgba_of(color: str) -> list[float]:
    """The red, the green, the blue and the alpha of a colour ``#rrggbb[aa]``, each 0 to 1.

    Raises:
        FusionError: It is not written so.
    """
    match = _HEX.fullmatch(color)
    if match is None:
        raise FusionError(
            f"color is #rrggbb or #rrggbbaa, alpha last, as #ffcc00 for a yellow; {color!r} is not"
        )
    digits = match.group(1) + (match.group(2) or "ff")
    return [round(int(digits[at : at + 2], 16) / 255, 4) for at in range(0, 8, 2)]


def _style(font: str | None, size: float | None, color: str | None) -> dict[str, Any]:
    """The settings of a Text+ that a style given makes, by their paths, checked.

    Raises:
        FusionError: One is not one that a Text+ takes, said with the engine's refusal.
    """
    style: dict[str, Any] = {}
    if font is not None:
        style["font"] = _words(_FONT, "font", font, "text_plus.font")
    if size is not None:
        style["size"] = _number(_TEXT_SIZE, "size", size, "text_plus.size")
    if color is not None:
        channels = zip("rgba", _rgba_of(color), strict=True)
        style.update({f"fill.color.{channel}": value for channel, value in channels})
    return style


def _frame_of(key: Mapping[str, Any]) -> int:
    return int(key["frame"])


def _spliced(now: object, keys: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """The keys of a setting with ``keys`` put in among those that it had.

    Those that it had between the first frame of ``keys`` and their last are taken away, and
    the others are kept.
    """
    low, high = _frame_of(keys[0]), _frame_of(keys[-1])
    kept = [key for key in _keys_of(now) if not low <= _frame_of(key) <= high]
    return sorted([*kept, *(dict(key) for key in keys)], key=_frame_of)


def _text_added(
    at: At,
    composition: Mapping[str, Any],
    text: str,
    style: Mapping[str, Any],
    outline: bool,
    animation: _Animation,
) -> list[dict[str, Any]]:
    """The operations that lay a new Text+, animated, over the picture of a composition.

    A Merge is added in the place of what the MediaOut reads: what it read into the Merge's
    background, the Text+ into its foreground, and the Merge into the MediaOut. The Text+ is
    added whole, its text, its style and the keys of the animation within it.
    """
    nodes = _nodes(composition)
    if len(nodes) + 2 > _MOST_NODES:
        raise FusionError(
            f"a composition has {_MOST_NODES} nodes at most, and a text adds two (the engine "
            "refuses more as out_of_range fusion_nodes)"
        )
    out = next((str(one.get("id")) for one in nodes if one.get("kind") == "media_out"), None)
    if out is None:
        raise FusionError("the composition has no media_out to lay the text over the picture by")
    under = _into(composition).get((out, "input"))
    params = new_params("text_plus")
    params["text"] = text
    for path, value in style.items():
        _put(params, path, value)
    if outline:
        params["outline"] = new_part("text_plus", "outline")
    if animation.follower is not None:
        params["follower"] = {**new_part("text_plus", "follower"), **animation.follower}
    for path, keys in animation.keys.items():
        _put(params, path, {"keys": keys})
    over = [edits.fusion_link("", out, "input")]
    if under is not None:
        over.insert(0, edits.fusion_link(under, "", "background"))
    merge = edits.fusion_node("merge", new_params("merge"), id="$over")
    words = edits.fusion_node("text_plus", params, id="$text")
    into = [edits.fusion_link("", "$over", "foreground")]
    return [
        edits.add_fusion_node(at[0], at[1], merge, links=over),
        edits.add_fusion_node(at[0], at[1], words, links=into),
    ]


def _text_animated(
    at: At,
    composition: Mapping[str, Any],
    id: str,
    text: str | None,
    style: Mapping[str, Any],
    outline: bool | None,
    animation: _Animation,
    said: list[str],
) -> list[dict[str, Any]]:
    """The operation that puts an animation on a Text+ that the composition has.

    Its text and style are changed where they are given; a follower that it has not is added
    whole, and one that it has is given the order and the delay of the animation; the keys of
    each setting animated are put in among those that it has, those between their first and
    their last frame taken away. What is to be said of it is put in ``said``.

    Raises:
        FusionError: The node is not a Text+, a setting of the style has keys, or nothing would
            change.
    """
    target = _node_of(composition, id)
    if target.get("kind") != "text_plus":
        raise FusionError(
            f"node {id} is {_a(target.get('kind'))}: video_text_animate animates a text_plus; "
            "leave node out to add one over the picture"
        )
    now = _object(target.get("params"))
    patch: dict[str, Any] = dict(style)
    if text is not None:
        patch["text"] = text
    keyed = [path for path in patch if _is_map(_at(now, path))]
    if keyed:
        raise FusionError(
            f"{', '.join(keyed)} of node {id} {'has' if len(keyed) == 1 else 'have'} keys, which "
            "a style would take away: video_fusion_keys changes them"
        )
    lined = _is_map(now.get("outline"))
    if outline is not None and outline != lined:
        patch["outline"] = new_part("text_plus", "outline") if outline else None
    had = now.get("follower")
    if animation.follower is not None and not _is_map(had):
        patch["follower"] = {**new_part("text_plus", "follower"), **animation.follower}
    elif animation.follower is not None:
        was = _object(had)
        moved = {key: value for key, value in animation.follower.items() if was.get(key) != value}
        patch.update({f"follower.{key}": value for key, value in moved.items()})
        if moved:
            before = ", ".join(f"{key} {was.get(key)}" for key in moved)
            after = ", ".join(f"{key} {value}" for key, value in moved.items())
            said.append(
                f"the follower of node {id} had {before}, and now {after}: whatever else of it "
                "has keys goes in that order too"
            )
    for path, keys in animation.keys.items():
        if path.startswith("follower.") and "follower" in patch:
            _put(patch["follower"], path.removeprefix("follower."), {"keys": keys})
        else:
            patch[path] = {"keys": _spliced(_at(now, path), keys)}
    changed = {path: value for path, value in patch.items() if _at(now, path) != value}
    if not changed:
        raise FusionError(
            f"node {id} has this animation already: nothing changes (the engine refuses it as "
            "empty)"
        )
    nested: dict[str, Any] = {}
    for path, value in changed.items():
        _put(nested, path, value)
    return [edits.set_fusion_node(at[0], at[1], id, params=nested)]


async def animate(
    engine: Engine,
    timeline: str | None,
    clip: str,
    preset: Preset,
    *,
    node: str | None = None,
    text: str | None = None,
    at: int | None = None,
    length: int | None = None,
    font: str | None = None,
    size: float | None = None,
    color: str | None = None,
    outline: bool | None = None,
) -> dict[str, Any]:
    """Put a ready-made animation on a Text+ of a clip's composition, one transaction and step.

    Without ``node``, a Text+ of ``text`` is added over the picture with a Merge (see
    :func:`_text_added`); with it, that Text+ is animated, and its ``text`` changed where it is
    given. ``font``, ``size`` (a share of the picture) and ``color`` (``#rrggbb[aa]``) style it,
    and ``outline`` gives it a black outline or takes it away. The preset (:data:`PRESETS`):

    - ``typewriter``: ``write_on.end`` from 0 to 1, the characters shown one after another.
    - ``fade_in``: each character fades in, from the left (the follower's opacity, 0 to 1).
    - ``rise``: each rises into its place from 0.05 of the picture below as it fades in.
    - ``pop``: each grows from nothing past its size (1.15) and settles, from the middle out.
    - ``fade_out``: each fades out, from the left, to the last frame of the clip by default.

    ``at`` is the frame where it begins, counted from the first of the clip; ``length`` how many
    frames it lasts (:func:`_animation`). The keys are put at the frames of the clip's keys that
    the clip shows there (:class:`columbina.video.clip_keys.KeyClock`).

    Returns:
        The revision made, how many operations it was, the node animated (``node``) and the
        Merge added with it (``merge``), the frames of the timeline that the animation goes
        from and to (``frames``), the keys put of each setting with the first frame of the
        timeline that shows each (``keys``), the follower's order and delay (``follower``) and
        what is to be said of it (``notes``).

    Raises:
        FusionError: What is asked is wrong, said before anything is sent.
        EngineError: The engine refused the change.
    """
    if text is not None:
        _words(_TEXT, "text", text, "text_plus.text")
    if node is None and not text:
        raise FusionError(
            "a new text_plus needs text, the words to animate; or give node, a text_plus of the "
            "composition"
        )
    if length is not None and length < 1:
        raise FusionError("length is how many frames the animation lasts, 1 at least")
    if at is not None and at < 0:
        raise FusionError("at is a frame of the clip, counted from its first, 0")
    style = _style(font, size, color)
    worked: list[tuple[str, KeyClock, _Animation]] = []
    said: list[str] = []

    def build(project: Mapping[str, Any]) -> list[dict[str, Any]]:
        on = _timeline(project, timeline)
        found = _picture_clip(on, clip)
        place = (str(on["id"]), clip)
        own = found.get("fusion")
        composition = _object(own) if _is_map(own) else edits.first_composition()
        clock = _clock(project, on, found)
        rate = Rate.model_validate(on["rate"])
        animation = _animation(preset, clip, clock, rate, at, length)
        said.clear()
        if node is None:
            ops = _text_added(place, composition, text or "", style, bool(outline), animation)
        else:
            ops = _text_animated(place, composition, node, text, style, outline, animation, said)
        worked[:] = [(place[0], clock, animation)]
        # A clip without a composition is given the first one before, as video_fusion_node does.
        return ops if _is_map(own) else [edits.set_fusion(place[0], place[1], composition), *ops]

    done, _ = await _sent(engine, build)
    ((on, clock, animation),) = worked
    added = [
        str(_object(op.get("node")).get("id"))
        for op in done.done
        if op.get("op") == "add_fusion_node"
    ]
    told: dict[str, Any] = {
        "revision": done.revision,
        "operations": len(done.done),
        "timeline": on,
        "clip": clip,
        "preset": preset,
        "node": added[-1] if added else node,
    }
    if len(added) > 1:
        told["merge"] = added[0]
    start = clock.start + animation.first
    told["frames"] = {"from": start, "to": start + animation.frames}
    told["keys"] = {
        path: [{"timeline_frame": clock.shown(_frame_of(key)), **key} for key in keys]
        for path, keys in animation.keys.items()
    }
    if animation.follower is not None:
        told["follower"] = dict(animation.follower)
    told["notes"] = [*said, _ANIMATED]
    return told


# ---------------------------------------------------------------------------------------------
# What a refusal of the engine means
# ---------------------------------------------------------------------------------------------

_RANGES = {
    "fusion_nodes": f"a composition has {_MOST_NODES} nodes at most",
    "fusion_id": "the id of a node has no dot in it",
    "label": f"a label is at most {_LONGEST_LABEL} characters",
    "links": (
        "the links of a node that is added each come from it or go into it; those of one that "
        "is removed each go into an input that it fed"
    ),
    "polygon.points": f"a polygon has {_MOST_POINTS} points at most",
    "delta_keyer.screen": "one channel of the screen colour is to be above its other two",
    "delta_keyer.low": "the lower threshold of a keyer is at most its upper one",
    "tracker.trackers": f"a tracker has {_MOST_TRACKERS} patterns at most",
    "tracker.trackers.name": f"the name of a pattern is at most {_LONGEST_LABEL} characters",
    "tracker.trackers.pattern": "a pattern's x and y are -15.5 to 16.5, its width, height 0 to 1",
    "tracker.trackers.search": "the search width and height of a pattern are 0 to 1",
    "tracker.trackers.offset": "the offset x and y of a pattern are -16 to 16",
    "tracker.trackers.path": "a path is filled by tracking: video_fusion_track",
    "tracker.pivot_point": "pivot_point.x and pivot_point.y are -15.5 to 16.5",
    "center.follow.tracker": f"a center follows a pattern from 0 to {_MOST_TRACKERS - 1}",
    "planar_tracker.region": "a region is 3 to 64 places, each x and y -15.5 to 16.5",
    "planar_tracker.corners": "the x and y of each corner of a planar_tracker are -15.5 to 16.5",
    "planar_tracker.path": "a path is filled by tracking: video_fusion_track",
    "text_plus.text": f"the text of a text_plus is {_TEXT.most} characters at most",
    "text_plus.font": f"the font of a text_plus is {_FONT.most} bytes of UTF-8 at most",
}
"""What is out of range where the engine names no setting of a node by its kind and path."""

_HINTS = {
    "missing": "video_fusion_info shows the nodes, the links and the settings that there are",
    "no_input": "video_fusion_info shows the inputs that each node has free and linked",
    "no_output": "the media_out is the end of the composition: nothing is linked from it",
    "input_linked": "an input takes one link: disconnect the one that it has first",
    "one_media_out": "a composition has exactly one media_out, neither added nor removed",
    "loop": "video_fusion_info shows how the nodes are linked: links make no loop",
    "wrong_track": "a composition is of a clip of a picture track",
    "params": "video_fusion_info shows the kind of the node and its settings",
    "out_of_list": "video_fusion_info shows the keys that the setting has",
    "empty": "every value given is the value there already",
    "in_use": 'a followed tracker stays: set {"center.follow": null} on its followers first',
    "wrong_node": "a center follows a tracker alone: video_fusion_info shows the kind of a node",
}
"""What a model can do about a refusal of a change of a composition, by its reason."""


def hint_of(error: EngineError) -> str | None:
    """What a model can do about a refusal of a change of a composition, if more is to be said."""
    data = error.data or {}
    reason, what = data.get("reason"), data.get("what")
    if reason == "out_of_range" and isinstance(what, str):
        kind, _, path = what.partition(".")
        known = KINDS.get(kind)
        spec = None if known is None else known.settings.get(path)
        if kind == "polygon" and path in ("points.x", "points.y"):
            spec = POINT[path[-1]]
        if isinstance(spec, Setting | Plain):
            return f"{path} of {_a(kind)} is {spec.told()}"
        return _RANGES.get(what)
    if reason == "missing" and what == "fusion":
        return "the clip has no composition: video_fusion_node gives it one with its first action"
    return _HINTS.get(str(reason))


_TRACK_HINTS = {
    "target": "video_fusion_info shows the clips that have a composition, and their nodes",
    "wrong_node": (
        "video_fusion_track follows the patterns of a tracker or the region of a planar_tracker: "
        "video_fusion_info shows the kind of each node"
    ),
    "region": (
        "the region cannot be followed from there: three places at least, larger than a pixel "
        "and within the picture, at a frame where the path can be undone (settings region)"
    ),
    "tracker": "video_fusion_info lists the patterns of the tracker, from 0, and which are on",
    "no_trackers": (
        "add a pattern or enable one (video_fusion_node set trackers.+ / trackers.N.enabled)"
    ),
    "untrackable": "a title, a colour or a still has no frames to track",
    "frame": "frame is a frame of the timeline within the clip, where it shows its media",
    "stale_revision": "the project changed meanwhile: call again",
    "busy": (
        "another track is running (one at a time): wait for it to end, or stop it where it was "
        "started"
    ),
}
"""What a model can do about a refusal of a track of a tracker, by its reason."""


def track_hint_of(error: EngineError) -> str | None:
    """What a model can do about a refusal of a track of a tracker, if more is to be said."""
    return _TRACK_HINTS.get(str((error.data or {}).get("reason")))
