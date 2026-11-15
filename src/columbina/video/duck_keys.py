"""Exact piecewise-linear duck envelopes, at the audio graph's source times."""

import copy
from fractions import Fraction
from itertools import pairwise
from typing import Any, cast

from columbina.video.edits import Rate
from columbina.video.retime import course_of


class UnsupportedCurveError(ValueError):
    """The model cannot represent this sum without changing its existing curve."""


def at_time(value: dict[str, int]) -> Fraction:
    """Read a model source time without rounding."""
    return Fraction(value["ticks"], value["scale"])


def wire_time(value: Fraction) -> dict[str, int]:
    """Write an exact nonnegative model Time, refusing an unrepresentable breakpoint."""
    if value < 0 or value.denominator > 0xFFFFFFFF or value.numerator > 0x7FFFFFFFFFFFFFFF:
        raise UnsupportedCurveError("Source time cannot be represented exactly; use mode='ducker'.")
    return {"ticks": value.numerator, "scale": value.denominator}


def sample_at(frame: int, rate: dict[str, int], sample_rate: int) -> int:
    """Return the first audio sample of a video frame, as in the graph."""
    return frame * sample_rate * rate["den"] // rate["num"]


def source_at(
    clip: dict[str, Any], sample: Fraction, rate: dict[str, int], sample_rate: int
) -> Fraction:
    """graph/clip.rs Mapping::new, including the last *audio sample* on reverse.

    A clip with speed points is read along them, where the engine reads the keys of its gain
    (``Sounding::seconds`` of ``retime.rs``): the time rises through a segment that goes
    forward, stands still through a freeze, and falls through one that goes backward.
    """
    course = course_of(clip)
    if course is not None:
        along = course.seconds_at(sample, clip["start"], Rate.model_validate(rate), sample_rate)
        return at_time(clip["source"]["in"]) + along
    speed = clip.get("speed") or {"num": 1, "den": 1}
    first = sample_at(clip["start"], rate, sample_rate)
    last = sample_at(clip["start"] + clip["length"], rate, sample_rate) - 1
    distance = last - sample if clip.get("reverse", False) else sample - first
    return at_time(clip["source"]["in"]) + distance * Fraction(
        speed["num"], speed["den"] * sample_rate
    )


def envelope(
    intervals: list[tuple[int, int]], rate: dict[str, int], sample_rate: int, amount: float
) -> list[tuple[Fraction, float]]:
    """15 ms ahead; 600 ms recovery. Hold across overlapping recovery/lookahead windows."""
    ahead, recovery = Fraction(15 * sample_rate, 1000), Fraction(600 * sample_rate, 1000)
    phrases: list[tuple[Fraction, Fraction]] = []
    for a, b in intervals:
        start, end = (
            Fraction(sample_at(a, rate, sample_rate)),
            Fraction(sample_at(b, rate, sample_rate)),
        )
        if phrases and start - ahead <= phrases[-1][1] + recovery:
            phrases[-1] = (phrases[-1][0], max(end, phrases[-1][1]))
        else:
            phrases.append((start, end))
    return [
        (at, value)
        for a, b in phrases
        for at, value in [(a - ahead, 0.0), (a, -amount), (b, -amount), (b + recovery, 0.0)]
    ]


def value_at(
    points: list[tuple[Fraction, float]], at: Fraction, outside: float | None = None
) -> float:
    """Interpolate a linear curve, holding endpoints unless an outside value is given."""
    if not points:
        return outside or 0.0
    if at <= points[0][0]:
        return points[0][1] if outside is None else outside if at < points[0][0] else points[0][1]
    for (left, a), (right, b) in pairwise(points):
        if at <= right:
            return a + (b - a) * float((at - left) / (right - left))
    return points[-1][1] if outside is None else outside


def gain_keys(
    clip: dict[str, Any],
    intervals: list[tuple[int, int]],
    rate: dict[str, int],
    sample_rate: int,
    amount: float,
) -> dict[str, Any] | None:
    """Add the envelope without erasing keys; unsupported affected easing is refused.

    The keys of a clip are at times of its source. A clip with speed points whose segments all
    play one way takes a key on each side of every speed point, where the line from the
    timeline to the source bends; one that holds a frame or turns back reads one time of its
    source at several times of the timeline, which no key can tell apart, and is refused.
    """
    if clip.get("source", {}).get("kind") != "media" or not clip.get("enabled", True):
        return None
    first = Fraction(sample_at(clip["start"], rate, sample_rate))
    last = Fraction(sample_at(clip["start"] + clip["length"], rate, sample_rate) - 1)
    if last < first:
        return None
    shape = envelope(intervals, rate, sample_rate, amount)
    course = course_of(clip)
    bends: set[Fraction] = set()
    if course is not None:
        starts = course.sound_starts(clip["start"], Rate.model_validate(rate), sample_rate)
        bends = {Fraction(at) for start in starts for at in (start - 1, start)}
    visible = sorted(
        {first, last, *(at for at in bends | {at for at, _ in shape} if first <= at <= last)}
    )
    if not any(value_at(shape, at, 0.0) < 0 for at in visible):
        return None
    if course is not None and not course.one_way:
        raise UnsupportedCurveError(
            f"Clip {clip['id']} has speed points that hold a frame or turn back, so Volume "
            "keys at times of its source cannot follow the timeline; use mode='ducker'."
        )
    mapped = [
        (source_at(clip, at, rate, sample_rate), value_at(shape, at, 0.0))
        for at in visible
        if value_at(shape, at, 0.0) < 0 or any(point == at for point, _ in shape)
    ]
    # One sample beyond each audible edge restores the unchanged source curve, including a
    # clip that begins or ends during speech. No visible sample loses the intended attenuation.
    for edge, at in [(first, first - 1), (last, last + 1)]:
        if value_at(shape, edge, 0.0) == 0:
            continue
        source = source_at(clip, at, rate, sample_rate)
        if source >= 0:
            mapped.append((source, 0.0))
    mapped.sort()
    raw = clip.get("gain_db", 0.0)
    old: list[dict[str, Any]] = (
        cast(dict[str, Any], raw).get("keys", []) if isinstance(raw, dict) else []
    )
    base = [(at_time(key["at"]), float(key["value"])) for key in old]
    for left, right in pairwise(old):
        if (left.get("interp") == "ease_out" or right.get("interp") == "ease_in") and any(
            (a != 0 or b != 0) and at_time(left["at"]) < high and at_time(right["at"]) > low
            for (low, a), (high, b) in pairwise(mapped)
        ):
            raise UnsupportedCurveError(
                f"Clip {clip['id']} has an affected eased Volume segment; "
                "use mode='ducker' to preserve it."
            )
    originals = {at_time(key["at"]): key for key in old}

    def baseline(at: Fraction) -> float:
        if at in originals:
            return float(originals[at]["value"])
        return value_at(base, at) if old else float(raw) if isinstance(raw, (float, int)) else 0.0

    times = sorted({*(at for at, _ in mapped), *(at for at, _ in base)})
    combined = {at: baseline(at) + value_at(mapped, at, 0.0) for at in times}
    # Preserve the exact clipped linear function at the model's -96 dB lower bound.
    for left, right in pairwise(times):
        a, b = combined[left], combined[right]
        if (a < -96 < b) or (b < -96 < a):
            crossing = left + (right - left) * (Fraction(-96) - Fraction(str(a))) / (
                Fraction(str(b)) - Fraction(str(a))
            )
            # A decimal float denominator need not fit Time. The value is below audibility;
            # refuse instead of pretending a rounded breakpoint is an exact curve.
            wire_time(crossing)
            combined[crossing] = -96.0
    keys: list[dict[str, Any]] = []
    for at in sorted(combined):
        original = originals.get(at)
        key: dict[str, Any] = (
            copy.deepcopy(original) if original else {"at": wire_time(at), "interp": "linear"}
        )
        # An endpoint's outward easing flag was unused under endpoint hold. Newly
        # inserted keys must not turn that formerly constant extension into a curve.
        if original and (
            (at == base[0][0] and key.get("interp") == "ease_in" and min(combined) < at)
            or (at == base[-1][0] and key.get("interp") == "ease_out" and max(combined) > at)
        ):
            key["interp"] = "linear"
        key["value"] = max(-96.0, combined[at])
        keys.append(key)
    return {"keys": keys}
