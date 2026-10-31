"""Shot Match and Auto Color worked out from the figures of frames, with no engine."""

import random

import pytest

from columbina.video import shotmatch
from columbina.video.shotmatch import BOUNDS, Figures, ShotMatch
from support.scenes import SCENE, figures_of, png_of, through


def largest_gap(one: Figures, other: Figures) -> float:
    return max(
        abs(a - b)
        for x, y in zip(one.levels, other.levels, strict=True)
        for a, b in zip(x, y, strict=True)
    )


TARGET = figures_of(SCENE)


@pytest.mark.parametrize(
    "cast",
    [
        ShotMatch(gain=(0.9, 1.0, 1.08), offset=(0.02, 0.0, -0.015)),
        ShotMatch(gamma=(1.15, 1.0, 0.9), gain=(0.85, 0.95, 0.9), offset=(0.03, 0.01, 0.0)),
        ShotMatch(gain=(0.8, 0.8, 0.8)),
        ShotMatch(contrast=1.25, pivot=0.45),
    ],
)
def test_a_known_cast_of_colour_or_exposure_is_taken_back(cast: ShotMatch) -> None:
    shot = through(SCENE, cast)
    solved = shotmatch.solve(figures_of(shot), TARGET)
    matched = figures_of(through(shot, solved.match))
    # Each level of each channel is within 0.005 of the target's (the bins are 0.001 wide).
    assert largest_gap(matched, TARGET) < 0.005
    assert largest_gap(figures_of(shot), TARGET) > 0.03
    assert solved.after < 0.005 < solved.before
    assert solved.limited == ()
    # The match undoes the cast at every signal of the frame's range, not only at the levels.
    for x in (0.1, 0.3, 0.5, 0.7, 0.85):
        for at in range(3):
            assert solved.match.channel(cast.channel(x, at), at) == pytest.approx(x, abs=0.004)


def test_a_gain_and_offset_cast_is_matched_by_their_inverse() -> None:
    cast = ShotMatch(gain=(0.85, 1.0, 1.1), offset=(0.03, 0.0, -0.02))
    match = shotmatch.solve(figures_of(through(SCENE, cast)), TARGET).match
    for at in range(3):
        assert match.gain[at] == pytest.approx(1 / cast.gain[at], abs=0.02)
        assert match.offset[at] == pytest.approx(-cast.offset[at] / cast.gain[at], abs=0.01)
        assert match.gamma[at] == pytest.approx(1.0, abs=0.02)
    assert match.contrast == pytest.approx(1.0, abs=0.02)


def test_the_same_figures_give_the_neutral_match_exactly() -> None:
    solved = shotmatch.solve(TARGET, TARGET)
    assert solved.match == ShotMatch()
    assert solved.match.neutral
    assert (solved.before, solved.after, solved.limited) == (0.0, 0.0, ())
    assert solved.match.params() == {
        "gamma": {"r": 1.0, "g": 1.0, "b": 1.0},
        "gain": {"r": 1.0, "g": 1.0, "b": 1.0},
        "offset": {"r": 0.0, "g": 0.0, "b": 0.0},
        "contrast": 1.0,
        "pivot": 0.435,
    }


@pytest.mark.parametrize(
    "cast",
    [
        ShotMatch(gain=(0.3, 1.0, 1.0)),
        ShotMatch(gain=(0.25, 0.25, 0.25), offset=(0.05, 0.05, 0.05)),
        ShotMatch(gamma=(4.0, 1.0, 0.25)),
        ShotMatch(contrast=3.5),
    ],
)
def test_a_match_beyond_its_bounds_is_held_at_them_and_told(cast: ShotMatch) -> None:
    shot = figures_of(through(SCENE, cast))
    solved = shotmatch.solve(shot, TARGET)
    match = solved.match
    for at in range(3):
        assert BOUNDS.gamma[0] - 1e-9 <= match.gamma[at] <= BOUNDS.gamma[1] + 1e-9
        assert BOUNDS.gain[0] - 1e-9 <= match.gain[at] <= BOUNDS.gain[1] + 1e-9
        assert BOUNDS.offset[0] - 1e-9 <= match.offset[at] <= BOUNDS.offset[1] + 1e-9
    assert BOUNDS.contrast[0] - 1e-9 <= match.contrast <= BOUNDS.contrast[1] + 1e-9
    assert BOUNDS.pivot[0] <= match.pivot <= BOUNDS.pivot[1]
    assert solved.limited
    # Held at its bounds, it still goes the right way: it splits the difference.
    assert solved.after < solved.before


def test_the_figures_of_a_frame_are_its_levels_means_mid_grey_and_chroma() -> None:
    counts = [0] * 1024
    counts[512] = 100
    flat = shotmatch.figures({"red": counts, "green": counts, "blue": counts}, [0.5, 0.5, 0.5])
    # The pixels of one bin are taken as spread across it.
    assert flat.levels[0][3] == pytest.approx(512 / 1023, abs=0.5 / 1023)
    assert all(abs(level - 0.5) < 0.001 for level in flat.levels[1])
    assert flat.grey == pytest.approx(0.5, abs=0.001)
    assert flat.chroma == pytest.approx((0.0, 0.0))
    warm = Figures(levels=flat.levels, means=(0.6, 0.5, 0.4))
    cb, cr = warm.chroma
    assert cb < 0 < cr
    assert shotmatch.percentile([0, 0, 0], 0.5) == 0.0
    assert shotmatch.percentile([0, 4, 0, 0], 1.0) == pytest.approx((1 + 0.5) / 3)
    told = warm.told()
    assert set(told["percentiles"]["red"]) == {"1", "5", "25", "50", "75", "95", "99"}
    assert told["means"] == {"red": 0.6, "green": 0.5, "blue": 0.4}


def test_a_match_held_by_a_node_is_undone_from_what_comes_out_of_it() -> None:
    held = ShotMatch(gamma=(1.1, 0.95, 1.0), gain=(1.05, 0.9, 1.05), offset=(0.01, 0.0, 0.01))
    before = figures_of(SCENE)
    after = held.mapped(before)
    back = held.unmapped(after)
    assert largest_gap(back, before) < 1e-6
    assert back.means == pytest.approx(before.means, abs=1e-6)
    # Rising, each level adjusted is the same level of the frame adjusted.
    assert largest_gap(after, figures_of(through(SCENE, held))) < 0.002
    with pytest.raises(ValueError, match="not a number"):
        ShotMatch.of({**held.params(), "contrast": {"keys": []}})
    assert ShotMatch.of(held.params()) == held


def test_auto_color_balances_black_and_white_as_the_a_button_does() -> None:
    blacks, whites = (0.1, 0.05, 0.0), (0.9, 0.85, 0.8)
    found = shotmatch.auto_color(blacks, whites)
    assert found is not None
    lift, gain = found
    black = 0.2126 * 0.1 + 0.7152 * 0.05 + 0.0722 * 0
    white = 0.2126 * 0.9 + 0.7152 * 0.85 + 0.0722 * 0.8

    def balanced(x: float, at: int) -> float:
        return gain[at] * (x + lift[at] * (1 - x))

    for at in range(3):
        assert balanced(blacks[at], at) == pytest.approx(black, abs=1e-9)
        assert balanced(whites[at], at) == pytest.approx(white, abs=1e-9)
    # After a gain that the node has: that gain, then the balance.
    later = shotmatch.auto_color(blacks, whites, (0, 0, 0), (2, 1, 1))
    assert later is not None
    a, shift = gain[0] * (1 - lift[0]), gain[0] * lift[0]
    assert later[1][0] == pytest.approx(2 * a + shift)
    assert later[0][0] == pytest.approx(shift / (2 * a + shift))
    assert shotmatch.auto_color((0.1, 0.05, 0.3), (0.9, 0.85, 0.31)) is None


@pytest.mark.parametrize("channels", [3, 4])
def test_the_figures_of_a_still_are_read_from_its_png(channels: int) -> None:
    rng = random.Random(3)
    width, height = 13, 11
    rows = [
        bytes(rng.randrange(256) if c < 3 else 255 for _ in range(width) for c in range(channels))
        for _ in range(height)
    ]
    frame = [
        (row[x] / 255, row[x + 1] / 255, row[x + 2] / 255)
        for row in rows
        for x in range(0, width * channels, channels)
    ]
    read = shotmatch.figures_of_png(png_of(rows, width, channels))
    wanted = figures_of(frame, bins=256)
    assert read.levels == wanted.levels
    assert read.means == pytest.approx(wanted.means)
    with pytest.raises(ValueError, match="not a PNG"):
        shotmatch.figures_of_png(b"GIF89a")
