//! Stereo Width (T10): width 0 is the same bits on both sides and width 1 the input to the bit,
//! the sides of width 2 written out, the mid never touched, Diffusion an all-pass of the side and
//! Sparkle its high-pass added as the cookbook has it, the other channels of a layout to the bit,
//! a mono track turned away, ramps of 10 ms, the same output in any blocks, no memory taken, no
//! zipper, no latency, a settling that holds, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    reason = "exact values compared on purpose, sample numbers, the letters of the cookbook"
)]

mod common;

use std::f64::consts::TAU;

use columbina_audio::constants::{WIDTH_DIFFUSION_DELAYS, WIDTH_SPARKLE_HZ};
use columbina_audio::fx::width::{settle_for, Param, Settings, StereoWidth};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{amplitude_at, no_zipper, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn width(width: f32, diffusion: f32, sparkle: f32) -> StereoWidth {
    StereoWidth::new(&Settings {
        on: true,
        width,
        diffusion,
        sparkle,
    })
}

/// Stereo noise whose sides are of very different sizes now and then.
fn stereo(frames: usize) -> Vec<Vec<f32>> {
    let mut left = noise(1, 1.0, frames);
    let right = noise(2, 0.5, frames);
    for n in (0..frames).step_by(97) {
        left[n] *= 1e-9;
    }
    vec![left, right]
}

#[test]
fn width_0_is_the_same_on_both_sides_and_width_1_is_the_input() {
    let spec = Spec::new(RATE, 9_600, Layout::Stereo);
    let input = stereo(9_600);
    let run = Run::new(input.clone());
    let mono = process(width(0.0, 0.0, 0.0), &spec, &run);
    for n in 0..9_600 {
        assert_eq!(mono[0][n].to_bits(), mono[1][n].to_bits(), "{n}");
        let mid = (f64::from(input[0][n]) + f64::from(input[1][n])) / 2.0;
        assert!((f64::from(mono[0][n]) - mid).abs() <= 1e-7);
    }
    // Diffusion and Sparkle work on the side alone: at width 0 there is none.
    let mono = process(width(0.0, 100.0, 100.0), &spec, &run);
    assert_eq!(mono[0], mono[1]);
    assert_eq!(process(width(1.0, 0.0, 0.0), &spec, &run), input);
    let off = StereoWidth::new(&Settings {
        on: false,
        width: 2.0,
        diffusion: 50.0,
        sparkle: 50.0,
    });
    assert_eq!(process(off, &spec, &run), input);
}

#[test]
fn width_2_doubles_the_side_and_the_mid_is_never_touched() {
    let spec = Spec::new(RATE, 9_600, Layout::Stereo);
    let input = stereo(9_600);
    let run = Run::new(input.clone());
    let wide = process(width(2.0, 0.0, 0.0), &spec, &run);
    for n in 0..9_600 {
        let (l, r) = (f64::from(input[0][n]), f64::from(input[1][n]));
        // M + 2S and M − 2S: 1.5 L − 0.5 R and 1.5 R − 0.5 L.
        assert!((f64::from(wide[0][n]) - (1.5 * l - 0.5 * r)).abs() < 1e-6);
        assert!((f64::from(wide[1][n]) - (1.5 * r - 0.5 * l)).abs() < 1e-6);
    }
    for (w, d, s) in [(0.3, 40.0, 0.0), (1.7, 100.0, 100.0), (1.0, 0.0, 60.0)] {
        let output = process(width(w, d, s), &spec, &run);
        for n in 0..9_600 {
            let mid_in = f64::from(input[0][n]) + f64::from(input[1][n]);
            let mid_out = f64::from(output[0][n]) + f64::from(output[1][n]);
            assert!((mid_in - mid_out).abs() < 1e-6, "{w} {d} {s} at {n}");
        }
    }
}

#[test]
#[allow(clippy::cast_possible_truncation, reason = "samples of a test")]
fn diffusion_keeps_the_power_of_the_side_and_loses_its_shape() {
    let frames = 96_000;
    let side = noise(3, 0.5, frames);
    let input = vec![side.clone(), side.iter().map(|x| -x).collect()];
    let spec = Spec::new(RATE, frames, Layout::Stereo);
    let output = process(width(1.0, 100.0, 0.0), &spec, &Run::new(input));
    let out_side: Vec<f64> = output[0]
        .iter()
        .zip(&output[1])
        .map(|(l, r)| (f64::from(*l) - f64::from(*r)) / 2.0)
        .collect();
    let power = |x: &[f64]| x.iter().map(|x| x * x).sum::<f64>() / x.len() as f64;
    let in_side: Vec<f64> = side.iter().map(|x| f64::from(*x)).collect();
    let ratio = power(&out_side[4_800..]) / power(&in_side[4_800..]);
    assert!((ratio - 1.0).abs() < 0.02, "{ratio}");
    // The impulse response of the chain of all-passes: none at once but −g⁴, then echoes at
    // each sum of delays.
    let mut impulse = vec![0.0_f32; 4_800];
    impulse[0] = 1.0;
    let output = process(
        width(1.0, 100.0, 0.0),
        &spec.with_max_block(4_800),
        &Run::new(vec![impulse.clone(), impulse.iter().map(|x| -x).collect()]),
    );
    assert!(
        (f64::from(output[0][0]) - 0.0625).abs() < 1e-7,
        "{}",
        output[0][0]
    );
    let first: usize = WIDTH_DIFFUSION_DELAYS
        .iter()
        .map(|seconds| (seconds * f64::from(RATE)).round() as usize)
        .min()
        .unwrap();
    assert!(output[0][1..first].iter().all(|x| *x == 0.0));
    assert!(output[0][first] != 0.0);
}

/// The gain of the high-pass of two poles of the cookbook, Butterworth, at `freq`, as a complex
/// number, written out here again.
fn cookbook_high_pass(f0: f64, freq: f64) -> (f64, f64) {
    let rate = f64::from(RATE);
    let w0 = TAU * f0 / rate;
    let alpha = w0.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let c = w0.cos();
    let b = [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0];
    let a = [1.0 + alpha, -2.0 * c, 1.0 - alpha];
    let w = TAU * freq / rate;
    let at = |k: [f64; 3]| {
        (
            k[0] + k[1] * w.cos() + k[2] * (2.0 * w).cos(),
            -(k[1] * w.sin() + k[2] * (2.0 * w).sin()),
        )
    };
    let (br, bi) = at(b);
    let (ar, ai) = at(a);
    let d = ar * ar + ai * ai;
    ((br * ar + bi * ai) / d, (bi * ar - br * ai) / d)
}

#[test]
fn sparkle_adds_the_high_side_by_the_high_pass_of_the_cookbook() {
    let spec = Spec::new(RATE, 48_000, Layout::Stereo);
    for (freq, sparkle) in [
        (12_000.0, 100.0),
        (5_000.0, 100.0),
        (8_000.0, 40.0),
        (200.0, 100.0),
    ] {
        let side = sine_at(freq, 0.25, 0.3, 48_000);
        let input = vec![side.clone(), side.iter().map(|x| -x).collect()];
        let output = process(width(1.0, 0.0, sparkle), &spec, &Run::new(input));
        let (re, im) = cookbook_high_pass(f64::from(WIDTH_SPARKLE_HZ), freq);
        let k = f64::from(sparkle) / 100.0;
        let expected = 0.25 * (1.0 + k * re).hypot(k * im);
        let measured = amplitude_at(&output[0][24_000..], freq);
        assert!(
            (measured / expected - 1.0).abs() < 1e-3,
            "{freq} Hz: {measured} against {expected}"
        );
    }
}

#[test]
fn the_other_channels_pass_to_the_bit_and_a_mono_track_is_turned_away() {
    let spec = Spec::new(RATE, 4_800, Layout::Surround51);
    let input: Vec<Vec<f32>> = (0..6).map(|c| noise(10 + c, 0.5, 4_800)).collect();
    let output = process(width(1.6, 30.0, 30.0), &spec, &Run::new(input.clone()));
    for channel in 2..6 {
        assert_eq!(output[channel], input[channel]);
    }
    let stereo = process(
        width(1.6, 30.0, 30.0),
        &Spec::new(RATE, 4_800, Layout::Stereo),
        &Run::new(input[..2].to_vec()),
    );
    assert_eq!(output[..2], stereo[..]);
    let mut node = width(1.0, 0.0, 0.0);
    for layout in [Layout::Mono, Layout::Empty, Layout::discrete(2)] {
        assert!(
            node.prepare(&Spec::new(RATE, 64, layout)).is_err(),
            "{layout:?}"
        );
    }
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_sidechain(2))
        .is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Surround71))
        .is_ok());
}

#[test]
fn width_ramps_over_ten_ms_and_no_change_makes_a_zipper() {
    // Left 1, right 0: M = S = 0.5, the left is 0.5 + 0.5 w.
    let spec = Spec::new(RATE, 4_800, Layout::Stereo);
    let run = Run::new(vec![vec![1.0; 4_800], vec![0.0; 4_800]]).with_change(
        1_000,
        Param::Width.id(),
        2.0,
    );
    let output = process(width(1.0, 0.0, 0.0), &spec, &run);
    assert_eq!(output[0][999], 1.0);
    for k in 0..480 {
        let w = 1.0 + (k + 1) as f32 / 480.0;
        assert!((output[0][1_000 + k] - (0.5 + 0.5 * w)).abs() < 1e-6, "{k}");
    }
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let tone = vec![
        sine_at(1_000.0, 0.5, 0.0, 24_000),
        sine_at(1_000.0, 0.1, 1.0, 24_000),
    ];
    let at = 12_012;
    no_zipper(
        |t| width(2.0 * t, 0.0, 0.0),
        &spec,
        &tone,
        9,
        (Param::Width.id(), 2.0),
        at,
    );
    no_zipper(
        |t| width(1.5, 100.0 * t, 0.0),
        &spec,
        &tone,
        11,
        (Param::Diffusion.id(), 100.0),
        at,
    );
    let bright = vec![
        sine_at(9_000.0, 0.5, 0.0, 24_000),
        sine_at(9_000.0, 0.1, 1.0, 24_000),
    ];
    no_zipper(
        |t| width(1.5, 0.0, 100.0 * t),
        &spec,
        &bright,
        11,
        (Param::Sparkle.id(), 100.0),
        12_000,
    );
}

#[test]
fn every_ramp_is_half_way_after_five_ms_and_a_lane_turns_the_insert_off() {
    let spec = Spec::new(RATE, 240, Layout::Stereo);
    let quiet = Run::new(vec![vec![0.0; 240], vec![0.0; 240]]);
    let mut node = width(0.0, 0.0, 100.0);
    node.prepare(&spec).unwrap();
    node.set(Param::Width.id(), 2.0);
    node.set(Param::Diffusion.id(), 100.0);
    node.set(Param::Sparkle.id(), 0.0);
    render(&mut node, &spec, &quiet, |left| left);
    let (w, d, s) = node.heard();
    assert!((w - 1.0).abs() < 1e-6, "{w}");
    assert!((d - 50.0).abs() < 1e-4, "{d}");
    assert!((s - 50.0).abs() < 1e-4, "{s}");
    render(&mut node, &spec, &quiet, |left| left);
    assert_eq!(node.heard(), (2.0, 100.0, 0.0));

    let spec = Spec::new(RATE, 4_800, Layout::Stereo);
    let input = vec![noise(70, 0.5, 4_800), noise(71, 0.5, 4_800)];
    let lane: Vec<f32> = (0..4_800)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(input.clone()).with_automation(Param::On.id(), lane);
    let output = process(width(2.0, 30.0, 30.0), &spec, &run);
    for channel in 0..2 {
        assert_eq!(output[channel][2_480..], input[channel][2_480..]);
    }
}

#[test]
fn the_width_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Surround51);
    let frames = 20_000;
    let input: Vec<Vec<f32>> = (0..6).map(|c| noise(20 + c, 0.7, frames)).collect();
    let run = Run::new(input)
        .with_change(100, Param::Width.id(), 0.2)
        .with_change(3_000, Param::Diffusion.id(), 80.0)
        .with_change(3_100, Param::Sparkle.id(), 90.0)
        .with_change(8_000, Param::On.id(), 0.0)
        .with_change(8_300, Param::On.id(), 1.0)
        .with_change(9_000, Param::Width.id(), 1.0)
        .with_change(9_000, Param::Diffusion.id(), 0.0)
        .with_change(9_000, Param::Sparkle.id(), 0.0);
    chunk_invariance(|| width(1.2, 10.0, 0.0), &spec, &run, &[1, 2]);
    let lane: Vec<f32> = (0..frames)
        .map(|n| 1.0 + (n as f32 / 700.0).sin())
        .collect();
    let automated = run.with_automation(Param::Width.id(), lane);
    chunk_invariance(|| width(1.2, 10.0, 0.0), &spec, &automated, &[3]);
}

#[test]
fn no_latency_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    assert_eq!(assert_latency(|| width(1.4, 50.0, 50.0), &spec), 0);
    let mut probe = width(1.4, 50.0, 50.0);
    probe.prepare(&spec).unwrap();
    let settle = probe.settle() as usize;
    assert_eq!(settle, settle_for(&Settings::default(), RATE) as usize);
    // 480, the four all-passes (82, 149, 206, 283 samples) twenty times each, and the high-pass.
    assert!(
        (480 + 20 * 720..480 + 20 * 720 + 100).contains(&settle),
        "{settle}"
    );
    let frames = 3 * settle;
    let run = Run::new(vec![noise(30, 0.9, frames), noise(31, 0.3, frames)])
        .with_change(settle, Param::Width.id(), 2.0)
        .with_change(settle + 50, Param::Diffusion.id(), 100.0);
    for from in [settle + 10, settle + 400] {
        assert_settle(|| width(1.4, 50.0, 50.0), &spec, &run, from);
    }
    // Diffusion off for the late node until the lane turns it up: the all-passes ran anyway.
    let lane: Vec<f32> = (0..frames)
        .map(|n| if n < settle + 2_000 { 0.0 } else { 100.0 })
        .collect();
    let run = Run::new(vec![noise(32, 0.9, frames), noise(33, 0.3, frames)])
        .with_automation(Param::Diffusion.id(), lane);
    assert_settle(|| width(1.4, 0.0, 0.0), &spec, &run, settle);
}

#[test]
fn input_and_lanes_that_are_not_numbers_and_the_work_of_the_thread_of_the_mix() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    let frames = 30_000;
    let mut left = noise(40, 0.5, frames);
    left[100] = f32::INFINITY;
    let right = noise(41, 0.5, frames);
    let clean = Run::new(vec![noise(40, 0.5, frames), right.clone()]);
    let odd = Run::new(vec![left, right]);
    let a = process(width(1.5, 100.0, 100.0), &spec, &clean);
    let b = process(width(1.5, 100.0, 100.0), &spec, &odd);
    let settle = settle_for(&Settings::default(), RATE) as usize;
    for channel in 0..2 {
        assert!(b[channel].iter().skip(101).all(|x| x.is_finite()));
        for n in 101 + settle..frames {
            assert!((a[channel][n] - b[channel][n]).abs() < 3.2e-5, "{n}");
        }
    }
    let nans = vec![f32::NAN; frames];
    let mut lanes = clean.clone();
    for param in [Param::On, Param::Width, Param::Diffusion, Param::Sparkle] {
        lanes = lanes.with_automation(param.id(), nans.clone());
    }
    assert_eq!(process(width(1.5, 100.0, 100.0), &spec, &lanes), a);

    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let changes = [
        (Param::Width.id(), 0.0),
        (Param::Diffusion.id(), 100.0),
        (Param::Sparkle.id(), 100.0),
        (Param::On.id(), 0.0),
    ];
    let lanes = vec![(Param::Width.id(), vec![1.8; 480])];
    let counted = realtime_work(&mut width(1.0, 0.0, 0.0), &spec, &changes, &lanes, 200);
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn its_parameters_are_those_of_the_model() {
    let paths: Vec<_> = Param::ALL.iter().map(|param| param.path()).collect();
    assert_eq!(paths, ["on", "width", "diffusion", "sparkle"]);
    assert_eq!(Param::Width.info().range, (0.0, 2.0));
    assert_eq!(Param::Width.info().default, 1.0);
    assert_eq!(Param::Diffusion.info().default, 0.0);
    let settings = Settings {
        on: true,
        width: 0.4,
        diffusion: 70.0,
        sparkle: 20.0,
    };
    let spec = Spec::new(RATE, 960, Layout::Stereo);
    let run = Run::new(vec![noise(50, 1.0, 960), noise(51, 1.0, 960)]);
    let mut set = StereoWidth::default();
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    assert_eq!(
        process(set, &spec, &run),
        process(StereoWidth::new(&settings), &spec, &run)
    );
}
