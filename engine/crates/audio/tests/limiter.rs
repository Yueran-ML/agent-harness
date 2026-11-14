//! Limiter (T10): the true peak of a sine of a quarter of the rate at 45°, a loud tone brought
//! under the ceiling in true peak (read by the interpolator, by an interpolator of 16 points of
//! our own, and by ffmpeg's `ebur128` when there is one), no sample over the ceiling, a quiet
//! sound to the bit after 64 samples, the attack inside the lookahead and its raised cosine with
//! Soft, the release by its time constant, Input Level before it, the sidechain and Listen, ramps
//! of 10 ms, the same output in any blocks, no memory taken, no zipper, a latency of 64, a
//! settling that holds, input that is not a number, the meters, and its parameters.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::manual_midpoint,
    clippy::similar_names,
    clippy::needless_range_loop,
    reason = "exact values compared on purpose, sample numbers, levels of tests, formulas as written"
)]

mod common;

use std::f64::consts::{FRAC_PI_4, PI, TAU};

use columbina_audio::constants::{LIMITER_LOOKAHEAD, LIMITER_TRUE_PEAK_MARGIN_DB};
use columbina_audio::fx::limiter::{
    settle_for, Limiter, Param, Settings, ATTACK, HOLD, TRUE_PEAK_DELAY,
};
use columbina_audio::fx::true_peak::TruePeak;
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, render, CountingAllocator, Run,
};
use columbina_audio::{Layout, Node, Spec};
use common::{amplitude_at, db, no_zipper, peak, process, realtime_work, sine_at, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const L: usize = LIMITER_LOOKAHEAD;

/// The level to which a steady sound is brought under a ceiling of `ceiling_db`: the ceiling less
/// the margin for what peaks between the points read.
fn held(ceiling_db: f64) -> f64 {
    10_f64.powf((ceiling_db - f64::from(LIMITER_TRUE_PEAK_MARGIN_DB)) / 20.0)
}

fn limiter(edit: impl FnOnce(&mut Settings)) -> Limiter {
    let mut settings = Settings::default();
    edit(&mut settings);
    Limiter::new(&settings)
}

/// The true peak of `signal` by an interpolator of our own, independent of the limiter's: 16
/// points between each two samples, a sinc windowed by Blackman over ±32 samples, from the sample
/// `from` to `to`, as a gain of amplitude.
fn true_peak_16(signal: &[f32], from: usize, to: usize) -> f64 {
    const HALF: isize = 32;
    let mut peak = 0.0_f64;
    for n in from..to {
        for point in 0..16 {
            let t = n as f64 + f64::from(point) / 16.0;
            let mut sum = 0.0;
            for k in (n as isize - HALF + 1)..=(n as isize + HALF) {
                let Some(&x) = usize::try_from(k).ok().and_then(|k| signal.get(k)) else {
                    continue;
                };
                let u = t - k as f64;
                let sinc = if u == 0.0 {
                    1.0
                } else {
                    (PI * u).sin() / (PI * u)
                };
                let r = (u / HALF as f64 + 1.0) / 2.0;
                let window = 0.42 - 0.5 * (2.0 * PI * r).cos() + 0.08 * (4.0 * PI * r).cos();
                sum += f64::from(x) * sinc * window;
            }
            peak = peak.max(sum.abs());
        }
    }
    peak
}

/// The true peak that [`TruePeak`] reads of `signal` from its sample `from` on: the readings of
/// the steady part, not of its start out of silence, where a step rings.
fn steady_true_peak(signal: &[f32], from: usize) -> f64 {
    let mut detector = TruePeak::new();
    signal
        .iter()
        .enumerate()
        .map(|(n, x)| (n, detector.push(f64::from(*x))))
        .filter(|&(n, _)| n >= from + TRUE_PEAK_DELAY)
        .fold(0.0, |peak, (_, reading)| peak.max(reading))
}

#[test]
fn the_true_peak_of_a_quarter_of_the_rate_at_45_degrees_is_3_db_over_its_samples() {
    // Samples of ±0.3536 (−9.03 dBFS) of a sine of 0.5 (−6.02 dBFS) between them.
    let signal = sine_at(12_000.0, 0.5, FRAC_PI_4, 4_800);
    assert!((db(f64::from(peak(&signal))) + 9.031).abs() < 0.01);
    let true_peak = db(steady_true_peak(&signal, 100));
    assert!((true_peak + 6.021).abs() < 0.01, "{true_peak}");
    // Read from silence, the start of the tone rings higher.
    assert!(f64::from(TruePeak::of(&signal)) >= 0.5);
    // A sine of any frequency below 0.3 of the rate and any phase: its amplitude within 0.05 dB.
    for (freq, phase) in [
        (997.0, 0.3),
        (5_000.0, 1.1),
        (9_871.0, 2.0),
        (14_000.0, 0.7),
    ] {
        let signal = sine_at(freq, 0.7, phase, 9_600);
        let error = db(steady_true_peak(&signal, 100) / 0.7);
        assert!(error.abs() < 0.05, "{freq} Hz: {error} dB");
    }
    // The interval read lags the sample pushed by 6; its first point is the sample itself.
    let mut detector = TruePeak::new();
    let mut impulse = [0.0; 40];
    impulse[2] = 0.8;
    let readings: Vec<f64> = impulse.iter().map(|&x| detector.push(x)).collect();
    assert_eq!(readings[2 + TRUE_PEAK_DELAY], 0.8);
}

/// The output of a limiter of `edit` for `input` (each channel), processed in blocks of 480.
fn limited(edit: impl FnOnce(&mut Settings), input: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let spec = Spec::new(RATE, 512, Layout::discrete(input.len()));
    let mut node = limiter(edit);
    node.prepare(&spec).unwrap();
    render(&mut node, &spec, &Run::new(input), |_| 480)
}

/// The response of `taps` (oldest sample first) at `freq` (a part of the rate), against the
/// delay of the point it reads, `delay` samples behind the newest: the error of the point from
/// the sound, as the size of their difference, a complex number.
fn interpolation_error(taps: &[f64], freq: f64, delay: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    let last = (taps.len() - 1) as f64;
    for (i, tap) in taps.iter().enumerate() {
        // The sample i is (last − i) behind the newest, (last − i − delay) behind the point.
        let lag = last - i as f64 - delay;
        re += tap * (TAU * freq * lag).cos();
        im -= tap * (TAU * freq * lag).sin();
    }
    (re - 1.0).hypot(im)
}

#[test]
fn the_interpolator_reads_8_points_flat_to_0_45_of_the_rate() {
    let detector = TruePeak::new();
    for point in 1..8 {
        let taps = detector.taps(point);
        assert!((taps.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        // The point k/8 after the sample 24 behind the newest: 23 + (8 − k)/8 behind it.
        let delay = TRUE_PEAK_DELAY as f64 - point as f64 / 8.0;
        for step in 0..=30 {
            let freq = 0.3 + 0.005 * f64::from(step);
            let error = interpolation_error(taps, freq, delay);
            // 6e-4 of the sound: 0.005 dB.
            assert!(error < 6e-4, "point {point} at {freq}: {error}");
        }
    }
    // Half way between two samples, near the Nyquist frequency: −2.5 dB at 0.48 of the rate.
    let half = detector.taps(4);
    let (mut re, mut im) = (0.0, 0.0);
    for (i, tap) in half.iter().enumerate() {
        re += tap * (TAU * 0.48 * i as f64).cos();
        im -= tap * (TAU * 0.48 * i as f64).sin();
    }
    let at_048 = db(re.hypot(im));
    assert!((-2.7..-2.3).contains(&at_048), "{at_048}");
    // A sine of 0.45 of the rate is read within 0.15 dB under its amplitude, whatever its phase.
    for phase in [0.0, 0.4, 1.1, 2.3] {
        let signal = sine_at(21_600.0, 0.5, phase, 9_600);
        let error = db(steady_true_peak(&signal, 200) / 0.5);
        assert!((-0.15..0.01).contains(&error), "{phase}: {error}");
    }
}

/// Gaussian noise of `deviation`, the same on every machine (the generator of the review's
/// probe).
fn gaussian(seed: u64, deviation: f64, frames: usize) -> Vec<f64> {
    let mut state = seed;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1_u64 << 53) as f64
    };
    (0..frames)
        .map(|_| {
            let u = next().max(1e-12);
            let v = next();
            deviation * (-2.0 * u.ln()).sqrt() * (TAU * v).cos()
        })
        .collect()
}

/// Bursts every `period` samples of `f` of the distance from the middle of each.
fn bursts(frames: usize, period: usize, f: impl Fn(isize) -> f64) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            let middle = (n / period) * period + period / 2;
            f(n as isize - middle as isize) as f32
        })
        .collect()
}

/// The signals of the review that read far over the ceiling through the interpolator of 4 points:
/// hard-clipped noise, the signs of a sinc, and an alternating sign, the last two in bursts.
fn wideband() -> Vec<(&'static str, Vec<f32>)> {
    let frames = 48_000;
    let clipped = |deviation: f64| -> Vec<f32> {
        gaussian(0x1234_5678_9abc_def1, deviation, frames)
            .into_iter()
            .map(|x| x.clamp(-1.0, 1.0) as f32)
            .collect()
    };
    let sinc_sign = |k: isize, amplitude: f64| {
        bursts(frames, 4_800, move |m| {
            if m.abs() <= k {
                let u = m as f64 - 0.5;
                amplitude * ((PI * u).sin() / (PI * u)).signum()
            } else {
                0.0
            }
        })
    };
    vec![
        ("clipped noise ×3", clipped(3.0)),
        ("clipped noise ×10", clipped(10.0)),
        ("sinc signs 8", sinc_sign(8, 0.9)),
        ("sinc signs 24", sinc_sign(24, 2.0)),
        (
            "alternating",
            bursts(frames, 4_800, |m| {
                if m.abs() < 300 {
                    let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                    if m < 0 {
                        sign
                    } else {
                        -sign
                    }
                } else {
                    0.0
                }
            }),
        ),
    ]
}

#[test]
fn wideband_sound_comes_out_under_the_ceiling_in_true_peak() {
    for (name, input) in wideband() {
        for soft in [false, true] {
            let mut input = input.clone();
            input.extend(std::iter::repeat_n(0.0, 2_000));
            let frames = input.len();
            let output = limited(|s| s.soft = soft, vec![input]).remove(0);
            let ours = db(true_peak_16(&output, 0, frames));
            assert!(
                ours <= -1.0 + 0.5,
                "{name}, soft {soft}: {ours} dBTP by 16 points"
            );
            if let Some(read) = ffmpeg_true_peak(&[output]) {
                eprintln!("{name}, soft {soft}: ffmpeg {read} dBTP, 16 points {ours:.2} dBTP");
                assert!(
                    read <= -1.0 + 0.2,
                    "{name}, soft {soft}: ffmpeg reads {read} dBTP"
                );
            }
        }
    }
}

#[test]
fn a_tone_of_6_dbfs_comes_out_under_a_ceiling_of_minus_1_dbtp() {
    let input = sine_at(997.0, 2.0, 0.0, 96_000);
    let output = limited(|_| {}, vec![input.clone(), input]).remove(0);
    let tail = &output[48_000..];
    // The tone is a tone still: its amplitude is its true peak.
    let amplitude = db(amplitude_at(tail, 997.0));
    // The ceiling less the margin of 0.2 dB.
    assert!((-1.25..=-1.15).contains(&amplitude), "{amplitude}");
    let read = db(steady_true_peak(&output, 48_000));
    assert!(read <= -1.0 + 1e-6, "{read}");
    let ours = db(true_peak_16(&output, 48_000, 52_800));
    assert!(ours <= -0.9, "{ours}");
}

/// The true peak that ffmpeg's `ebur128` reads of `channels` (48 kHz), if there is an ffmpeg
/// (`FFMPEG`, else the LGPL build of the toolchain, else `ffmpeg` on the path).
fn ffmpeg_true_peak(channels: &[Vec<f32>]) -> Option<f64> {
    let frames = channels[0].len();
    let count = channels.len();
    let mut wav = Vec::new();
    let data = u32::try_from(frames * count * 4).unwrap();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&3_u16.to_le_bytes());
    wav.extend_from_slice(&u16::try_from(count).unwrap().to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 4 * u32::try_from(count).unwrap()).to_le_bytes());
    wav.extend_from_slice(&u16::try_from(4 * count).unwrap().to_le_bytes());
    wav.extend_from_slice(&32_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for n in 0..frames {
        for channel in channels {
            wav.extend_from_slice(&channel[n].to_le_bytes());
        }
    }
    let path = std::env::temp_dir().join(format!("columbina-limiter-{}.wav", std::process::id()));
    std::fs::write(&path, &wav).unwrap();
    // The LGPL build of the toolchain (convention 5), unless FFMPEG names another.
    let lgpl = r"F:\agent harness\_review\toolchain\ffmpeg-lgpl-8.1\bin\ffmpeg.exe";
    let ffmpeg = std::env::var("FFMPEG").unwrap_or_else(|_| {
        if std::path::Path::new(lgpl).exists() {
            lgpl.into()
        } else {
            "ffmpeg".into()
        }
    });
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(&path)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output();
    let _ = std::fs::remove_file(&path);
    let output = output.ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    let summary = &text[text.rfind("True peak:")?..];
    let line = summary
        .lines()
        .find(|line| line.trim().starts_with("Peak:"))?;
    line.trim()
        .trim_start_matches("Peak:")
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[test]
fn ffmpeg_reads_the_tone_under_the_ceiling() {
    let input = sine_at(997.0, 2.0, 0.0, 5 * 48_000);
    let output = limited(|_| {}, vec![input.clone(), input]);
    // A second for the release to settle, then four.
    let tail: Vec<Vec<f32>> = output.iter().map(|c| c[48_000..].to_vec()).collect();
    let Some(true_peak) = ffmpeg_true_peak(&tail) else {
        eprintln!("no ffmpeg: set FFMPEG or put it on the path to read the true peak with it");
        return;
    };
    eprintln!("ffmpeg's ebur128 reads a true peak of {true_peak} dBTP");
    assert!(true_peak <= -1.0 + 0.1, "ffmpeg reads {true_peak} dBTP");
    assert!(true_peak >= -1.5, "ffmpeg reads {true_peak} dBTP");
}

#[test]
fn a_quiet_sound_passes_to_the_bit_after_64_samples() {
    let input = vec![
        sine_at(997.0, 0.1, 0.0, 48_000),
        sine_at(440.0, 0.1, 1.0, 48_000),
    ];
    let output = limited(|_| {}, input.clone());
    for (channel, output) in output.iter().enumerate() {
        assert!(output[..L].iter().all(|x| *x == 0.0));
        assert_eq!(output[L..], input[channel][..48_000 - L]);
    }
    // Off, anything passes so, delayed the same.
    let loud = vec![noise(1, 4.0, 9_600)];
    let off = limited(|s| s.on = false, loud.clone()).remove(0);
    assert_eq!(off[L..], loud[0][..9_600 - L]);
}

#[test]
fn a_lane_that_turns_the_insert_off_lets_the_input_through_after_ten_ms() {
    let input = noise(70, 4.0, 9_600);
    let spec = Spec::new(RATE, 9_600, Layout::Mono);
    let lane: Vec<f32> = (0..9_600)
        .map(|n| if n < 2_000 { 1.0 } else { 0.0 })
        .collect();
    let run = Run::new(vec![input.clone()]).with_automation(Param::On.id(), lane);
    let output = process(limiter(|_| {}), &spec, &run).remove(0);
    assert_eq!(output[2_480..], input[2_480 - L..9_600 - L]);
    assert!(peak(&output[L..2_000]) <= 10_f32.powf(-1.0 / 20.0));
}

#[test]
fn a_tone_whose_samples_are_under_the_ceiling_is_limited_by_its_true_peak() {
    // Its samples at −9.03 dBFS, its true peak at −6.02 dBTP, the ceiling at −9: a limiter of
    // sample peaks would let it through.
    let input = sine_at(12_000.0, 0.5, FRAC_PI_4, 48_000);
    let output = limited(|s| s.ceiling_db = -9.0, vec![input]).remove(0);
    let amplitude = db(amplitude_at(&output[24_000..], 12_000.0));
    assert!((-9.3..=-8.95).contains(&amplitude), "{amplitude}");
}

#[test]
fn no_sample_goes_over_the_ceiling_and_program_stays_under_it_in_true_peak() {
    let frames = 48_000;
    // White noise at +20 dBFS: every sample, whatever the oversampling misses between them.
    for (soft, input_db, ceiling_db) in
        [(false, 0.0, -3.0), (true, 18.0, -24.0), (false, -6.0, 0.0)]
    {
        let input = vec![noise(2, 10.0, frames), noise(3, 3.0, frames)];
        let output = limited(
            |s| {
                s.soft = soft;
                s.input_db = input_db;
                s.ceiling_db = ceiling_db;
                s.release_ms = 5.0;
            },
            input,
        );
        let ceiling = 10_f32.powf(ceiling_db / 20.0);
        for channel in &output {
            assert!(
                peak(channel) <= ceiling * (1.0 + 1e-6),
                "{soft} {input_db} {ceiling_db}"
            );
        }
    }
    // Program below 10 kHz, in bursts, at up to +12 dBFS: under the ceiling between the samples
    // too, read 16 times a sample by an interpolator of its own.
    let mut program = vec![0.0_f32; frames];
    for (freq, amplitude, phase) in [(220.0, 1.2, 0.0), (1_234.0, 0.9, 1.0), (5_000.0, 0.6, 2.0)] {
        for (x, s) in program
            .iter_mut()
            .zip(sine_at(freq, amplitude, phase, frames))
        {
            *x += s;
        }
    }
    for (n, x) in program.iter_mut().enumerate() {
        let burst = if (n / 2_400) % 3 == 0 { 1.4 } else { 0.3 };
        *x *= burst;
    }
    let mut bright = sine_at(9_000.0, 1.5, 0.4, frames);
    for (n, x) in bright.iter_mut().enumerate() {
        if (n / 1_000) % 2 == 1 {
            *x = 0.0;
        }
    }
    for input in [program, bright] {
        let output = limited(|s| s.release_ms = 20.0, vec![input]).remove(0);
        let true_peak = db(true_peak_16(&output, 2_000, 26_000));
        assert!(true_peak <= -0.95, "{true_peak}");
    }
}

/// The gain of a limiter of `edit` on a constant `level` of the main input, keyed by its
/// sidechain `key` (so that what it needs comes from `key` alone): the output over `level`.
fn gains(edit: impl FnOnce(&mut Settings), level: f32, key: &[f32]) -> Vec<f64> {
    let frames = key.len();
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let mut node = limiter(|s| {
        s.sidechain_on = true;
        edit(s);
    });
    node.prepare(&spec).unwrap();
    let run = Run::new(vec![vec![level; frames]]).with_sidechain(vec![key.to_vec()]);
    let output = render(&mut node, &spec, &run, |_| 480).remove(0);
    output
        .iter()
        .map(|y| f64::from(*y) / f64::from(level))
        .collect()
}

#[test]
fn the_attack_falls_within_the_lookahead_and_soft_falls_by_a_raised_cosine() {
    // The key steps from silence to 2.0 at the sample 4000; the ceiling is −1. A step rings in
    // the interpolator, a little over 2.0 just after it: what it needs there is held a while and
    // released (in 1 ms here) to what 2.0 needs.
    let mut key = vec![0.0_f32; 9_600];
    key[4_000..].fill(2.0);
    // Where the gain goes: the ceiling less the margin.
    let ceiling = held(-1.0);
    let arrives = 4_000 + L;
    for soft in [false, true] {
        let gain = gains(
            |s| {
                s.soft = soft;
                s.release_ms = 1.0;
            },
            0.25,
            &key,
        );
        // Untouched (after the 64 samples of silence of the lookahead) until the lookahead, then
        // down, never back up, by the time the step comes
        // out; and from there on, under the ceiling.
        assert!(gain[L..arrives - L - 1].iter().all(|g| *g == 1.0));
        for n in arrives - L..arrives {
            assert!(gain[n + 1] <= gain[n], "{soft} at {n}");
        }
        for n in arrives..arrives + 2_000 {
            assert!(gain[n] * 2.0 <= ceiling * (1.0 + 1e-7), "{soft} at {n}");
        }
        assert!((gain[arrives + 2_000] - ceiling / 2.0).abs() < 1e-7);
    }
    let hard = gains(|s| s.release_ms = 1.0, 0.25, &key);
    let soft = gains(
        |s| {
            s.soft = true;
            s.release_ms = 1.0;
        },
        0.25,
        &key,
    );
    // A quarter into the attack: the box has fallen by about a quarter, the raised cosine by
    // about a tenth (0.25 − 1/2π).
    let deepest = 1.0 - ceiling / 2.0;
    let quarter = arrives - ATTACK * 3 / 4;
    let (hard_part, soft_part) = (
        (1.0 - hard[quarter]) / deepest,
        (1.0 - soft[quarter]) / deepest,
    );
    assert!((0.15..0.35).contains(&hard_part), "{hard_part}");
    assert!(soft_part < 0.5 * hard_part, "{soft_part} {hard_part}");
    // No corner: the box falls by about 1/59 of the way a sample, never by much more.
    let largest = hard.windows(2).map(|w| w[0] - w[1]).fold(0.0, f64::max);
    assert!(largest < 1.5 * deepest / ATTACK as f64, "{largest}");
}

#[test]
fn the_release_returns_by_its_time_constant() {
    for release_ms in [100.0, 500.0, 10.0] {
        let mut key = vec![0.0_f32; 4 * 48_000];
        key[4_000..8_800].fill(2.0);
        let gain = gains(|s| s.release_ms = release_ms, 0.25, &key);
        // Where the release starts: the last of the deepest gain as the burst comes out (the
        // mean full of what is held, which falls from the next sample on).
        let around = 8_800..8_800 + L + HOLD + ATTACK + 50;
        let lowest = around.clone().map(|n| gain[n]).fold(1.0, f64::min);
        let start = around.rev().find(|&n| gain[n] == lowest).unwrap();
        let deepest = 1.0 - lowest;
        let back = (start..gain.len())
            .find(|&n| 1.0 - gain[n] <= deepest / std::f64::consts::E)
            .unwrap();
        // The release of the mean of 59 values lags the release held by 29 samples.
        let expected = f64::from(release_ms) / 1000.0 * f64::from(RATE) + (ATTACK - 1) as f64 / 2.0;
        let measured = (back - start) as f64;
        assert!(
            (measured / expected - 1.0).abs() < 0.02,
            "{release_ms} ms: {measured} samples, {expected} expected"
        );
        // It starts back once the burst has come out and been held.
        assert!(
            (8_800 + L - 1..=8_800 + L + HOLD).contains(&start),
            "{start}"
        );
    }
}

#[test]
fn input_level_is_before_the_limiter_and_ramps_over_ten_ms() {
    let input = sine_at(440.0, 0.1, 0.0, 9_600);
    let output = limited(|s| s.input_db = 6.0, vec![input.clone()]).remove(0);
    let gain = 10_f64.powf(6.0 / 20.0);
    for n in L..9_600 {
        let expected = f64::from(input[n - L]) * gain;
        assert!((f64::from(output[n]) - expected).abs() < 1e-7);
    }
    // Pushed over the ceiling by Input Level alone: limited.
    let output = limited(|s| s.input_db = 18.0, vec![input]).remove(0);
    assert!(peak(&output) <= 10_f32.powf(-1.0 / 20.0));
    // A constant level: the ramp, sample by sample, 64 later.
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![vec![0.01; 4_800]]).with_change(1_000, Param::InputDb.id(), 18.0);
    let output = process(limiter(|_| {}), &spec, &run).remove(0);
    let top = 10_f64.powf(18.0 / 20.0) as f32;
    for k in 0..480 {
        let expected = 0.01 * (1.0 + (top - 1.0) * (k + 1) as f32 / 480.0);
        assert!((output[1_000 + L + k] - expected).abs() < 1e-6, "{k}");
    }
}

#[test]
fn every_ramp_is_half_way_after_five_ms_the_release_in_its_logarithm() {
    let spec = Spec::new(RATE, 240, Layout::Mono);
    let quiet = Run::new(vec![vec![0.0; 240]]);
    let mut node = limiter(|s| {
        s.release_ms = 10.0;
        s.ceiling_db = 0.0;
    });
    node.prepare(&spec).unwrap();
    node.set(Param::InputDb.id(), 6.020_6);
    node.set(Param::CeilingDb.id(), -6.020_6);
    node.set(Param::ReleaseMs.id(), 1_000.0);
    render(&mut node, &spec, &quiet, |left| left);
    let (input, ceiling, release) = node.heard();
    assert!((input - 1.5).abs() < 1e-4, "{input}");
    assert!((ceiling - 0.75).abs() < 1e-4, "{ceiling}");
    // √(10 · 1000) = 100, not 505.
    assert!((release - 100.0).abs() < 1e-3, "{release}");
    render(&mut node, &spec, &quiet, |left| left);
    assert_eq!(node.heard().2, 1_000.0);
    // A ceiling lowered on a steady level: the output follows it down over the ramp, not in the
    // 59 samples of the attack.
    let spec = Spec::new(RATE, 9_600, Layout::Mono);
    let run = Run::new(vec![vec![2.0; 9_600]]).with_change(4_000, Param::CeilingDb.id(), -12.0);
    // A release of 1 ms lets the ringing of the step at the start go by.
    let output = process(limiter(|s| s.release_ms = 1.0), &spec, &run).remove(0);
    let (high, low) = (held(-1.0) as f32, held(-12.0) as f32);
    assert!((output[3_900] - high).abs() < 1e-6, "{}", output[3_900]);
    assert!((output[9_000] - low).abs() < 1e-6, "{}", output[9_000]);
    let largest = output[1_000..]
        .windows(2)
        .map(|w| (w[0] - w[1]).abs())
        .fold(0.0, f32::max);
    assert!(largest <= 1.2 * (high - low) / 480.0, "{largest}");
}

#[test]
fn every_change_makes_no_zipper() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let tone = vec![sine_at(1_000.0, 1.0, 0.0, 24_000)];
    let at = 12_012;
    no_zipper(
        |t| limiter(|s| s.ceiling_db = -1.0 - 11.0 * t),
        &spec,
        &tone,
        12,
        (Param::CeilingDb.id(), -12.0),
        at,
    );
    no_zipper(
        |t| limiter(|s| s.input_db = 12.0 * t),
        &spec,
        &tone,
        13,
        (Param::InputDb.id(), 12.0),
        at,
    );
    no_zipper(
        |t| limiter(|s| s.on = t < 0.5),
        &spec,
        &tone,
        2,
        (Param::On.id(), 0.0),
        at,
    );
    no_zipper(
        |t| limiter(|s| s.soft = t > 0.5),
        &spec,
        &tone,
        2,
        (Param::Soft.id(), 1.0),
        at,
    );
}

/// A main input of loud tones and a sidechain of noise in bursts, `frames` long.
fn keyed_run(frames: usize) -> Run {
    let mut key = noise(7, 1.5, frames);
    for (n, x) in key.iter_mut().enumerate() {
        if (n / 3_000) % 2 == 0 {
            *x *= 0.01;
        }
    }
    Run::new(vec![
        sine_at(300.0, 1.5, 0.0, frames),
        sine_at(3_000.0, 0.7, 1.0, frames),
    ])
    .with_sidechain(vec![key])
}

#[test]
fn the_sidechain_keys_it_and_listen_lets_the_sidechain_out() {
    let frames = 24_000;
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    let main = sine_at(1_000.0, 0.5, 0.0, frames);
    let mut key = vec![0.0_f32; frames];
    key[12_000..18_000].copy_from_slice(&noise(8, 2.0, 6_000));
    let run = Run::new(vec![main.clone(), main.clone()]).with_sidechain(vec![key.clone()]);
    let keyed = |edit: fn(&mut Settings)| {
        process(
            limiter(|s| {
                s.ceiling_db = -12.0;
                s.sidechain_on = true;
                edit(s);
            }),
            &spec,
            &run,
        )
    };
    // The main at −6 dBFS is over the ceiling of −12 but does not key it: through to the bit
    // until the key comes, then down by more than 6 dB.
    let output = keyed(|_| {});
    for channel in &output {
        assert_eq!(channel[L..12_000 - 100], main[..12_000 - 100 - L]);
        let during = amplitude_at(&channel[13_000 + L..17_000 + L], 1_000.0);
        assert!(db(during / 0.5) < -6.0, "{}", db(during / 0.5));
    }
    // Without its sidechain on, it hears the main and limits it.
    let own = process(limiter(|s| s.ceiling_db = -12.0), &spec, &run);
    assert!(peak(&own[0]) <= 10_f32.powf(-12.0 / 20.0) * (1.0 + 1e-6));
    // Listen: the sidechain, 64 samples later, in both channels.
    let listened = keyed(|s| s.sidechain_listen = true);
    for channel in &listened {
        assert!(channel[..L].iter().all(|x| *x == 0.0));
        assert_eq!(channel[L..], key[..frames - L]);
    }
    // Listen is a button of its own: with the sidechain off, it lets the sidechain out too.
    let off = process(
        limiter(|s| {
            s.ceiling_db = -12.0;
            s.sidechain_listen = true;
        }),
        &spec,
        &run,
    );
    assert_eq!(off, listened);
    assert_ne!(off, own);
    // A lane of Listen is not heard: it is for the monitor.
    let lane = run
        .clone()
        .with_automation(Param::SidechainListen.id(), vec![1.0; frames]);
    let laned = process(
        limiter(|s| {
            s.ceiling_db = -12.0;
            s.sidechain_on = true;
        }),
        &spec,
        &lane,
    );
    assert_eq!(laned, output);
    assert!(!Param::SidechainListen.is_automatable());
    // A block without its sidechain is heard as one with none.
    let mut node = limiter(|s| {
        s.ceiling_db = -12.0;
        s.sidechain_on = true;
    });
    node.prepare(&spec).unwrap();
    let bare = Run::new(vec![main.clone(), main.clone()]);
    let mut blocks = columbina_audio::Planar::new(2, 1_024);
    let mut out = columbina_audio::Planar::new(2, 1_024);
    let mut heard = vec![Vec::new(), Vec::new()];
    for start in (0..frames).step_by(1_000) {
        for channel in 0..2 {
            blocks.channel_mut(channel)[..1_000]
                .copy_from_slice(&bare.input[channel][start..start + 1_000]);
        }
        let block = columbina_audio::Block::new(start as u64, RATE, 1_000);
        blocks.read(1_000, |input| {
            out.write(1_000, |output| node.process(&block, input, output));
        });
        for (channel, heard) in heard.iter_mut().enumerate() {
            heard.extend_from_slice(&out.channel(channel)[..1_000]);
        }
    }
    let none = process(
        limiter(|s| s.ceiling_db = -12.0),
        &Spec::new(RATE, 1_024, Layout::Stereo),
        &bare,
    );
    assert_eq!(heard, none);
}

#[test]
fn the_sidechain_is_heard_as_it_comes_not_raised_by_input_level() {
    // A key at −6 dBFS, under the ceiling of −1; Input Level +12 dB on a quiet main. Raised by
    // Input Level the key would be +6 dBFS and duck the main; as it comes, it does not.
    let frames = 9_600;
    let spec = Spec::new(RATE, 1_024, Layout::Mono).with_sidechain(1);
    let main = sine_at(440.0, 0.05, 0.0, frames);
    let key = sine_at(1_000.0, 0.5, 0.0, frames);
    let run = Run::new(vec![main.clone()]).with_sidechain(vec![key]);
    let keyed = process(
        limiter(|s| {
            s.input_db = 12.0;
            s.sidechain_on = true;
        }),
        &spec,
        &run,
    )
    .remove(0);
    let gain = 10_f64.powf(12.0 / 20.0);
    for n in L..frames {
        let expected = f64::from(main[n - L]) * gain;
        assert!((f64::from(keyed[n]) - expected).abs() < 1e-7, "{n}");
    }
}

#[test]
fn the_limiter_is_the_same_in_any_blocks() {
    let spec = Spec::new(RATE, 2_048, Layout::Stereo).with_sidechain(1);
    let frames = 30_000;
    let run = keyed_run(frames)
        .with_change(200, Param::CeilingDb.id(), -6.0)
        .with_change(3_000, Param::Soft.id(), 1.0)
        .with_change(5_000, Param::InputDb.id(), 9.0)
        .with_change(6_000, Param::ReleaseMs.id(), 3.0)
        .with_change(9_000, Param::SidechainOn.id(), 1.0)
        .with_change(12_000, Param::SidechainListen.id(), 1.0)
        .with_change(12_300, Param::SidechainListen.id(), 0.0)
        .with_change(15_000, Param::On.id(), 0.0)
        .with_change(15_100, Param::On.id(), 1.0)
        .with_change(20_000, Param::ReleaseMs.id(), 800.0);
    chunk_invariance(|| limiter(|_| {}), &spec, &run, &[1, 2, 3]);
    let ceiling: Vec<f32> = (0..frames)
        .map(|n| -12.0 + 10.0 * (n as f32 / 1_500.0).sin())
        .collect();
    let toggles: Vec<f32> = (0..frames).map(|n| ((n / 2_200) % 2) as f32).collect();
    let releases: Vec<f32> = (0..frames).map(|n| 1.0 + (n % 500) as f32).collect();
    let automated = run
        .with_automation(Param::CeilingDb.id(), ceiling)
        .with_automation(Param::SidechainOn.id(), toggles.clone())
        .with_automation(Param::Soft.id(), toggles)
        .with_automation(Param::ReleaseMs.id(), releases);
    chunk_invariance(|| limiter(|_| {}), &spec, &automated, &[4]);
}

#[test]
fn the_latency_is_64_whatever_is_set_and_a_late_start_settles_as_reported() {
    let spec = Spec::new(RATE, 1_024, Layout::Stereo);
    assert_eq!(assert_latency(|| limiter(|_| {}), &spec), 64);
    assert_eq!(assert_latency(|| limiter(|s| s.on = false), &spec), 64);
    assert_eq!(
        assert_latency(|| limiter(|s| s.input_db = 18.0), &spec.with_sidechain(2)),
        64
    );
    assert!(
        settle_for(
            &Settings {
                release_ms: 1_000.0,
                ..Settings::default()
            },
            RATE
        ) > settle_for(&Settings::default(), RATE)
    );
    // 14 time constants of 100 ms, and the rest.
    let settle = settle_for(&Settings::default(), RATE) as usize;
    assert!((67_200..69_000).contains(&settle), "{settle}");

    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    let release = |ms: f32| move || limiter(|s| s.release_ms = ms);
    let mut probe = release(20.0)();
    probe.prepare(&spec).unwrap();
    let settle = probe.settle() as usize;
    let frames = 2 * settle + 20_000;
    let run = keyed_run(frames)
        .with_change(settle - 300, Param::CeilingDb.id(), -9.0)
        .with_change(settle + 1_000, Param::SidechainOn.id(), 1.0);
    for from in [settle, settle + 1_234] {
        assert_settle(release(20.0), &spec, &run, from);
    }
}

/// A limiter that reports the settling of other settings: the graph's, from the worst values of
/// the lanes in its preroll (T4b).
struct Worst(Limiter, Settings);

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
    fn set(&mut self, param: columbina_audio::ParamId, value: f32) {
        self.0.set(param, value);
    }
}

#[test]
fn a_lane_slower_than_the_setting_and_a_lane_that_turns_it_on_settle_by_settle_for() {
    // Set to 5 ms and off; its lanes hold a release of 300 ms and turn it on. What it reports for
    // itself is too short; settle_for of the worst values is enough.
    let set = Settings {
        on: false,
        release_ms: 5.0,
        ..Settings::default()
    };
    let worst = Settings {
        on: true,
        release_ms: 300.0,
        ..Settings::default()
    };
    let spec = Spec::new(RATE, 1_024, Layout::Stereo).with_sidechain(1);
    let mut probe = Limiter::new(&set);
    probe.prepare(&spec).unwrap();
    assert!(probe.settle() < settle_for(&worst, RATE) / 10);
    let from = settle_for(&worst, RATE) as usize;
    let frames = 2 * from + 10_000;
    let run = keyed_run(frames)
        .with_automation(Param::ReleaseMs.id(), vec![300.0; frames])
        .with_automation(Param::On.id(), vec![1.0; frames]);
    assert_settle(|| Worst(Limiter::new(&set), worst), &spec, &run, from);
}

#[test]
fn input_that_is_not_a_number_is_silence_and_lanes_that_are_not_keep_the_values_set() {
    let spec = Spec::new(RATE, 1_024, Layout::Mono);
    let frames = 20_000;
    let clean = noise(9, 3.0, frames);
    let mut odd = clean.clone();
    odd[500] = f32::INFINITY;
    odd[600] = f32::NAN;
    let settings = |s: &mut Settings| s.release_ms = 10.0;
    let a = process(limiter(settings), &spec, &Run::new(vec![clean.clone()])).remove(0);
    let b = process(limiter(settings), &spec, &Run::new(vec![odd])).remove(0);
    assert!(b.iter().all(|x| x.is_finite()));
    let mut zeroed = clean.clone();
    zeroed[500] = 0.0;
    zeroed[600] = 0.0;
    let c = process(limiter(settings), &spec, &Run::new(vec![zeroed])).remove(0);
    assert_eq!(b, c);
    let settle = settle_for(
        &Settings {
            release_ms: 10.0,
            ..Settings::default()
        },
        RATE,
    ) as usize;
    for n in 600 + settle..frames {
        assert!((a[n] - b[n]).abs() < 3.2e-5, "{n}");
    }
    let nans = vec![f32::NAN; frames];
    let mut run = Run::new(vec![clean]);
    for param in Param::ALL {
        run = run.with_automation(param.id(), nans.clone());
    }
    assert_eq!(process(limiter(settings), &spec, &run).remove(0), a);
}

#[test]
fn the_work_of_the_thread_of_the_mix_takes_no_memory_and_the_meters_read() {
    let spec = Spec::new(RATE, 512, Layout::Surround51).with_sidechain(2);
    let changes = [
        (Param::CeilingDb.id(), -12.0),
        (Param::InputDb.id(), 6.0),
        (Param::Soft.id(), 1.0),
        (Param::ReleaseMs.id(), 2.0),
        (Param::SidechainOn.id(), 1.0),
        (Param::SidechainListen.id(), 1.0),
        (Param::On.id(), 0.0),
    ];
    let lanes = vec![
        (Param::CeilingDb.id(), vec![-3.0; 480]),
        (Param::ReleaseMs.id(), vec![30.0; 480]),
    ];
    let mut node = limiter(|_| {});
    let counted = realtime_work(&mut node, &spec, &changes, &lanes, 300);
    assert!(counted.none(), "{counted:?}");

    let mut node = limiter(|_| {});
    let meter = node.meter();
    let spec = Spec::new(RATE, 512, Layout::Mono);
    node.prepare(&spec).unwrap();
    let run = Run::new(vec![sine_at(997.0, 2.0, 0.0, 48_000)]);
    render(&mut node, &spec, &run, |_| 480);
    let (input, reduction, output) = meter.read();
    assert!((input - 6.02).abs() < 0.05, "{input}");
    assert!((reduction + 7.2).abs() < 0.1, "{reduction}");
    assert!((output + 1.2).abs() < 0.1, "{output}");
}

#[test]
fn its_parameters_are_those_of_the_model() {
    let paths: Vec<_> = Param::ALL.iter().map(|param| param.path()).collect();
    assert_eq!(
        paths,
        [
            "on",
            "input_db",
            "soft",
            "ceiling_db",
            "release_ms",
            "sidechain.on",
            "sidechain.listen"
        ]
    );
    assert_eq!(Param::InputDb.info().range, (-18.0, 18.0));
    assert_eq!(Param::CeilingDb.info().range, (-24.0, 0.0));
    assert_eq!(Param::CeilingDb.info().default, -1.0);
    assert_eq!(Param::ReleaseMs.info().range, (0.01, 1_000.0));
    assert_eq!(Param::ReleaseMs.info().default, 100.0);
    let settings = Settings {
        on: true,
        input_db: 4.0,
        soft: true,
        ceiling_db: -6.0,
        release_ms: 33.0,
        sidechain_on: false,
        sidechain_listen: false,
    };
    let spec = Spec::new(RATE, 4_800, Layout::Mono);
    let run = Run::new(vec![noise(12, 2.0, 4_800)]);
    let mut set = Limiter::default();
    for (param, value) in settings.params() {
        set.set(param, value);
    }
    assert_eq!(
        process(set, &spec, &run),
        process(Limiter::new(&settings), &spec, &run)
    );
    let mut node = Limiter::default();
    assert!(node.prepare(&Spec::new(RATE, 64, Layout::Empty)).is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
}
