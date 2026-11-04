//! Pitch (p. 3971, T13): a tone moved by semitones and cents within 5 cents, the latency it
//! reports, its output independent of its blocks, settled as it says after a start in the middle,
//! its ramps, the bypass and Dry/Wet to the bit, and no memory taken on the thread of the mix.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sample numbers and levels of tests"
)]

mod common;

use columbina_audio::fx::pitch::{self, Param, Pitch, Settings};
use columbina_audio::fx::stretch::CHUNK;
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use columbina_stretch::Config;

use common::{cents, level_holds_across, peak_frequency, process, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn spec(layout: Layout) -> Spec {
    Spec::new(RATE, 512, layout)
}

fn pitch(semitones: f32, cents: f32, dry_wet: f32) -> Pitch {
    Pitch::new(&Settings {
        on: true,
        semitones,
        cents,
        dry_wet,
    })
}

const SECOND: usize = RATE as usize;

#[test]
fn twelve_semitones_move_440_hz_to_880_hz_within_5_cents() {
    let input = sine_at(440.0, 0.5, 0.0, 6 * SECOND);
    let out = process(
        pitch(12.0, 0.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(vec![input]),
    );
    // Across the point at 4 s where the stretch starts again.
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(cents(measured, 880.0).abs() < 5.0, "{measured} Hz");
    let measured = peak_frequency(&out[0][3 * SECOND..]);
    assert!(
        cents(measured, 880.0).abs() < 5.0,
        "{measured} Hz after 3 s"
    );
}

#[test]
fn semitones_and_cents_add_up() {
    let input = sine_at(440.0, 0.5, 0.0, 3 * SECOND);
    let out = process(
        pitch(-7.0, -50.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(vec![input]),
    );
    let expected = 440.0 * 2.0_f64.powf(-7.5 / 12.0);
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(
        cents(measured, expected).abs() < 5.0,
        "{measured} Hz for {expected}"
    );
    assert!((pitch::factor(12.0, 0.0) - 2.0).abs() < 1e-6);
}

#[test]
fn its_latency_is_a_chunk_a_crossfade_and_both_of_the_library_and_the_sound_is_where_it_says() {
    let config = Config::of(RATE).unwrap();
    let mut node = pitch(0.0, 0.0, 100.0);
    node.prepare(&spec(Layout::Stereo)).unwrap();
    let expected = CHUNK + 480 + config.input_latency + config.output_latency;
    assert_eq!(node.latency() as usize, expected);
    // Dry, the input delayed; wet, the stretch of no move lined up with it.
    assert_eq!(
        assert_latency(|| pitch(0.0, 0.0, 0.0), &spec(Layout::Stereo)),
        expected
    );
    assert_eq!(
        assert_latency(|| pitch(0.0, 0.0, 100.0), &spec(Layout::Stereo)),
        expected
    );
    // Setting a parameter does not move it (render checks it at each change).
    let run = Run::new(vec![noise(3, 0.2, SECOND)])
        .with_change(1_000, Param::Semitones.id(), 5.0)
        .with_change(9_000, Param::DryWet.id(), 30.0)
        .with_change(20_000, Param::On.id(), 0.0);
    let mut node = pitch(0.0, 0.0, 100.0);
    node.prepare(&spec(Layout::Mono)).unwrap();
    let _ = render(&mut node, &spec(Layout::Mono), &run, |_| 300);
}

#[test]
fn its_output_does_not_depend_on_its_blocks_across_a_point_where_the_stretch_starts_again() {
    let frames = 4 * SECOND + SECOND / 5;
    let semitones: Vec<f32> = (0..frames)
        .map(|n| if n < 2 * SECOND { 3.0 } else { -2.0 })
        .collect();
    let run = Run::new(vec![
        noise(5, 0.3, frames),
        sine_at(330.0, 0.4, 0.3, frames),
    ])
    .with_automation(Param::Semitones.id(), semitones)
    .with_change(30_000, Param::Cents.id(), 40.0)
    .with_change(100_000, Param::DryWet.id(), 60.0)
    .with_change(150_000, Param::On.id(), 0.0)
    .with_change(160_000, Param::On.id(), 1.0);
    let whole = chunk_invariance(
        || pitch(5.0, 0.0, 100.0),
        &spec(Layout::Stereo),
        &run,
        &[1, 2],
    );
    assert!(
        whole[0][SECOND..].iter().any(|s| s.abs() > 0.05),
        "it sounds"
    );
}

#[test]
fn started_in_the_middle_it_sounds_as_one_started_earlier_once_settled() {
    let settle = pitch::settle_for(&Settings::default(), RATE) as usize;
    let from = SECOND + 1_234;
    let frames = from + settle + SECOND / 2;
    let run = Run::new(vec![sine_at(220.0, 0.3, 0.0, frames)])
        .with_change(0, Param::Semitones.id(), 4.0)
        .with_change(from / 2, Param::Cents.id(), -30.0);
    let largest = assert_settle(|| pitch(0.0, 0.0, 100.0), &spec(Layout::Mono), &run, from);
    assert!(largest == 0.0, "to the bit, not {largest}");
    let mut node = pitch(0.0, 0.0, 100.0);
    node.prepare(&spec(Layout::Mono)).unwrap();
    assert_eq!(node.settle() as usize, settle);
}

#[test]
fn off_and_all_dry_it_lets_the_input_out_delayed_to_the_bit() {
    let input = noise(9, 0.5, SECOND);
    let mut node = pitch(7.0, 0.0, 0.0);
    let latency = {
        node.prepare(&spec(Layout::Mono)).unwrap();
        node.latency() as usize
    };
    let dry = process(node, &spec(Layout::Mono), &Run::new(vec![input.clone()]));
    let mut off = pitch(7.0, 0.0, 100.0);
    off.set(Param::On.id(), 0.0);
    let bypassed = process(off, &spec(Layout::Mono), &Run::new(vec![input.clone()]));
    for out in [&dry[0], &bypassed[0]] {
        assert!(out[..latency].iter().all(|&s| s == 0.0));
        let same = out[latency..]
            .iter()
            .zip(&input)
            .all(|(a, b)| a.to_bits() == b.to_bits());
        assert!(same, "the input, delayed by {latency}");
    }
}

#[test]
fn dry_wet_ramps_over_10_ms_between_the_dry_and_the_wet_sound() {
    let frames = SECOND;
    let input = vec![sine_at(500.0, 0.5, 0.0, frames)];
    let at = 20_000;
    let dry = process(
        pitch(5.0, 0.0, 0.0),
        &spec(Layout::Mono),
        &Run::new(input.clone()),
    );
    let wet = process(
        pitch(5.0, 0.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(input.clone()),
    );
    let mixed = process(
        pitch(5.0, 0.0, 0.0),
        &spec(Layout::Mono),
        &Run::new(input).with_change(at, Param::DryWet.id(), 100.0),
    );
    let ramp = 480;
    for n in at - 10..at + ramp + 10 {
        let weight = ((n + 1).saturating_sub(at) as f64 / ramp as f64).min(1.0);
        let expected =
            f64::from(dry[0][n]) + (f64::from(wet[0][n]) - f64::from(dry[0][n])) * weight;
        assert!(
            (f64::from(mixed[0][n]) - expected).abs() < 1e-6,
            "sample {n}: {} for {expected}",
            mixed[0][n]
        );
    }
}

#[test]
fn a_change_of_semitones_or_of_dry_wet_makes_no_click() {
    let frames = SECOND;
    let input = vec![sine_at(300.0, 0.5, 0.0, frames)];
    // At a peak of the tone.
    let at = 30_000 + (RATE as usize / 300) / 4;
    for (param, value) in [(Param::Semitones, 2.0), (Param::DryWet, 0.0)] {
        common::no_zipper(
            |t| match param {
                Param::Semitones => pitch(2.0 * t, 0.0, 100.0),
                _ => pitch(0.0, 0.0, 100.0 * (1.0 - t)),
            },
            &spec(Layout::Mono),
            &input,
            5,
            (param.id(), value),
            at,
        );
    }
}

#[test]
fn a_lane_that_is_not_a_number_leaves_the_setting_and_an_infinite_input_passes_within_the_settling()
{
    let frames = 6 * SECOND;
    let input = sine_at(250.0, 0.4, 0.0, frames);
    let plain = process(
        pitch(3.0, 0.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(vec![input.clone()]),
    );
    let nan = process(
        pitch(3.0, 0.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(vec![input.clone()])
            .with_automation(Param::Semitones.id(), vec![f32::NAN; frames])
            .with_automation(Param::DryWet.id(), vec![f32::INFINITY; frames]),
    );
    assert_eq!(plain, nan);
    let mut broken = input.clone();
    broken[1_000] = f32::INFINITY;
    broken[1_001] = f32::NAN;
    let out = process(
        pitch(3.0, 0.0, 100.0),
        &spec(Layout::Mono),
        &Run::new(vec![broken]),
    );
    assert!(out[0].iter().all(|s| s.is_finite()));
    let settle = pitch::settle_for(&Settings::default(), RATE) as usize;
    let mut node = pitch(3.0, 0.0, 100.0);
    node.prepare(&spec(Layout::Mono)).unwrap();
    let after = 1_001 + node.latency() as usize + settle;
    assert!(after < frames);
    let worst = out[0][after..]
        .iter()
        .zip(&plain[0][after..])
        .fold(0.0_f32, |worst, (a, b)| worst.max((a - b).abs()));
    assert!(worst < 3.2e-5, "{worst}");
}

#[test]
fn it_takes_no_memory_on_the_thread_of_the_mix() {
    let mut node = pitch(4.0, 10.0, 80.0);
    let lanes = vec![
        (Param::Semitones.id(), vec![-3.0; 480]),
        (Param::DryWet.id(), vec![50.0; 480]),
    ];
    let counted = common::realtime_work(
        &mut node,
        &spec(Layout::Stereo),
        &[
            (Param::Semitones.id(), 6.0),
            (Param::Cents.id(), -20.0),
            (Param::DryWet.id(), 40.0),
            (Param::On.id(), 0.0),
            (Param::On.id(), 1.0),
        ],
        &lanes,
        1_000,
    );
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn a_tone_keeps_its_level_where_the_stretch_starts_again() {
    for (semitones, hertz) in [(0.0, 440.0), (-3.0, 1_234.0), (7.0, 330.0)] {
        let input = sine_at(hertz, 0.5, 0.0, 10 * SECOND);
        let mut node = pitch(semitones, 0.0, 100.0);
        node.prepare(&spec(Layout::Mono)).unwrap();
        let latency = node.latency() as usize;
        let out = process(node, &spec(Layout::Mono), &Run::new(vec![input]));
        for point in [4 * SECOND + latency, 8 * SECOND + latency] {
            level_holds_across(&out[0], point, &format!("{semitones} semitones at {point}"));
        }
    }
}
