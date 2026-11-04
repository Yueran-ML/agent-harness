//! Soft Clipper (T10): the hyperbolic tangent and the hard clip and their mixes against values
//! written out, nothing over 0 dBFS before Output Level, the insert off and a hard clip under
//! 0 dBFS to the bit, ramps of 10 ms, the same output in any blocks, no memory taken, no zipper,
//! no latency, a settling that holds, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::manual_midpoint,
    reason = "exact values compared on purpose, sample numbers, formulas as written"
)]

mod common;

use columbina_audio::fx::soft_clip::{curve, settle_for, Param, Settings, SoftClipper};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{no_zipper, peak, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn clipper(threshold_db: f32, shape: f32, output_db: f32) -> SoftClipper {
    SoftClipper::new(&Settings {
        on: true,
        threshold_db,
        shape,
        output_db,
    })
}

/// The output of `node` for each of `xs`, each held for a while.
fn through(node: SoftClipper, xs: &[f32]) -> Vec<f32> {
    let input: Vec<f32> = xs.iter().flat_map(|&x| [x; 10]).collect();
    let spec = Spec::new(RATE, input.len(), Layout::Mono);
    let output = process(node, &spec, &Run::new(vec![input])).remove(0);
    output.chunks(10).map(|chunk| chunk[9]).collect()
}

#[test]
fn the_soft_shape_is_the_hyperbolic_tangent_and_the_hard_one_a_clip_at_0_dbfs() {
    // tanh(1), tanh(0.1), tanh(−2), tanh(0.5), written out.
    let soft = through(clipper(0.0, 100.0, 0.0), &[1.0, 0.1, -2.0, 0.5]);
    let expected = [
        0.761_594_155_955_764_9,
        0.099_667_994_624_955_82,
        -0.964_027_580_075_816_9,
        0.462_117_157_260_009_8,
    ];
    for (y, e) in soft.iter().zip(expected) {
        assert!((f64::from(*y) - e).abs() < 1e-7, "{y} {e}");
    }
    // +6.0206 dB is ×2: 0.25 → 0.5; 0.75 → 1.5, clipped to 1; −0.9 → −1.
    let hard = through(clipper(6.020_6, 0.0, 0.0), &[0.25, 0.75, -0.9]);
    for (y, e) in hard.iter().zip([0.5, 1.0, -1.0]) {
        assert!((f64::from(*y) - e).abs() < 1e-5, "{y} {e}");
    }
    // Half and half, then halved by Output Level: (1 + tanh 1.5)/2 · 0.5.
    let half = through(clipper(6.020_6, 50.0, -6.020_6), &[0.75]);
    let expected = (1.0 + 0.905_148_253_644_866_4) / 2.0 * 0.5;
    assert!((f64::from(half[0]) - expected).abs() < 1e-5, "{}", half[0]);
    for x in [-3.0_f64, -1.0, -0.3, 0.0, 0.2, 0.9, 1.0, 4.0] {
        for shape in [0.0, 0.3, 1.0] {
            let expected = (1.0 - shape) * x.clamp(-1.0, 1.0) + shape * f64::tanh(x);
            assert!((curve(x, shape) - expected).abs() < 1e-12);
        }
    }
}

#[test]
fn nothing_goes_over_0_dbfs_before_output_level() {
    let spec = Spec::new(RATE, 48_000, Layout::Stereo);
    let loud = Run::new(vec![noise(1, 8.0, 48_000), noise(2, 8.0, 48_000)]);
    for shape in [0.0, 37.0, 100.0] {
        for threshold in [0.0, 24.0] {
            let output = process(clipper(threshold, shape, 0.0), &spec, &loud);
            for channel in &output {
                assert!(peak(channel) <= 1.0, "{shape} {threshold}");
            }
            let output = process(clipper(threshold, shape, -6.0), &spec, &loud);
            for channel in &output {
                assert!(peak(channel) <= 0.502, "{shape} {threshold}");
            }
        }
    }
}

#[test]
fn off_and_a_hard_clip_under_0_dbfs_pass_the_sound_to_the_bit() {
    let input = vec![noise(3, 0.99, 9_600), noise(4, 0.2, 9_600)];
    let spec = Spec::new(RATE, 9_600, Layout::Stereo);
    let run = Run::new(input.clone());
    assert_eq!(process(clipper(0.0, 0.0, 0.0), &spec, &run), input);
    let off = SoftClipper::new(&Settings {
        on: false,
        threshold_db: 12.0,
        shape: 100.0,
        output_db: -3.0,
    });
    assert_eq!(process(off, &spec, &run), input);
}

#[test]
fn output_level_ramps_over_ten_ms_in_its_amplitude() {
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![vec![0.05; 4_800]]).with_change(2_000, Param::OutputDb.id(), 12.0);
    let output = process(clipper(0.0, 0.0, 0.0), &spec, &run).remove(0);
    assert_eq!(output[1_999], 0.05);
    let top = 10_f32.powf(12.0 / 20.0);
    for k in 0..480 {
        let expected = 0.05 * (1.0 + (top - 1.0) * (k + 1) as f32 / 480.0);
        assert!((output[2_000 + k] - expected).abs() < 1e-6, "{k}");
    }
    // Threshold, as a gain before a hard clip on a level under it: the same ramp.
    let run = Run::new(vec![vec![0.05; 4_800]]).with_change(2_000, Param::ThresholdDb.id(), 20.0);
    let output = process(clipper(0.0, 0.0, 0.0), &spec, &run).remove(0);
    assert!((output[2_239] - 0.05 * 5.5).abs() < 1e-6);
}

#[test]
fn every_ramp_is_half_way_after_five_ms_and_a_lane_turns_the_insert_off() {
    let spec = Spec::new(RATE, 240, Layout::Mono);
    let quiet = Run::new(vec![vec![0.0; 240]]);
    let mut node = clipper(0.0, 100.0, 0.0);
    node.prepare(&spec).unwrap();
    node.set(Param::ThresholdDb.id(), 20.0);
    node.set(Param::Shape.id(), 0.0);
    node.set(Param::OutputDb.id(), -20.0);
    render(&mut node, &spec, &quiet, |left| left);
    // In the amplitudes of the gains, and in the value of the shape.
    let (threshold, shape, output) = node.heard();
    assert!((threshold - 5.5).abs() < 1e-5, "{threshold}");
    assert!((shape - 50.0).abs() < 1e-4, "{shape}");
    assert!((output - 0.55).abs() < 1e-6, "{output}");
    render(&mut node, &spec, &quiet, |left| left);
    assert_eq!(node.heard(), (10.0, 0.0, 0.1));

    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let input = noise(70, 2.0, 4_800);
    let lane: Vec<f32> = (0..4_800)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(vec![input.clone()]).with_automation(Param::On.id(), lane);
    let output = process(clipper(6.0, 100.0, 0.0), &spec, &run).remove(0);
    assert_eq!(output[2_480..], input[2_480..]);
}

#[test]
fn every_change_makes_no_zipper() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let tone = vec![sine_at(1_000.0, 0.5, 0.0, 24_000)];
    let at = 12_012;
    no_zipper(
        |t| clipper(12.0 * t, 100.0, 0.0),
        &spec,
        &tone,
        13,
        (Param::ThresholdDb.id(), 12.0),
        at,
    );
    no_zipper(
        |t| clipper(12.0, 100.0 * (1.0 - t), 0.0),
        &spec,
        &tone,
        11,
        (Param::Shape.id(), 0.0),
        at,
    );
    no_zipper(
        |t| clipper(0.0, 100.0, 12.0 * t),
        &spec,
        &tone,
        9,
        (Param::OutputDb.id(), 12.0),
        at,
    );
}

#[test]
fn the_clipper_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Stereo);
    let frames = 20_000;
    let run = Run::new(vec![noise(5, 1.5, frames), noise(6, 1.5, frames)])
        .with_change(300, Param::Shape.id(), 20.0)
        .with_change(2_000, Param::ThresholdDb.id(), 18.0)
        .with_change(2_100, Param::OutputDb.id(), -9.0)
        .with_change(6_000, Param::On.id(), 0.0)
        .with_change(6_200, Param::On.id(), 1.0);
    chunk_invariance(|| clipper(3.0, 80.0, 0.0), &spec, &run, &[1, 2]);
    let lane: Vec<f32> = (0..frames)
        .map(|n| 50.0 + 50.0 * (n as f32 / 900.0).sin())
        .collect();
    let gains: Vec<f32> = (0..frames).map(|n| (n % 25) as f32).collect();
    let automated = run
        .with_automation(Param::Shape.id(), lane)
        .with_automation(Param::ThresholdDb.id(), gains);
    chunk_invariance(|| clipper(3.0, 80.0, 0.0), &spec, &automated, &[3]);
}

#[test]
fn no_latency_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    assert_eq!(assert_latency(|| clipper(6.0, 100.0, -3.0), &spec), 0);
    assert_eq!(settle_for(&Settings::default(), RATE), 480);
    let run = Run::new(vec![noise(7, 0.9, 9_600), noise(8, 0.9, 9_600)])
        .with_change(4_000, Param::ThresholdDb.id(), 9.0)
        .with_change(4_200, Param::Shape.id(), 0.0);
    for from in [4_100, 4_300, 7_000] {
        assert_settle(|| clipper(0.0, 100.0, 0.0), &spec, &run, from);
    }
}

#[test]
fn lanes_that_are_not_numbers_keep_the_values_set_and_the_work_takes_no_memory() {
    let spec = Spec::new(RATE, 960, Layout::Mono);
    let input = noise(9, 2.0, 960);
    let plain = process(
        clipper(6.0, 30.0, -2.0),
        &spec,
        &Run::new(vec![input.clone()]),
    );
    let odd = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY]
        .into_iter()
        .cycle()
        .take(960)
        .collect::<Vec<_>>();
    let mut run = Run::new(vec![input]);
    for param in [Param::On, Param::ThresholdDb, Param::Shape, Param::OutputDb] {
        run = run.with_automation(param.id(), odd.clone());
    }
    assert_eq!(process(clipper(6.0, 30.0, -2.0), &spec, &run), plain);

    let spec = Spec::new(RATE, 512, Layout::Surround51);
    let changes = [
        (Param::ThresholdDb.id(), 12.0),
        (Param::Shape.id(), 10.0),
        (Param::OutputDb.id(), -6.0),
        (Param::On.id(), 0.0),
    ];
    let lanes = vec![(Param::Shape.id(), vec![70.0; 480])];
    let counted = realtime_work(&mut clipper(1.0, 50.0, 0.0), &spec, &changes, &lanes, 200);
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn its_parameters_are_those_of_the_model() {
    let paths: Vec<_> = Param::ALL.iter().map(|param| param.path()).collect();
    assert_eq!(paths, ["on", "threshold_db", "shape", "output_db"]);
    assert_eq!(Param::ThresholdDb.info().range, (0.0, 24.0));
    assert_eq!(Param::Shape.info().default, 100.0);
    assert_eq!(Param::OutputDb.info().range, (-24.0, 12.0));
    assert!(Param::ALL.iter().all(|param| param.is_automatable()));
    let settings = Settings {
        on: true,
        threshold_db: 7.0,
        shape: 12.0,
        output_db: -4.0,
    };
    let spec = Spec::new(RATE, 960, Layout::Mono);
    let run = Run::new(vec![noise(11, 1.0, 960)]);
    let mut set = SoftClipper::default();
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    assert_eq!(
        process(set, &spec, &run),
        process(SoftClipper::new(&settings), &spec, &run)
    );
}
