//! The smoothing of a parameter changed while the sound plays: a linear ramp of 10 ms, the same
//! however the samples are asked for.

use columbina_audio::rt::{Domain, Smoothed};
use proptest::prelude::*;

#[test]
fn a_step_of_gain_from_zero_to_one_ramps_over_ten_milliseconds() {
    let mut gain = Smoothed::new(0.0, Domain::Linear);
    gain.prepare(48_000);
    assert_eq!(gain.ramp_len(), 480);
    gain.set(1.0);
    let ramp: Vec<f32> = (0..600).map(|_| gain.next_value()).collect();
    // The 240th sample after the change (the 1st being the first sample processed after it).
    assert!((ramp[239] - 0.5).abs() <= 1e-6, "{}", ramp[239]);
    for (k, value) in ramp.iter().enumerate().take(480) {
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let expected = (k + 1) as f32 / 480.0;
        assert!(
            (value - expected).abs() <= 1e-6,
            "sample {k}: {value}, not {expected}"
        );
    }
    assert!(ramp[..479].iter().all(|&value| value < 1.0));
    assert!(ramp[479..]
        .iter()
        .all(|&value| value.to_bits() == 1.0_f32.to_bits()));
    assert!(!gain.is_ramping());
}

#[test]
fn a_frequency_ramps_in_the_logarithmic_domain() {
    let mut frequency = Smoothed::new(100.0, Domain::Log);
    frequency.prepare(48_000);
    frequency.set(10_000.0);
    let ramp: Vec<f32> = (0..480).map(|_| frequency.next_value()).collect();
    // Half way, the geometric mean: 1 kHz.
    assert!((ramp[239] - 1_000.0).abs() <= 1e-2, "{}", ramp[239]);
    assert_eq!(ramp[479].to_bits(), 10_000.0_f32.to_bits());
}

#[test]
fn a_change_in_the_middle_of_a_ramp_starts_from_where_it_was() {
    let mut gain = Smoothed::new(0.0, Domain::Linear);
    gain.prepare(1_000); // 10 samples of ramp
    gain.set(1.0);
    for _ in 0..5 {
        gain.next_value();
    }
    assert!((gain.current() - 0.5).abs() < 1e-7);
    gain.set(0.0);
    let back: Vec<f32> = (0..10).map(|_| gain.next_value()).collect();
    assert!((back[0] - 0.45).abs() < 1e-6, "{back:?}");
    assert_eq!(back[9], 0.0);
}

#[test]
fn a_reset_ends_the_ramp_at_its_target() {
    let mut gain = Smoothed::new(0.0, Domain::Linear);
    gain.prepare(48_000);
    gain.set(0.25);
    gain.next_value();
    gain.finish();
    assert!(!gain.is_ramping());
    assert_eq!(gain.next_value(), 0.25);
    gain.jump(0.75);
    assert_eq!(gain.next_value(), 0.75);
}

#[test]
fn a_frequency_ramped_to_zero_and_back_never_gives_nan() {
    let mut frequency = Smoothed::new(1_000.0, Domain::Log);
    frequency.prepare(48_000);
    frequency.set(0.0);
    assert_eq!(frequency.target(), f32::MIN_POSITIVE);
    let down: Vec<f32> = (0..300).map(|_| frequency.next_value()).collect();
    frequency.set(500.0);
    let up: Vec<f32> = (0..600).map(|_| frequency.next_value()).collect();
    for value in down.iter().chain(&up) {
        assert!(value.is_finite() && *value > 0.0, "{value}");
    }
    assert_eq!(up[599], 500.0);
    let mut from_zero = Smoothed::new(0.0, Domain::Log);
    from_zero.prepare(48_000);
    from_zero.set(500.0);
    assert!((0..480).all(|_| from_zero.next_value().is_finite()));
    from_zero.jump(0.0);
    assert_eq!(from_zero.next_value(), f32::MIN_POSITIVE);
}

#[test]
fn before_it_is_prepared_a_change_takes_at_once() {
    let mut gain = Smoothed::new(0.0, Domain::Linear);
    gain.set(1.0);
    assert_eq!(gain.next_value(), 1.0);
}

proptest! {
    /// Whatever the blocks and wherever the changes fall, the samples given are those of the
    /// samples asked for one by one: the ramp depends on the samples since the change alone.
    #[test]
    fn the_ramp_does_not_depend_on_the_blocks(
        rate in prop_oneof![Just(44_100_u32), Just(48_000), Just(96_000), 1_000_u32..200_000],
        log in any::<bool>(),
        start in 0.01_f32..10.0,
        changes in prop::collection::vec((0_usize..2_000, 0.01_f32..10.0), 0..8),
        blocks in prop::collection::vec(1_usize..700, 1..40),
    ) {
        let domain = if log { Domain::Log } else { Domain::Linear };
        let total: usize = 3_000;
        let mut changes = changes;
        changes.sort_by_key(|&(at, _)| at);

        let mut one = Smoothed::new(start, domain);
        one.prepare(rate);
        let mut expected = Vec::with_capacity(total);
        let mut pending = changes.iter().peekable();
        for n in 0..total {
            while let Some(&&(at, value)) = pending.peek() {
                if at != n { break; }
                one.set(value);
                pending.next();
            }
            expected.push(one.next_value());
        }

        let mut blocked = Smoothed::new(start, domain);
        blocked.prepare(rate);
        let mut got = vec![0.0_f32; total];
        let mut n = 0;
        let mut pending = changes.iter().peekable();
        let mut sizes = blocks.iter().cycle();
        while n < total {
            while let Some(&&(at, value)) = pending.peek() {
                if at != n { break; }
                blocked.set(value);
                pending.next();
            }
            let next_change = pending.peek().map_or(total, |&&(at, _)| at.max(n + 1));
            let size = (*sizes.next().unwrap()).min(next_change - n).min(total - n);
            blocked.fill(&mut got[n..n + size]);
            n += size;
        }
        for (k, (a, b)) in expected.iter().zip(&got).enumerate() {
            prop_assert_eq!(a.to_bits(), b.to_bits(), "sample {}", k);
        }
    }

    /// A ramp goes straight from where it was to its target in exactly 10 ms, and stays there.
    #[test]
    fn a_ramp_is_straight_and_ends_on_its_target(
        rate in 8_000_u32..192_000,
        from in -4.0_f32..4.0,
        to in -4.0_f32..4.0,
    ) {
        let mut gain = Smoothed::new(from, Domain::Linear);
        gain.prepare(rate);
        let len = gain.ramp_len() as usize;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the rates of sound"
        )]
        let expected_len = (f64::from(rate) * 0.010).round() as usize;
        prop_assert_eq!(len, expected_len);
        gain.set(to);
        for k in 1..=len {
            let value = gain.next_value();
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let line = f64::from(from) + (f64::from(to) - f64::from(from)) * k as f64 / len as f64;
            let error = (f64::from(value) - line).abs();
            prop_assert!(error <= 1e-6, "sample {}: {} not {}", k, value, line);
        }
        for _ in 0..10 {
            prop_assert_eq!(gain.next_value().to_bits(), to.to_bits());
        }
    }
}
