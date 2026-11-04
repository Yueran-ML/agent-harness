//! De-Esser (T10): a burst of 6 kHz lowered by the reduction of its Amount, to the hundredth of a
//! dB, a tone of 1 kHz not touched at all, the bands against their widths and the cookbook, All
//! High as the low-pass kept, the Reaction's speeds, Listen to Ess Only as what is taken away, the
//! LFE not heard, ramps of 10 ms, the same output in any blocks, no memory taken, no zipper, no
//! latency, a settling that holds, input that is not a number, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::manual_midpoint,
    reason = "exact values compared on purpose, sample numbers, test signals in f32"
)]

mod common;

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use columbina_audio::fx::biquad::Coefficients;
use columbina_audio::fx::deesser::{
    above, band_pass, band_q, reduction_db, settle_for, DeEsser, Param, Range, Reaction, Settings,
};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{amplitude_at, db, no_zipper, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn deesser(edit: impl FnOnce(&mut Settings)) -> DeEsser {
    let mut settings = Settings::default();
    edit(&mut settings);
    DeEsser::new(&settings)
}

/// A tone of `freq` at −10 dBFS from the sample 4800 to 33600, faded in and out over 20 ms (a
/// step of a tone of 1 kHz would be a click, which is high), silence around it.
fn burst(freq: f64) -> Vec<f32> {
    let mut signal = sine_at(freq, 0.316_227_77, 0.0, 38_400);
    signal[..4_800].fill(0.0);
    signal[33_600..].fill(0.0);
    fade(&mut signal[4_800..33_600]);
    signal
}

/// `signal` faded in and out by a raised cosine over 20 ms at each end.
fn fade(signal: &mut [f32]) {
    let len = signal.len();
    for k in 0..960 {
        let gain = (0.5 - 0.5 * (std::f64::consts::PI * k as f64 / 960.0).cos()) as f32;
        signal[k] *= gain;
        signal[len - 1 - k] *= gain;
    }
}

/// What a De-Esser of `edit` does to the component of `freq` of `input` (mono) in the steady
/// part of the burst, in dB.
fn change_db(edit: impl FnOnce(&mut Settings), input: &[f32], freq: f64) -> f64 {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let output = process(deesser(edit), &spec, &Run::new(vec![input.to_vec()])).remove(0);
    db(amplitude_at(&output[14_400..31_200], freq) / amplitude_at(&input[14_400..31_200], freq))
}

#[test]
fn a_burst_of_6_khz_is_lowered_by_the_reduction_of_its_amount() {
    let ess = burst(6_000.0);
    // Alone, the band is the whole sound: the most of Amount, 20 dB at 100, 10 at 50.
    for range in [Range::Wide, Range::Narrow] {
        let full = change_db(
            |s| {
                s.amount = 100.0;
                s.range = range;
            },
            &ess,
            6_000.0,
        );
        assert!((full + 20.0).abs() < 0.05, "{range:?}: {full}");
        let half = change_db(|s| s.range = range, &ess, 6_000.0);
        assert!((half + 10.0).abs() < 0.05, "{range:?}: {half}");
    }
    // Off, or at Amount 0: to the bit.
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let run = Run::new(vec![ess.clone()]);
    assert_eq!(process(deesser(|s| s.amount = 0.0), &spec, &run)[0], ess);
    assert_eq!(process(deesser(|s| s.on = false), &spec, &run)[0], ess);
}

#[test]
fn a_tone_of_1_khz_is_not_touched() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let tone = burst(1_000.0);
    for range in Range::ALL {
        let output = process(
            deesser(|s| {
                s.amount = 100.0;
                s.range = range;
            }),
            &spec,
            &Run::new(vec![tone.clone()]),
        );
        if let Some(n) = output[0].iter().zip(&tone).position(|(a, b)| a != b) {
            panic!("{range:?} at {n}: {} {}", output[0][n], tone[n]);
        }
    }
}

/// The gain of `filter` at `freq` as a complex number.
fn response(filter: &Coefficients, freq: f64) -> (f64, f64) {
    let w = TAU * freq / f64::from(RATE);
    let at = |k: [f64; 3]| {
        (
            k[0] + k[1] * w.cos() + k[2] * (2.0 * w).cos(),
            -(k[1] * w.sin() + k[2] * (2.0 * w).sin()),
        )
    };
    let (br, bi) = at([filter.b0, filter.b1, filter.b2]);
    let (ar, ai) = at([1.0, filter.a1, filter.a2]);
    let d = ar * ar + ai * ai;
    ((br * ar + bi * ai) / d, (bi * ar - br * ai) / d)
}

/// The frequency between `low` and `high` where `filter` crosses half power.
fn half_power(filter: &Coefficients, mut low: f64, mut high: f64, rising: bool) -> f64 {
    for _ in 0..60 {
        let middle = (low * high).sqrt();
        let above = filter.magnitude(f64::from(RATE), middle) > FRAC_1_SQRT_2;
        if above == rising {
            high = middle;
        } else {
            low = middle;
        }
    }
    (low * high).sqrt()
}

#[test]
fn the_bands_are_as_wide_as_they_are_named_and_all_high_keeps_the_low_pass() {
    // A third of an octave: Q = 2^(1/6)/(2^(1/3) − 1) = 4.3185; an octave: √2.
    assert!((band_q(1.0 / 3.0) - 4.318_473_046).abs() < 1e-6);
    assert!((band_q(1.0) - std::f64::consts::SQRT_2).abs() < 1e-12);
    let rate = f64::from(RATE);
    for (octaves, f0) in [(1.0 / 3.0, 2_000.0), (1.0, 2_000.0), (1.0, 3_000.0)] {
        let filter = band_pass(rate, f0, band_q(octaves));
        let (re, im) = response(&filter, f0);
        assert!((re - 1.0).abs() < 1e-9 && im.abs() < 1e-9, "{re} {im}");
        let low = half_power(&filter, f0 / 4.0, f0, true);
        let high = half_power(&filter, f0, f0 * 4.0, false);
        // Bilinear warping narrows the band by about a percent at these frequencies.
        let width = (high / low).log2();
        assert!(
            (width / octaves - 1.0).abs() < 0.03,
            "{octaves} at {f0}: {width}"
        );
    }
    // All High: the sound less the low-pass of the cookbook, written out here.
    let f0 = 6_000.0;
    let w0 = TAU * f0 / rate;
    let alpha = w0.sin() / (2.0 * FRAC_1_SQRT_2);
    let lp = Coefficients {
        b0: (1.0 - w0.cos()) / 2.0 / (1.0 + alpha),
        b1: (1.0 - w0.cos()) / (1.0 + alpha),
        b2: (1.0 - w0.cos()) / 2.0 / (1.0 + alpha),
        a1: -2.0 * w0.cos() / (1.0 + alpha),
        a2: (1.0 - alpha) / (1.0 + alpha),
    };
    let high = above(rate, f0);
    for freq in [100.0, 1_000.0, 6_000.0, 12_000.0, 20_000.0] {
        let (hr, hi) = response(&high, freq);
        let (lr, li) = response(&lp, freq);
        assert!(
            (hr + lr - 1.0).abs() < 1e-9 && (hi + li).abs() < 1e-9,
            "{freq}"
        );
    }
    // A burst two octaves over a target of 4 kHz, at Amount 100: g·x + (1 − g)·LP(x), with the
    // high-pass hearing all of it, g of −20 dB, and the low-pass of the cookbook at 16 kHz.
    let input = burst(16_000.0);
    let measured = change_db(
        |s| {
            s.amount = 100.0;
            s.range = Range::AllHigh;
            s.frequency_hz = 4_000.0;
        },
        &input,
        16_000.0,
    );
    let w0 = TAU * 4_000.0 / rate;
    let alpha = w0.sin() / (2.0 * FRAC_1_SQRT_2);
    let lp = Coefficients {
        b0: (1.0 - w0.cos()) / 2.0 / (1.0 + alpha),
        b1: (1.0 - w0.cos()) / (1.0 + alpha),
        b2: (1.0 - w0.cos()) / 2.0 / (1.0 + alpha),
        a1: -2.0 * w0.cos() / (1.0 + alpha),
        a2: (1.0 - alpha) / (1.0 + alpha),
    };
    let (lr, li) = response(&lp, 16_000.0);
    let g = 0.1;
    let expected = db((g + (1.0 - g) * lr).hypot((1.0 - g) * li));
    assert!((measured - expected).abs() < 0.1, "{measured} {expected}");
    assert!(measured < -18.0, "{measured}");
}

#[test]
fn the_reduction_follows_how_far_the_band_stands_out() {
    assert_eq!(reduction_db(-20.0, -30.0, 100.0), 0.0);
    assert_eq!(reduction_db(-20.0, -12.0, 100.0), 0.0);
    assert!((reduction_db(-20.0, -6.0, 100.0) - 10.0).abs() < 1e-12);
    assert!((reduction_db(-20.0, -3.0, 50.0) - 7.5).abs() < 1e-12);
    assert_eq!(reduction_db(-20.0, 0.0, 100.0), 20.0);
    assert_eq!(reduction_db(-20.0, 2.0, 100.0), 20.0);
    // Far down, nothing; from −60 dBFS, all of it; half way, half.
    assert_eq!(reduction_db(-75.0, 0.0, 100.0), 0.0);
    assert_eq!(reduction_db(-60.0, 0.0, 100.0), 20.0);
    assert!((reduction_db(-65.0, 0.0, 100.0) - 10.0).abs() < 1e-12);
    // A tone of 1 kHz and a burst of 6 kHz together, each at −10 dBFS: the band (6 kHz, and
    // 1 kHz down by 18 dB) is about 6 dB under the peaks of the whole, about 10 dB of
    // reduction; and outside the bursts, the tone to the bit.
    let mut input = sine_at(1_000.0, 0.316_227_77, 0.0, 38_400);
    fade(&mut input);
    for (x, s) in input.iter_mut().zip(burst(6_000.0)) {
        *x += s;
    }
    let reduced = change_db(|s| s.amount = 100.0, &input, 6_000.0);
    assert!((-15.0..-10.0).contains(&reduced), "{reduced}");
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let output = process(
        deesser(|s| s.amount = 100.0),
        &spec,
        &Run::new(vec![input.clone()]),
    )
    .remove(0);
    assert_eq!(output[..4_800], input[..4_800]);
}

/// The reduction that a De-Esser of `reaction` reads at each millisecond of `input`.
fn reductions(reaction: Reaction, input: &[f32]) -> Vec<f32> {
    let spec = Spec::new(RATE, 48, Layout::Mono);
    let mut node = deesser(|s| {
        s.amount = 100.0;
        s.reaction = reaction;
    });
    let meter = node.meter();
    node.prepare(&spec).unwrap();
    input
        .chunks(48)
        .map(|chunk| {
            render(&mut node, &spec, &Run::new(vec![chunk.to_vec()]), |left| {
                left
            });
            meter.read().0
        })
        .collect()
}

#[test]
fn the_reaction_sets_how_soon_it_acts() {
    // A tone of 1 kHz throughout (faded in) and a burst of 6 kHz from 100 ms.
    let mut input = sine_at(1_000.0, 0.316_227_77, 0.0, 9_600);
    fade(&mut input[..4_800]);
    for (x, s) in input[4_800..]
        .iter_mut()
        .zip(sine_at(6_000.0, 0.316_227_77, 0.0, 4_800))
    {
        *x += s;
    }
    let slow = reductions(Reaction::Slow, &input);
    let medium = reductions(Reaction::Medium, &input);
    let fast = reductions(Reaction::Fast, &input);
    for reductions in [&slow, &medium, &fast] {
        assert!(reductions[199] < -8.0, "{}", reductions[199]);
        // Before the burst, nothing.
        assert!(reductions[30..100].iter().all(|r| *r == 0.0));
    }
    // 2 ms into the burst, Fast is almost there and Slow has hardly begun.
    assert!(fast[101] < 0.8 * fast[199], "{} {}", fast[101], fast[199]);
    assert!(slow[101] > 0.2 * slow[199], "{} {}", slow[101], slow[199]);
    assert!(medium[101] < slow[101] && medium[101] > fast[101]);
}

#[test]
fn listen_lets_out_what_is_taken_away_and_the_lfe_is_not_heard() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let mut input = sine_at(1_000.0, 0.3, 0.0, 38_400);
    fade(&mut input);
    for (x, s) in input.iter_mut().zip(burst(7_000.0)) {
        *x += s;
    }
    let run = Run::new(vec![input.clone()]);
    let heard = process(deesser(|s| s.amount = 80.0), &spec, &run).remove(0);
    let taken = process(
        deesser(|s| {
            s.amount = 80.0;
            s.listen = true;
        }),
        &spec,
        &run,
    )
    .remove(0);
    for n in 0..input.len() {
        assert!((heard[n] + taken[n] - input[n]).abs() < 1e-6, "{n}");
    }
    assert!(amplitude_at(&taken[14_400..33_600], 7_000.0) > 0.1);
    // A lane of Listen is not heard.
    let laned = process(
        deesser(|s| s.amount = 80.0),
        &spec,
        &run.clone()
            .with_automation(Param::Listen.id(), vec![1.0; input.len()]),
    )
    .remove(0);
    assert_eq!(laned, heard);
    assert!(!Param::Listen.is_automatable());
    // Ess on the LFE of 5.1 alone: not heard, nothing lowered.
    let spec = Spec::new(RATE, 1_024, Layout::Surround51);
    let mut channels = vec![vec![0.0_f32; 38_400]; 6];
    channels[3] = burst(6_000.0);
    channels[0] = burst(1_000.0);
    let output = process(
        deesser(|s| s.amount = 100.0),
        &spec,
        &Run::new(channels.clone()),
    );
    assert_eq!(output, channels);
}

#[test]
fn every_change_makes_no_zipper() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut tone = sine_at(6_000.0, 0.3, 0.3, 24_000);
    for (x, s) in tone.iter_mut().zip(sine_at(1_000.0, 0.3, 0.0, 24_000)) {
        *x += s;
    }
    let tone = vec![tone];
    let at = 12_012;
    no_zipper(
        |t| deesser(|s| s.amount = 100.0 * t),
        &spec,
        &tone,
        11,
        (Param::Amount.id(), 100.0),
        at,
    );
    no_zipper(
        |t| {
            deesser(|s| {
                s.amount = 100.0;
                s.frequency_hz = 3_000.0 * 3.0_f32.powf(t);
            })
        },
        &spec,
        &tone,
        21,
        (Param::FrequencyHz.id(), 9_000.0),
        at,
    );
    no_zipper(
        |t| {
            deesser(|s| {
                s.amount = 100.0;
                s.range = if t < 0.5 { Range::Wide } else { Range::AllHigh };
            })
        },
        &spec,
        &tone,
        2,
        (Param::Range.id(), Range::AllHigh.code()),
        at,
    );
    no_zipper(
        |t| {
            deesser(|s| {
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
fn the_target_ramps_over_ten_ms_in_its_logarithm_and_so_does_its_lane() {
    let spec = Spec::new(RATE, 240, Layout::Mono);
    let quiet = Run::new(vec![vec![0.0; 240]]);
    let mut node = deesser(|s| s.frequency_hz = 2_000.0);
    node.prepare(&spec).unwrap();
    node.set(Param::FrequencyHz.id(), 8_000.0);
    render(&mut node, &spec, &quiet, |left| left);
    // Half way: the geometric mean, 4 kHz, not the arithmetic one, 5 kHz.
    assert!(
        (node.heard().0 - 4_000.0).abs() < 0.01,
        "{}",
        node.heard().0
    );
    render(&mut node, &spec, &quiet, |left| left);
    assert_eq!(node.heard().0, 8_000.0);
    // A lane that jumps is followed by the same ramp.
    let mut node = deesser(|s| s.frequency_hz = 2_000.0);
    node.prepare(&spec).unwrap();
    let lane = quiet
        .clone()
        .with_automation(Param::FrequencyHz.id(), vec![8_000.0; 240]);
    render(&mut node, &spec, &lane, |left| left);
    assert!(
        (node.heard().0 - 4_000.0).abs() < 0.01,
        "{}",
        node.heard().0
    );
}

#[test]
fn the_amount_ramps_over_ten_ms_and_so_does_the_reduction() {
    // A steady ess, the whole of the sound: the reduction is 20 dB · Amount/100, read by the
    // meter in blocks of 1 ms after Amount goes from 0 to 100.
    let spec = Spec::new(RATE, 48, Layout::Mono);
    let mut node = deesser(|s| s.amount = 0.0);
    let meter = node.meter();
    node.prepare(&spec).unwrap();
    let tone = sine_at(6_000.0, 0.3, 0.0, 9_600);
    let mut blocks = tone.chunks(48);
    for _ in 0..100 {
        let block = Run::new(vec![blocks.next().unwrap().to_vec()]);
        render(&mut node, &spec, &block, |left| left);
    }
    assert_eq!(meter.read().0, 0.0);
    node.set(Param::Amount.id(), 100.0);
    for k in 1..=12_usize {
        let block = Run::new(vec![blocks.next().unwrap().to_vec()]);
        render(&mut node, &spec, &block, |left| left);
        let expected = -20.0 * (48.0 * k as f32 / 480.0).min(1.0);
        assert!(
            (meter.read().0 - expected).abs() < 0.05,
            "{k}: {}",
            meter.read().0
        );
    }
    assert_eq!(node.heard().1, 100.0);
}

#[test]
fn a_lane_that_turns_the_insert_off_lets_the_input_through_after_ten_ms() {
    let spec = Spec::new(RATE, 9_600, Layout::Mono);
    let input = sine_at(6_000.0, 0.3, 0.0, 9_600);
    let lane: Vec<f32> = (0..9_600)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(vec![input.clone()]).with_automation(Param::On.id(), lane);
    let output = process(deesser(|s| s.amount = 100.0), &spec, &run).remove(0);
    assert_eq!(output[2_480..], input[2_480..]);
    assert_ne!(output[..2_000], input[..2_000]);
}

#[test]
fn the_de_esser_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Stereo);
    let frames = 24_000;
    let mut left = noise(1, 0.3, frames);
    for (x, s) in left.iter_mut().zip(sine_at(6_500.0, 0.4, 0.0, frames)) {
        *x += s * if (frames / 3_000) % 2 == 0 { 1.0 } else { 0.1 };
    }
    let run = Run::new(vec![left, noise(2, 0.3, frames)])
        .with_change(100, Param::Amount.id(), 90.0)
        .with_change(3_000, Param::FrequencyHz.id(), 4_000.0)
        .with_change(3_200, Param::Range.id(), Range::AllHigh.code())
        .with_change(3_300, Param::Range.id(), Range::Narrow.code())
        .with_change(7_000, Param::Reaction.id(), Reaction::Fast.code())
        .with_change(9_000, Param::Listen.id(), 1.0)
        .with_change(9_200, Param::Listen.id(), 0.0)
        .with_change(12_000, Param::On.id(), 0.0)
        .with_change(12_100, Param::On.id(), 1.0);
    chunk_invariance(|| deesser(|_| {}), &spec, &run, &[1, 2]);
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 6_000.0 * 2_f32.powf((n as f32 / 1_000.0).sin()))
        .collect();
    let ranges: Vec<f32> = (0..frames).map(|n| ((n / 1_700) % 3) as f32).collect();
    let amounts: Vec<f32> = (0..frames).map(|n| (n % 101) as f32).collect();
    let automated = run
        .with_automation(Param::FrequencyHz.id(), sweep)
        .with_automation(Param::Range.id(), ranges.clone())
        .with_automation(Param::Reaction.id(), ranges)
        .with_automation(Param::Amount.id(), amounts);
    chunk_invariance(|| deesser(|_| {}), &spec, &automated, &[3]);
}

#[test]
fn no_latency_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    assert_eq!(assert_latency(|| deesser(|s| s.amount = 100.0), &spec), 0);
    let settle = settle_for(&Settings::default(), RATE) as usize;
    // 16 ramps of 10 ms, the slowest band, and 14 releases of 80 ms.
    assert!(
        (16 * 480 + 53_760..16 * 480 + 53_760 + 500).contains(&settle),
        "{settle}"
    );
    let frames = 2 * settle + 10_000;
    let mut left = noise(3, 0.2, frames);
    for (n, (x, s)) in left
        .iter_mut()
        .zip(sine_at(6_000.0, 0.5, 0.0, frames))
        .enumerate()
    {
        if (n / 2_000) % 2 == 0 {
            *x += s;
        }
    }
    let run = Run::new(vec![left, noise(4, 0.2, frames)])
        .with_change(settle - 100, Param::Amount.id(), 100.0)
        .with_change(settle + 500, Param::FrequencyHz.id(), 5_000.0);
    for from in [settle, settle + 2_345] {
        assert_settle(|| deesser(|_| {}), &spec, &run, from);
    }
    // Following a lane of the target from where the late node starts.
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 4_000.0 * 2_f32.powf((n as f32 / 3_000.0).sin()))
        .collect();
    let run = run.with_automation(Param::FrequencyHz.id(), sweep);
    assert_settle(|| deesser(|_| {}), &spec, &run, settle);
}

#[test]
fn input_and_lanes_that_are_not_numbers_and_the_work_of_the_thread_of_the_mix() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let frames = 90_000;
    let clean = noise(5, 0.5, frames);
    let mut odd = clean.clone();
    odd[300] = f32::INFINITY;
    odd[301] = f32::NAN;
    let a = process(deesser(|_| {}), &spec, &Run::new(vec![clean.clone()])).remove(0);
    let b = process(deesser(|_| {}), &spec, &Run::new(vec![odd])).remove(0);
    assert!(b[302..].iter().all(|x| x.is_finite()));
    let settle = settle_for(&Settings::default(), RATE) as usize;
    for n in 302 + settle..frames {
        assert!((a[n] - b[n]).abs() < 3.2e-5, "{n}");
    }
    let nans = vec![f32::NAN; frames];
    let mut run = Run::new(vec![clean]);
    for param in Param::ALL {
        run = run.with_automation(param.id(), nans.clone());
    }
    assert_eq!(process(deesser(|_| {}), &spec, &run).remove(0), a);

    let spec = Spec::new(RATE, 512, Layout::Surround51);
    let changes = [
        (Param::FrequencyHz.id(), 9_000.0),
        (Param::Range.id(), 2.0),
        (Param::Amount.id(), 100.0),
        (Param::Reaction.id(), 0.0),
        (Param::Listen.id(), 1.0),
        (Param::On.id(), 0.0),
    ];
    let lanes = vec![(Param::FrequencyHz.id(), vec![3_000.0; 480])];
    let counted = realtime_work(&mut deesser(|_| {}), &spec, &changes, &lanes, 200);
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn its_parameters_are_those_of_the_model() {
    let paths: Vec<_> = Param::ALL.iter().map(|param| param.path()).collect();
    assert_eq!(
        paths,
        [
            "on",
            "frequency_hz",
            "range",
            "amount",
            "reaction",
            "listen"
        ]
    );
    assert_eq!(Param::FrequencyHz.info().range, (2_000.0, 16_000.0));
    assert_eq!(Param::FrequencyHz.info().default, 6_000.0);
    assert_eq!(Param::Amount.info().default, 50.0);
    assert_eq!(Param::Range.info().default, Range::Wide.code());
    assert_eq!(Param::Reaction.info().default, Reaction::Medium.code());
    assert_eq!(
        [Range::Narrow, Range::Wide, Range::AllHigh].map(Range::code),
        [0.0, 1.0, 2.0]
    );
    assert_eq!(
        [Reaction::Slow, Reaction::Medium, Reaction::Fast].map(Reaction::code),
        [0.0, 1.0, 2.0]
    );
    let settings = Settings {
        on: true,
        frequency_hz: 7_500.0,
        range: Range::AllHigh,
        amount: 77.0,
        reaction: Reaction::Slow,
        listen: false,
    };
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![noise(12, 0.8, 4_800)]);
    let mut set = DeEsser::default();
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    assert_eq!(
        process(set, &spec, &run),
        process(DeEsser::new(&settings), &spec, &run)
    );
    // A choice out of range or between codes goes to the nearest.
    let mut odd = DeEsser::default();
    odd.set(Param::Range.id(), 7.0);
    odd.set(Param::Reaction.id(), 0.4);
    let near = deesser(|s| {
        s.range = Range::AllHigh;
        s.reaction = Reaction::Slow;
    });
    assert_eq!(process(odd, &spec, &run), process(near, &spec, &run));
}
