"""Shot Match and Auto Color, worked out from the figures of frames alone.

DaVinci Resolve's Shot Match (Reference Manual 20, pp. 3047-3049) makes one or more clips look
like a target clip, and puts what it does "invisibly, as the very last adjustment to the node
that was selected", similar to a LUT. The engine keeps that adjustment as the ``shot_match`` of a
corrector (``engine/PROTOCOL.md``, Grades): on each channel ``S(gain * x ** gamma + offset)``,
the power keeping the sign of ``x``, and ``S`` the S-curve of contrast ``contrast`` about
``pivot`` that keeps 0, 1 and the values beyond them. Resolve does not publish how it finds its
adjustment; this module finds the parameters of the engine's by least squares:

- **What is matched.** The figures of a frame of each clip (:class:`Figures`): the levels below
  which 1, 5, 25, 50, 75, 95 and 99 percent of the pixels of each of R', G' and B' are, the mean
  of each channel (so their mean chroma, which is linear in them) and the mid grey, the luma of
  the three medians. Since the adjustment is a rising function of each channel, a level of the
  clip mapped by it is the same level of the clip adjusted, so the percentiles are matched
  directly; the means are matched by a quadrature over the levels.
- **Solving.** Levenberg-Marquardt over ten unknowns (the gamma, gain and offset of each channel,
  in logarithms for gamma and gain, and the contrast), with a numerical Jacobian, a small pull
  towards the neutral values that settles what the data leave open, and bounds (:data:`BOUNDS`).
  The bounds are narrower than the engine's ranges: Shot Match is "designed to do no harm to the
  image" and to "split the difference" between clips far apart (p. 3048), so what would need
  more is held at the bound, and told.
- **Pivot.** The S-curve turns about the target's mid grey, clamped to 0.18-0.82.

Resolve's Shot Match is made for Rec.709 at a gamma of 2.4 (p. 3047); the figures are taken
where the adjustment works, in the timeline's space, which is that in a project that is not
colour managed and under the SDR Rec.709 preset.

:func:`auto_color` is the A button of the Primaries palette (pp. 3045-3046), as the desktop does
it (``desktop/src/studio/color/corrector.ts``): the black and white of each channel (its levels
of 1 and 99 percent) are brought to their luma-weighted mean, into the visible Lift and Gain.

Nothing here needs the engine; numbers are plain floats, so that no library is needed.
"""

import math
import struct
import zlib
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, replace
from typing import Any

__all__ = [
    "AUTO_COLOR",
    "BOUNDS",
    "LEVELS",
    "LUMA",
    "Bounds",
    "Figures",
    "ShotMatch",
    "Solved",
    "auto_color",
    "figures",
    "figures_of_png",
    "percentile",
    "solve",
]

LEVELS = (1, 5, 25, 50, 75, 95, 99)
"""The percentiles of each channel that a frame is matched by."""

LUMA = (0.2126, 0.7152, 0.0722)
"""Rec.709's coefficients of luma, which Shot Match is made for (p. 3047)."""

_CB = 1.8556
_CR = 1.5748
"""Cb is (B' - Y') / 1.8556 and Cr is (R' - Y') / 1.5748 (Rec.709)."""

# The share of the pixels that each level stands for, by the trapezoids between the levels, the
# first and last also for the 1 percent beyond them: they add up to 1.
_WEIGHTS = (0.03, 0.12, 0.225, 0.25, 0.225, 0.12, 0.03)

_RGB = ("r", "g", "b")


@dataclass(frozen=True)
class Figures:
    """What a frame is matched by.

    Attributes:
        levels: For R', G' and B', the signal below which each of :data:`LEVELS` percent of the
            pixels are.
        means: The mean of each of R', G' and B'.
    """

    levels: tuple[tuple[float, ...], tuple[float, ...], tuple[float, ...]]
    means: tuple[float, float, float]

    @property
    def grey(self) -> float:
        """The mid grey: the luma of the medians of the three channels."""
        return sum(k * one[LEVELS.index(50)] for k, one in zip(LUMA, self.levels, strict=True))

    @property
    def chroma(self) -> tuple[float, float]:
        """The mean chroma, Cb and Cr of the means (Rec.709), each -0.5 to 0.5 in gamut."""
        return _chroma(self.means)

    def told(self) -> dict[str, Any]:
        """The figures as a tool tells them, rounded to four places."""
        return {
            "percentiles": {
                name: {
                    str(level): round(value, 4) for level, value in zip(LEVELS, one, strict=True)
                }
                for name, one in zip(("red", "green", "blue"), self.levels, strict=True)
            },
            "means": {
                name: round(value, 4)
                for name, value in zip(("red", "green", "blue"), self.means, strict=True)
            },
            "mid_grey": round(self.grey, 4),
            "chroma": [round(value, 4) for value in self.chroma],
        }


def _chroma(rgb: Sequence[float]) -> tuple[float, float]:
    y = sum(k * one for k, one in zip(LUMA, rgb, strict=True))
    return (rgb[2] - y) / _CB, (rgb[0] - y) / _CR


def _expected(levels: Sequence[float]) -> float:
    """The mean of a channel as its levels tell it."""
    return sum(w * one for w, one in zip(_WEIGHTS, levels, strict=True))


def percentile(counts: Sequence[int], share: float) -> float:
    """The signal below which ``share`` (0 to 1) of the pixels of a histogram are.

    The histogram is as ``scopes.statistics`` gives it: ``len(counts)`` bins, bin ``b`` holding
    the signals nearest ``b / (bins - 1)``. The signal is found within its bin, as if its pixels
    were spread evenly across it, which the engine's own percentiles (the middle of the bin) are
    not; it is clamped to 0-1.
    """
    bins = len(counts)
    total = sum(counts)
    if bins < 2 or total <= 0:
        return 0.0
    wanted = min(max(share, 0.0), 1.0) * total
    below = 0
    for bin, count in enumerate(counts):
        if count > 0 and below + count >= wanted:
            within = (wanted - below) / count
            return min(max((bin - 0.5 + within) / (bins - 1), 0.0), 1.0)
        below += count
    return 1.0


def figures(histogram: Mapping[str, Sequence[int]], means: Sequence[float]) -> Figures:
    """The figures of a frame, from the histograms of R', G' and B' and their means."""
    levels = tuple(
        tuple(percentile(histogram[name], level / 100) for level in LEVELS)
        for name in ("red", "green", "blue")
    )
    return Figures(
        levels=(levels[0], levels[1], levels[2]),
        means=(float(means[0]), float(means[1]), float(means[2])),
    )


def figures_of_png(png: bytes) -> Figures:
    """The figures of a picture in a PNG of 8 bits, RGB or RGBA, not interlaced (alpha unread).

    That is how the engine writes the thumbnail of a still (``gallery.thumbnail``), whose
    R'G'B' are those of the still's picture.

    Raises:
        ValueError: It is not such a PNG.
    """
    width, height, channels, rows = _png_rows(png)
    histogram = {name: [0] * 256 for name in ("red", "green", "blue")}
    sums = [0, 0, 0]
    reds, greens, blues = histogram["red"], histogram["green"], histogram["blue"]
    for row in rows:
        for x in range(0, width * channels, channels):
            r, g, b = row[x], row[x + 1], row[x + 2]
            reds[r] += 1
            greens[g] += 1
            blues[b] += 1
            sums[0] += r
            sums[1] += g
            sums[2] += b
    count = width * height * 255
    return figures(histogram, [one / count for one in sums])


def _png_rows(png: bytes) -> tuple[int, int, int, list[bytes]]:
    """The width, height, channels and rows of bytes of a PNG of 8 bits, RGB or RGBA."""
    if png[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    at = 8
    header = b""
    data = bytearray()
    while at + 8 <= len(png):
        length, kind = struct.unpack(">I4s", png[at : at + 8])
        body = png[at + 8 : at + 8 + length]
        at += 12 + length
        if kind == b"IHDR":
            header = body
        elif kind == b"IDAT":
            data += body
        elif kind == b"IEND":
            break
    if len(header) != 13:
        raise ValueError("a PNG without its header")
    width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", header)
    if depth != 8 or colour not in (2, 6) or interlace != 0:
        raise ValueError("only a PNG of 8 bits, RGB or RGBA, not interlaced, is read")
    channels = 3 if colour == 2 else 4
    stride = width * channels
    raw = zlib.decompress(bytes(data))
    if len(raw) != (stride + 1) * height:
        raise ValueError("a PNG cut short")
    rows: list[bytes] = []
    above = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind = raw[start]
        row = bytearray(raw[start + 1 : start + 1 + stride])
        if kind == 1:
            for x in range(channels, stride):
                row[x] = (row[x] + row[x - channels]) & 0xFF
        elif kind == 2:
            for x in range(stride):
                row[x] = (row[x] + above[x]) & 0xFF
        elif kind == 3:
            for x in range(stride):
                left = row[x - channels] if x >= channels else 0
                row[x] = (row[x] + ((left + above[x]) >> 1)) & 0xFF
        elif kind == 4:
            for x in range(stride):
                left = row[x - channels] if x >= channels else 0
                corner = above[x - channels] if x >= channels else 0
                guess = left + above[x] - corner
                near_left, near_up, near_corner = (
                    abs(guess - left),
                    abs(guess - above[x]),
                    abs(guess - corner),
                )
                if near_left <= near_up and near_left <= near_corner:
                    best = left
                elif near_up <= near_corner:
                    best = above[x]
                else:
                    best = corner
                row[x] = (row[x] + best) & 0xFF
        elif kind != 0:
            raise ValueError(f"a PNG with an unknown filter {kind}")
        rows.append(bytes(row))
        above = row
    return width, height, channels, rows


# ---------------------------------------------------------------------------------------------
# The adjustment
# ---------------------------------------------------------------------------------------------


def _spow(x: float, e: float) -> float:
    if e == 1.0 or x == 0.0:
        return x
    return math.copysign(abs(x) ** e, x)


def _s_contrast(x: float, k: float, pivot: float) -> float:
    """``maths::s_contrast`` of the engine: 0, 1 and the values beyond them are kept."""
    if k == 1.0 or x <= 0.0 or x >= 1.0:
        return x
    if x < pivot:
        return pivot * (x / pivot) ** k
    return 1.0 - (1.0 - pivot) * ((1.0 - x) / (1.0 - pivot)) ** k


@dataclass(frozen=True)
class ShotMatch:
    """The ``shot_match`` of a corrector, as the engine does it.

    Attributes:
        gamma: The exponent of each channel (1 changes nothing).
        gain: The gain of each channel (1).
        offset: The offset of each channel (0).
        contrast: The slope of the S-curve at its pivot (1).
        pivot: Where the S-curve turns (0.435).
    """

    gamma: tuple[float, float, float] = (1.0, 1.0, 1.0)
    gain: tuple[float, float, float] = (1.0, 1.0, 1.0)
    offset: tuple[float, float, float] = (0.0, 0.0, 0.0)
    contrast: float = 1.0
    pivot: float = 0.435

    @property
    def neutral(self) -> bool:
        """Whether it changes nothing (the engine then skips it)."""
        return (
            self.gamma == (1.0, 1.0, 1.0)
            and self.gain == (1.0, 1.0, 1.0)
            and self.offset == (0.0, 0.0, 0.0)
            and self.contrast == 1.0
        )

    def channel(self, x: float, at: int) -> float:
        """A signal of channel ``at`` (0 red, 1 green, 2 blue) through the adjustment."""
        moved = self.gain[at] * _spow(x, self.gamma[at]) + self.offset[at]
        return _s_contrast(moved, self.contrast, self.pivot)

    def undone(self, y: float, at: int) -> float:
        """The signal that the adjustment makes ``y`` of, where it rises; ``y`` where it does not.

        The adjustment rises with its input while gain and contrast are above 0, so it is
        undone by halving an interval in which it crosses ``y``.
        """
        if self.gain[at] <= 0.0 or self.contrast <= 0.0:
            return y
        low, high = -4.0, 4.0
        if not self.channel(low, at) <= y <= self.channel(high, at):
            return y
        for _ in range(80):
            middle = (low + high) / 2
            if self.channel(middle, at) < y:
                low = middle
            else:
                high = middle
        return (low + high) / 2

    def mapped(self, frame: Figures) -> Figures:
        """The figures of a frame after the adjustment, as far as they can be told from these.

        A level of a channel is the same level of it adjusted, the function rising (clamped to
        0-1 as a histogram holds signals); a mean moves as the quadrature over the levels does.
        """
        return self._moved(frame, self.channel)

    def unmapped(self, frame: Figures) -> Figures:
        """The figures of a frame before the adjustment, from those after it."""
        return self._moved(frame, self.undone)

    def _moved(self, frame: Figures, by: Callable[[float, int], float]) -> Figures:
        levels: list[tuple[float, ...]] = []
        means: list[float] = []
        for at in range(3):
            moved = tuple(min(max(by(x, at), 0.0), 1.0) for x in frame.levels[at])
            levels.append(moved)
            means.append(frame.means[at] + _expected(moved) - _expected(frame.levels[at]))
        return Figures(
            levels=(levels[0], levels[1], levels[2]), means=(means[0], means[1], means[2])
        )

    def params(self) -> dict[str, Any]:
        """The adjustment as ``set_node`` takes it in ``{"shot_match": ...}``."""
        return {
            "gamma": dict(zip(_RGB, self.gamma, strict=True)),
            "gain": dict(zip(_RGB, self.gain, strict=True)),
            "offset": dict(zip(_RGB, self.offset, strict=True)),
            "contrast": self.contrast,
            "pivot": self.pivot,
        }

    @classmethod
    def of(cls, params: Mapping[str, Any]) -> "ShotMatch":
        """The adjustment that a corrector's ``shot_match`` holds.

        Raises:
            ValueError: A value is not a number (it is keyframed, as a model may write it).
        """

        def number(value: object) -> float:
            if isinstance(value, bool) or not isinstance(value, int | float):
                raise ValueError("a value of the shot match is not a number")
            return float(value)

        def three(name: str) -> tuple[float, float, float]:
            given: object = params.get(name)
            if not isinstance(given, Mapping):
                raise ValueError(f"{name} of the shot match is not an object")
            channels: Mapping[str, object] = given  # pyright: ignore[reportUnknownVariableType]
            return number(channels.get("r")), number(channels.get("g")), number(channels.get("b"))

        return cls(
            gamma=three("gamma"),
            gain=three("gain"),
            offset=three("offset"),
            contrast=number(params.get("contrast")),
            pivot=number(params.get("pivot")),
        )


@dataclass(frozen=True)
class Bounds:
    """How far Shot Match may go, each a range of a parameter, so that it does no harm."""

    gamma: tuple[float, float] = (0.5, 2.0)
    gain: tuple[float, float] = (0.5, 2.0)
    offset: tuple[float, float] = (-0.2, 0.2)
    contrast: tuple[float, float] = (2 / 3, 1.5)
    pivot: tuple[float, float] = (0.18, 0.82)


BOUNDS = Bounds()
"""Ours: the engine allows a gamma of 0.1-10, a gain of 0-16, an offset of -1-1 and a contrast
of 0-4, far more than one shot is matched to another by without breaking it."""


@dataclass(frozen=True)
class Solved:
    """What :func:`solve` found.

    Attributes:
        match: The adjustment.
        before: The root mean square of the differences of the levels of the clip and the target,
            of those clipped (at 0 or 1) in neither.
        after: The same, of the clip adjusted, as :meth:`ShotMatch.mapped` foretells it.
        limited: The parameters held at a bound, as ``gain.r`` or ``contrast``.
    """

    match: ShotMatch
    before: float
    after: float
    limited: tuple[str, ...]


# The weights of the residuals: a level whose signal is clipped (at 0 or 1) in either frame tells
# little of where it would be, the mean chroma and the mid grey are each one number for a frame.
_CLIPPED = 0.2
_EDGE = 0.002
_CHROMA_WEIGHT = 2.0
_GREY_WEIGHT = 2.0
_PULL = 1e-2
"""The pull towards the neutral values, per parameter measured in its half range."""
_NAMES = (
    "gamma.r", "gamma.g", "gamma.b", "gain.r", "gain.g", "gain.b", "offset.r", "offset.g",
    "offset.b", "contrast",
)  # fmt: skip
_SNAP = 1e-4
"""A parameter this close to neutral is made neutral, so that the engine skips what it does."""


def _clipped(value: float) -> bool:
    return value <= _EDGE or value >= 1.0 - _EDGE


def _rms(one: Figures, other: Figures, kept: Figures) -> float:
    """The root mean square of the differences of the levels of two frames.

    Only levels not clipped in ``kept`` or in ``other`` count: a clipped one tells nothing.
    """
    squares = [
        (a - b) ** 2
        for x, y, z in zip(one.levels, other.levels, kept.levels, strict=True)
        for a, b, c in zip(x, y, z, strict=True)
        if not (_clipped(b) or _clipped(c))
    ]
    return math.sqrt(sum(squares) / len(squares)) if squares else 0.0


def solve(clip: Figures, target: Figures, bounds: Bounds = BOUNDS) -> Solved:
    """The Shot Match that makes the figures of ``clip`` those of ``target``, within ``bounds``.

    The same figures give the neutral adjustment, exactly.
    """
    pivot = min(max(target.grey, bounds.pivot[0]), bounds.pivot[1])
    lows = [math.log(bounds.gamma[0])] * 3 + [math.log(bounds.gain[0])] * 3
    highs = [math.log(bounds.gamma[1])] * 3 + [math.log(bounds.gain[1])] * 3
    lows += [bounds.offset[0]] * 3 + [math.log(bounds.contrast[0])]
    highs += [bounds.offset[1]] * 3 + [math.log(bounds.contrast[1])]
    spans = [(high - low) / 2 for low, high in zip(lows, highs, strict=True)]
    weights = [
        [_CLIPPED if _clipped(x) or _clipped(t) else 1.0 for x, t in zip(xs, ts, strict=True)]
        for xs, ts in zip(clip.levels, target.levels, strict=True)
    ]
    wanted_chroma = target.chroma

    def match_of(theta: Sequence[float]) -> ShotMatch:
        return ShotMatch(
            gamma=(math.exp(theta[0]), math.exp(theta[1]), math.exp(theta[2])),
            gain=(math.exp(theta[3]), math.exp(theta[4]), math.exp(theta[5])),
            offset=(theta[6], theta[7], theta[8]),
            contrast=math.exp(theta[9]),
            pivot=pivot,
        )

    def residuals(theta: Sequence[float]) -> list[float]:
        match = match_of(theta)
        out: list[float] = []
        medians: list[float] = []
        means: list[float] = []
        for at in range(3):
            moved = [match.channel(x, at) for x in clip.levels[at]]
            out += [
                w * (y - t) for w, y, t in zip(weights[at], moved, target.levels[at], strict=True)
            ]
            medians.append(moved[LEVELS.index(50)])
            means.append(clip.means[at] + _expected(moved) - _expected(clip.levels[at]))
        cb, cr = _chroma(means)
        out += [_CHROMA_WEIGHT * (cb - wanted_chroma[0]), _CHROMA_WEIGHT * (cr - wanted_chroma[1])]
        grey = sum(k * one for k, one in zip(LUMA, medians, strict=True))
        out.append(_GREY_WEIGHT * (grey - target.grey))
        out += [_PULL * value / span for value, span in zip(theta, spans, strict=True)]
        return out

    theta = _least_squares(residuals, [0.0] * len(_NAMES), lows, highs)
    theta = [0.0 if abs(value) < _SNAP else value for value in theta]
    match = match_of(theta)
    if match.neutral:
        match = replace(match, pivot=ShotMatch().pivot)
    limited = tuple(
        name
        for name, value, low, high in zip(_NAMES, theta, lows, highs, strict=True)
        if value <= low + 1e-9 or value >= high - 1e-9
    )
    return Solved(
        match=match,
        before=_rms(clip, target, clip),
        after=_rms(match.mapped(clip), target, clip),
        limited=limited,
    )


def _least_squares(
    residuals: Callable[[Sequence[float]], list[float]],
    start: list[float],
    lows: Sequence[float],
    highs: Sequence[float],
) -> list[float]:
    """The parameters that least square ``residuals``, by Levenberg-Marquardt within bounds.

    A parameter at a bound that the step would take beyond it is held there, and each step is
    clamped to the bounds.
    """
    theta = list(start)
    now = residuals(theta)
    cost = sum(r * r for r in now)
    damping = 1e-3
    for _ in range(200):
        jacobian = _jacobian(residuals, theta, lows, highs)
        size = len(theta)
        gradient = [sum(j[i] * r for j, r in zip(jacobian, now, strict=True)) for i in range(size)]
        normal = [[sum(j[i] * j[k] for j in jacobian) for k in range(size)] for i in range(size)]
        free = [
            i
            for i in range(size)
            if not (theta[i] <= lows[i] and gradient[i] > 0)
            and not (theta[i] >= highs[i] and gradient[i] < 0)
        ]
        if not free or max(abs(gradient[i]) for i in free) < 1e-14:
            break
        improved = False
        while damping < 1e12:
            system = [
                [normal[i][k] + (damping * (normal[i][i] + 1e-12) if i == k else 0.0) for k in free]
                for i in free
            ]
            step = _solved(system, [-gradient[i] for i in free])
            trial = list(theta)
            for i, delta in zip(free, step, strict=True):
                trial[i] = min(max(theta[i] + delta, lows[i]), highs[i])
            after = residuals(trial)
            trial_cost = sum(r * r for r in after)
            if trial_cost < cost:
                gained = cost - trial_cost
                theta, now, cost = trial, after, trial_cost
                damping = max(damping / 3, 1e-12)
                improved = gained > 1e-16 * max(cost, 1e-30) and gained > 1e-20
                break
            damping *= 4
        if not improved:
            break
    return theta


def _jacobian(
    residuals: Callable[[Sequence[float]], list[float]],
    theta: Sequence[float],
    lows: Sequence[float],
    highs: Sequence[float],
) -> list[list[float]]:
    """The derivatives of each residual by each parameter, as rows of the residuals.

    They are central differences, one-sided at a bound.
    """
    columns: list[list[float]] = []
    for i, value in enumerate(theta):
        h = 1e-6
        up = list(theta)
        down = list(theta)
        up[i] = min(value + h, highs[i])
        down[i] = max(value - h, lows[i])
        width = up[i] - down[i]
        upper, lower = residuals(up), residuals(down)
        columns.append([(a - b) / width for a, b in zip(upper, lower, strict=True)])
    return [list(row) for row in zip(*columns, strict=True)]


def _solved(matrix: list[list[float]], vector: list[float]) -> list[float]:
    """``x`` of ``matrix x = vector``, by elimination with partial pivoting."""
    size = len(vector)
    rows = [[*row, value] for row, value in zip(matrix, vector, strict=True)]
    for column in range(size):
        best = max(range(column, size), key=lambda row: abs(rows[row][column]))
        rows[column], rows[best] = rows[best], rows[column]
        lead = rows[column][column]
        if abs(lead) < 1e-300:
            continue
        for row in range(column + 1, size):
            factor = rows[row][column] / lead
            if factor:
                for k in range(column, size + 1):
                    rows[row][k] -= factor * rows[column][k]
    out = [0.0] * size
    for row in reversed(range(size)):
        lead = rows[row][row]
        rest = sum(rows[row][k] * out[k] for k in range(row + 1, size))
        out[row] = (rows[row][size] - rest) / lead if abs(lead) >= 1e-300 else 0.0
    return out


# ---------------------------------------------------------------------------------------------
# Auto Color
# ---------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _AutoColor:
    least_range: float = 0.02
    most_gain: float = 2.0
    least_gain: float = 0.5
    most_lift: float = 0.25


AUTO_COLOR = _AutoColor()
"""How far Auto Color may go, as the desktop's ``AUTO_COLOR`` (ours, to be set against
Resolve's, which the manual calls "somewhat hit-or-miss", p. 3046)."""

_LIFT = (-1.0, 1.0)
_GAIN = (0.0, 16.0)


def auto_color(
    blacks: Sequence[float],
    whites: Sequence[float],
    lift: Sequence[float] = (0.0, 0.0, 0.0),
    gain: Sequence[float] = (1.0, 1.0, 1.0),
) -> tuple[tuple[float, float, float], tuple[float, float, float]] | None:
    """The Lift and Gain of R, G and B that Auto Color puts on a node; ``None`` for a flat frame.

    ``blacks`` and ``whites`` are the levels of 1 and 99 percent of each channel of the picture
    measured, and ``lift`` and ``gain`` what the node has where that picture is already through
    them (neutral where it is the node's input). Each channel's black and white are brought to
    their luma-weighted means, so they are neutral and their lightness kept. Where the picture is
    through the node, the line that does it is put after the node's: ``gain * (x + lift * (1 -
    x))`` is ``B + A x`` with ``A = gain * (1 - lift)`` and ``B = gain * lift``, and a line after
    a line is a line (exact where Gamma is neutral).
    """
    black = sum(k * one for k, one in zip(LUMA, blacks, strict=True))
    white = sum(k * one for k, one in zip(LUMA, whites, strict=True))
    if white - black < AUTO_COLOR.least_range or any(
        w - b < AUTO_COLOR.least_range for b, w in zip(blacks, whites, strict=True)
    ):
        return None
    lifts: list[float] = []
    gains: list[float] = []
    for at in range(3):
        a = (white - black) / (whites[at] - blacks[at])
        a = min(AUTO_COLOR.most_gain, max(AUTO_COLOR.least_gain, a))
        shift = min(AUTO_COLOR.most_lift, max(-AUTO_COLOR.most_lift, black - a * blacks[at]))
        slope = gain[at] * (1 - lift[at])
        base = gain[at] * lift[at]
        new_slope = a * slope
        new_base = a * base + shift
        g = new_slope + new_base
        if not g > 0:
            return None
        gains.append(min(max(g, _GAIN[0]), _GAIN[1]))
        lifts.append(min(max(new_base / g, _LIFT[0]), _LIFT[1]))
    return (lifts[0], lifts[1], lifts[2]), (gains[0], gains[1], gains[2])
