//! Ducker (T11): the case of the plan (a tone ducked by the amount within the rise, a lookahead
//! ahead of the source, held and recovered in straight lines of decibels), a source that is
//! silent or under the threshold leaving the sound to the bit, the threshold and its hysteresis,
//! the window of the RMS, the channels of the source and of the track, ramps of 10 ms, lanes,
//! the same output in any blocks, a latency of the lookahead, a settling that holds (lanes
//! slower than the settings too), no memory taken, input that is not a number, the meters, and
//! the parameters against the model's Ducker.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    reason = "exact values compared on purpose, sample numbers, levels of tests"
)]

mod common;

use columbina_audio::fx::ducker::{settle_for, Ducker, Param, Settings};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, ParamId, Spec};
use common::{amplitude_at, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Samples of `ms` milliseconds at 48 kHz.
const fn ms(ms: usize) -> usize {
    ms * 48
}

/// The lookahead, the window of the RMS, the rise, the hold and the recovery of a new Ducker, in
/// samples at 48 kHz: 15, 10, 10, 150 and 750 ms (p. 3941; C-X2).
const L: usize = ms(15);
const W: usize = ms(10);
const R: usize = ms(10);
const H: usize = ms(150);
const C: usize = ms(750);

/// The level of the target of the gain tests: a constant, so that the gain is out / in at any
/// sample.
const LEVEL: f32 = 0.25;

/// A Ducker on, 12 dB deep, the rest as a new one, then `edit`.
fn ducker(edit: impl FnOnce(&mut Settings)) -> Ducker {
    let mut settings = Settings {
        on: true,
        amount_db: 12.0,
        ..Settings::default()
    };
    edit(&mut settings);
    Ducker::new(&settings)
}

/// A 1 kHz sine whose RMS is `db` dBFS.
fn sine_rms(db: f64, frames: usize) -> Vec<f32> {
    sine_at(
        1_000.0,
        10_f64.powf(db / 20.0) * std::f64::consts::SQRT_2,
        0.0,
        frames,
    )
}

/// White noise whose RMS is `db` dBFS, from `seed`.
fn noise_rms(db: f64, seed: u64, frames: usize) -> Vec<f32> {
    noise(seed, (10_f64.powf(db / 20.0) * 3_f64.sqrt()) as f32, frames)
}

/// `signal` but silent outside `range`.
fn only(mut signal: Vec<f32>, range: std::ops::Range<usize>) -> Vec<f32> {
    for (n, sample) in signal.iter_mut().enumerate() {
        if !range.contains(&n) {
            *sample = 0.0;
        }
    }
    signal
}

/// The output of `node` for a mono track at [`LEVEL`] keyed by the mono `source`, processed
/// whole.
fn ducked(node: Ducker, source: &[f32]) -> Vec<f32> {
    let frames = source.len();
    let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![vec![LEVEL; frames]]).with_sidechain(vec![source.to_vec()]);
    process(node, &spec, &run).swap_remove(0)
}

/// The gain at each sample of an output of a track at [`LEVEL`], from the latency `latency` on
/// (0 before it), checking that the samples before it are silent.
fn gains(out: &[f32], latency: usize) -> Vec<f64> {
    assert!(
        out[..latency].iter().all(|&s| s.to_bits() == 0),
        "the lookahead is silent"
    );
    out.iter()
        .enumerate()
        .map(|(n, &s)| {
            if n < latency {
                0.0
            } else {
                f64::from(s) / f64::from(LEVEL)
            }
        })
        .collect()
}

fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

/// The gain of `db` decibels.
fn gain(db: f64) -> f64 {
    10_f64.powf(db / 20.0)
}

/// The output is the input delayed by `latency`, to the bit.
fn assert_delayed(out: &[Vec<f32>], input: &[Vec<f32>], latency: usize) {
    for (channel, (out, input)) in out.iter().zip(input).enumerate() {
        for n in 0..out.len() {
            let want = if n < latency { 0.0 } else { input[n - latency] };
            assert_eq!(
                out[n].to_bits(),
                want.to_bits(),
                "channel {channel}, sample {n}: {} for {want}",
                out[n]
            );
        }
    }
}

/// The last sample whose 10 ms RMS of `source` is at or above `db` dBFS, worked out here in
/// `f64` from the definition, apart from the node.
fn last_at_or_above(source: &[f32], db: f64) -> usize {
    let threshold = 10_f64.powf(db / 10.0);
    (0..source.len())
        .rev()
        .find(|&n| {
            let from = (n + 1).saturating_sub(W);
            let sum: f64 = source[from..=n].iter().map(|&x| f64::from(x).powi(2)).sum();
            sum / W as f64 >= threshold
        })
        .expect("the source is heard somewhere")
}

// --- The case of the plan ----------------------------------------------------------------------

#[test]
fn speech_ducks_the_music_by_the_amount_within_the_rise_a_lookahead_ahead_and_lets_it_back() {
    // The plan: noise at −10 dBFS in the source from 1.000 s to 2.000 s, a Ducker 12 dB deep,
    // the rest as a new one. The output of the node is the track's 15 ms late; the graph takes
    // that latency off, so a sample n out of the node is sample n − L of the timeline.
    let frames = 3_200 * 48;
    let source = only(noise_rms(-10.0, 3, frames), 48_000..96_000);
    let gains = gains(&ducked(ducker(|_| {}), &source), L);
    let depth = gain(-12.0);

    // The attenuation starts with the source (within 1 ms), 15 ms ahead of it on the timeline,
    // and goes down to −12 dB in a straight line of decibels over 10 ms.
    let first = (L..frames).find(|&n| gains[n] != 1.0).unwrap();
    assert!((48_000..=48_000 + ms(1)).contains(&first), "{first}");
    let timeline = (first - L) as f64 / 48_000.0;
    assert!((timeline - 0.985).abs() <= 0.001, "{timeline}");
    for k in 0..R {
        let want = -12.0 * (k + 1) as f64 / R as f64;
        let got = db(gains[first + k]);
        assert!((got - want).abs() <= 1e-3, "rise, {k}: {got} for {want}");
    }
    assert!((db(gains[first + R]) + 12.0).abs() <= 0.1);

    // Held at the amount while the source sounds and for 150 ms after its 10 ms RMS falls under
    // the threshold less the hysteresis (−48 dBFS), its window empty 10 ms after the source stops.
    let last = last_at_or_above(&source, -48.0);
    assert!((96_000 + W - ms(1)..96_000 + W).contains(&last), "{last}");
    let back = (first + R..frames)
        .find(|&n| gains[n] > depth * 1.000_001)
        .unwrap();
    assert!(
        back.abs_diff(last + H + 1) <= 2,
        "{back}, the source last heard at {last}"
    );
    for n in first + R..back {
        assert!(
            (db(gains[n]) + 12.0).abs() <= 1e-4,
            "held, {n}: {}",
            db(gains[n])
        );
    }
    let timeline = (back - L) as f64 / 48_000.0;
    assert!(
        (timeline - (2.0 + 0.010 + 0.150 - 0.015)).abs() <= 0.001,
        "{timeline}"
    );

    // Back to 0 dB in a straight line of decibels over 750 ms, −6 at its middle.
    for k in 0..C - 1 {
        let want = -12.0 * (1.0 - (k + 1) as f64 / C as f64);
        let got = db(gains[back + k]);
        assert!(
            (got - want).abs() <= 1e-3,
            "recovery, {k}: {got} for {want}"
        );
    }
    assert!((db(gains[back + C / 2 - 1]) + 6.0).abs() <= 0.2);
    for n in back + C..frames {
        assert_eq!(gains[n], 1.0, "recovered, {n}");
    }
}

#[test]
fn a_tone_at_minus_20_dbfs_comes_out_at_minus_32_while_the_source_speaks() {
    let frames = 3_200 * 48;
    let source = only(noise_rms(-10.0, 4, frames), 48_000..96_000);
    let tone = sine_at(1_000.0, 0.1, 0.0, frames);
    let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![tone]).with_sidechain(vec![source]);
    let out = process(ducker(|_| {}), &spec, &run).swap_remove(0);
    // Whole periods of the tone, on the timeline (the output less the lookahead).
    let level = |from: f64, to: f64| {
        let at = |s: f64| (s * 48_000.0) as usize + L;
        db(amplitude_at(&out[at(from)..at(to)], 1_000.0))
    };
    assert!(
        (level(0.2, 0.9) + 20.0).abs() <= 0.01,
        "{}",
        level(0.2, 0.9)
    );
    assert!((level(1.0, 2.0) + 32.0).abs() <= 0.1, "{}", level(1.0, 2.0));
    assert!(
        (level(2.95, 3.15) + 20.0).abs() <= 0.01,
        "{}",
        level(2.95, 3.15)
    );
}

// --- What ducks and what does not ----------------------------------------------------------------

#[test]
fn a_silent_source_leaves_the_sound_delayed_to_the_bit() {
    let frames = 20_000;
    let input = vec![noise(1, 0.9, frames), noise(2, 0.9, frames)];
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    let run = Run::new(input.clone()).with_sidechain(vec![vec![0.0; frames]]);
    let out = process(ducker(|_| {}), &spec, &run);
    assert_delayed(&out, &input, L);
    // And with no source at all.
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    let out = process(ducker(|_| {}), &spec, &Run::new(input.clone()));
    assert_delayed(&out, &input, L);
}

#[test]
fn a_reset_in_the_middle_of_speech_forgets_it() {
    // Reset while the source is loud, then a silent source: nothing of it is heard again.
    let frames = 10_000;
    let input = vec![noise(3, 0.9, frames)];
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let mut node = ducker(|_| {});
    node.prepare(&spec).unwrap();
    let loud = Run::new(input.clone()).with_sidechain(vec![noise_rms(-10.0, 4, frames)]);
    columbina_audio::testing::render(&mut node, &spec, &loud, |_| 1_000);
    node.reset();
    let silent = Run::new(input.clone()).with_sidechain(vec![vec![0.0; frames]]);
    let out = columbina_audio::testing::render(&mut node, &spec, &silent, |_| 1_000);
    assert_delayed(&out, &input, L);
}

#[test]
fn a_source_ducks_at_minus_45_dbfs_of_rms_and_not_under() {
    let frames = 24_000;
    let quiet = ducked(ducker(|_| {}), &sine_rms(-45.1, frames));
    assert!(quiet[L..].iter().all(|&s| s == LEVEL), "−45.1 dBFS ducks");
    let loud = gains(&ducked(ducker(|_| {}), &sine_rms(-44.9, frames)), L);
    assert!(
        (db(loud[frames - 1]) + 12.0).abs() <= 1e-4,
        "−44.9 dBFS does not duck"
    );
    // Between the threshold and the threshold less the hysteresis, from silence: not yet.
    let between = ducked(ducker(|_| {}), &sine_rms(-46.5, frames));
    assert!(between[L..].iter().all(|&s| s == LEVEL));
}

/// A source that changes from `high` to `low` dBFS of RMS and back every 100 ms, a tone that does
/// not break between them; and the gains of a Ducker of no hold, a rise of 10 ms and a recovery
/// of 50 ms keyed by it.
fn hovering(high: f64, low: f64) -> (Vec<f64>, usize) {
    let frames = 2_000 * 48;
    let segment = ms(100);
    let (high, low) = (gain(high), gain(low));
    let source: Vec<f32> = sine_rms(0.0, frames)
        .into_iter()
        .enumerate()
        .map(|(n, x)| {
            x * if (n / segment).is_multiple_of(2) {
                high
            } else {
                low
            } as f32
        })
        .collect();
    let node = ducker(|s| {
        s.hold_ms = 0.0;
        s.recovery_ms = 50.0;
    });
    (gains(&ducked(node, &source), L), segment)
}

#[test]
fn the_hysteresis_keeps_a_source_that_hovers_at_the_threshold_from_chattering() {
    let depth = gain(-12.0);
    // Between −44 and −47.9 dBFS the Ducker, once down, stays down.
    let (gains, _) = hovering(-44.0, -47.9);
    let down = (L..gains.len()).find(|&n| gains[n] <= depth).unwrap();
    assert!(down < W + R + ms(1), "{down}");
    for n in down..gains.len() {
        assert!((db(gains[n]) + 12.0).abs() <= 1e-4, "{n}: {}", db(gains[n]));
    }
    // Between −44 and −48.1 it lets the sound back up in each quiet stretch: 3 dB of hysteresis.
    let (gains, segment) = hovering(-44.0, -48.1);
    for quiet in (1..20).step_by(2) {
        let stretch = &gains[quiet * segment..(quiet + 1) * segment];
        let highest = stretch.iter().copied().fold(0.0, f64::max);
        assert_eq!(highest, 1.0, "the quiet stretch {quiet}");
    }
}

/// Speech (noise at −10 dBFS) until sample 20000, then room tone at −46.5 dBFS, inside the
/// hysteresis, for the rest of `frames`.
fn speech_then_room_tone(frames: usize) -> Vec<f32> {
    let speech = noise_rms(-10.0, 70, frames);
    let tone = sine_rms(-46.5, frames);
    (0..frames)
        .map(|n| if n < 20_000 { speech[n] } else { tone[n] })
        .collect()
}

#[test]
fn room_tone_inside_the_hysteresis_lets_go_half_a_second_after_the_threshold_was_last_reached() {
    // The hysteresis does not hold the sources as sounding for ever: an export that played the
    // speech and a preview started in the pause after it sound alike once the pause is 0.5 s on
    // (T11's review, P1).
    let frames = 200_000;
    let source = speech_then_room_tone(frames);
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![noise(71, 0.5, frames)]).with_sidechain(vec![source.clone()]);
    for from in [30_000, 60_000, 90_000] {
        assert_settle(|| ducker(|_| {}), &spec, &run, from);
    }
    // Held as sounding for 0.5 s after the window last reached −45 dBFS, then the hold, then up.
    let last_on = last_at_or_above(&source, -45.0);
    let gains = gains(&ducked(ducker(|_| {}), &source), L);
    let depth = gain(-12.0);
    let back = (last_on..frames)
        .find(|&n| gains[n] > depth * 1.000_001)
        .unwrap();
    assert!(
        back.abs_diff(last_on + ms(500) + H + 1) <= 2,
        "{back}, last on at {last_on}"
    );
    assert_eq!(gains[back + C + 1], 1.0);
}

#[test]
fn every_duck_goes_all_the_way_down_so_a_source_that_keeps_toggling_settles() {
    // A rise of 1 s and no hold, the source speaking 100 ms in every 300: the hold is at least the
    // rise, so the level reaches the bottom and forgets where it started (T11's review, P2-3).
    let settings = |s: &mut Settings| {
        s.rise_ms = 1_000.0;
        s.hold_ms = 0.0;
        s.recovery_ms = 2_000.0;
    };
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let mut probe = ducker(settings);
    probe.prepare(&spec).unwrap();
    let settle = probe.settle() as usize;
    let frames = ms(2_600) + settle + 1_000;
    let speech = noise_rms(-10.0, 72, frames);
    let source: Vec<f32> = (0..frames)
        .map(|n| {
            if n % ms(300) < ms(100) {
                speech[n]
            } else {
                0.0
            }
        })
        .collect();
    let run = Run::new(vec![noise(73, 0.5, frames)]).with_sidechain(vec![source]);
    for from in [ms(1_500), ms(2_600)] {
        assert_settle(|| ducker(settings), &spec, &run, from);
    }
}

#[test]
fn the_rms_is_taken_over_10_ms() {
    // One sample of 0.13 makes a mean square of 0.13² / 480 over 10 ms, −44.53 dBFS: it ducks,
    // for the 480 samples that it is in the window; one of 0.12 makes −45.23 and does not.
    let frames = 6_000;
    let node = || {
        ducker(|s| {
            s.rise_ms = 1.0;
            s.hold_ms = 0.0;
            s.recovery_ms = 10.0;
        })
    };
    let gains = gains(&ducked(node(), &only(vec![0.13; frames], 1_000..1_001)), L);
    let depth = gain(-12.0);
    assert!(gains[999] == 1.0 && gains[1_000] < 1.0);
    assert!((db(gains[1_000 + ms(1) - 1]) + 12.0).abs() <= 1e-4);
    // Heard for the 480 samples of the window, then held for the rise of 1 ms (the hold is
    // at least the rise), then up.
    assert!((db(gains[1_000 + W + ms(1) - 1]) + 12.0).abs() <= 1e-4);
    assert!(
        gains[1_000 + W + ms(1)] > depth * 1.000_001,
        "the window is 480 samples"
    );
    let out = ducked(node(), &only(vec![0.12; frames], 1_000..1_001));
    assert!(out[L..].iter().all(|&s| s == LEVEL));
}

#[test]
fn the_source_is_heard_by_its_loudest_channel() {
    // −44 dBFS in one channel of two: −47 in the mean of their powers, which would not duck.
    let frames = 24_000;
    for loud in 0..2 {
        let mut source = vec![vec![0.0; frames]; 2];
        source[loud] = sine_rms(-44.0, frames);
        let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(2);
        let run = Run::new(vec![vec![LEVEL; frames]]).with_sidechain(source);
        let gains = gains(&process(ducker(|_| {}), &spec, &run)[0], L);
        assert!(
            (db(gains[frames - 1]) + 12.0).abs() <= 1e-4,
            "channel {loud}"
        );
    }
}

#[test]
fn every_channel_of_the_track_is_ducked_alike_the_lfe_too() {
    let frames = 12_000;
    let input: Vec<Vec<f32>> = (0..6).map(|c| noise(10 + c, 0.5, frames)).collect();
    let spec = Spec::new(RATE, 4_096, Layout::Surround51).with_sidechain(1);
    let run = Run::new(input.clone())
        .with_sidechain(vec![only(noise_rms(-10.0, 5, frames), 3_000..frames)]);
    let out = process(ducker(|_| {}), &spec, &run);
    let depth = gain(-12.0);
    for (channel, (out, input)) in out.iter().zip(&input).enumerate() {
        for n in 3_000 + R + ms(1)..frames {
            let want = (f64::from(input[n - L]) * depth) as f32;
            assert!(
                (out[n] - want).abs() <= 1e-6,
                "channel {channel}, {n}: {} for {want}",
                out[n]
            );
        }
    }
}

// --- Changes while it plays --------------------------------------------------------------------

/// The gains of a Ducker of `settings`, keyed by noise at −10 dBFS from sample 1000 on, `run`
/// edited by `edit`.
fn changed(settings: impl FnOnce(&mut Settings), edit: impl FnOnce(Run) -> Run) -> Vec<f64> {
    let frames = 20_000;
    let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![vec![LEVEL; frames]])
        .with_sidechain(vec![only(noise_rms(-10.0, 6, frames), 1_000..frames)]);
    gains(&process(ducker(settings), &spec, &edit(run))[0], L)
}

#[test]
fn a_change_of_the_amount_ramps_over_10_ms_in_the_gain() {
    let gains = changed(
        |_| {},
        |run| run.with_change(10_000, Param::AmountDb.id(), 24.0),
    );
    let (from, to) = (gain(-12.0), gain(-24.0));
    assert!((gains[9_999] - from).abs() <= 1e-6);
    let most = (from - to) / ms(10) as f64 * 1.000_1 + 1e-7;
    for n in 9_999..10_000 + ms(10) {
        let step = gains[n] - gains[n + 1];
        assert!(
            step >= -1e-7 && step <= most,
            "{n}: a step of {step}, at most {most}"
        );
    }
    // Straight in the gain: half way at half the time.
    assert!((gains[10_000 + ms(5) - 1] - f64::midpoint(from, to)).abs() <= 1e-4);
    assert!((gains[10_000 + ms(10)] - to).abs() <= 1e-6);
}

#[test]
fn switching_it_on_crossfades_over_10_ms() {
    let gains = changed(
        |s| s.on = false,
        |run| run.with_change(10_000, Param::On.id(), 1.0),
    );
    assert!(
        gains[L..10_000].iter().all(|&g| g == 1.0),
        "off, the sound is left as it is"
    );
    let depth = gain(-12.0);
    let most = (1.0 - depth) / ms(10) as f64 * 1.000_1 + 1e-7;
    for n in 9_999..10_000 + ms(10) {
        let step = gains[n] - gains[n + 1];
        assert!(step >= -1e-7 && step <= most, "{n}: a step of {step}");
    }
    assert!((gains[10_000 + ms(10)] - depth).abs() <= 1e-6);
}

#[test]
fn lanes_are_taken_at_each_sample_and_a_lane_that_is_no_number_keeps_the_setting() {
    let frames = 20_000;
    // Amount: deeper at once where its lane says so.
    let gains = changed(
        |_| {},
        |run| run.with_automation(Param::AmountDb.id(), vec![24.0; frames]),
    );
    assert!((db(gains[frames - 1]) + 24.0).abs() <= 1e-4);
    // On, where the setting is off.
    let gains = changed(
        |s| s.on = false,
        |run| run.with_automation(Param::On.id(), vec![1.0; frames]),
    );
    assert!((db(gains[frames - 1]) + 12.0).abs() <= 1e-4);
    // The rise read from its lane: 100 ms, not the 10 set.
    let gains = changed(
        |_| {},
        |run| run.with_automation(Param::RiseMs.id(), vec![100.0; frames]),
    );
    let first = (L..frames).find(|&n| gains[n] < 1.0).unwrap();
    let want = -12.0 * (ms(50) as f64) / ms(100) as f64;
    assert!((db(gains[first + ms(50) - 1]) - want).abs() <= 1e-3);
    // A lane that is not a number, and one of the lookahead, change nothing.
    let plain = changed(|_| {}, |run| run);
    for (param, value) in [
        (Param::AmountDb, f32::NAN),
        (Param::RiseMs, f32::INFINITY),
        (Param::On, f32::NAN),
        (Param::LookaheadMs, 50.0),
    ] {
        let laned = changed(
            |_| {},
            |run| run.with_automation(param.id(), vec![value; frames]),
        );
        assert_eq!(laned, plain, "{param:?}");
    }
}

/// A source that sounds from 0 to 0.1 s and from 0.3 to 0.32 s, quiet after.
fn bursts(frames: usize) -> Vec<f32> {
    let noise = noise_rms(-10.0, 7, frames);
    noise
        .iter()
        .enumerate()
        .map(|(n, &x)| {
            if n < ms(100) || (ms(300)..ms(320)).contains(&n) {
                x
            } else {
                0.0
            }
        })
        .collect()
}

#[test]
fn the_hold_and_the_recovery_are_read_from_their_lanes() {
    let frames = 30_000;
    let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(1);
    let source = only(noise_rms(-10.0, 8, frames), 0..ms(100));
    let last = last_at_or_above(&source, -48.0);
    let run = Run::new(vec![vec![LEVEL; frames]]).with_sidechain(vec![source]);
    let depth = gain(-12.0);
    // No hold, which is a hold of the rise (10 ms), a recovery of 100 ms: up from the rise after
    // the source is last heard, −6 dB half way.
    let run = run
        .with_automation(Param::HoldMs.id(), vec![0.0; frames])
        .with_automation(Param::RecoveryMs.id(), vec![100.0; frames]);
    let gains = gains(&process(ducker(|_| {}), &spec, &run)[0], L);
    assert!((db(gains[last + R]) + 12.0).abs() <= 1e-4);
    assert!(gains[last + R + 1] > depth * 1.000_001);
    assert!((db(gains[last + R + ms(50)]) + 6.0).abs() <= 1e-3);
    assert_eq!(gains[last + R + ms(100) + 1], 1.0);
}

#[test]
fn a_hold_cut_short_ends_at_once_and_one_made_longer_after_it_ended_does_not_duck_again() {
    let frames = 30_000;
    let spec = Spec::new(RATE, 4_096, Layout::Mono).with_sidechain(1);
    let source = only(noise_rms(-10.0, 9, frames), 0..ms(100));
    let last = last_at_or_above(&source, -48.0);
    let depth = gain(-12.0);
    let run = Run::new(vec![vec![LEVEL; frames]]).with_sidechain(vec![source]);
    // 50 ms into a hold of 150, the hold set to 10 ms: it has held longer, and lets go.
    let at = last + 1 + ms(50);
    let cut = run.clone().with_change(at, Param::HoldMs.id(), 10.0);
    let held = gains(&process(ducker(|_| {}), &spec, &cut)[0], L);
    assert!((db(held[at - 1]) + 12.0).abs() <= 1e-4);
    assert!(held[at] > depth * 1.000_001);
    // No hold (the rise of 10 ms), then 4 s of it 5 ms into the recovery: it goes on up.
    let longer = run.with_change(0, Param::HoldMs.id(), 0.0).with_change(
        last + R + 1 + ms(5),
        Param::HoldMs.id(),
        4_000.0,
    );
    let rising = gains(
        &process(ducker(|s| s.recovery_ms = 100.0), &spec, &longer)[0],
        L,
    );
    assert!(rising[last + R + 1] > depth * 1.000_001);
    assert!(rising[last + R + 1..]
        .windows(2)
        .all(|pair| pair[1] >= pair[0]));
    assert_eq!(rising[frames - 1], 1.0);
}

#[test]
fn the_output_does_not_depend_on_its_blocks() {
    let frames = 30_000;
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(2);
    let run = Run::new(vec![noise(20, 0.5, frames), noise(21, 0.5, frames)])
        .with_sidechain(vec![
            bursts(frames),
            only(noise(22, 0.01, frames), 9_000..12_000),
        ])
        .with_change(2_000, Param::AmountDb.id(), 20.0)
        .with_change(6_100, Param::RiseMs.id(), 3.0)
        .with_change(9_000, Param::On.id(), 0.0)
        .with_change(9_300, Param::On.id(), 1.0)
        .with_change(12_000, Param::HoldMs.id(), 20.0)
        .with_change(13_000, Param::RecoveryMs.id(), 40.0)
        .with_change(14_000, Param::LookaheadMs.id(), 60.0)
        .with_automation(
            Param::AmountDb.id(),
            (0..frames)
                .map(|n| 6.0 + 18.0 * (n as f32 / frames as f32))
                .collect(),
        )
        .with_automation(
            Param::HoldMs.id(),
            (0..frames)
                .map(|n| if n < 20_000 { 30.0 } else { 0.0 })
                .collect(),
        );
    chunk_invariance(|| ducker(|_| {}), &spec, &run, &[1, 2, 3]);
}

// --- Latency, settling, memory -------------------------------------------------------------------

#[test]
fn the_latency_is_the_lookahead_to_the_sample() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    assert_eq!(assert_latency(|| ducker(|_| {}), &spec), L);
    assert_eq!(
        assert_latency(|| ducker(|s| s.lookahead_ms = 0.0), &spec),
        0
    );
    assert_eq!(
        assert_latency(|| ducker(|s| s.lookahead_ms = 100.0), &spec),
        ms(100)
    );
    // 15 ms at 44.1 kHz is 661.5 samples: 662.
    let mut node = ducker(|_| {});
    node.prepare(&Spec::new(44_100, 1_024, Layout::Mono))
        .unwrap();
    assert_eq!(node.latency(), 662);
    // Set while it plays, the lookahead waits for a new graph.
    node.set(Param::LookaheadMs.id(), 50.0);
    assert_eq!(node.latency(), 662);
    assert!(!Param::LookaheadMs.is_automatable());
}

#[test]
fn the_settling_of_a_new_ducker_is_its_ramp_lookahead_window_hold_rise_and_recovery() {
    let settings = Settings::default();
    assert_eq!(
        settle_for(&settings, RATE) as usize,
        ms(10) + L + W + ms(500) + H + R + C
    );
    let most = Settings {
        lookahead_ms: 100.0,
        rise_ms: 1_000.0,
        hold_ms: 4_000.0,
        recovery_ms: 4_000.0,
        ..settings
    };
    assert_eq!(
        settle_for(&most, RATE) as usize,
        ms(10 + 100 + 10 + 500 + 4_000 + 1_000 + 4_000)
    );
    // A hold shorter than the rise counts as the rise.
    let short = Settings {
        rise_ms: 1_000.0,
        hold_ms: 0.0,
        ..settings
    };
    assert_eq!(
        settle_for(&short, RATE) as usize,
        ms(10) + L + W + ms(500) + ms(1_000) + ms(1_000) + C
    );
    // Off, it settles as on: a lane may turn it on.
    let off = Settings {
        on: false,
        ..settings
    };
    assert_eq!(settle_for(&off, RATE), settle_for(&settings, RATE));
}

#[test]
fn a_run_started_late_sounds_as_one_from_earlier_after_the_settling() {
    let frames = 200_000;
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    let source: Vec<f32> = noise_rms(-10.0, 30, frames)
        .into_iter()
        .enumerate()
        .map(|(n, x)| {
            let on = (10_000..30_000).contains(&n)
                || (60_000..62_000).contains(&n)
                || (100_000..140_000).contains(&n);
            if on {
                x
            } else {
                0.0
            }
        })
        .collect();
    let run =
        Run::new(vec![noise(31, 0.5, frames), noise(32, 0.5, frames)]).with_sidechain(vec![source]);
    // In the source, in the hold, in the recovery, just as the source starts again.
    for from in [15_000, 31_000, 40_000, 59_990, 61_000] {
        assert_settle(|| ducker(|_| {}), &spec, &run, from);
    }
}

/// A ducker that reports the settling of other settings: the graph's, from the worst values of
/// its lanes (T4b).
struct Worst(Ducker, Settings);

impl Node for Worst {
    fn prepare(&mut self, spec: &Spec) -> Result<(), columbina_audio::Unsupported> {
        self.0.prepare(spec)
    }
    fn process(
        &mut self,
        ctx: &columbina_audio::Block,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
    ) {
        self.0.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.0.latency()
    }
    fn settle(&self) -> u32 {
        settle_for(&self.1, RATE)
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn set(&mut self, param: ParamId, value: f32) {
        self.0.set(param, value);
    }
}

#[test]
fn lanes_slower_than_the_settings_settle_by_settle_for_of_their_worst_values() {
    // Off and quick as set; its lanes turn it on, rise over 1 s and hold for 2 s.
    let set = Settings {
        on: false,
        amount_db: 12.0,
        rise_ms: 10.0,
        hold_ms: 0.0,
        recovery_ms: 50.0,
        ..Settings::default()
    };
    let worst = Settings {
        on: true,
        rise_ms: 1_000.0,
        hold_ms: 2_000.0,
        ..set
    };
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let mut probe = Ducker::new(&set);
    probe.prepare(&spec).unwrap();
    assert!(probe.settle() < settle_for(&worst, RATE) / 5);
    let frames = 250_000;
    let run = Run::new(vec![noise(40, 0.5, frames)])
        .with_sidechain(vec![only(noise_rms(-10.0, 41, frames), 20_000..100_000)])
        .with_automation(Param::On.id(), vec![1.0; frames])
        .with_automation(Param::RiseMs.id(), vec![1_000.0; frames])
        .with_automation(Param::HoldMs.id(), vec![2_000.0; frames]);
    assert_settle(|| Worst(Ducker::new(&set), worst), &spec, &run, 60_000);
}

#[test]
fn it_takes_no_memory_while_it_plays() {
    let spec = Spec::new(RATE, 512, Layout::Surround51).with_sidechain(2);
    let mut node = ducker(|_| {});
    let lanes = vec![
        (Param::AmountDb.id(), vec![20.0; 512]),
        (Param::HoldMs.id(), vec![30.0; 512]),
        (Param::On.id(), vec![1.0; 512]),
    ];
    let changes = [
        (Param::AmountDb.id(), 25.0),
        (Param::On.id(), 0.0),
        (Param::RiseMs.id(), 50.0),
        (Param::HoldMs.id(), 10.0),
        (Param::RecoveryMs.id(), 100.0),
        (Param::LookaheadMs.id(), 50.0),
    ];
    let counted = realtime_work(&mut node, &spec, &changes, &lanes, 300);
    assert!(counted.none(), "{counted:?}");
    let _ = node.meter().read();
}

// --- Input that is not a number --------------------------------------------------------------------

#[test]
fn a_source_that_is_not_a_number_is_silence_and_so_is_none() {
    let frames = 20_000;
    let clean = only(noise_rms(-10.0, 50, frames), 8_000..12_000);
    let mut dirty = clean.clone();
    for (n, x) in [
        (2_000, f32::INFINITY),
        (2_001, f32::NAN),
        (15_000, f32::NEG_INFINITY),
    ] {
        dirty[n] = x;
    }
    assert_eq!(
        ducked(ducker(|_| {}), &dirty),
        ducked(ducker(|_| {}), &clean)
    );
}

#[test]
fn a_sample_of_the_track_that_is_not_a_number_leaves_with_the_lookahead() {
    let frames = 10_000;
    let clean = noise(51, 0.5, frames);
    let mut dirty = clean.clone();
    dirty[3_000] = f32::INFINITY;
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let source = vec![only(noise_rms(-10.0, 52, frames), 2_000..5_000)];
    let out = |input: Vec<f32>| {
        let run = Run::new(vec![input]).with_sidechain(source.clone());
        process(ducker(|_| {}), &spec, &run).swap_remove(0)
    };
    let (clean, dirty) = (out(clean), out(dirty));
    assert!(dirty[3_000 + L].is_infinite());
    assert_eq!(dirty[3_000 + L + 1..], clean[3_000 + L + 1..]);
}

// --- What the window reads, and the parameters ------------------------------------------------------

#[test]
fn the_meters_read_the_input_the_source_and_the_reduction() {
    let frames = 24_000;
    let spec = Spec::new(RATE, 480, Layout::Mono).with_sidechain(1);
    let mut node = ducker(|_| {});
    node.prepare(&spec).unwrap();
    let meter = node.meter();
    assert_eq!(meter.read(), (-200.0, -200.0, 0.0), "before any sound");
    let run = Run::new(vec![vec![0.1; frames]]).with_sidechain(vec![sine_rms(-10.0, frames)]);
    columbina_audio::testing::render(&mut node, &spec, &run, |_| 480);
    let (input, source, reduction) = meter.read();
    assert!((input + 20.0).abs() <= 1e-3, "{input}");
    assert!((source + 10.0).abs() <= 0.01, "{source}");
    assert!((reduction + 12.0).abs() <= 1e-3, "{reduction}");
}

#[test]
fn the_meters_read_a_sample_that_is_not_a_number_as_silence() {
    let frames = 4_800;
    let spec = Spec::new(RATE, 480, Layout::Mono).with_sidechain(1);
    let mut node = ducker(|_| {});
    node.prepare(&spec).unwrap();
    let mut input = vec![0.1; frames];
    input[frames - 10] = f32::INFINITY;
    input[frames - 9] = f32::NEG_INFINITY;
    let run = Run::new(vec![input]).with_sidechain(vec![vec![0.0; frames]]);
    columbina_audio::testing::render(&mut node, &spec, &run, |_| 480);
    let (input, source, reduction) = node.meter().read();
    assert!((input + 20.0).abs() <= 1e-3, "{input}");
    assert_eq!((source, reduction), (-200.0, 0.0));
}

#[test]
fn the_parameters_are_the_fields_of_the_model_s_ducker_in_its_ranges() {
    use columbina_model::audio::fx::Ducker as Model;
    let model = Model::default();
    let settings = Settings::default();
    let paths: Vec<_> = Param::ALL.iter().map(|p| p.path()).collect();
    assert_eq!(
        paths,
        [
            "on",
            "amount_db",
            "lookahead_ms",
            "rise_ms",
            "hold_ms",
            "recovery_ms"
        ]
    );
    assert_eq!(settings.on, model.on);
    for (param, value) in [
        (Param::AmountDb, model.amount_db),
        (Param::LookaheadMs, model.lookahead_ms),
        (Param::RiseMs, model.rise_ms),
        (Param::HoldMs, model.hold_ms),
        (Param::RecoveryMs, model.recovery_ms),
    ] {
        assert_eq!(param.info().default, value as f32, "{param:?}");
        assert_eq!(settings.value(param), value as f32, "{param:?}");
        // The ends of the range are the model's: in at each end, out beyond them.
        let (low, high) = param.info().range;
        let with = |v: f64| {
            let mut m = Model::default();
            match param {
                Param::AmountDb => m.amount_db = v,
                Param::LookaheadMs => m.lookahead_ms = v,
                Param::RiseMs => m.rise_ms = v,
                Param::HoldMs => m.hold_ms = v,
                _ => m.recovery_ms = v,
            }
            m.check().is_ok()
        };
        assert!(with(f64::from(low)) && with(f64::from(high)), "{param:?}");
        assert!(
            !with(f64::from(low) - 0.01) && !with(f64::from(high) + 0.01),
            "{param:?}"
        );
    }
    let automatable: Vec<_> = Param::ALL.iter().filter(|p| p.is_automatable()).collect();
    assert_eq!(
        automatable,
        [
            &Param::On,
            &Param::AmountDb,
            &Param::RiseMs,
            &Param::HoldMs,
            &Param::RecoveryMs
        ]
    );
    for (n, param) in Param::ALL.iter().enumerate() {
        assert_eq!(param.id(), ParamId(n as u32));
        assert_eq!(Param::from_id(param.id()), Some(*param));
    }
    assert_eq!(Param::from_id(ParamId(6)), None);
}

#[test]
fn settings_set_one_by_one_sound_as_the_settings_made_at_once() {
    let settings = Settings {
        on: true,
        amount_db: 18.0,
        lookahead_ms: 15.0,
        rise_ms: 3.0,
        hold_ms: 40.0,
        recovery_ms: 90.0,
    };
    let frames = 20_000;
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![noise(60, 0.5, frames)]).with_sidechain(vec![bursts(frames)]);
    let made = process(Ducker::new(&settings), &spec, &run);
    let mut node = Ducker::default();
    let spec_whole = spec.with_max_block(frames);
    node.prepare(&spec_whole).unwrap();
    for (param, value) in settings.params() {
        node.set(param, value);
    }
    node.reset();
    let set = columbina_audio::testing::render(&mut node, &spec_whole, &run, |left| left);
    assert_eq!(made, set);
    // Out of range and not numbers are kept in range or ignored.
    let mut node = ducker(|_| {});
    node.set(Param::AmountDb.id(), 99.0);
    node.set(Param::RiseMs.id(), f32::NAN);
    node.set(ParamId(42), 1.0);
    assert_eq!(node.heard(), (30.0, 10.0, 150.0, 750.0));
}

#[test]
fn it_takes_a_track_of_any_channels_and_gives_out_the_same() {
    let mut node = Ducker::default();
    assert!(node.prepare(&Spec::new(RATE, 512, Layout::Empty)).is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 512, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
    for layout in [
        Layout::Mono,
        Layout::Stereo,
        Layout::Surround51,
        Layout::Adaptive(24),
    ] {
        assert!(node
            .prepare(&Spec::new(RATE, 512, layout).with_sidechain(23))
            .is_ok());
    }
}
