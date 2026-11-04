//! De-Hummer (T10): 50, 100, 150 and 200 Hz each lowered by 40 dB at Amount 100, a tone of 1 kHz
//! not touched, the slope's shares, each notch as wide as its Q, 60 Hz, Amount 0 and the insert
//! off to the bit, Listen to Hum Only as what is taken away, ramps of 10 ms (the frequency in its
//! logarithm, lanes too), the same output in any blocks, no memory taken, no zipper, no latency,
//! a settling that holds, input that is not a number, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::manual_midpoint,
    reason = "exact values compared on purpose, sample numbers, test signals in f32"
)]

mod common;

use columbina_audio::fx::dehummer::{depths, notches, settle_for, DeHummer, Param, Settings};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{amplitude_at, db, no_zipper, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn dehummer(edit: impl FnOnce(&mut Settings)) -> DeHummer {
    let mut settings = Settings::default();
    edit(&mut settings);
    DeHummer::new(&settings)
}

/// Hum of `fundamental` and its harmonics up to the fourth, each at −20 dBFS, and a tone of
/// 1 kHz at −20 dBFS, 4 s.
fn hum(fundamental: f64) -> Vec<f32> {
    let frames = 4 * 48_000;
    let mut signal = sine_at(1_000.0, 0.1, 0.5, frames);
    for k in 1..=4 {
        for (x, s) in signal.iter_mut().zip(sine_at(
            fundamental * f64::from(k),
            0.1,
            f64::from(k),
            frames,
        )) {
            *x += s;
        }
    }
    signal
}

/// What a De-Hummer of `edit` does to each of `freqs` in `input`, in dB, over its last 2 s.
fn changes_db(edit: impl FnOnce(&mut Settings), input: &[f32], freqs: &[f64]) -> Vec<f64> {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let output = process(dehummer(edit), &spec, &Run::new(vec![input.to_vec()])).remove(0);
    let tail = input.len() - 2 * 48_000;
    freqs
        .iter()
        .map(|&freq| db(amplitude_at(&output[tail..], freq) / amplitude_at(&input[tail..], freq)))
        .collect()
}

#[test]
fn hum_is_lowered_by_40_db_at_every_harmonic_and_1_khz_is_not_touched() {
    let input = hum(50.0);
    let changes = changes_db(
        |s| {
            s.amount = 100.0;
            s.slope = 0.5;
        },
        &input,
        &[50.0, 100.0, 150.0, 200.0, 1_000.0],
    );
    for change in &changes[..4] {
        assert!((change + 40.0).abs() < 0.3, "{changes:?}");
    }
    assert!(changes[4].abs() < 0.001, "{changes:?}");
    // 60 Hz the same.
    let changes = changes_db(
        |s| {
            s.amount = 100.0;
            s.frequency_hz = 60.0;
        },
        &hum(60.0),
        &[60.0, 120.0, 180.0, 240.0, 1_000.0],
    );
    for change in &changes[..4] {
        assert!((change + 40.0).abs() < 0.3, "{changes:?}");
    }
    assert!(changes[4].abs() < 0.001, "{changes:?}");
}

#[test]
fn the_slope_shares_the_depth_out_among_the_harmonics() {
    let input = hum(50.0);
    let freqs = [50.0, 100.0, 150.0, 200.0];
    for (slope, expected) in [
        (0.0_f64, [40.0_f64, 80.0 / 3.0, 40.0 / 3.0, 0.0]),
        (1.0, [0.0, 40.0 / 3.0, 80.0 / 3.0, 40.0]),
        (0.25, [40.0, 100.0 / 3.0, 80.0 / 3.0, 20.0]),
    ] {
        for (depth, expected) in depths(100.0, slope).iter().zip(expected) {
            assert!((depth - expected).abs() < 1e-9, "{slope}: {depth}");
        }
        let changes = changes_db(
            |s| {
                s.amount = 100.0;
                s.slope = slope as f32;
            },
            &input,
            &freqs,
        );
        for (change, depth) in changes.iter().zip(expected) {
            assert!((change + depth).abs() < 0.3, "{slope}: {changes:?}");
        }
    }
    let half = changes_db(|_| {}, &input, &freqs);
    for change in &half {
        assert!((change + 20.0).abs() < 0.2, "{half:?}");
    }
}

#[test]
fn each_notch_is_its_depth_at_its_centre_and_as_wide_as_its_q() {
    for (freq, amount, slope) in [(50.0, 100.0, 0.5), (60.0, 75.0, 0.0), (43.0, 100.0, 1.0)] {
        let depths = depths(amount, slope);
        for (k, notch) in notches(RATE, freq, amount, slope).iter().enumerate() {
            let f0 = freq * (k + 1) as f64;
            let gain = |f: f64| notch.magnitude(f64::from(RATE), f);
            assert!((db(gain(f0)) + depths[k]).abs() < 1e-6, "{f0}");
            // Far from it, nothing: under 0.005 dB an octave away.
            assert!(db(gain(2.0 * f0)).abs() < 5e-3 && db(gain(f0 / 2.0)).abs() < 5e-3);
            if depths[k] < 39.0 {
                continue;
            }
            // Deep, its points of half power are those of its band-pass, f0/Q apart.
            let edge = |mut low: f64, mut high: f64, falling: bool| {
                for _ in 0..80 {
                    let middle = 0.5 * (low + high);
                    if (gain(middle) > std::f64::consts::FRAC_1_SQRT_2) == falling {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                0.5 * (low + high)
            };
            let width = edge(f0, 2.0 * f0, false) - edge(f0 / 2.0, f0, true);
            assert!((width / (f0 / 30.0) - 1.0).abs() < 0.01, "{f0}: {width}");
        }
    }
}

#[test]
fn amount_0_and_the_insert_off_pass_the_sound_to_the_bit_and_listen_takes_the_hum() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    let input = vec![hum(50.0), noise(1, 0.5, 4 * 48_000)];
    let run = Run::new(input.clone());
    assert_eq!(process(dehummer(|s| s.amount = 0.0), &spec, &run), input);
    assert_eq!(process(dehummer(|s| s.on = false), &spec, &run), input);
    let heard = process(dehummer(|s| s.amount = 100.0), &spec, &run);
    let taken = process(
        dehummer(|s| {
            s.amount = 100.0;
            s.listen = true;
        }),
        &spec,
        &run,
    );
    for channel in 0..2 {
        for n in 0..input[channel].len() {
            assert!(
                (heard[channel][n] + taken[channel][n] - input[channel][n]).abs() < 1e-6,
                "{channel} {n}"
            );
        }
    }
    // What is taken is the hum: 99 % of each harmonic, and of 1 kHz under 2 % (the skirts of the
    // band-passes, a quarter of a turn out of phase, which leave its level within 0.001 dB).
    let tail = &taken[0][96_000..];
    for freq in [50.0, 100.0, 150.0, 200.0] {
        assert!(
            (amplitude_at(tail, freq) / 0.1 - 0.99).abs() < 0.01,
            "{freq}"
        );
    }
    assert!(amplitude_at(tail, 1_000.0) < 0.002);
    // A lane of Listen is not heard.
    let laned = process(
        dehummer(|s| s.amount = 100.0),
        &spec,
        &run.clone()
            .with_automation(Param::Listen.id(), vec![1.0; 4 * 48_000]),
    );
    assert_eq!(laned, heard);
    assert!(!Param::Listen.is_automatable());
}

#[test]
fn the_frequency_ramps_in_its_logarithm_and_lanes_ramp_the_same() {
    let spec = Spec::new(RATE, 240, Layout::Mono);
    let quiet = Run::new(vec![vec![0.0; 240]]);
    let mut node = dehummer(|s| s.frequency_hz = 40.0);
    node.prepare(&spec).unwrap();
    node.set(Param::FrequencyHz.id(), 70.0);
    node.set(Param::Amount.id(), 100.0);
    render(&mut node, &spec, &quiet, |left| left);
    let (freq, amount, slope) = node.heard();
    // Half way: √(40·70) = 52.915, not 55; the amount half way in its value.
    assert!((freq - 52.915_026).abs() < 1e-3, "{freq}");
    assert!((amount - 75.0).abs() < 1e-3, "{amount}");
    assert_eq!(slope, 0.5);
    render(&mut node, &spec, &quiet, |left| left);
    assert_eq!(node.heard(), (70.0, 100.0, 0.5));
    let mut node = dehummer(|s| s.frequency_hz = 40.0);
    node.prepare(&spec).unwrap();
    let lanes = quiet
        .clone()
        .with_automation(Param::FrequencyHz.id(), vec![70.0; 240])
        .with_automation(Param::Slope.id(), vec![1.0; 240]);
    render(&mut node, &spec, &lanes, |left| left);
    let (freq, _, slope) = node.heard();
    assert!((freq - 52.915_026).abs() < 1e-3, "{freq}");
    assert!((slope - 0.75).abs() < 1e-6, "{slope}");
}

#[test]
fn a_lane_that_turns_the_insert_off_lets_the_input_through_after_ten_ms() {
    let spec = Spec::new(RATE, 9_600, Layout::Mono);
    let input = hum(50.0)[..9_600].to_vec();
    let lane: Vec<f32> = (0..9_600)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(vec![input.clone()]).with_automation(Param::On.id(), lane);
    let output = process(dehummer(|s| s.amount = 100.0), &spec, &run).remove(0);
    assert_eq!(output[2_480..], input[2_480..]);
    assert_ne!(output[..2_000], input[..2_000]);
}

#[test]
fn every_change_makes_no_zipper() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut tone = sine_at(1_000.0, 0.3, 0.0, 48_000);
    for (x, s) in tone.iter_mut().zip(sine_at(55.0, 0.3, 0.0, 48_000)) {
        *x += s;
    }
    let tone = vec![tone];
    let at = 24_012;
    no_zipper(
        |t| dehummer(|s| s.amount = 100.0 * t),
        &spec,
        &tone,
        11,
        (Param::Amount.id(), 100.0),
        at,
    );
    no_zipper(
        |t| {
            dehummer(|s| {
                s.amount = 100.0;
                s.frequency_hz = 50.0 * 1.2_f32.powf(t);
            })
        },
        &spec,
        &tone,
        11,
        (Param::FrequencyHz.id(), 60.0),
        at,
    );
    no_zipper(
        |t| {
            dehummer(|s| {
                s.amount = 100.0;
                s.on = t < 0.5;
            })
        },
        &spec,
        &tone,
        2,
        (Param::On.id(), 0.0),
        at,
    );
}

#[test]
fn the_de_hummer_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Stereo);
    let frames = 24_000;
    let run = Run::new(vec![hum(50.0)[..frames].to_vec(), noise(2, 0.3, frames)])
        .with_change(100, Param::Amount.id(), 90.0)
        .with_change(3_000, Param::FrequencyHz.id(), 60.0)
        .with_change(5_000, Param::Slope.id(), 0.0)
        .with_change(9_000, Param::Listen.id(), 1.0)
        .with_change(9_200, Param::Listen.id(), 0.0)
        .with_change(12_000, Param::On.id(), 0.0)
        .with_change(12_100, Param::On.id(), 1.0);
    chunk_invariance(|| dehummer(|_| {}), &spec, &run, &[1, 2]);
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 55.0 + 15.0 * (n as f32 / 1_000.0).sin())
        .collect();
    let amounts: Vec<f32> = (0..frames).map(|n| (n % 101) as f32).collect();
    let automated = run
        .with_automation(Param::FrequencyHz.id(), sweep)
        .with_automation(Param::Amount.id(), amounts);
    chunk_invariance(|| dehummer(|_| {}), &spec, &automated, &[3]);
}

#[test]
fn no_latency_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    assert_eq!(assert_latency(|| dehummer(|s| s.amount = 100.0), &spec), 0);
    let at_40 = settle_for(
        &Settings {
            frequency_hz: 40.0,
            ..Settings::default()
        },
        RATE,
    );
    let at_70 = settle_for(
        &Settings {
            frequency_hz: 70.0,
            ..Settings::default()
        },
        RATE,
    );
    assert!(at_40 > at_70);
    // The same at any amount: counted at 0 dB, the slowest.
    assert_eq!(
        settle_for(
            &Settings {
                amount: 100.0,
                ..Settings::default()
            },
            RATE
        ),
        settle_for(&Settings::default(), RATE)
    );
    let settle = settle_for(&Settings::default(), RATE) as usize;
    let frames = settle + 60_000;
    let input = hum(50.0)
        .into_iter()
        .cycle()
        .take(frames)
        .collect::<Vec<_>>();
    let run = Run::new(vec![input.clone()])
        .with_change(settle - 200, Param::Amount.id(), 100.0)
        .with_change(settle + 500, Param::Slope.id(), 0.0);
    for from in [settle, settle + 30_000] {
        assert_settle(|| dehummer(|_| {}), &spec, &run, from.min(frames - 1_000));
    }
    // Set to 0 and turned up by a lane after the late start: the notches ran all along.
    let lane: Vec<f32> = (0..frames)
        .map(|n| if n < settle + 1_000 { 0.0 } else { 100.0 })
        .collect();
    let run = Run::new(vec![input]).with_automation(Param::Amount.id(), lane);
    assert_settle(|| dehummer(|s| s.amount = 0.0), &spec, &run, settle);
}

#[test]
fn input_and_lanes_that_are_not_numbers_and_the_work_of_the_thread_of_the_mix() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let settle = settle_for(&Settings::default(), RATE) as usize;
    let frames = settle + 10_000;
    let clean = hum(50.0)
        .into_iter()
        .cycle()
        .take(frames)
        .collect::<Vec<_>>();
    let mut odd = clean.clone();
    odd[300] = f32::INFINITY;
    odd[301] = f32::NAN;
    let a = process(dehummer(|_| {}), &spec, &Run::new(vec![clean.clone()])).remove(0);
    let b = process(dehummer(|_| {}), &spec, &Run::new(vec![odd])).remove(0);
    assert!(b[302..].iter().all(|x| x.is_finite()));
    for n in 302 + settle..frames {
        assert!((a[n] - b[n]).abs() < 3.2e-5, "{n}");
    }
    let nans = vec![f32::NAN; frames];
    let mut run = Run::new(vec![clean]);
    for param in Param::ALL {
        run = run.with_automation(param.id(), nans.clone());
    }
    assert_eq!(process(dehummer(|_| {}), &spec, &run).remove(0), a);

    let spec = Spec::new(RATE, 512, Layout::Surround51);
    let changes = [
        (Param::FrequencyHz.id(), 62.0),
        (Param::Amount.id(), 100.0),
        (Param::Slope.id(), 0.1),
        (Param::Listen.id(), 1.0),
        (Param::On.id(), 0.0),
    ];
    let lanes = vec![(Param::FrequencyHz.id(), vec![45.0; 480])];
    let counted = realtime_work(&mut dehummer(|_| {}), &spec, &changes, &lanes, 200);
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn its_parameters_are_those_of_the_model() {
    let paths: Vec<_> = Param::ALL.iter().map(|param| param.path()).collect();
    assert_eq!(paths, ["on", "frequency_hz", "amount", "slope", "listen"]);
    assert_eq!(Param::FrequencyHz.info().range, (40.0, 70.0));
    assert_eq!(Param::FrequencyHz.info().default, 50.0);
    assert_eq!(Param::Amount.info().default, 50.0);
    assert_eq!(Param::Slope.info().range, (0.0, 1.0));
    assert_eq!(Param::Slope.info().default, 0.5);
    let settings = Settings {
        on: true,
        frequency_hz: 59.0,
        amount: 66.0,
        slope: 0.2,
        listen: false,
    };
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![noise(12, 0.8, 4_800)]);
    let mut set = DeHummer::default();
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    assert_eq!(
        process(set, &spec, &run),
        process(DeHummer::new(&settings), &spec, &run)
    );
    let mut node = DeHummer::default();
    assert!(node.prepare(&Spec::new(RATE, 64, Layout::Empty)).is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Mono).with_sidechain(1))
        .is_err());
}
