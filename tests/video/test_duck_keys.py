"""Source-mapped envelopes agree with the graph's sample/time and interpolation contracts."""

import copy
from fractions import Fraction
from itertools import pairwise
from typing import Any

import pytest

from columbina.video.duck_keys import (
    UnsupportedCurveError,
    at_time,
    gain_keys,
    sample_at,
    source_at,
)


def clip() -> dict[str, Any]:
    return {
        "id": "c1",
        "start": 0,
        "length": 200,
        "source": {"kind": "media", "media": "m1", "in": {"ticks": 0, "scale": 48000}},
        "gain_db": 0,
    }


def evaluate(raw: Any, at: Fraction) -> float:
    if isinstance(raw, (int, float)):
        return float(raw)
    keys = raw["keys"]
    if at <= at_time(keys[0]["at"]):
        return float(keys[0]["value"])
    for left, right in pairwise(keys):
        a, b = at_time(left["at"]), at_time(right["at"])
        if at <= b:
            s = float((at - a) / (b - a))
            rise = right["value"] - left["value"]
            leaves = 0 if left.get("interp") == "ease_out" else 1
            arrives = 0 if right.get("interp") == "ease_in" else 1
            return (
                (2 * s**3 - 3 * s**2 + 1) * left["value"]
                + (s**3 - 2 * s**2 + s) * leaves * rise
                + (-2 * s**3 + 3 * s**2) * right["value"]
                + (s**3 - s**2) * arrives * rise
            )
    return float(keys[-1]["value"])


@pytest.mark.parametrize("reverse", [False, True])
@pytest.mark.parametrize("speed", [{"num": 1, "den": 1}, {"num": 3, "den": 2}])
@pytest.mark.parametrize("linear", [False, True])
def test_envelope_preserves_baseline_in_source_time(
    reverse: bool, speed: dict[str, int], linear: bool
) -> None:
    raw = clip()
    raw.update(start=25, reverse=reverse, speed=speed)
    raw["source"]["in"] = {"ticks": 48000, "scale": 48000}
    if linear:
        raw["gain_db"] = {
            "keys": [
                {"at": {"ticks": 0, "scale": 1}, "value": -3, "interp": "linear"},
                {"at": {"ticks": 20, "scale": 1}, "value": 7, "interp": "linear"},
            ]
        }
    before = copy.deepcopy(raw)
    rate = {"num": 25, "den": 1}
    changed = gain_keys(raw, [(75, 125)], rate, 48000, 12)
    assert changed is not None
    # Every 1 ms across speech and ramps, with the exact 15 ms and 600 ms boundaries.
    for millisecond in range(1000, 9000):
        t = Fraction(millisecond, 1000)
        offset = 0.0
        if Fraction(2985, 1000) < t < 3:
            offset = -12 * float((t - Fraction(2985, 1000)) / Fraction(15, 1000))
        elif 3 <= t <= 5:
            offset = -12.0
        elif 5 < t < Fraction(56, 10):
            offset = -12 * float((Fraction(56, 10) - t) / Fraction(6, 10))
        source = source_at(raw, t * 48000, rate, 48000)
        assert evaluate(changed, source) == pytest.approx(
            evaluate(raw["gain_db"], source) + offset, abs=1e-10
        )
    assert raw == before
    if linear:
        assert all(
            any(key["at"] == old["at"] for key in changed["keys"]) for old in raw["gain_db"]["keys"]
        )


def test_reverse_last_position_is_the_last_audio_sample_even_at_fractional_frame_rate() -> None:
    raw = clip()
    raw.update(start=7, length=19, reverse=True, speed={"num": 2, "den": 1})
    raw["source"]["in"] = {"ticks": 1, "scale": 3}
    rate = {"num": 30000, "den": 1001}
    first, last = sample_at(7, rate, 48000), sample_at(26, rate, 48000) - 1
    assert source_at(raw, Fraction(last), rate, 48000) == Fraction(1, 3)
    assert source_at(raw, Fraction(first), rate, 48000) == Fraction(1, 3) + Fraction(
        (last - first) * 2, 48000
    )
    changed = gain_keys(raw, [(7, 26)], rate, 48000, 12)
    assert changed is not None
    assert evaluate(changed, Fraction(1, 3)) == -12


def test_unaffected_ease_and_original_key_times_survive_without_approximation() -> None:
    raw = clip()
    raw["gain_db"] = {
        "keys": [
            {"at": {"ticks": 0, "scale": 25}, "value": -2, "interp": "ease_out"},
            {"at": {"ticks": 25, "scale": 25}, "value": 2, "interp": "ease_in"},
            {"at": {"ticks": 50, "scale": 25}, "value": 2, "interp": "linear"},
            {"at": {"ticks": 200, "scale": 25}, "value": 2, "interp": "linear"},
        ]
    }
    changed = gain_keys(raw, [(75, 100)], {"num": 25, "den": 1}, 48000, 12)
    assert changed is not None
    assert changed["keys"][:2] == raw["gain_db"]["keys"][:2]
    for n in range(100):
        assert evaluate(changed, Fraction(n, 100)) == evaluate(raw["gain_db"], Fraction(n, 100))


def test_affected_ease_refuses_and_does_not_change_original_keys() -> None:
    raw = clip()
    raw["gain_db"] = {
        "keys": [
            {"at": {"ticks": 0, "scale": 1}, "value": -2, "interp": "ease_out"},
            {"at": {"ticks": 8, "scale": 1}, "value": 2, "interp": "linear"},
        ]
    }
    original = copy.deepcopy(raw)
    with pytest.raises(UnsupportedCurveError, match="affected eased"):
        gain_keys(raw, [(50, 100)], {"num": 25, "den": 1}, 48000, 12)
    assert raw == original


def test_floor_has_exact_crossings_and_keeps_quiet_baseline() -> None:
    raw = clip()
    raw["gain_db"] = -90
    changed = gain_keys(raw, [(50, 100)], {"num": 25, "den": 1}, 48000, 12)
    assert changed is not None
    assert evaluate(changed, Fraction(19925, 10000)) == -96
    assert evaluate(changed, Fraction(199, 100)) == pytest.approx(-94)
    assert evaluate(changed, Fraction(43, 10)) == -96
    assert evaluate(changed, Fraction(45, 10)) == pytest.approx(-92)
    assert evaluate(changed, Fraction(6)) == -90


def test_endpoint_unused_ease_flag_cannot_bend_new_extension() -> None:
    raw = clip()
    raw["gain_db"] = {
        "keys": [
            {"at": {"ticks": 6, "scale": 1}, "value": -2, "interp": "ease_in"},
            {"at": {"ticks": 7, "scale": 1}, "value": 2, "interp": "linear"},
        ]
    }
    changed = gain_keys(raw, [(50, 100)], {"num": 25, "den": 1}, 48000, 12)
    assert changed is not None
    assert evaluate(changed, Fraction(19925, 10000)) == pytest.approx(-8)
    assert evaluate(changed, Fraction(5)) == -2
    assert evaluate(changed, Fraction(13, 2)) == evaluate(raw["gain_db"], Fraction(13, 2))


def pointed(*points: tuple[int, int, int]) -> dict[str, Any]:
    """The clip, with speed points: each a frame and the ``num`` and ``den`` of its speed."""
    raw = clip()
    raw["retime"] = {
        "points": [{"frame": at, "speed": {"num": num, "den": den}} for at, num, den in points]
    }
    return raw


def test_a_clip_whose_speed_points_go_one_way_takes_a_key_on_each_side_of_them() -> None:
    # 100 frames at the speed of the source and 100 twice as fast: 4 s of the timeline are
    # 4 s of the source, and the 4 s after them are 8 s. Speech from 3 s to 3.8 s: the
    # recovery, to 4.4 s, goes through the speed point at 4 s, sample 192000.
    raw = pointed((0, 1, 1), (100, 2, 1))
    before = copy.deepcopy(raw)
    rate = {"num": 25, "den": 1}

    changed = gain_keys(raw, [(75, 95)], rate, 48000, 12)

    assert changed is not None
    assert raw == before
    assert source_at(raw, Fraction(192000), rate, 48000) == 4
    assert source_at(raw, Fraction(240000), rate, 48000) == 6
    times = {at_time(key["at"]) for key in changed["keys"]}
    assert {Fraction(191999, 48000), Fraction(4), Fraction(48, 10)} <= times
    # Every 1 ms of the timeline, with the exact 15 ms and 600 ms boundaries.
    for millisecond in range(1000, 7000):
        t = Fraction(millisecond, 1000)
        offset = 0.0
        if Fraction(2985, 1000) < t < 3:
            offset = -12 * float((t - Fraction(2985, 1000)) / Fraction(15, 1000))
        elif 3 <= t <= Fraction(38, 10):
            offset = -12.0
        elif Fraction(38, 10) < t < Fraction(44, 10):
            offset = -12 * float((Fraction(44, 10) - t) / Fraction(6, 10))
        source = source_at(raw, t * 48000, rate, 48000)
        assert evaluate(changed, source) == pytest.approx(offset, abs=1e-10)


def test_speed_points_that_hold_a_frame_or_turn_back_refuse_keys_where_speech_is() -> None:
    rate = {"num": 25, "den": 1}
    for raw in (
        pointed((0, 1, 1), (100, 0, 1), (150, 1, 1)),
        pointed((0, 1, 1), (100, -1, 1)),
    ):
        before = copy.deepcopy(raw)
        with pytest.raises(UnsupportedCurveError, match="speed points"):
            gain_keys(raw, [(50, 100)], rate, 48000, 12)
        assert raw == before
        # Where no speech is over the clip, nothing is asked of it.
        assert gain_keys(raw, [(300, 320)], rate, 48000, 12) is None
