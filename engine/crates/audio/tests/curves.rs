//! The curves of crossfades and fades, kept with the other numbers we decide (convention 11).

#![allow(clippy::float_cmp, reason = "the curves are compared to the bit")]

use std::f64::consts::FRAC_PI_2;

use columbina_audio::constants::{crossfade_gain, fade_gain, CrossfadeLevel};
use proptest::prelude::*;

fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

#[test]
fn half_way_the_three_crossfades_are_three_db_apart() {
    let linear = crossfade_gain(CrossfadeLevel::Zero, 0.5, false);
    assert_eq!(linear, 0.5);
    let minus = db(crossfade_gain(CrossfadeLevel::Minus3, 0.5, false) / linear);
    let plus = db(crossfade_gain(CrossfadeLevel::Plus3, 0.5, false) / linear);
    assert!((minus + 3.01).abs() < 0.005, "{minus}");
    assert!((plus - 3.01).abs() < 0.005, "{plus}");
}

/// The equal-power crossfade of phase 2, as `Envelope::value` of `model/src/resolve.rs` computes
/// it (lines 274–278): the side that comes in and the side that goes.
fn phase_two(along: f64, falling: bool) -> f64 {
    if falling {
        (along * FRAC_PI_2).cos()
    } else {
        (along * FRAC_PI_2).sin()
    }
}

proptest! {
    /// The +3 dB crossfade is phase 2's equal power to the bit, so that old projects sound the
    /// same.
    #[test]
    fn the_plus_three_crossfade_is_that_of_phase_two_to_the_bit(t in 0.0_f64..=1.0, falling: bool) {
        prop_assert_eq!(
            crossfade_gain(CrossfadeLevel::Plus3, t, falling).to_bits(),
            phase_two(t, falling).to_bits()
        );
    }

    /// Each side that goes is the side that comes in, turned in time.
    #[test]
    fn the_side_that_goes_is_the_side_that_comes_in_turned(t in 0.0_f64..=1.0) {
        for level in [CrossfadeLevel::Minus3, CrossfadeLevel::Zero, CrossfadeLevel::Plus3] {
            let rising = crossfade_gain(level, 1.0 - t, false);
            let falling = crossfade_gain(level, t, true);
            prop_assert!((rising - falling).abs() < 1e-12, "{:?} {} {}", level, rising, falling);
        }
        for (curve, skew) in [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.3), (0.4, -0.7)] {
            let rising = fade_gain(curve, skew, 1.0 - t, false);
            let falling = fade_gain(curve, skew, t, true);
            prop_assert_eq!(rising.to_bits(), falling.to_bits());
        }
    }

    /// A fade goes from 0 to 1 and never down, for any curve and skew.
    #[test]
    fn a_fade_rises_from_nothing_to_all(curve in -1.0_f64..=1.0, skew in -1.0_f64..=1.0) {
        prop_assert_eq!(fade_gain(curve, skew, 0.0, false), 0.0);
        prop_assert!((fade_gain(curve, skew, 1.0, false) - 1.0).abs() < 1e-15);
        let mut last = 0.0;
        for k in 0..=100 {
            let gain = fade_gain(curve, skew, f64::from(k) / 100.0, false);
            prop_assert!(gain >= last - 1e-15, "{} after {}", gain, last);
            last = gain;
        }
    }
}

#[test]
fn a_fade_is_linear_bent_by_its_curve_and_moved_by_its_skew() {
    for t in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
        assert_eq!(fade_gain(0.0, 0.0, t, false), t);
        assert_eq!(fade_gain(1.0, 0.0, t, false), (t * FRAC_PI_2).sin());
        assert_eq!(fade_gain(-1.0, 0.0, t, false), 1.0 - (t * FRAC_PI_2).cos());
        let half = 0.5 * t + 0.5 * (t * FRAC_PI_2).sin();
        assert!((fade_gain(0.5, 0.0, t, false) - half).abs() < 1e-15);
    }
    // A skew of 1 makes t' = t^(1/2): the middle comes at a quarter of the way.
    assert!((fade_gain(0.0, 1.0, 0.25, false) - 0.5).abs() < 1e-15);
    assert!((fade_gain(0.0, -1.0, 0.5, false) - 0.25).abs() < 1e-15);
    assert_eq!(CrossfadeLevel::from_db(-3), Some(CrossfadeLevel::Minus3));
    assert_eq!(CrossfadeLevel::from_db(3), Some(CrossfadeLevel::Plus3));
    assert_eq!(CrossfadeLevel::from_db(0), Some(CrossfadeLevel::Zero));
    assert_eq!(CrossfadeLevel::from_db(6), None);
}
