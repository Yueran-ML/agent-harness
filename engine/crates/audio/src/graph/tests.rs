//! Tests of what is private to the graph: the resampler of a clip, moved with its test from the
//! mixer of phase 2 (`server/src/play/mix.rs`).

use super::clip::{radius, resampled};

#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "taps and samples far within an f64; the case that is given back as it is"
)]
fn a_sample_between_samples_is_a_windowed_sinc_of_those_about_it() {
    use std::f64::consts::PI;
    let left: Vec<f32> = (0..2048_i32)
        .map(|n| {
            #[allow(clippy::cast_possible_truncation)]
            let value = (0.4 * (f64::from(n) * 0.37).sin()) as f32;
            value
        })
        .collect();
    let right: Vec<f32> = (0..2048_i32)
        .map(|n| {
            #[allow(clippy::cast_possible_truncation)]
            let value = (0.3 * (f64::from(n) * 1.91).cos()) as f32;
            value
        })
        .collect();
    let pcm = [left.as_slice(), right.as_slice()];
    for cutoff in [1.0, 0.91875, 0.5, 1.0 / 16.0] {
        for phase in [0.0, 0.25, 0.5, 0.875] {
            if phase == 0.0 && cutoff == 1.0 {
                continue;
            }
            let index = 1000;
            let reach = radius(cutoff);
            let (mut sum, mut weights) = ([0.0; 2], 0.0);
            for tap in 1 - reach..=reach {
                let x = tap as f64 - phase;
                let angle = PI * cutoff * x;
                let sinc = if angle.abs() < 1e-12 {
                    1.0
                } else {
                    angle.sin() / angle
                };
                let weight = sinc * (0.5 + 0.5 * (PI * x / reach as f64).cos());
                let at = usize::try_from(index + tap).unwrap();
                sum[0] += f64::from(pcm[0][at]) * weight;
                sum[1] += f64::from(pcm[1][at]) * weight;
                weights += weight;
            }
            let mut got = [0.0; 2];
            resampled(&pcm, 1000, phase, cutoff, &mut got);
            for side in 0..2 {
                let want = sum[side] / weights;
                assert!(
                    (f64::from(got[side]) - want).abs() < 1e-5,
                    "{cutoff} at {phase}: {} != {want}",
                    got[side]
                );
            }
        }
    }
    // Unity speed at a whole sample gives the sample itself.
    let mut got = [0.0; 2];
    resampled(&pcm, 77, 0.0, 1.0, &mut got);
    assert_eq!(got, [pcm[0][77], pcm[1][77]]);
}
