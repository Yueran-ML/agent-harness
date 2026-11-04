//! The EQ (T7): its response against the formulas of the cookbook and of Butterworth, the curve
//! for the window against its impulse response, a bypass to the bit, the same output in any
//! blocks, no memory taken, no zipper when a parameter moves, no latency, a settling that holds,
//! stability at the ends of every range, every layout, no denormals, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::cast_possible_truncation,
    reason = "exact values compared on purpose, the letters of the cookbook, test samples in f32"
)]

use std::f64::consts::{PI, TAU};

use columbina_audio::constants::{samples, EQ_CROSSFADE, SILENCE_DB};
use columbina_audio::fx::biquad::{butterworth_q, butterworth_q_of, Coefficients};
use columbina_audio::fx::eq::{
    param, response, settle_for, Band, Eq, Field, Kind, Settings, Variant, Zone, FREQUENCY_RANGE,
    GAIN, GAIN_RANGE_DB, LANE_SETTLE, MAX_BANDS, ON, Q_RANGE,
};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, impulse, noise, realtime, render, sine,
    CountingAllocator, Run,
};
use columbina_audio::{
    Automated, Automation, Block, Layout, Node, ParamId, Planar, Spec, Unsupported,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const RATE: u32 = 48_000;

/// The settings of a new strip, on, changed by `edit`.
fn strip(edit: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings::strip();
    settings.on = true;
    edit(&mut settings);
    settings
}

/// The settings of a new clip, on, changed by `edit`.
fn clip(edit: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings::clip();
    settings.on = true;
    edit(&mut settings);
    settings
}

/// The output of a new EQ of `settings` for `input` (mono), processed whole.
fn process(settings: &Settings, rate: u32, input: Vec<f32>) -> Vec<f32> {
    let spec = Spec::new(rate, input.len(), Layout::Mono);
    let mut eq = Eq::new(settings);
    eq.prepare(&spec).unwrap();
    render(&mut eq, &spec, &Run::new(vec![input]), |left| left).remove(0)
}

/// The impulse response of `settings`, `frames` long.
fn impulse_response(settings: &Settings, rate: u32, frames: usize) -> Vec<f32> {
    process(settings, rate, impulse(frames, 0, 1.0))
}

/// The gain in dB at `freq` of the filter of impulse response `h`: its transform at that
/// frequency exactly.
#[allow(clippy::cast_precision_loss, reason = "sample numbers")]
fn gain_at(h: &[f32], rate: u32, freq: f64) -> f64 {
    let w = TAU * freq / f64::from(rate);
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &x) in h.iter().enumerate() {
        let phase = w * n as f64;
        re += f64::from(x) * phase.cos();
        im -= f64::from(x) * phase.sin();
    }
    20.0 * re.hypot(im).log10()
}

/// `count` frequencies from 20 Hz to 20 kHz, evenly on a logarithmic scale.
#[allow(clippy::cast_precision_loss, reason = "a few")]
fn log_frequencies(count: usize) -> Vec<f64> {
    (0..count)
        .map(|k| 20.0 * 1000_f64.powf(k as f64 / (count - 1) as f64))
        .collect()
}

/// The response in dB of the peaking EQ of the cookbook, written out here again from the
/// cookbook and evaluated as a complex fraction.
fn cookbook_peaking_db(rate: f64, f0: f64, q: f64, gain_db: f64, freq: f64) -> f64 {
    let a = 10_f64.powf(gain_db / 40.0);
    let w0 = TAU * f0 / rate;
    let alpha = w0.sin() / (2.0 * q);
    let b = [1.0 + alpha * a, -2.0 * w0.cos(), 1.0 - alpha * a];
    let d = [1.0 + alpha / a, -2.0 * w0.cos(), 1.0 - alpha / a];
    let w = TAU * freq / rate;
    let at = |c: [f64; 3]| {
        let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
        let im = -(c[1] * w.sin() + c[2] * (2.0 * w).sin());
        re.hypot(im)
    };
    20.0 * (at(b) / at(d)).log10()
}

// --- The response ----------------------------------------------------------------------------

#[test]
fn a_bell_of_six_db_at_one_kilohertz_is_six_db_there_and_the_cookbook_everywhere() {
    let settings = strip(|s| {
        s.bands[3].freq = 1_000.0;
        s.bands[3].gain_db = 6.0;
    });
    let h = impulse_response(&settings, RATE, 1 << 15);
    let at = gain_at(&h, RATE, 1_000.0);
    assert!((at - 6.0).abs() <= 0.01, "{at} dB at 1 kHz");
    for freq in log_frequencies(50) {
        let expected = cookbook_peaking_db(f64::from(RATE), 1_000.0, 1.0, 6.0, freq);
        let measured = gain_at(&h, RATE, freq);
        assert!(
            (measured - expected).abs() <= 0.01,
            "{freq} Hz: {measured} dB, the cookbook {expected}"
        );
    }
}

/// The strip's band 1 a high pass of `slope` at 100 Hz.
fn high_pass(slope: u8) -> Settings {
    strip(|s| {
        s.bands[0] = Band {
            on: true,
            slope,
            ..Band::new(true, Kind::HighPass, 100.0, None)
        };
    })
}

#[test]
fn the_high_passes_fall_as_butterworth_filters_of_their_slopes() {
    let h12 = impulse_response(&high_pass(12), RATE, 1 << 15);
    let at = gain_at(&h12, RATE, 100.0);
    assert!((at + 3.01).abs() <= 0.05, "12 dB/oct at 100 Hz: {at}");
    let at = gain_at(&h12, RATE, 50.0);
    assert!((at + 12.30).abs() <= 0.1, "12 dB/oct at 50 Hz: {at}");
    let h24 = impulse_response(&high_pass(24), RATE, 1 << 15);
    let at = gain_at(&h24, RATE, 50.0);
    assert!((at + 24.1).abs() <= 0.2, "24 dB/oct at 50 Hz: {at}");
    let at = gain_at(&h24, RATE, 100.0);
    assert!((at + 3.01).abs() <= 0.05, "24 dB/oct at 100 Hz: {at}");
    let h6 = impulse_response(&high_pass(6), RATE, 1 << 15);
    let at = gain_at(&h6, RATE, 50.0);
    assert!((at + 6.99).abs() <= 0.1, "6 dB/oct at 50 Hz: {at}");
    // 18 dB/oct: 1/(1 + 2^6) at half the frequency, −18.13 dB.
    let h18 = impulse_response(&high_pass(18), RATE, 1 << 15);
    let at = gain_at(&h18, RATE, 50.0);
    assert!((at + 18.13).abs() <= 0.1, "18 dB/oct at 50 Hz: {at}");
    let at = gain_at(&h18, RATE, 100.0);
    assert!((at + 3.01).abs() <= 0.05, "18 dB/oct at 100 Hz: {at}");
}

#[test]
fn the_low_passes_fall_as_butterworth_filters_of_their_slopes() {
    for (slope, order) in [(6, 1), (12, 2), (18, 3), (24, 4)] {
        let settings = strip(|s| {
            s.bands[5] = Band {
                slope,
                ..Band::new(true, Kind::LowPass, 1_000.0, None)
            };
        });
        let h = impulse_response(&settings, RATE, 1 << 14);
        // Butterworth: 1/(1 + (f/fc)^2n) at 2 kHz, the bilinear transform warped at 1 kHz
        // taking the frequency to tan(πf/rate)/tan(πfc/rate).
        let warped =
            (PI * 2_000.0 / f64::from(RATE)).tan() / (PI * 1_000.0 / f64::from(RATE)).tan();
        let expected = -10.0 * (1.0 + warped.powi(2 * order)).log10();
        let at = gain_at(&h, RATE, 2_000.0);
        assert!(
            (at - expected).abs() <= 0.01,
            "{slope} dB/oct: {at}, {expected}"
        );
        let at = gain_at(&h, RATE, 1_000.0);
        assert!((at + 3.0103).abs() <= 0.01, "{slope} dB/oct at 1 kHz: {at}");
    }
}

#[test]
fn a_notch_of_q_ten_takes_away_more_than_forty_db() {
    let settings = strip(|s| {
        s.bands[3].kind = Kind::Notch;
        s.bands[3].freq = 1_000.0;
        s.bands[3].q = 10.0;
    });
    let h = impulse_response(&settings, RATE, 1 << 15);
    let at = gain_at(&h, RATE, 1_000.0);
    assert!(at < -40.0, "{at} dB at 1 kHz");
    let at = gain_at(&h, RATE, 2_000.0);
    assert!(at.abs() < 0.1, "{at} dB an octave away");
}

/// Settings that use every kind and slope, with a gain after.
fn busy_strip() -> Settings {
    strip(|s| {
        s.gain_db = 2.5;
        s.bands[0] = Band {
            slope: 18,
            ..Band::new(true, Kind::HighPass, 40.0, None)
        };
        s.bands[1] = Band {
            gain_db: 4.0,
            ..Band::new(true, Kind::LowShelf, 150.0, Some(Zone::Low))
        };
        s.bands[2] = Band {
            gain_db: -7.0,
            q: 3.0,
            ..Band::new(true, Kind::Bell, 700.0, Some(Zone::MediumLow))
        };
        s.bands[3] = Band {
            q: 4.0,
            ..Band::new(true, Kind::Notch, 3_000.0, Some(Zone::MediumHigh))
        };
        s.bands[4] = Band {
            gain_db: 5.0,
            ..Band::new(true, Kind::HighShelf, 6_000.0, Some(Zone::High))
        };
        s.bands[5] = Band {
            slope: 24,
            ..Band::new(true, Kind::LowPass, 12_000.0, None)
        };
    })
}

fn busy_clip() -> Settings {
    clip(|s| {
        s.gain_db = -3.0;
        s.bands[0] = Band {
            gain_db: -6.0,
            ..Band::new(true, Kind::LowShelf, 90.0, None)
        };
        s.bands[1] = Band {
            gain_db: 9.0,
            q: 0.5,
            ..Band::new(true, Kind::Bell, 400.0, None)
        };
        s.bands[2] = Band {
            q: 2.0,
            ..Band::new(true, Kind::Notch, 5_000.0, None)
        };
        s.bands[3] = Band::new(true, Kind::LowPass, 15_000.0, None);
    })
}

#[test]
fn the_curve_for_the_window_is_the_response_of_the_sound() {
    let cases = [
        (busy_strip(), RATE),
        (busy_clip(), RATE),
        (busy_strip(), 44_100),
        (high_pass(24), 96_000),
        // A low pass at 20 kHz at 32 kHz: designed at 0.45 of the rate.
        (
            strip(|s| s.bands[5] = Band::new(true, Kind::LowPass, 20_000.0, None)),
            32_000,
        ),
    ];
    for (settings, rate) in cases {
        let h = impulse_response(&settings, rate, 1 << 15);
        let freqs = log_frequencies(100);
        #[allow(clippy::cast_possible_truncation, reason = "frequencies")]
        let curve = response(
            &settings,
            rate,
            &freqs.iter().map(|&f| f as f32).collect::<Vec<_>>(),
        );
        for (&freq, &curve) in freqs.iter().zip(&curve) {
            if freq > 0.49 * f64::from(rate) {
                continue;
            }
            let measured = gain_at(&h, rate, f64::from(freq as f32));
            if curve >= -60.0 {
                assert!(
                    (measured - f64::from(curve)).abs() <= 0.01,
                    "{rate}: {freq} Hz: the curve {curve}, the sound {measured}"
                );
            } else {
                let difference =
                    (10_f64.powf(measured / 20.0) - 10_f64.powf(f64::from(curve) / 20.0)).abs();
                assert!(difference <= 1e-5, "{rate}: {freq} Hz: {curve}, {measured}");
            }
        }
    }
}

#[test]
fn the_curve_is_flat_when_the_eq_is_off_and_floored_at_silence() {
    let mut settings = busy_strip();
    settings.on = false;
    assert!(response(&settings, RATE, &[20.0, 1_000.0, 20_000.0])
        .iter()
        .all(|&db| db == 0.0));
    let notch = strip(|s| {
        s.bands[3].kind = Kind::Notch;
        s.bands[3].freq = 1_000.0;
    });
    let curve = response(&notch, RATE, &[1_000.0, 1_000.000_1]);
    assert!(curve.iter().all(|db| db.is_finite() && *db >= SILENCE_DB));
    let flat = response(&strip(|_| {}), RATE, &[20.0, 440.0, 20_000.0]);
    assert!(flat.iter().all(|db| db.abs() < 1e-9), "{flat:?}");
}

// --- Bypass ----------------------------------------------------------------------------------

#[test]
fn an_eq_off_or_with_nothing_on_passes_the_sound_to_the_bit() {
    let input = noise(7, 0.9, 20_000);
    let mut off = busy_strip();
    off.on = false;
    let mut nothing = busy_strip();
    for band in &mut nothing.bands {
        band.on = false;
    }
    nothing.gain_db = 0.0;
    let mut clip_off = busy_clip();
    clip_off.on = false;
    // The middle bands of a new strip are on at no gain: they pass the sound to the bit too.
    for settings in [off, nothing, clip_off, strip(|_| {}), clip(|_| {})] {
        let output = process(&settings, RATE, input.clone());
        assert!(
            output
                .iter()
                .zip(&input)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{settings:?}"
        );
    }
}

#[test]
fn an_eq_turned_off_passes_the_sound_to_the_bit_once_its_crossfade_is_over() {
    let input = noise(8, 0.9, 20_000);
    let run = Run::new(vec![input.clone()]).with_change(5_000, ON, 0.0);
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut eq = Eq::new(&busy_strip());
    eq.prepare(&spec).unwrap();
    let output = render(&mut eq, &spec, &run, |_| 300).remove(0);
    let fade = samples(EQ_CROSSFADE, RATE);
    assert!(output[5_000..5_000 + fade - 1]
        .iter()
        .zip(&input[5_000..])
        .any(|(a, b)| a != b));
    for n in 5_000 + fade..input.len() {
        assert_eq!(output[n].to_bits(), input[n].to_bits(), "sample {n}");
    }
}

// --- Blocks, memory, latency, settling -------------------------------------------------------

/// Noise on each of `channels`.
fn noises(channels: usize, frames: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|c| noise(20 + c as u64, 0.5, frames))
        .collect()
}

#[test]
fn the_eq_is_the_same_in_any_blocks_with_changes_of_every_parameter() {
    let frames = 9_000;
    let run = Run::new(noises(2, frames))
        .with_change(300, param(2, Field::Gain), 9.0)
        .with_change(700, param(3, Field::Freq), 4_000.0)
        .with_change(900, param(3, Field::Q), 6.0)
        .with_change(1_000, param(0, Field::On), 1.0)
        .with_change(1_200, param(0, Field::Slope), 24.0)
        .with_change(1_500, param(4, Field::Kind), Kind::HighShelf.code())
        .with_change(1_510, param(4, Field::Gain), -8.0)
        .with_change(1_600, param(4, Field::Kind), Kind::Notch.code())
        .with_change(2_000, param(1, Field::Zone), Zone::MediumLow.code())
        .with_change(2_300, GAIN, 6.0)
        .with_change(3_000, ON, 0.0)
        .with_change(3_200, ON, 1.0)
        .with_change(4_000, param(5, Field::On), 1.0)
        .with_change(4_100, param(5, Field::Freq), 3_000.0)
        .with_change(4_300, param(2, Field::On), 0.0)
        .with_change(6_000, param(2, Field::On), 1.0);
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let whole = chunk_invariance(|| Eq::new(&strip(|_| {})), &spec, &run, &[1, 2, 3]);
    assert!(whole[0].iter().all(|x| x.is_finite()));
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "sample numbers")]
fn the_eq_is_the_same_in_any_blocks_under_automation() {
    let frames = 9_000;
    let ramp = |from: f32, to: f32| -> Vec<f32> {
        (0..frames)
            .map(|n| from + (to - from) * n as f32 / frames as f32)
            .collect()
    };
    let square = |period: usize| -> Vec<f32> {
        (0..frames)
            .map(|n| {
                if (n / period).is_multiple_of(2) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect()
    };
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 100.0 * 100_f32.powf(n as f32 / frames as f32))
        .collect();
    let run = Run::new(noises(2, frames))
        .with_automation(param(0, Field::Freq), sweep)
        .with_automation(param(2, Field::Gain), ramp(-12.0, 12.0))
        .with_automation(param(2, Field::Q), ramp(0.3, 10.3))
        .with_automation(param(3, Field::On), square(1_300))
        .with_automation(ON, square(4_100))
        .with_automation(GAIN, ramp(3.0, -3.0))
        .with_change(2_000, param(2, Field::Gain), 3.0);
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let settings = busy_strip();
    chunk_invariance(|| Eq::new(&settings), &spec, &run, &[4, 5]);
}

#[test]
fn a_clip_eq_on_five_one_is_the_same_in_any_blocks() {
    let frames = 6_000;
    let run = Run::new(noises(6, frames))
        .with_change(1_000, param(1, Field::Freq), 2_000.0)
        .with_change(2_000, param(0, Field::Kind), Kind::HighPass.code())
        .with_change(2_500, param(3, Field::Kind), Kind::Bell.code());
    let spec = Spec::new(RATE, 256, Layout::Surround51);
    chunk_invariance(|| Eq::new(&busy_clip()), &spec, &run, &[6]);
}

#[test]
fn the_eq_has_no_latency() {
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let settings = strip(|s| {
        s.bands[2].gain_db = 4.0;
        s.bands[4].gain_db = -3.0;
    });
    assert_eq!(assert_latency(|| Eq::new(&settings), &spec), 0);
    assert_eq!(Eq::new(&busy_strip()).latency(), 0);
}

/// Checks the settling of `settings` on mono noise: a node started at `from` (at least its
/// settling time, so that the node from the start is fully charged), `run` given the changes
/// and lanes of `edit` (told `from`).
fn settles(settings: &Settings, from: usize, edit: impl FnOnce(Run, usize) -> Run) {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let mut probe = Eq::new(settings);
    probe.prepare(&spec).unwrap();
    let settle = probe.settle() as usize;
    let from = from.max(settle);
    let frames = from + settle + 4_800;
    let run = edit(Run::new(vec![noise(30, 0.5, frames)]), from);
    let largest = assert_settle(|| Eq::new(settings), &spec, &run, from);
    assert!(largest.is_finite());
}

#[test]
fn a_late_start_settles_as_reported() {
    settles(&busy_strip(), 7_000, |run, _| run);
    settles(&busy_clip(), 3_000, |run, from| {
        run.with_change(from - 100, param(1, Field::Gain), -4.0)
            .with_change(from - 10, param(2, Field::Kind), Kind::Bell.code())
            .with_change(from + 100, param(1, Field::Freq), 600.0)
    });
    // The slowest corner: a bell of +20 dB and Q 10.3 at 20 Hz, and a high pass at 20 Hz.
    let slow = strip(|s| {
        s.bands[0] = Band {
            slope: 24,
            ..Band::new(true, Kind::HighPass, 20.0, None)
        };
        s.bands[1] = Band {
            gain_db: 20.0,
            q: 10.3,
            ..Band::new(true, Kind::Bell, 20.0, Some(Zone::Low))
        };
    });
    settles(&slow, 1_000, |run, _| run);
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "sample numbers")]
fn a_late_start_under_automation_settles_as_reported() {
    let settings = busy_strip();
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let mut probe = Eq::new(&settings);
    probe.prepare(&spec).unwrap();
    let from = probe.settle() as usize;
    let frames = 2 * from + 4_800;
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 500.0 * 10_f32.powf((n as f32 / 3_000.0).sin()))
        .collect();
    let toggles: Vec<f32> = (0..frames)
        .map(|n| {
            if (n / 2_500).is_multiple_of(2) {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let run = Run::new(vec![noise(31, 0.5, frames)])
        .with_automation(param(2, Field::Freq), sweep)
        .with_automation(param(4, Field::On), toggles);
    assert_settle(|| Eq::new(&settings), &spec, &run, from);
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "sample numbers")]
fn a_late_start_follows_a_moving_lane_as_reported() {
    // Quick filters, so that what settles last is the frequency following its lane from where
    // the late node starts (300 Hz, as set) to where the lane is.
    let settings = clip(|s| {
        s.bands[1].gain_db = 12.0;
        s.bands[1].q = 0.3;
    });
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let mut probe = Eq::new(&settings);
    probe.prepare(&spec).unwrap();
    let from = 20_000;
    let frames = from + probe.settle() as usize + 4_800;
    let sweep: Vec<f32> = (0..frames)
        .map(|n| 3_000.0 * 0.5_f32.powf(1.0 + (n as f32 / 700.0).sin()))
        .collect();
    let run = Run::new(vec![noise(32, 0.5, frames)]).with_automation(param(1, Field::Freq), sweep);
    assert_settle(|| Eq::new(&settings), &spec, &run, from);
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "the counts of a test")]
fn the_work_of_the_eq_on_the_thread_of_the_mix_takes_no_memory() {
    const BLOCK: usize = 480;
    let spec = Spec::new(RATE, 512, Layout::Surround51);
    let mut eq = Eq::new(&busy_strip());
    eq.prepare(&spec).unwrap();
    let mut input = Planar::new(6, 512);
    for channel in 0..6 {
        input
            .channel_mut(channel)
            .copy_from_slice(&noise(40 + channel as u64, 0.5, 512));
    }
    let mut output = Planar::new(6, 512);
    let lane: Vec<f32> = (0..512).map(|n| 200.0 + n as f32 * 10.0).collect();
    let toggles: Vec<f32> = (0..512).map(|n| if n < 256 { 1.0 } else { 0.0 }).collect();
    let mut processed = 0_u64;
    let ((), counted) = realtime(|| {
        for n in 0..500 {
            if n % 7 == 0 {
                eq.set(param(n % 6, Field::Gain), (n % 13) as f32 - 6.0);
                eq.set(param(n % 6, Field::Freq), 100.0 + (n % 50) as f32 * 300.0);
                eq.set(param(0, Field::Kind), Kind::ALL[n % 6].code());
                eq.set(param(n % 6, Field::On), (n % 2) as f32);
                eq.set(param(0, Field::Slope), (6 * (1 + n % 4)) as f32);
                eq.set(ON, if n % 11 == 0 { 0.0 } else { 1.0 });
            }
            if n == 250 {
                eq.reset();
                processed = 0;
            }
            let lanes = [
                Automated {
                    param: param(2, Field::Freq),
                    values: &lane[..BLOCK],
                },
                Automated {
                    param: param(3, Field::On),
                    values: &toggles[..BLOCK],
                },
            ];
            let automation = if n % 3 == 0 {
                Automation::new(&lanes)
            } else {
                Automation::NONE
            };
            let block = Block::new(processed, RATE, BLOCK).with_automation(automation);
            processed += BLOCK as u64;
            input.read(BLOCK, |input| {
                output.write(BLOCK, |output| eq.process(&block, input, output));
            });
            let _ = eq.settle();
        }
    });
    assert!(counted.none(), "{counted:?}");
}

// --- No zipper -------------------------------------------------------------------------------

/// The largest step between two samples of `signal`.
fn largest_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

/// The largest step of the output of a new EQ of `settings` once settled, for `input`.
fn steady_step(settings: &Settings, input: &[f32]) -> f32 {
    let output = process(settings, RATE, input.to_vec());
    largest_step(&output[output.len() / 2..])
}

const TONE_FRAMES: usize = 24_000;
/// A quarter of a period after 250 periods of the tone: where it peaks, the worst place for a step.
const CHANGE: usize = 12_012;

/// A tone of 1 kHz at −12 dBFS.
fn tone() -> Vec<f32> {
    sine(RATE, 1_000.0, 0.25, TONE_FRAMES)
}

/// Checks that the steps of the output of `before`, changed by `run`'s changes and lanes from
/// sample 12000, are at most 1.1 times the largest step of the steady output of any of `path`,
/// the settings that the change goes through.
fn no_zipper(before: &Settings, path: &[Settings], edit: impl FnOnce(Run) -> Run) {
    let input = tone();
    let bound = path
        .iter()
        .chain(std::iter::once(before))
        .map(|settings| steady_step(settings, &input))
        .fold(0.0, f32::max);
    let run = edit(Run::new(vec![input]));
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut eq = Eq::new(before);
    eq.prepare(&spec).unwrap();
    let output = render(&mut eq, &spec, &run, |_| 480).remove(0);
    let measured = largest_step(&output[CHANGE - 2_000..]);
    assert!(
        measured <= 1.1 * bound,
        "a step of {measured}, the steady ones at most {bound}"
    );
}

/// `count` settings from `edit(0)` to `edit(1)`.
#[allow(clippy::cast_precision_loss, reason = "a few")]
fn path(count: usize, edit: impl Fn(f32, &mut Settings)) -> Vec<Settings> {
    (0..count)
        .map(|k| {
            let mut settings = strip(|_| {});
            edit(k as f32 / (count - 1) as f32, &mut settings);
            settings
        })
        .collect()
}

#[test]
fn a_gain_a_frequency_and_a_q_set_ramp_without_zipper() {
    let bell = |t: f32, s: &mut Settings| {
        s.bands[3].freq = 1_000.0;
        s.bands[3].gain_db = 12.0 * t;
    };
    no_zipper(&path(2, bell)[0], &path(9, bell), |run| {
        run.with_change(CHANGE, param(3, Field::Gain), 12.0)
    });
    // A clip's band 2 has no zone: from 100 Hz to 10 kHz across the tone.
    let moving = |t: f32, s: &mut Settings| {
        s.variant = Variant::Clip;
        s.bands = Settings::clip().bands;
        s.bands[1].gain_db = 12.0;
        s.bands[1].freq = 100.0 * 100_f32.powf(t);
    };
    no_zipper(&path(2, moving)[0], &path(41, moving), |run| {
        run.with_change(CHANGE, param(1, Field::Freq), 10_000.0)
    });
    let narrowing = |t: f32, s: &mut Settings| {
        s.bands[3].freq = 1_200.0;
        s.bands[3].gain_db = 12.0;
        s.bands[3].q = 0.3 * (10.3_f32 / 0.3).powf(t);
    };
    no_zipper(&path(2, narrowing)[0], &path(21, narrowing), |run| {
        run.with_change(CHANGE, param(3, Field::Q), 10.3)
    });
    let louder = |t: f32, s: &mut Settings| s.gain_db = 12.0 * t;
    no_zipper(&path(2, louder)[0], &path(9, louder), |run| {
        run.with_change(CHANGE, GAIN, 12.0)
    });
}

#[test]
fn a_type_a_band_and_the_eq_switched_crossfade_without_zipper() {
    let bell = strip(|s| {
        s.gain_db = 6.0;
        s.bands[3].freq = 1_000.0;
        s.bands[3].gain_db = 12.0;
    });
    let mut shelf = bell;
    shelf.bands[3].kind = Kind::HighShelf;
    no_zipper(&bell, &[shelf], |run| {
        run.with_change(CHANGE, param(3, Field::Kind), Kind::HighShelf.code())
    });
    let mut band_off = bell;
    band_off.bands[3].on = false;
    no_zipper(&bell, &[band_off], |run| {
        run.with_change(CHANGE, param(3, Field::On), 0.0)
    });
    let mut off = bell;
    off.on = false;
    no_zipper(&bell, &[off], |run| run.with_change(CHANGE, ON, 0.0));
    no_zipper(&off, &[bell], |run| run.with_change(CHANGE, ON, 1.0));
    // Passes switched in over the tone: the new filter starts from the past of the sound.
    let plain = strip(|_| {});
    let mut high_pass = plain;
    high_pass.bands[0] = Band {
        slope: 24,
        ..Band::new(true, Kind::HighPass, 2_000.0, None)
    };
    no_zipper(&plain, &[high_pass], |run| {
        run.with_change(CHANGE, param(0, Field::On), 1.0)
            .with_change(CHANGE, param(0, Field::Slope), 24.0)
            .with_change(CHANGE, param(0, Field::Freq), 2_000.0)
    });
    let mut low_pass = plain;
    low_pass.bands[5] = Band {
        slope: 24,
        ..Band::new(true, Kind::LowPass, 500.0, None)
    };
    no_zipper(&plain, &[low_pass], |run| {
        run.with_change(CHANGE, param(5, Field::On), 1.0)
            .with_change(CHANGE, param(5, Field::Slope), 24.0)
            .with_change(CHANGE, param(5, Field::Freq), 500.0)
    });
    // Back before the crossfade is over.
    no_zipper(&bell, &[shelf], |run| {
        run.with_change(CHANGE, param(3, Field::Kind), Kind::HighShelf.code())
            .with_change(CHANGE + 200, param(3, Field::Kind), Kind::Bell.code())
    });
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "sample numbers")]
fn a_frequency_automated_across_the_tone_has_no_zipper() {
    let sweep: Vec<f32> = (0..TONE_FRAMES)
        .map(|n| {
            let t = (n as f32 - CHANGE as f32).max(0.0) / 4_800.0;
            100.0 * 100_f32.powf(t.min(1.0))
        })
        .collect();
    let moving = |t: f32, s: &mut Settings| {
        s.variant = Variant::Clip;
        s.bands = Settings::clip().bands;
        s.bands[1].gain_db = 12.0;
        s.bands[1].freq = 100.0 * 100_f32.powf(t);
    };
    no_zipper(&path(2, moving)[0], &path(41, moving), |run| {
        run.with_automation(param(1, Field::Freq), sweep)
    });
}

// --- Stability, layouts, denormals -----------------------------------------------------------

#[test]
fn every_filter_at_the_ends_of_its_ranges_is_stable_at_every_rate() {
    for rate in [8_000_u32, 22_050, 32_000, 44_100, 48_000, 96_000, 192_000] {
        let rate = f64::from(rate);
        for freq in [20.0, 1_000.0, 20_000.0_f64] {
            let freq = freq.min(0.45 * rate);
            for q in [0.3, 1.0, 10.3] {
                for gain_db in [-20.0, 0.0, 20.0_f64] {
                    let gain = 10_f64.powf(gain_db / 20.0);
                    let filters = [
                        Coefficients::peaking(rate, freq, q, gain),
                        Coefficients::low_shelf(rate, freq, gain, 1.0),
                        Coefficients::high_shelf(rate, freq, gain, 1.0),
                        Coefficients::notch(rate, freq, q),
                        Coefficients::high_pass(rate, freq, q),
                        Coefficients::low_pass(rate, freq, q),
                        Coefficients::high_pass_first(rate, freq),
                        Coefficients::low_pass_first(rate, freq),
                    ];
                    for filter in filters {
                        let r = filter.pole_radius();
                        assert!(r < 1.0, "{rate} {freq} {q} {gain_db}: {filter:?}, {r}");
                    }
                }
            }
        }
    }
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "the numbers of a test")]
fn automation_that_jumps_at_every_sample_keeps_the_sound_bounded() {
    for rate in [8_000, 48_000] {
        let frames = rate as usize;
        let mut rng = columbina_audio::testing::Rng::new(77);
        let mut lane = |low: f32, high: f32| -> Vec<f32> {
            (0..frames)
                .map(|_| low + (high - low) * rng.unit() as f32)
                .collect()
        };
        let mut run = Run::new(vec![noise(50, 1.0, frames)]);
        for band in 0..MAX_BANDS {
            run = run
                .with_automation(param(band, Field::Freq), lane(20.0, 20_000.0))
                .with_automation(param(band, Field::Gain), lane(-20.0, 20.0))
                .with_automation(param(band, Field::Q), lane(0.3, 10.3));
        }
        let spec = Spec::new(rate, 512, Layout::Mono);
        let mut eq = Eq::new(&busy_strip());
        eq.prepare(&spec).unwrap();
        let output = render(&mut eq, &spec, &run, |_| 512).remove(0);
        let peak = output.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
        assert!(peak.is_finite() && peak < 10.0, "{rate}: a peak of {peak}");
    }
}

#[test]
fn every_layout_is_taken_and_each_channel_lfe_too_has_the_same_eq() {
    let settings = busy_strip();
    for layout in [
        Layout::Mono,
        Layout::Stereo,
        Layout::Lcr,
        Layout::Lcrs,
        Layout::Surround50,
        Layout::Surround51,
        Layout::Surround70,
        Layout::Surround71,
        Layout::Adaptive(1),
        Layout::Adaptive(24),
    ] {
        let spec = Spec::new(RATE, 256, layout);
        let mut eq = Eq::new(&settings);
        eq.prepare(&spec).unwrap();
        let channels = layout.channels();
        let input = noises(channels, 3_000);
        let output = render(&mut eq, &spec, &Run::new(input.clone()), |_| 200);
        for (channel, (output, input)) in output.iter().zip(input).enumerate() {
            let alone = process(&settings, RATE, input);
            assert!(
                output
                    .iter()
                    .zip(&alone)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{layout:?}, channel {channel} ({:?})",
                layout.role(channel)
            );
        }
    }
    let mut eq = Eq::new(&settings);
    assert!(eq.prepare(&Spec::new(RATE, 256, Layout::Empty)).is_err());
    assert!(eq
        .prepare(&Spec::new(RATE, 256, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
}

#[test]
fn a_tail_into_silence_ends_in_zeros_and_never_in_denormals() {
    let settings = strip(|s| {
        s.bands[0] = Band {
            slope: 24,
            ..Band::new(true, Kind::HighPass, 40.0, None)
        };
        s.bands[1] = Band {
            gain_db: 12.0,
            ..Band::new(true, Kind::Bell, 100.0, Some(Zone::Low))
        };
        s.bands[2].gain_db = 6.0;
    });
    let output = impulse_response(&settings, RATE, 2 * RATE as usize);
    assert!(output
        .iter()
        .all(|x| *x == 0.0 || x.abs() >= f32::MIN_POSITIVE));
    let last = output.iter().rposition(|x| *x != 0.0).unwrap();
    // From 1 to below 1e-30 by the slowest poles, about 1.5 s; silence from there on.
    assert!(last < 7 * RATE as usize / 4, "the last sound at {last}");
    // The slowest corner rings for longer than the 20 s here, and never in denormals.
    let settings = strip(|s| {
        s.bands[0] = Band {
            slope: 24,
            ..Band::new(true, Kind::HighPass, 20.0, None)
        };
        s.bands[1] = Band {
            gain_db: 20.0,
            q: 10.3,
            ..Band::new(true, Kind::Bell, 20.0, Some(Zone::Low))
        };
        s.bands[2].gain_db = 6.0;
    });
    let output = impulse_response(&settings, RATE, 20 * RATE as usize);
    assert!(output
        .iter()
        .all(|x| *x == 0.0 || x.abs() >= f32::MIN_POSITIVE));
}

// --- Parameters ------------------------------------------------------------------------------

/// The output of `eq`, prepared and reset, for mono noise.
fn output_of(mut eq: Eq) -> Vec<f32> {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    eq.prepare(&spec).unwrap();
    eq.reset();
    render(
        &mut eq,
        &spec,
        &Run::new(vec![noise(60, 0.5, 6_000)]),
        |_| 512,
    )
    .remove(0)
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

#[test]
fn the_parameters_of_settings_set_make_the_eq_of_those_settings() {
    for settings in [busy_strip(), busy_clip()] {
        let expected = output_of(Eq::new(&settings));
        let mut before = Eq::new(&Settings::of(settings.variant));
        for (param, value) in settings.params() {
            before.set(param, value);
        }
        assert!(
            same_bits(&output_of(before), &expected),
            "set before prepare"
        );
        let mut after = Eq::new(&Settings::of(settings.variant));
        after.prepare(&Spec::new(RATE, 512, Layout::Mono)).unwrap();
        for (param, value) in settings.params() {
            after.set(param, value);
        }
        assert!(same_bits(&output_of(after), &expected), "set after prepare");
    }
}

#[test]
fn values_out_of_range_are_kept_in_and_what_a_band_cannot_be_is_ignored() {
    let with = |edit: &dyn Fn(&mut Eq)| {
        let mut eq = Eq::new(&busy_strip());
        edit(&mut eq);
        output_of(eq)
    };
    let as_settings = |edit: &dyn Fn(&mut Settings)| {
        let mut settings = busy_strip();
        edit(&mut settings);
        output_of(Eq::new(&settings))
    };
    let unchanged = as_settings(&|_| {});
    // Band 2 is in L, 20–200 Hz.
    assert!(same_bits(
        &with(&|eq| eq.set(param(1, Field::Freq), 5_000.0)),
        &as_settings(&|s| s.bands[1].freq = 200.0)
    ));
    // A zone of ML takes the frequency into it.
    assert!(same_bits(
        &with(&|eq| eq.set(param(1, Field::Zone), Zone::High.code())),
        &as_settings(&|s| {
            s.bands[1].zone = Some(Zone::High);
            s.bands[1].freq = 2_000.0;
        })
    ));
    assert!(same_bits(
        &with(&|eq| eq.set(param(2, Field::Gain), 40.0)),
        &as_settings(&|s| s.bands[2].gain_db = 20.0)
    ));
    assert!(same_bits(
        &with(&|eq| eq.set(param(2, Field::Q), 0.01)),
        &as_settings(&|s| s.bands[2].q = 0.3)
    ));
    assert!(same_bits(
        &with(&|eq| eq.set(param(0, Field::Slope), 20.0)),
        &as_settings(&|s| s.bands[0].slope = 18)
    ));
    for ignored in [
        (param(0, Field::Kind), Kind::Notch.code()),
        (param(5, Field::Kind), Kind::HighPass.code()),
        (param(2, Field::Kind), Kind::LowPass.code()),
        (param(2, Field::Kind), 2.5),
        (param(0, Field::Zone), Zone::Low.code()),
        (param(2, Field::Gain), f32::NAN),
        (param(2, Field::Freq), f32::INFINITY),
        (param(6, Field::Gain), 3.0),
        (columbina_audio::ParamId(2), 3.0),
    ] {
        assert!(
            same_bits(&with(&|eq| eq.set(ignored.0, ignored.1)), &unchanged),
            "{ignored:?}"
        );
    }
    // The slope of a clip is 12 dB an octave.
    let clip_with = |slope: f32| {
        let mut eq = Eq::new(&clip(|s| s.bands[0].on = true));
        eq.set(param(0, Field::Slope), slope);
        output_of(eq)
    };
    assert!(same_bits(&clip_with(24.0), &clip_with(12.0)));
    let mut sloped = clip(|s| s.bands[0].on = true);
    sloped.bands[0].slope = 6;
    assert_eq!(sloped.sanitized().bands[0].slope, 12);
}

#[test]
fn the_new_eqs_are_those_of_the_model() {
    let strip = Settings::strip();
    assert!(!strip.on);
    assert_eq!(strip.bands().len(), 6);
    let kinds: Vec<_> = strip
        .bands()
        .iter()
        .map(|b| (b.on, b.kind, b.freq))
        .collect();
    assert_eq!(
        kinds,
        [
            (false, Kind::HighPass, 80.0),
            (true, Kind::Bell, 100.0),
            (true, Kind::Bell, 500.0),
            (true, Kind::Bell, 2_000.0),
            (true, Kind::Bell, 8_000.0),
            (false, Kind::LowPass, 16_000.0),
        ]
    );
    let zones: Vec<_> = strip.bands().iter().map(|b| b.zone).collect();
    assert_eq!(
        zones,
        [
            None,
            Some(Zone::Low),
            Some(Zone::MediumLow),
            Some(Zone::MediumHigh),
            Some(Zone::High),
            None
        ]
    );
    assert_eq!(Zone::Low.bounds(), (20.0, 200.0));
    assert_eq!(Zone::High.bounds(), (2_000.0, 20_000.0));
    let clip = Settings::clip();
    let kinds: Vec<_> = clip
        .bands()
        .iter()
        .map(|b| (b.on, b.kind, b.freq))
        .collect();
    assert_eq!(
        kinds,
        [
            (false, Kind::HighPass, 80.0),
            (true, Kind::Bell, 300.0),
            (true, Kind::Bell, 3_000.0),
            (false, Kind::LowPass, 16_000.0),
        ]
    );
    for settings in [strip, clip] {
        assert_eq!(settings.sanitized(), settings);
        assert!(settings
            .bands()
            .iter()
            .all(|b| b.q == 1.0 && b.gain_db == 0.0 && b.slope == 12));
    }
    // The codes are the places in the model's `EqKind`.
    assert_eq!(Kind::LowShelf.code(), 0.0);
    assert_eq!(Kind::Notch.code(), 5.0);
    assert_eq!(Kind::from_code(3.0), Some(Kind::HighPass));
    assert_eq!(param(0, Field::On).0, 16);
    assert_eq!(param(5, Field::Zone).0, 16 + 40 + 6);
}

#[test]
fn the_butterworth_qs_are_those_of_the_angles_of_the_poles() {
    for order in 2..=4_u8 {
        let qs = butterworth_q(order);
        assert_eq!(qs.len(), usize::from(order / 2));
        for (pair, q) in qs.iter().enumerate() {
            let expected = butterworth_q_of(order, u8::try_from(pair).unwrap() + 1);
            assert!((q - expected).abs() < 1e-12, "{order}: {q}, {expected}");
        }
    }
    assert!((butterworth_q(4)[0] - 0.5412).abs() < 1e-4);
    assert!((butterworth_q(4)[1] - 1.3066).abs() < 1e-4);
}

// --- Cost ------------------------------------------------------------------------------------

/// The cost of a 6-band strip on stereo at 48 kHz, every band on, still and with every band's
/// frequency, gain and Q automated (designed again at every sample). Run in release:
/// `cargo test --release -p columbina-audio --test eq -- --ignored --nocapture`.
#[test]
#[ignore = "a measure of time, run in release by hand"]
#[allow(clippy::cast_precision_loss, reason = "the numbers of a measure")]
fn the_cost_of_a_strip_on_stereo() {
    let frames = 10 * RATE as usize;
    let input = noises(2, frames);
    let mut settings = busy_strip();
    settings.bands[3].kind = Kind::Bell;
    settings.bands[3].gain_db = 3.0;
    let still = Run::new(input.clone());
    let mut moving = Run::new(input);
    for band in 0..6 {
        let lane: Vec<f32> = (0..frames)
            .map(|n| 1_000.0 + 500.0 * ((n as f32) / 4_000.0 + band as f32).sin())
            .collect();
        moving = moving
            .with_automation(param(band, Field::Freq), lane.clone())
            .with_automation(
                param(band, Field::Gain),
                lane.iter().map(|f| f / 200.0).collect(),
            )
            .with_automation(
                param(band, Field::Q),
                lane.iter().map(|f| f / 500.0).collect(),
            );
    }
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    for (what, run) in [("still", still), ("automated", moving)] {
        let mut eq = Eq::new(&settings);
        eq.prepare(&spec).unwrap();
        let start = std::time::Instant::now();
        let output = render(&mut eq, &spec, &run, |_| 480);
        let elapsed = start.elapsed();
        assert!(output[0].iter().all(|x| x.is_finite()));
        let per_sample = elapsed.as_secs_f64() * 1e9 / frames as f64;
        println!(
            "{what}: {per_sample:.1} ns a sample of stereo, {:.2} % of real time",
            elapsed.as_secs_f64() / 10.0 * 100.0
        );
    }
}

// --- The review of T7 ------------------------------------------------------------------------

/// An EQ that reports the settling of other settings: what the graph does with `settle_for`
/// and the worst values of the lanes in the preroll (T4b).
struct Settled {
    eq: Eq,
    settle: u32,
}

impl Node for Settled {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        self.eq.prepare(spec)
    }
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.eq.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.eq.latency()
    }
    fn settle(&self) -> u32 {
        self.settle
    }
    fn reset(&mut self) {
        self.eq.reset();
    }
    fn set(&mut self, param: ParamId, value: f32) {
        self.eq.set(param, value);
    }
}

#[test]
fn a_lane_slower_than_the_setting_settles_in_the_time_of_its_worst_values() {
    // A bell of +20 dB and Q 10.3 set at 5 kHz, its lane at 20 Hz, a tone of 20 Hz.
    let set = clip(|s| {
        s.bands[1].freq = 5_000.0;
        s.bands[1].gain_db = 20.0;
        s.bands[1].q = 10.3;
    });
    let mut worst = set;
    worst.bands[1].freq = 20.0;
    let settle = settle_for(&worst, RATE);
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let from = settle as usize;
    let frames = 2 * from + 4_800;
    let run = Run::new(vec![sine(RATE, 20.0, 0.25, frames)])
        .with_automation(param(1, Field::Freq), vec![20.0; frames]);
    assert_settle(
        || Settled {
            eq: Eq::new(&set),
            settle,
        },
        &spec,
        &run,
        from,
    );
}

#[test]
fn an_eq_set_off_and_turned_on_by_its_lane_settles_as_reported() {
    let mut settings = high_pass(24);
    settings.bands[0].freq = 20.0;
    settings.on = false;
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let mut probe = Eq::new(&settings);
    probe.prepare(&spec).unwrap();
    let from = probe.settle() as usize;
    let frames = 2 * from + 4_800;
    let run = Run::new(vec![sine(RATE, 25.0, 0.5, frames)]).with_automation(ON, vec![1.0; frames]);
    assert_settle(|| Eq::new(&settings), &spec, &run, from);
    // And a band set off, turned on by its lane.
    let mut band_off = high_pass(24);
    band_off.bands[0].freq = 20.0;
    band_off.bands[0].on = false;
    let run = Run::new(vec![sine(RATE, 25.0, 0.5, frames)])
        .with_automation(param(0, Field::On), vec![1.0; frames]);
    assert_settle(|| Eq::new(&band_off), &spec, &run, from);
}

#[test]
fn settle_for_counts_every_band_as_on_and_is_what_the_eq_reports() {
    let settings = busy_strip();
    let mut off = settings;
    off.on = false;
    for band in &mut off.bands {
        band.on = false;
    }
    assert_eq!(settle_for(&off, RATE), settle_for(&settings, RATE));
    let mut eq = Eq::new(&off);
    eq.prepare(&Spec::new(RATE, 512, Layout::Mono)).unwrap();
    assert_eq!(eq.settle(), settle_for(&settings, RATE));
    // Slower poles settle later.
    let mut slower = settings;
    slower.bands[2].q = 10.0;
    assert!(settle_for(&slower, RATE) > settle_for(&settings, RATE));
}

#[test]
fn the_ramps_that_follow_a_lane_are_counted_from_the_widest_range() {
    // Two values following the same lane from the ends of a range come within 1e-6 of each
    // other in LANE_SETTLE ramps: (1 − 1/N)^(kN) < e^(−k).
    let widest = [
        f64::from(FREQUENCY_RANGE.1 / FREQUENCY_RANGE.0).ln(),
        10_f64.powf(f64::from(GAIN_RANGE_DB.1) / 20.0)
            - 10_f64.powf(f64::from(GAIN_RANGE_DB.0) / 20.0),
        f64::from(Q_RANGE.1 / Q_RANGE.0).ln(),
    ]
    .into_iter()
    .fold(0.0, f64::max);
    #[allow(clippy::cast_sign_loss, reason = "a count")]
    let ramps = (widest / 1e-6).ln().ceil() as u64;
    assert_eq!(LANE_SETTLE, ramps);
}

/// The response in dB of a shelf of the cookbook (S given), written out again here.
fn cookbook_shelf_db(rate: f64, f0: f64, gain_db: f64, slope: f64, high: bool, freq: f64) -> f64 {
    let a = 10_f64.powf(gain_db / 40.0);
    let w0 = TAU * f0 / rate;
    let (c, s) = (w0.cos(), w0.sin());
    let alpha = s / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
    let k = 2.0 * a.sqrt() * alpha;
    let (b, d) = if high {
        (
            [
                a * ((a + 1.0) + (a - 1.0) * c + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - k),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + k,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - k,
            ],
        )
    } else {
        (
            [
                a * ((a + 1.0) - (a - 1.0) * c + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - k),
            ],
            [
                (a + 1.0) + (a - 1.0) * c + k,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - k,
            ],
        )
    };
    let w = TAU * freq / rate;
    let at = |c: [f64; 3]| {
        let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
        let im = -(c[1] * w.sin() + c[2] * (2.0 * w).sin());
        re.hypot(im)
    };
    20.0 * (at(b) / at(d)).log10()
}

#[test]
fn the_shelves_are_those_of_the_cookbook_and_half_their_gain_at_their_frequency() {
    for (kind, high) in [(Kind::LowShelf, false), (Kind::HighShelf, true)] {
        for gain_db in [12.0, -9.0] {
            let settings = clip(|s| {
                s.bands[1] = Band {
                    gain_db,
                    ..Band::new(true, kind, 1_000.0, None)
                };
            });
            let h = impulse_response(&settings, RATE, 1 << 15);
            for freq in log_frequencies(50) {
                let expected = cookbook_shelf_db(
                    f64::from(RATE),
                    1_000.0,
                    f64::from(gain_db),
                    1.0,
                    high,
                    freq,
                );
                let measured = gain_at(&h, RATE, freq);
                assert!(
                    (measured - expected).abs() <= 0.01,
                    "{kind:?} {gain_db}: {freq} Hz: {measured}, the cookbook {expected}"
                );
            }
            let (low_end, high_end) = if high {
                (0.0, f64::from(gain_db))
            } else {
                (f64::from(gain_db), 0.0)
            };
            let at = gain_at(&h, RATE, 1_000.0);
            assert!(
                (at - f64::from(gain_db) / 2.0).abs() <= 0.01,
                "{kind:?} at f0: {at}"
            );
            let at = gain_at(&h, RATE, 20.0);
            assert!((at - low_end).abs() <= 0.1, "{kind:?} at 20 Hz: {at}");
            let at = gain_at(&h, RATE, 20_000.0);
            assert!((at - high_end).abs() <= 0.1, "{kind:?} at 20 kHz: {at}");
        }
    }
}

/// The frequency between `low` and `high` at which the response of `h` crosses `level` dB, by
/// halving; `rising` when it is below `level` at `low`.
fn crossing(h: &[f32], level: f64, mut low: f64, mut high: f64, rising: bool) -> f64 {
    for _ in 0..50 {
        let middle = (low * high).sqrt();
        if (gain_at(h, RATE, middle) < level) == rising {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low * high).sqrt()
}

#[test]
fn a_notch_is_three_db_down_over_its_frequency_over_q() {
    for q in [2.0_f32, 5.0] {
        let settings = clip(|s| {
            s.bands[1] = Band {
                q,
                ..Band::new(true, Kind::Notch, 1_000.0, None)
            };
        });
        let h = impulse_response(&settings, RATE, 1 << 15);
        let half = -10.0 * 2_f64.log10();
        let below = crossing(&h, half, 100.0, 1_000.0, false);
        let above = crossing(&h, half, 1_000.0, 10_000.0, true);
        let width = above - below;
        let expected = 1_000.0 / f64::from(q);
        assert!(
            (width / expected - 1.0).abs() <= 0.005,
            "Q {q}: {below}–{above}, {width} Hz wide"
        );
        assert!(((below * above).sqrt() / 1_000.0 - 1.0).abs() <= 0.005);
    }
}

#[test]
fn values_of_lanes_that_are_not_numbers_keep_the_values_set() {
    let frames = 9_000;
    let with_nan = |at: usize, value: f32| {
        let mut lane = vec![0.0; frames];
        lane[at] = value;
        lane
    };
    let mut freq = vec![700.0; frames];
    freq[3_000] = f32::NAN;
    let mut q = vec![3.0; frames];
    q[4_000] = f32::INFINITY;
    let mut on = vec![1.0; frames];
    on[5_000] = f32::NAN;
    let run = Run::new(vec![noise(35, 0.5, frames)])
        .with_automation(GAIN, with_nan(1_000, f32::NAN))
        .with_automation(param(2, Field::Gain), with_nan(2_000, f32::NAN))
        .with_automation(param(2, Field::Freq), freq)
        .with_automation(param(2, Field::Q), q)
        .with_automation(ON, on.clone())
        .with_automation(param(3, Field::On), on);
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut eq = Eq::new(&busy_strip());
    eq.prepare(&spec).unwrap();
    let output = render(&mut eq, &spec, &run, |_| 500).remove(0);
    if let Some(n) = output.iter().position(|x| !x.is_finite()) {
        panic!("sample {n}: {}", output[n]);
    }
}

#[test]
fn a_band_is_designed_at_most_at_0_45_of_the_rate() {
    let settings = strip(|s| s.bands[5] = Band::new(true, Kind::LowPass, 20_000.0, None));
    let h = impulse_response(&settings, 32_000, 1 << 14);
    let at = gain_at(&h, 32_000, 14_400.0);
    assert!((at + 3.0103).abs() <= 0.05, "{at} dB at 14.4 kHz");
}

#[test]
fn a_frequency_ramps_in_its_logarithm_and_a_gain_in_its_amplitude() {
    let settings = clip(|s| {
        s.bands[1].freq = 100.0;
        s.bands[1].gain_db = 0.0;
    });
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut eq = Eq::new(&settings);
    eq.prepare(&spec).unwrap();
    eq.set(param(1, Field::Freq), 10_000.0);
    eq.set(param(1, Field::Gain), 20.0);
    let half = samples(columbina_audio::constants::SMOOTHING, RATE) / 2;
    let input = vec![0.0; half];
    let mut output = vec![0.0; half];
    eq.process(
        &Block::new(0, RATE, half),
        &[&input],
        &mut [output.as_mut_slice()],
    );
    let (freq, gain_db, q) = eq.heard(1);
    assert!((freq / 1_000.0 - 1.0).abs() < 1e-4, "{freq} Hz half way");
    let gain = 10_f32.powf(gain_db / 20.0);
    assert!((gain - 5.5).abs() < 1e-4, "{gain} half way from 1 to 10");
    assert_eq!(q, 1.0);
}

#[test]
fn the_eq_takes_no_sidechain() {
    let mut eq = Eq::strip();
    assert!(eq
        .prepare(&Spec::new(RATE, 512, Layout::Stereo).with_sidechain(2))
        .is_err());
}
