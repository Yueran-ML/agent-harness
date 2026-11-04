//! Gain (T10): +6 dB is ×1.99526, 0 dB and off pass the sound to the bit, the gain ramps in its
//! amplitude over 10 ms and its lane is taken as it is, the same output in any blocks, no memory
//! taken, no zipper, no latency, a settling that holds, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "exact values compared on purpose, sample numbers"
)]

mod common;

use columbina_audio::fx::gain::{settle_for, Gain, Param, Settings};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, ramp_samples, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{no_zipper, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn gain(gain_db: f32) -> Gain {
    Gain::new(&Settings {
        gain_db,
        ..Settings::default()
    })
}

#[test]
fn six_db_multiply_by_1_99526() {
    // 10^(6/20), written out.
    let expected = 1.995_262_314_968_879_5_f64;
    let input = noise(1, 0.4, 4_800);
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let output = process(gain(6.0), &spec, &Run::new(vec![input.clone()])).remove(0);
    for (x, y) in input.iter().zip(&output) {
        if x.abs() > 1e-3 {
            let ratio = f64::from(*y) / f64::from(*x);
            assert!((ratio - expected).abs() <= 1e-6, "{ratio}");
        }
    }
    let minus = process(gain(-24.0), &spec, &Run::new(vec![vec![1.0; 4_800]])).remove(0);
    assert!((f64::from(minus[100]) - 0.063_095_734_448_019_3).abs() < 1e-7);
}

#[test]
fn no_gain_and_the_insert_off_pass_the_sound_to_the_bit() {
    let input = vec![noise(2, 1.0, 4_800), noise(3, 0.01, 4_800)];
    let spec = Spec::new(RATE, 4_800, Layout::Stereo);
    let run = Run::new(input.clone());
    assert_eq!(process(gain(0.0), &spec, &run), input);
    let off = Gain::new(&Settings {
        on: false,
        gain_db: 12.0,
    });
    assert_eq!(process(off, &spec, &run), input);
    // Turned off while it plays: after the crossfade of 10 ms, the input to the bit.
    let switched = process(
        gain(12.0),
        &spec,
        &run.with_change(1_000, Param::On.id(), 0.0),
    );
    for (channel, output) in switched.iter().enumerate() {
        assert_eq!(output[1_480..], input[channel][1_480..]);
        assert_ne!(output[1_478], input[channel][1_478]);
    }
}

#[test]
fn a_gain_set_ramps_over_ten_ms_in_its_amplitude() {
    // A constant level: the output is the gain, sample by sample.
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![vec![1.0; 4_800]]).with_change(1_000, Param::GainDb.id(), 20.0);
    let output = process(gain(0.0), &spec, &run).remove(0);
    let ramp = ramp_samples(RATE);
    assert_eq!(ramp, 480);
    assert_eq!(output[999], 1.0);
    // Half way, the mean of the amplitudes 1 and 10, not of the decibels (3.16).
    assert!((output[1_000 + ramp / 2 - 1] - 5.5).abs() < 1e-6);
    for k in 0..ramp {
        let expected = 1.0 + 9.0 * (k + 1) as f32 / ramp as f32;
        assert!((output[1_000 + k] - expected).abs() < 1e-5, "{k}");
    }
    assert_eq!(output[1_000 + ramp], 10.0);
}

#[test]
fn a_lane_is_taken_as_it_is_and_one_that_is_not_a_number_keeps_the_gain_set() {
    let spec = Spec::new(RATE, 960, Layout::Mono);
    let lane: Vec<f32> = (0..960).map(|n| (n % 49) as f32 - 24.0).collect();
    let run = Run::new(vec![vec![0.5; 960]]).with_automation(Param::GainDb.id(), lane.clone());
    let output = process(gain(3.0), &spec, &run).remove(0);
    for (y, db) in output.iter().zip(&lane) {
        let expected = 0.5 * 10_f64.powf(f64::from(*db) / 20.0);
        assert!((f64::from(*y) - expected).abs() < 1e-6);
    }
    let mut odd = lane.clone();
    odd[10] = f32::NAN;
    odd[20] = f32::INFINITY;
    odd[30] = f32::NEG_INFINITY;
    let run = Run::new(vec![vec![0.5; 960]]).with_automation(Param::GainDb.id(), odd);
    let output = process(gain(3.0), &spec, &run).remove(0);
    let set = 0.5 * 10_f64.powf(3.0 / 20.0);
    for n in [10, 20, 30] {
        assert!((f64::from(output[n]) - set).abs() < 1e-6, "{n}");
    }
}

#[test]
fn a_lane_that_turns_the_insert_off_lets_the_input_through_after_ten_ms() {
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let input = noise(70, 0.5, 4_800);
    let lane: Vec<f32> = (0..4_800)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(vec![input.clone()]).with_automation(Param::On.id(), lane);
    let output = process(gain(6.0), &spec, &run).remove(0);
    assert_eq!(output[2_480..], input[2_480..]);
    assert_ne!(output[2_478], input[2_478]);
}

#[test]
fn the_gain_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Surround51);
    let input: Vec<Vec<f32>> = (0..6).map(|c| noise(10 + c, 0.5, 20_000)).collect();
    let toggles: Vec<f32> = (0..20_000)
        .map(|n| if (n / 3_000) % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let sweep: Vec<f32> = (0..20_000)
        .map(|n| 20.0 * (n as f32 / 2_000.0).sin())
        .collect();
    let run = Run::new(input)
        .with_change(500, Param::GainDb.id(), 9.0)
        .with_change(700, Param::GainDb.id(), -9.0)
        .with_change(4_000, Param::On.id(), 0.0)
        .with_change(4_100, Param::On.id(), 1.0)
        .with_change(9_000, Param::GainDb.id(), 24.0);
    chunk_invariance(|| gain(-3.0), &spec, &run, &[1, 2, 3]);
    let automated = run
        .with_automation(Param::On.id(), toggles)
        .with_automation(Param::GainDb.id(), sweep);
    chunk_invariance(|| gain(-3.0), &spec, &automated, &[4, 5]);
}

#[test]
fn no_latency_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    assert_eq!(assert_latency(|| gain(6.0), &spec), 0);
    let mut probe = gain(6.0);
    probe.prepare(&spec).unwrap();
    assert_eq!(probe.settle(), 480);
    assert_eq!(settle_for(&Settings::default(), 96_000), 960);
    let run = Run::new(vec![noise(20, 0.5, 9_600), noise(21, 0.5, 9_600)])
        .with_change(4_000, Param::GainDb.id(), -12.0)
        .with_change(4_700, Param::On.id(), 0.0);
    for from in [4_100, 4_790, 6_000] {
        assert_settle(|| gain(6.0), &spec, &run, from);
    }
}

#[test]
fn a_gain_and_the_insert_switched_make_no_zipper() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let tone = vec![sine_at(1_000.0, 0.25, 0.0, 24_000)];
    // At a peak of the tone: 250 periods and a quarter.
    let at = 12_012;
    no_zipper(
        |t| gain(12.0 * t),
        &spec,
        &tone,
        9,
        (Param::GainDb.id(), 12.0),
        at,
    );
    let off = |t: f32| {
        Gain::new(&Settings {
            on: t < 0.5,
            gain_db: 12.0,
        })
    };
    no_zipper(off, &spec, &tone, 2, (Param::On.id(), 0.0), at);
}

#[test]
fn the_work_of_the_thread_of_the_mix_takes_no_memory() {
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let lanes = vec![(Param::GainDb.id(), vec![-6.0; 480])];
    let changes = [(Param::GainDb.id(), 7.0), (Param::On.id(), 0.0)];
    let counted = realtime_work(&mut gain(1.0), &spec, &changes, &lanes, 200);
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn its_parameters_are_those_of_the_model_and_bad_values_are_kept_out() {
    assert_eq!(Param::ALL.len(), 2);
    for (index, param) in Param::ALL.iter().enumerate() {
        assert_eq!(param.id().0 as usize, index);
        assert_eq!(Param::from_id(param.id()), Some(*param));
        assert!(param.is_automatable());
    }
    assert_eq!(Param::GainDb.path(), "gain_db");
    assert_eq!(Param::On.path(), "on");
    assert_eq!(Param::GainDb.info().range, (-24.0, 24.0));
    assert_eq!(Param::GainDb.info().default, 0.0);
    // Set as the settings give them, the same as new.
    let settings = Settings {
        on: true,
        gain_db: -7.5,
    };
    let spec = Spec::new(RATE, 960, Layout::Mono);
    let run = Run::new(vec![noise(30, 0.5, 960)]);
    let mut set = gain(0.0);
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    set.prepare(&spec).unwrap();
    set.reset();
    let set = columbina_audio::testing::render(&mut set, &spec, &run, |left| left);
    assert_eq!(set, process(Gain::new(&settings), &spec, &run));
    // Out of range, not a number, unknown: kept in, or ignored.
    let mut odd = gain(0.0);
    odd.set(Param::GainDb.id(), 100.0);
    odd.set(Param::GainDb.id(), f32::NAN);
    odd.set(columbina_audio::ParamId(99), 1.0);
    odd.prepare(&spec).unwrap();
    odd.reset();
    let odd = columbina_audio::testing::render(&mut odd, &spec, &run, |left| left);
    assert_eq!(odd, process(gain(24.0), &spec, &run));
    // What a gain cannot take.
    let mut node = gain(0.0);
    assert!(node.prepare(&Spec::new(RATE, 64, Layout::Empty)).is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_sidechain(1))
        .is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::discrete(24)))
        .is_ok());
}
