//! The spectral gate of Noise Reduction (T12): its transform and windows checked on their own,
//! how much noise it takes away and how much of a voice it keeps, the learned noise, the mix,
//! the delay, the settling, blocks, bad input, the thread of the mix.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    reason = "the numbers of tests"
)]

mod denoise_signals;

use columbina_audio::constants::{DENOISE_FLOOR_DB, SMOOTHING};
use columbina_audio::fft::Complex32;
use columbina_audio::fx::denoise::{
    analysis_window, learn, synthesis_window, window_power, Band, Frame, Mode, NoiseReduction,
    Param, Preset, Settings, Spectral, Stft, BINS, FFT, HOP, LATENCY, PARAMS,
};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, realtime, render, rt_violations, sine,
    CountingAllocator, Rng, Run,
};
use columbina_audio::{Automated, Automation, Block, Layout, Node, ParamId, Planar, Spec};
use denoise_signals::{kept, left, pink, silences, speeches, sum, voice, white};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const RATE: u32 = 48_000;

fn mono() -> Spec {
    Spec::new(RATE, 1024, Layout::Mono)
}

fn stereo() -> Spec {
    Spec::new(RATE, 1024, Layout::Stereo)
}

/// `settings` with `edit` made.
fn with(edit: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings::default();
    edit(&mut settings);
    settings
}

/// The output of a new gate of `settings` through `input`, whole.
fn run(settings: &Settings, spec: &Spec, input: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let mut node = Spectral::new(settings);
    let spec = spec.with_max_block(input[0].len());
    node.prepare(&spec).unwrap();
    render(&mut node, &spec, &Run::new(input), |left| left)
}

fn run_mono(settings: &Settings, input: &[f32]) -> Vec<f32> {
    run(settings, &mono(), vec![input.to_vec()]).remove(0)
}

// --- The transform ---------------------------------------------------------------------------

/// The windows of the module, worked out here again from their definitions: a periodic Hann of
/// 2048 points, and a Hann of 1536 points on the end of the frame divided by it, scaled by 2/3.
/// The windows of the module, worked out here again from their definitions: the analysis
/// window a quarter sine up to 1280 and half a sine of 1536 down to the end, the synthesis
/// window such that the product is a Hann window of 1536 on the end of the frame, scaled by 2/3.
#[test]
fn the_windows_overlap_to_one_and_the_synthesis_falls_to_zero_at_the_end() {
    use std::f64::consts::PI;
    let (analysis, synthesis) = (analysis_window(), synthesis_window());
    assert_eq!((FFT, HOP, LATENCY, BINS), (2048, 512, 1536, 1025));
    let wa = |n: usize| -> f64 {
        if n < 1280 {
            (PI * n as f64 / 2560.0).sin()
        } else {
            (PI * (n - 512) as f64 / 1536.0).sin()
        }
    };
    for n in 0..FFT {
        assert!(
            (f64::from(analysis[n]) - wa(n)).abs() < 1e-7,
            "analysis at {n}"
        );
        let product = if n < 512 {
            0.0
        } else {
            2.0 / 3.0 * (PI * (n - 512) as f64 / 1536.0).sin().powi(2)
        };
        assert!(
            (f64::from(analysis[n]) * f64::from(synthesis[n]) - product).abs() < 1e-6,
            "the product at {n}"
        );
        // On the last 768 samples the synthesis window is 2/3·sin, down to 0 at the end.
        if n >= 1280 {
            let expected = 2.0 / 3.0 * (PI * (n - 512) as f64 / 1536.0).sin();
            assert!(
                (f64::from(synthesis[n]) - expected).abs() < 1e-6,
                "synthesis at {n}"
            );
        }
    }
    assert!(synthesis[FFT - 1] < 2e-3 && synthesis.iter().all(|&w| w <= 0.6667));
    // Nothing of the first hop comes out: the frame that ends 1536 samples after a sample is
    // the last that adds to it.
    assert!(synthesis[..HOP].iter().all(|&w| w == 0.0));
    // Over the hops, the products add up to 1 at every sample.
    for n in 0..HOP {
        let total: f64 = (0..4)
            .map(|hop| f64::from(analysis[n + hop * HOP]) * f64::from(synthesis[n + hop * HOP]))
            .sum();
        assert!((total - 1.0).abs() < 1e-6, "{total} at {n}");
    }
    let power: f64 = (0..FFT).map(|n| wa(n) * wa(n)).sum();
    assert!((power - 1024.0).abs() < 1e-9 && (window_power() - 1024.0).abs() < 1e-9);
}

/// A gain that jumps between frames, at random in each bin (the "musical" changes of a gate),
/// leaves little above a low tone: the synthesis window that falls to 0 at the end of the frame
/// does not cut it off (a window that ends high left −37 dB above 3 kHz).
#[test]
fn gains_that_jump_between_frames_do_not_spread_a_tone() {
    let frames = 4 * RATE as usize;
    let tone = sine(RATE, 200.0, 0.5, frames);
    let mut stft = Stft::new(1);
    let mut wet = vec![0.0; frames];
    let mut dry = vec![0.0; frames];
    let mut rng = Rng::new(9);
    stft.process(
        frames,
        &[&tone],
        &mut [&mut wet],
        &mut [&mut dry],
        |_, spectrum| {
            for bin in spectrum.iter_mut() {
                *bin *= 0.0316 + 0.9684 * rng.unit() as f32;
            }
        },
    );
    let n = 1 << 17;
    let segment = &wet[2 * FFT..2 * FFT + n];
    let mut fft = columbina_audio::fft::RealFft::new(n);
    let mut signal: Vec<f32> = segment
        .iter()
        .enumerate()
        .map(|(i, x)| x * (std::f64::consts::PI * i as f64 / n as f64).sin().powi(2) as f32)
        .collect();
    let mut spectrum = fft.spectrum();
    fft.forward(&mut signal, &mut spectrum);
    let hz = f64::from(RATE) / n as f64;
    let (mut above, mut all) = (0.0_f64, 0.0_f64);
    for (k, bin) in spectrum.iter().enumerate() {
        let power = f64::from(bin.norm_sqr());
        all += power;
        if k as f64 * hz > 3_000.0 {
            above += power;
        }
    }
    let spread = denoise_signals::db(above / all);
    println!("above 3 kHz: {spread:.1} dB");
    assert!(spread < -80.0, "{spread} dB");
}

/// The transform with every frame left as it is.
fn identity(
    input: &[Vec<f32>],
    blocks: &mut dyn FnMut() -> usize,
    origin: u64,
) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let channels = input.len();
    let frames = input[0].len();
    let mut stft = Stft::new(channels);
    stft.align(origin);
    let mut wet = vec![vec![0.0; frames]; channels];
    let mut dry = vec![vec![0.0; frames]; channels];
    let mut at = 0;
    while at < frames {
        let size = blocks().min(frames - at);
        let x: Vec<&[f32]> = input.iter().map(|c| &c[at..at + size]).collect();
        let (mut w, mut d): (Vec<&mut [f32]>, Vec<&mut [f32]>) = (
            wet.iter_mut().map(|c| &mut c[at..at + size]).collect(),
            dry.iter_mut().map(|c| &mut c[at..at + size]).collect(),
        );
        stft.process(size, &x, &mut w, &mut d, |_: Frame, _: &mut [Complex32]| {});
        at += size;
    }
    (wet, dry)
}

#[test]
fn a_transform_that_changes_nothing_gives_the_input_back_1536_samples_late() {
    let input = vec![noise(1, 0.5, 20_000), sine(RATE, 997.0, 0.9, 20_000)];
    let mut rng = Rng::new(3);
    let (wet, dry) = identity(&input, &mut || rng.range(1, 700), 0);
    for (channel, (wet, dry)) in wet.iter().zip(&dry).enumerate() {
        for n in 0..20_000 {
            let expected = if n >= LATENCY {
                input[channel][n - LATENCY]
            } else {
                0.0
            };
            assert_eq!(dry[n].to_bits(), expected.to_bits(), "dry {channel} at {n}");
            assert!(
                (wet[n] - expected).abs() <= 1e-6,
                "wet {channel} at {n}: {} {expected}",
                wet[n]
            );
        }
    }
    // An impulse comes out at 1536 samples, on any grid.
    for origin in [0, 1, 300, 511] {
        let (wet, _) = identity(
            &[columbina_audio::testing::impulse(8_000, 3_000, 1.0)],
            &mut || 64,
            origin,
        );
        let peak = wet[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        assert_eq!(peak.0, 3_000 + LATENCY, "origin {origin}");
        assert!((peak.1 - 1.0).abs() < 1e-6);
    }
}

#[test]
fn the_frames_of_the_transform_end_on_the_grid_of_the_timeline() {
    for origin in [0_u64, 5, 511, 512, 1_000_000_007] {
        let mut stft = Stft::new(1);
        stft.align(origin);
        let input = vec![0.0; 3_000];
        let mut wet = vec![0.0; 3_000];
        let mut dry = vec![0.0; 3_000];
        let mut ends = Vec::new();
        stft.process(
            3_000,
            &[&input],
            &mut [&mut wet],
            &mut [&mut dry],
            |frame, _| {
                ends.push(frame.at);
            },
        );
        assert_ne!(ends, []);
        for at in ends {
            assert_eq!(
                (origin + at as u64 + 1) % HOP as u64,
                0,
                "origin {origin}, at {at}"
            );
        }
    }
}

// --- What it takes away and what it keeps ----------------------------------------------------

/// The plan's sound: white noise at −40 dBFS and a voice at −20 dBFS that comes and goes every
/// half second, 10 s at 48 kHz.
fn voice_over(noise: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let clean = voice(RATE, 0.1, noise.len());
    (sum(&clean, noise), clean)
}

#[test]
fn the_defaults_take_the_noise_down_between_the_words_and_keep_the_voice() {
    let frames = 10 * RATE as usize;
    for (name, noise) in [
        ("white", white(11, 0.01, frames)),
        ("pink", pink(12, 0.01, frames)),
    ] {
        let (input, clean) = voice_over(&noise);
        let output = run_mono(&Settings::default(), &input);
        // From the second second on: the profiles have found the noise.
        let floor = left(
            &input,
            &output,
            LATENCY,
            &silences(RATE, frames - LATENCY, 2.0, 0.1),
        );
        let voice = kept(
            &clean,
            &output,
            LATENCY,
            &speeches(RATE, frames - LATENCY, 2.0, 0.05),
        );
        println!("{name}: the noise between the words {floor:.2} dB, the voice {voice:.3} dB");
        assert!(floor <= -10.0, "{name}: {floor} dB");
        assert!(voice.abs() <= 1.0, "{name}: {voice} dB");
    }
}

/// Noise alone, at −40 dBFS, is taken down nearly to the floor of the gate (−30 dB): white
/// 28.7 dB, pink 26.6 dB (its strong low bins open the gate a little more often).
#[test]
fn noise_alone_is_taken_down_nearly_to_the_floor_of_the_gate() {
    let frames = 8 * RATE as usize;
    for (name, noise, expected) in [
        ("white", white(21, 0.01, frames), -28.7),
        ("pink", pink(22, 0.01, frames), -26.6),
    ] {
        let output = run_mono(&Settings::default(), &noise);
        let range = 2 * RATE as usize..frames - LATENCY;
        let depth = left(&noise, &output, LATENCY, std::slice::from_ref(&range));
        println!("{name}: {depth:.2} dB");
        assert!((depth - expected).abs() <= 1.0, "{name}: {depth} dB");
        assert!(
            depth >= f64::from(DENOISE_FLOOR_DB),
            "{name}: below the floor"
        );
    }
}

#[test]
fn a_noise_learned_takes_the_same_noise_down_15_db_at_least() {
    let frames = 6 * RATE as usize;
    let noise = white(31, 0.01, frames);
    let profile = learn(&[&noise[..RATE as usize]]).unwrap();
    let settings = with(|s| {
        s.mode = Mode::Manual;
        s.profile = Some(profile);
    });
    let output = run_mono(&settings, &noise);
    let range = RATE as usize..frames - LATENCY;
    let depth = left(&noise, &output, LATENCY, std::slice::from_ref(&range));
    println!("learned: {depth:.2} dB");
    assert!(depth <= -15.0, "{depth} dB");
    // With nothing learned, Manual follows the noise as Auto Speech does.
    let manual = run_mono(&with(|s| s.mode = Mode::Manual), &noise);
    let auto = run_mono(&Settings::default(), &noise);
    assert_eq!(manual, auto);
}

/// The power of white noise of RMS σ in a bin of the window is σ²·Σw² = σ²·768: the learned
/// noise against it, worked out without the transform.
#[test]
fn the_noise_learned_is_the_mean_power_of_each_bin() {
    let sigma = 0.05_f32;
    let noise = white(41, sigma, 40 * FFT);
    let profile = learn(&[&noise]).unwrap();
    assert_eq!(profile.len(), BINS);
    let expected = f64::from(sigma).powi(2) * 1024.0;
    let mean = profile[10..BINS - 10]
        .iter()
        .map(|&p| f64::from(p))
        .sum::<f64>()
        / (BINS - 20) as f64;
    assert!(
        (mean / expected - 1.0).abs() < 0.03,
        "{mean} against {expected}"
    );
    // A sine at a bin: its power is (A·Σw/2)², Σw worked out from the window's definition.
    let tone = sine(RATE, 100.0 * 48_000.0 / 2048.0, 0.5, 8 * FFT);
    let profile = learn(&[&tone]).unwrap();
    let sum: f64 = (0..FFT)
        .map(|n| {
            use std::f64::consts::PI;
            if n < 1280 {
                (PI * n as f64 / 2560.0).sin()
            } else {
                (PI * (n - 512) as f64 / 1536.0).sin()
            }
        })
        .sum();
    let expected = (0.5 * sum / 2.0).powi(2);
    assert!(
        (f64::from(profile[100]) / expected - 1.0).abs() < 1e-3,
        "{}",
        profile[100]
    );
    assert_eq!(learn(&[&tone[..FFT - 1]]), None);
    // Two channels count as one stretch twice as long.
    let two = learn(&[&noise[..8 * FFT], &noise[8 * FFT..16 * FFT]]).unwrap();
    let one = learn(&[&noise[..8 * FFT]]).unwrap();
    let other = learn(&[&noise[8 * FFT..16 * FFT]]).unwrap();
    for k in 0..BINS {
        let mean = f32::midpoint(one[k], other[k]);
        assert!((two[k] - mean).abs() <= mean * 1e-5, "bin {k}");
    }
}

#[test]
fn de_hiss_and_de_rumble_gate_only_their_band() {
    let frames = 6 * RATE as usize;
    let noise = white(51, 0.01, frames);
    let band = |output: &[f32], low: f64, high: f64| -> f64 {
        // The power of a band by a long transform of the last 2^17 samples.
        let n = 1 << 17;
        let mut fft = columbina_audio::fft::RealFft::new(n);
        let mut signal = output[output.len() - n..].to_vec();
        let mut spectrum = fft.spectrum();
        fft.forward(&mut signal, &mut spectrum);
        let hz = f64::from(RATE) / n as f64;
        spectrum
            .iter()
            .enumerate()
            .filter(|(k, _)| (low..high).contains(&(*k as f64 * hz)))
            .map(|(_, bin)| f64::from(bin.norm_sqr()))
            .sum()
    };
    let input_low = band(&noise, 40.0, 200.0);
    let input_mid = band(&noise, 400.0, 3_500.0);
    let input_high = band(&noise, 5_000.0, 20_000.0);
    for preset in Preset::ALL {
        let output = run_mono(&Settings::preset(preset), &noise);
        let low = denoise_signals::db(band(&output, 40.0, 200.0) / input_low);
        let mid = denoise_signals::db(band(&output, 400.0, 3_500.0) / input_mid);
        let high = denoise_signals::db(band(&output, 5_000.0, 20_000.0) / input_high);
        println!("{}: {low:.2} {mid:.2} {high:.2}", preset.name());
        let gates = |b: Band, hz: f32| b.gates(hz);
        for (level, hz) in [(low, 100.0), (mid, 1_000.0), (high, 10_000.0)] {
            if gates(preset.band(), hz) {
                assert!(level <= -20.0, "{}: {level} at {hz}", preset.name());
            } else {
                assert!(level.abs() <= 0.5, "{}: {level} at {hz}", preset.name());
            }
        }
    }
}

// --- The mix ---------------------------------------------------------------------------------

#[test]
fn all_dry_and_bypassed_give_the_input_late_to_the_bit() {
    let frames = 3 * RATE as usize;
    let input = vec![
        sum(&white(61, 0.05, frames), &voice(RATE, 0.2, frames)),
        noise(62, 0.3, frames),
    ];
    let late = |channel: &[f32]| -> Vec<f32> {
        (0..frames)
            .map(|n| {
                if n >= LATENCY {
                    channel[n - LATENCY]
                } else {
                    0.0
                }
            })
            .collect()
    };
    for settings in [
        with(|s| s.dry_wet = 0.0),
        with(|s| s.on = false),
        with(|s| {
            s.on = false;
            s.listen = true;
            s.level_db = 12.0;
        }),
    ] {
        let output = run(&settings, &stereo(), input.clone());
        for (channel, output) in output.iter().enumerate() {
            assert_eq!(
                output,
                &late(&input[channel]),
                "{settings:?}, channel {channel}"
            );
        }
    }
}

#[test]
fn listen_lets_out_what_is_taken_away_and_level_raises_what_is_kept() {
    let frames = 3 * RATE as usize;
    let input = sum(&white(71, 0.01, frames), &voice(RATE, 0.1, frames));
    let wet = run_mono(&Settings::default(), &input);
    let noise_only = run_mono(&with(|s| s.listen = true), &input);
    let louder = run_mono(&with(|s| s.level_db = 6.0), &input);
    let half = run_mono(&with(|s| s.dry_wet = 50.0), &input);
    let gain = 10.0_f32.powf(6.0 / 20.0);
    for n in LATENCY..frames {
        let dry = input[n - LATENCY];
        assert!(
            (noise_only[n] + wet[n] - dry).abs() <= 1e-7,
            "listen at {n}"
        );
        assert!(
            (louder[n] - gain * wet[n]).abs() <= 1e-6 * gain,
            "level at {n}"
        );
        assert!(
            (half[n] - f32::midpoint(dry, wet[n])).abs() <= 1e-7,
            "mix at {n}"
        );
    }
}

/// A change of the mix ramps over 10 ms: on a steady level, each sample moves the gain at most
/// 1/480 of the way.
#[test]
fn the_mix_ramps_over_ten_milliseconds() {
    let ramp = (SMOOTHING * f64::from(RATE)) as usize;
    let frames = 2 * RATE as usize;
    let at = 70_000;
    // A constant input: the processed sound is the input under the gain of the bins at 0 Hz.
    let input = vec![0.25_f32; frames];
    for (param, from, to) in [
        (Param::DryWet, 100.0, 0.0),
        (Param::LevelDb, 0.0, 12.0),
        (Param::On, 1.0, 0.0),
        (Param::Listen, 0.0, 1.0),
    ] {
        let settings = with(|s| s.set(param, from));
        let mut node = Spectral::new(&settings);
        let spec = mono().with_max_block(frames);
        node.prepare(&spec).unwrap();
        let run = Run::new(vec![input.clone()]).with_change(at, param.id(), to);
        let output = render(&mut node, &spec, &run, |left| left).remove(0);
        let still = run_mono(&settings, &input);
        let moved = run_mono(&with(|s| s.set(param, to)), &input);
        let span = (moved[at + 2 * ramp] - still[at + 2 * ramp]).abs();
        assert!(span > 0.01, "{param:?} moves the output: {span}");
        for n in at..at + ramp + 2 {
            let step = (output[n] - output[n - 1]).abs();
            assert!(
                step <= span / ramp as f32 * 1.01 + 1e-6,
                "{param:?} at {n}: {step} of {span}"
            );
        }
        assert!(
            (output[at + ramp + 10] - moved[at + ramp + 10]).abs() <= 1e-6,
            "{param:?} ends"
        );
    }
}

/// No zipper when the mix or a number of the gate changes at the peak of a tone over noise.
#[test]
fn changes_do_not_click() {
    let frames = 3 * RATE as usize;
    // A tone at −46 dBFS over noise at −60 dBFS, the noise learned: Threshold 40 dB shuts on the
    // tone, 10 dB lets it through, so that the changes of the gate change the sound.
    let tone = sine(RATE, 1000.0, 0.005, frames);
    let input = sum(&tone, &white(81, 0.001, frames));
    let manual = with(|s| {
        s.mode = Mode::Manual;
        s.profile = learn(&[&white(82, 0.001, RATE as usize)]);
    });
    let with = |edit: &dyn Fn(&mut Settings)| {
        let mut settings = manual.clone();
        edit(&mut settings);
        settings
    };
    let at = 96_012; // a peak of the tone
    let largest_step = |x: &[f32]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max)
    };
    for (param, from, to) in [
        (Param::DryWet, 100.0, 0.0),
        (Param::LevelDb, 0.0, 18.0),
        (Param::On, 1.0, 0.0),
        (Param::Listen, 0.0, 1.0),
        (Param::ThresholdDb, 10.0, 40.0),
        (Param::Sensitivity, 50.0, 100.0),
        (Param::FrequencySmoothing, 25.0, 100.0),
        (Param::TimeSmoothing, 1.0, 0.0),
        (Param::Band, 0.0, 2.0),
    ] {
        let settings = with(&|s| s.set(param, from));
        let mut node = Spectral::new(&settings);
        let spec = mono().with_max_block(frames);
        node.prepare(&spec).unwrap();
        let run = Run::new(vec![input.clone()]).with_change(at, param.id(), to);
        let output = render(&mut node, &spec, &run, |left| left).remove(0);
        let before = run_mono(&settings, &input);
        let after = run_mono(&with(&|s| s.set(param, to)), &input);
        let steady =
            largest_step(&before[at - 4_000..at]).max(largest_step(&after[at + 4_000..at + 8_000]));
        let changed = largest_step(&output[at - 10..at + 4_000]);
        assert!(
            changed <= 1.1 * steady,
            "{param:?}: {changed} against {steady}"
        );
    }
}

// --- The node --------------------------------------------------------------------------------

#[test]
fn the_parameters_are_those_of_the_model() {
    let defaults = Settings::default();
    assert_eq!(
        (
            defaults.on,
            defaults.mode,
            defaults.time_smoothing,
            defaults.listen,
            defaults.band
        ),
        (true, Mode::AutoSpeech, true, false, Band::Full)
    );
    assert_eq!(
        [
            defaults.threshold_db,
            defaults.attack_ms,
            defaults.sensitivity,
            defaults.ratio,
            defaults.frequency_smoothing,
            defaults.dry_wet,
            defaults.level_db,
        ],
        [10.0, 1000.0, 50.0, 4.0, 25.0, 100.0, 0.0]
    );
    assert_eq!(defaults.profile, None);
    assert_eq!(Param::ALL.len(), PARAMS);
    let mut paths: Vec<_> = Param::ALL.iter().map(|p| p.path()).collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), PARAMS);
    for (index, param) in Param::ALL.iter().enumerate() {
        assert_eq!(param.id(), ParamId(index as u32));
        assert_eq!(Param::from_id(param.id()), Some(*param));
    }
    assert_eq!(Param::from_id(ParamId(PARAMS as u32)), None);
    assert!(!Param::Listen.is_automatable() && !Param::Band.is_automatable());
    assert_eq!(
        Param::ALL.iter().filter(|p| p.is_automatable()).count(),
        PARAMS - 2
    );
    assert_eq!(Param::LevelDb.clamp(30.0), 18.0);
    assert_eq!(Param::LevelDb.clamp(-30.0), -6.0);
    assert_eq!(Param::AttackMs.clamp(1.0), 50.0);
    assert_eq!(Param::Band.clamp(1.6), 2.0);
    let mut settings = Settings::default();
    settings.set(Param::ThresholdDb, f32::NAN);
    settings.set(Param::LevelDb, f32::INFINITY);
    settings.set(Param::DryWet, f32::NEG_INFINITY);
    assert_eq!(settings, Settings::default());
    let mut node = Spectral::default_node();
    for param in Param::ALL {
        node.set(param.id(), f32::NAN);
        node.set(param.id(), f32::INFINITY);
    }
    node.set(ParamId(99), 1.0);
    assert_eq!(node.settings(), &Settings::default());
    for preset in Preset::ALL {
        let mut node = Spectral::new(&Settings::default());
        for (param, value) in Settings::preset(preset).params() {
            node.set(param, value);
        }
        assert_eq!(
            node.settings(),
            &Settings::preset(preset),
            "{}",
            preset.name()
        );
    }
    assert_eq!(Preset::DeRumbleAndHiss.name(), "De-Rumble and Hiss");
    assert_eq!(Band::from_code(3.0), Some(Band::RumbleAndHiss));
    assert_eq!(Band::from_code(1.5), None);
}

#[test]
fn it_takes_any_layout_in_as_out_and_no_sidechain() {
    for layout in [
        Layout::Mono,
        Layout::Stereo,
        Layout::Surround51,
        Layout::Adaptive(24),
    ] {
        assert!(Spectral::default_node()
            .prepare(&Spec::new(RATE, 64, layout))
            .is_ok());
    }
    let mut node = Spectral::default_node();
    assert!(node.prepare(&Spec::new(RATE, 64, Layout::Empty)).is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_sidechain(2))
        .is_err());
}

trait DefaultNode {
    fn default_node() -> Self;
}

impl DefaultNode for Spectral {
    fn default_node() -> Self {
        Spectral::new(&Settings::default())
    }
}

#[test]
fn its_delay_is_1536_samples_at_every_rate() {
    for rate in [44_100, 48_000, 96_000] {
        let spec = Spec::new(rate, 512, Layout::Stereo);
        assert_eq!(assert_latency(Spectral::default_node, &spec), 1536);
        assert_eq!(
            assert_latency(|| Spectral::new(&with(|s| s.dry_wet = 40.0)), &spec),
            1536
        );
    }
}

/// A run of a voice over noise with every kind of change and lane.
fn busy_run(frames: usize, channels: usize) -> Run {
    let input: Vec<Vec<f32>> = (0..channels)
        .map(|c| {
            sum(
                &voice(RATE, 0.1, frames),
                &white(90 + c as u64, 0.01, frames),
            )
        })
        .collect();
    let ramp = |from: f32, to: f32| -> Vec<f32> {
        (0..frames)
            .map(|n| from + (to - from) * n as f32 / frames as f32)
            .collect()
    };
    let square =
        |period: usize| -> Vec<f32> { (0..frames).map(|n| ((n / period) % 2) as f32).collect() };
    Run::new(input)
        .with_change(5_000, Param::ThresholdDb.id(), 20.0)
        .with_change(9_000, Param::AttackMs.id(), 100.0)
        .with_change(13_000, Param::Ratio.id(), 20.0)
        .with_change(17_000, Param::Sensitivity.id(), 100.0)
        .with_change(21_000, Param::FrequencySmoothing.id(), 0.0)
        .with_change(25_000, Param::TimeSmoothing.id(), 0.0)
        .with_change(29_000, Param::DryWet.id(), 60.0)
        .with_change(33_000, Param::LevelDb.id(), 6.0)
        .with_change(37_000, Param::Listen.id(), 1.0)
        .with_change(41_000, Param::Listen.id(), 0.0)
        .with_change(45_000, Param::On.id(), 0.0)
        .with_change(45_300, Param::On.id(), 1.0)
        .with_change(50_000, Param::Band.id(), 3.0)
        .with_automation(Param::ThresholdDb.id(), ramp(0.0, 40.0))
        .with_automation(Param::AttackMs.id(), ramp(50.0, 3000.0))
        .with_automation(Param::DryWet.id(), ramp(100.0, 30.0))
        .with_automation(Param::LevelDb.id(), ramp(-6.0, 6.0))
        .with_automation(Param::TimeSmoothing.id(), square(7_000))
        .with_automation(
            Param::On.id(),
            square(20_011).iter().map(|x| 1.0 - x).collect(),
        )
}

#[test]
fn its_output_does_not_depend_on_the_blocks() {
    chunk_invariance(
        Spectral::default_node,
        &stereo(),
        &busy_run(60_000, 2),
        &[1, 2, 3],
    );
    let profile = learn(&[&white(99, 0.01, 10_000)]).unwrap();
    let manual = with(|s| {
        s.mode = Mode::Manual;
        s.profile = Some(profile.clone());
    });
    let run = Run::new(vec![sum(
        &voice(44_100, 0.1, 40_000),
        &white(98, 0.01, 40_000),
    )])
    .with_change(10_000, Param::ThresholdDb.id(), 30.0);
    chunk_invariance(
        || Spectral::new(&manual),
        &Spec::new(44_100, 700, Layout::Mono),
        &run,
        &[4, 5],
    );
    chunk_invariance(
        || NoiseReduction::new(&Settings::default()),
        &Spec::new(RATE, 300, Layout::Surround51),
        &busy_run(20_000, 6),
        &[6],
    );
}

#[test]
fn it_settles_as_it_reports() {
    let frames = 30 * RATE as usize;
    let input = sum(&voice(RATE, 0.1, frames), &pink(101, 0.01, frames));
    let run = Run::new(vec![input.clone()]);
    let settle = Spectral::default_node().settle_for_rate(RATE);
    println!(
        "the defaults settle in {:.2} s",
        settle as f64 / f64::from(RATE)
    );
    assert!(settle < 25 * RATE as usize);
    // In the voice, in the silence, off the grid.
    for from in [12_345, RATE as usize / 2 + 7, 3 * RATE as usize / 4 + 300] {
        let largest = assert_settle(Spectral::default_node, &mono(), &run, from);
        println!("from {from}: {largest:e}");
    }
    // Fast profiles.
    let fast = with(|s| {
        s.attack_ms = 50.0;
        s.ratio = 20.0;
    });
    let short = Run::new(vec![input[..5 * RATE as usize].to_vec()]);
    assert_settle(|| Spectral::new(&fast), &mono(), &short, 33_333);
    // A noise learned.
    let manual = with(|s| {
        s.mode = Mode::Manual;
        s.profile = learn(&[&pink(102, 0.01, RATE as usize)]);
    });
    assert_settle(|| Spectral::new(&manual), &mono(), &short, 23_456);
}

trait SettleForRate {
    fn settle_for_rate(&self, rate: u32) -> usize;
}

impl SettleForRate for Spectral {
    fn settle_for_rate(&self, rate: u32) -> usize {
        Spectral::settle_for(self.settings(), rate) as usize
    }
}

/// A node set one way and its settling reported for another (the slowest that the lanes take).
struct Reported {
    node: Spectral,
    settle: u32,
}

impl Node for Reported {
    fn prepare(&mut self, spec: &Spec) -> Result<(), columbina_audio::Unsupported> {
        self.node.prepare(spec)
    }
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.node.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.node.latency()
    }
    fn settle(&self) -> u32 {
        self.settle
    }
    fn reset(&mut self) {
        self.node.reset();
    }
    fn align(&mut self, origin: u64) {
        self.node.align(origin);
    }
    fn set(&mut self, param: ParamId, value: f32) {
        self.node.set(param, value);
    }
}

#[test]
fn it_settles_for_the_slowest_settings_that_its_lanes_take() {
    // Set fast, the lane slow: Attack 1000 ms, Ratio 1 (31.5 s to settle).
    let fast = with(|s| {
        s.attack_ms = 50.0;
        s.ratio = 20.0;
        s.time_smoothing = false;
    });
    let slowest = with(|s| {
        s.attack_ms = 1000.0;
        s.ratio = 1.0;
    });
    let settle = Spectral::settle_for(&slowest, RATE);
    assert!(settle > Spectral::settle_for(&fast, RATE));
    // Time Smoothing and On count as on, whatever they are set to.
    assert_eq!(
        Spectral::settle_for(&with(|s| s.time_smoothing = false), RATE),
        Spectral::settle_for(&Settings::default(), RATE)
    );
    assert_eq!(
        Spectral::settle_for(&with(|s| s.on = false), RATE),
        Spectral::settle_for(&Settings::default(), RATE)
    );
    let from = RATE as usize + 999;
    let frames = from + settle as usize + 5 * RATE as usize;
    let input = sum(&voice(RATE, 0.1, frames), &white(111, 0.01, frames));
    let run = Run::new(vec![input.clone()])
        .with_automation(Param::AttackMs.id(), vec![1000.0; frames])
        .with_automation(Param::Ratio.id(), vec![1.0; frames])
        .with_automation(Param::TimeSmoothing.id(), vec![1.0; frames]);
    assert_settle(
        || Reported {
            node: Spectral::new(&fast),
            settle,
        },
        &mono(),
        &run,
        from,
    );
    // Set off, the lane on: Time Smoothing and On, at the settling of the defaults.
    let off = with(|s| {
        s.time_smoothing = false;
        s.on = false;
    });
    let settle = Spectral::settle_for(&Settings::default(), RATE);
    let frames = from + settle as usize + 5 * RATE as usize;
    let run = Run::new(vec![input[..frames].to_vec()])
        .with_automation(Param::TimeSmoothing.id(), vec![1.0; frames])
        .with_automation(Param::On.id(), vec![1.0; frames]);
    assert_settle(
        || Reported {
            node: Spectral::new(&off),
            settle,
        },
        &mono(),
        &run,
        from,
    );
}

#[test]
fn bad_input_and_bad_lanes_do_not_stick() {
    let frames = 30 * RATE as usize;
    let clean = sum(&voice(RATE, 0.1, frames), &white(121, 0.01, frames));
    let mut bad = clean.clone();
    bad[20_000] = f32::INFINITY;
    bad[20_001] = f32::NAN;
    bad[30_000] = f32::NEG_INFINITY;
    bad[40_000] = 1e30;
    let settings = Settings::default();
    let good = run_mono(&settings, &clean);
    let spoiled = run_mono(&settings, &bad);
    assert!(spoiled.iter().all(|x| x.is_finite()));
    let settle = Spectral::settle_for(&settings, RATE) as usize;
    for n in 40_000 + settle..frames {
        assert!(
            (good[n] - spoiled[n]).abs() < 3.2e-5,
            "at {n}: {} {}",
            good[n],
            spoiled[n]
        );
    }
    // Lanes of NaN and infinities keep the values set.
    let short = clean[..3 * RATE as usize].to_vec();
    let length = short.len();
    let mut spoilt_lane = vec![f32::NAN; length];
    spoilt_lane[100] = f32::INFINITY;
    spoilt_lane[200] = f32::NEG_INFINITY;
    let plain = run(&settings, &mono(), vec![short.clone()]);
    for param in Param::ALL.into_iter().filter(|p| p.is_automatable()) {
        let mut node = Spectral::new(&settings);
        let spec = mono().with_max_block(length);
        node.prepare(&spec).unwrap();
        let run = Run::new(vec![short.clone()]).with_automation(param.id(), spoilt_lane.clone());
        let output = render(&mut node, &spec, &run, |left| left);
        assert_eq!(output, plain, "{param:?}");
    }
    // Listen and the band have no lanes.
    for param in [Param::Listen, Param::Band] {
        let mut node = Spectral::new(&settings);
        let spec = mono().with_max_block(length);
        node.prepare(&spec).unwrap();
        let run = Run::new(vec![short.clone()]).with_automation(param.id(), vec![1.0; length]);
        assert_eq!(
            render(&mut node, &spec, &run, |left| left),
            plain,
            "{param:?}"
        );
    }
}

#[test]
fn it_takes_no_memory_on_the_thread_of_the_mix() {
    let spec = Spec::new(RATE, 480, Layout::Stereo);
    let mut node = Spectral::new(&Settings::default());
    node.prepare(&spec).unwrap();
    node.align(12_345);
    let mut input = Planar::new(2, 480);
    let mut output = Planar::new(2, 480);
    let lane = vec![20.0_f32; 480];
    let signal = sum(&voice(RATE, 0.1, 480 * 1000), &white(131, 0.01, 480 * 1000));
    let ((), allocations) = realtime(|| {
        for block in 0..1000 {
            for channel in 0..2 {
                input
                    .channel_mut(channel)
                    .copy_from_slice(&signal[block * 480..(block + 1) * 480]);
            }
            if block % 7 == 0 {
                node.set(Param::ThresholdDb.id(), (block % 40) as f32);
                node.set(Param::DryWet.id(), 50.0 + (block % 50) as f32);
                node.set(Param::Listen.id(), (block % 2) as f32);
            }
            if block == 500 {
                node.reset();
            }
            let lanes = [Automated {
                param: Param::ThresholdDb.id(),
                values: &lane,
            }];
            let automation = if block % 3 == 0 {
                Automation::new(&lanes)
            } else {
                Automation::NONE
            };
            let ctx = Block::new((block % 500 * 480) as u64, RATE, 480).with_automation(automation);
            input.read(480, |input| {
                output.write(480, |output| node.process(&ctx, input, output));
            });
            let _ = node.settle();
        }
    });
    assert!(allocations.none(), "{allocations:?}");
}

/// With a noise learned (a profile in the settings), the node and its settling take no memory
/// either.
#[test]
fn a_learned_noise_takes_no_memory_on_the_thread_of_the_mix() {
    let spec = Spec::new(RATE, 256, Layout::Mono);
    let settings = with(|s| {
        s.mode = Mode::Manual;
        s.profile = learn(&[&white(132, 0.01, 3 * FFT)]);
    });
    let mut node = NoiseReduction::new(&settings);
    node.prepare(&spec).unwrap();
    let signal = white(133, 0.01, 256 * 200);
    let mut output = vec![0.0; 256];
    let ((), allocations) = realtime(|| {
        for block in 0..200 {
            let ctx = Block::new((block * 256) as u64, RATE, 256);
            node.process(
                &ctx,
                &[&signal[block * 256..(block + 1) * 256]],
                &mut [&mut output],
            );
            node.set(Param::AttackMs.id(), 100.0 + block as f32);
            let _ = node.settle();
        }
        let _ = NoiseReduction::settle_for(&settings, RATE);
    });
    assert!(allocations.none(), "{allocations:?}");
    assert_eq!(
        node.settle(),
        Spectral::settle_for(&settings, RATE),
        "the settling of the node is that of its settings"
    );
    assert!(
        Spectral::settle_for(&settings, RATE) < Spectral::settle_for(&Settings::default(), RATE)
    );
}

#[test]
fn the_gate_holds_no_word_of_locks_allocations_or_logs() {
    let source = include_str!("../src/fx/denoise.rs");
    assert_eq!(rt_violations(source), []);
}

/// The plan's time: mono at 48 kHz, 60 s in less than 1.5 s (run in release:
/// `cargo test --release -p columbina-audio --test denoise -- --ignored --nocapture`).
#[test]
#[ignore = "a measure of time, in release"]
fn a_minute_of_mono_takes_less_than_a_second_and_a_half() {
    let frames = 60 * RATE as usize;
    let input = sum(&voice(RATE, 0.1, frames), &white(141, 0.01, frames));
    let spec = mono();
    let mut node = Spectral::new(&Settings::default());
    node.prepare(&spec).unwrap();
    let mut output = vec![0.0; 1024];
    let start = std::time::Instant::now();
    for (block, chunk) in input.chunks(1024).enumerate() {
        let ctx = Block::new((block * 1024) as u64, RATE, chunk.len());
        node.process(&ctx, &[chunk], &mut [&mut output[..chunk.len()]]);
    }
    let elapsed = start.elapsed();
    println!("60 s of mono in {elapsed:?}");
    assert!(elapsed.as_secs_f64() < 1.5);
}

/// The plan's comparison with ffmpeg's `afftdn` on the same sound: the order of what each takes
/// away between the words, recorded, not judged (run by hand:
/// `cargo test --release -p columbina-audio --test denoise -- --ignored --nocapture`, with the
/// LGPL ffmpeg of the toolchain, or `COLUMBINA_FFMPEG` naming another).
#[test]
#[ignore = "runs ffmpeg; recorded, not judged"]
fn beside_ffmpeg_afftdn() {
    let ffmpeg = std::env::var("COLUMBINA_FFMPEG").unwrap_or_else(|_| {
        r"F:\agent harness\_review\toolchain\ffmpeg-lgpl-8.1\bin\ffmpeg.exe".to_owned()
    });
    let frames = 10 * RATE as usize;
    let (input, clean) = voice_over(&white(11, 0.01, frames));
    let dir = std::env::temp_dir().join(format!("columbina-t12-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (raw_in, raw_out) = (dir.join("in.f32"), dir.join("out.f32"));
    let bytes: Vec<u8> = input.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(&raw_in, bytes).unwrap();
    for filter in ["afftdn", "afftdn=nf=-40", "afftdn=nr=30:nf=-40"] {
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "f32le",
                "-ar",
                "48000",
            ])
            .args(["-ac", "1", "-i"])
            .arg(&raw_in)
            .args(["-af", filter, "-f", "f32le"])
            .arg(&raw_out)
            .status()
            .expect("ffmpeg");
        assert!(status.success());
        let bytes = std::fs::read(&raw_out).unwrap();
        let output: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let floor = left(&input, &output, 0, &silences(RATE, frames, 2.0, 0.1));
        let voice = kept(&clean, &output, 0, &speeches(RATE, frames, 2.0, 0.05));
        println!("{filter}: the noise between the words {floor:.2} dB, the voice {voice:.3} dB");
    }
    let output = run_mono(&Settings::default(), &input);
    let floor = left(
        &input,
        &output,
        LATENCY,
        &silences(RATE, frames - LATENCY, 2.0, 0.1),
    );
    let voice = kept(
        &clean,
        &output,
        LATENCY,
        &speeches(RATE, frames - LATENCY, 2.0, 0.05),
    );
    println!("Noise Reduction: the noise between the words {floor:.2} dB, the voice {voice:.3} dB");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A node aligned to `origin` whenever it is prepared or reset, as the graph aligns it to where
/// a run starts.
struct Aligned<N> {
    node: N,
    origin: u64,
}

impl<N: Node> Node for Aligned<N> {
    fn prepare(&mut self, spec: &Spec) -> Result<(), columbina_audio::Unsupported> {
        self.node.prepare(spec)?;
        self.node.align(self.origin);
        Ok(())
    }
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.node.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.node.latency()
    }
    fn settle(&self) -> u32 {
        self.node.settle()
    }
    fn reset(&mut self) {
        self.node.reset();
        self.node.align(self.origin);
    }
    fn set(&mut self, param: columbina_audio::ParamId, value: f32) {
        self.node.set(param, value);
    }
}

/// Laid on a grid that does not start at 0, the output still does not depend on the blocks.
#[test]
fn aligned_off_the_start_its_output_does_not_depend_on_the_blocks() {
    for origin in [12_345_u64, 1_000_000_000_007] {
        chunk_invariance(
            || Aligned {
                node: Spectral::default_node(),
                origin,
            },
            &stereo(),
            &busy_run(30_000, 2),
            &[7],
        );
    }
}

// --- Digital silence and the start (review of T12) ------------------------------------------

/// Exact zeros between two clips do not make the gate forget the noise: right after a gap of
/// 0.25 s the noise is down by 20 dB at least (in its steady state, 28.7 dB).
#[test]
fn a_gap_of_digital_silence_keeps_the_noise_found_before_it() {
    let rate = RATE as usize;
    let mut input = white(301, 0.01, 4 * rate);
    input.extend(std::iter::repeat_n(0.0, rate / 4));
    let after = input.len();
    input.extend(white(302, 0.01, 3 * rate));
    let output = run_mono(&Settings::default(), &input);
    for (from, to) in [(0, rate / 2), (rate / 2, rate)] {
        let range = after + from..after + to;
        let depth = left(&input, &output, LATENCY, std::slice::from_ref(&range));
        println!("{from}..{to} after the gap: {depth:.2} dB");
        assert!(depth <= -20.0, "{depth} dB in {range:?}");
    }
}

/// A voice from the first sample is not taken for the noise: its first half second keeps its
/// level within 1 dB.
#[test]
fn a_voice_from_the_start_is_kept() {
    let frames = 4 * RATE as usize;
    let clean = voice(RATE, 0.1, frames);
    let input = sum(&clean, &white(311, 0.01, frames));
    let output = run_mono(&Settings::default(), &input);
    let first = 0..RATE as usize / 2;
    let voice = kept(&clean, &output, LATENCY, std::slice::from_ref(&first));
    println!("the first half second of the voice: {voice:.3} dB");
    assert!(voice.abs() <= 1.0, "{voice} dB");
    // And the later words as before.
    let voice = kept(
        &clean,
        &output,
        LATENCY,
        &speeches(RATE, frames - LATENCY, 1.0, 0.05),
    );
    assert!(voice.abs() <= 1.0, "{voice} dB");
}

// --- The numbers of the gate, checked on their own (review of T12) ---------------------------

/// The gate worked out here from its definitions, for a learned noise flat at `noise` in every
/// bin and a tone at the bin `k0` (a multiple of 4, so that every frame holds the same samples):
/// the output over one hop in its steady state, from a transform of its own in f64.
fn by_hand(
    amplitude: f64,
    k0: usize,
    noise: f64,
    threshold_db: f64,
    sensitivity: f64,
    smoothing: f64,
) -> Vec<f64> {
    use std::f64::consts::PI;
    let n = FFT;
    let wa = |m: usize| -> f64 {
        if m < 1280 {
            (PI * m as f64 / 2560.0).sin()
        } else {
            (PI * (m - 512) as f64 / 1536.0).sin()
        }
    };
    let ws = |m: usize| -> f64 {
        if m < 512 {
            0.0
        } else {
            2.0 / 3.0 * (PI * (m - 512) as f64 / 1536.0).sin().powi(2) / wa(m)
        }
    };
    // A frame starts on a multiple of 512, where the tone's phase is 0.
    let frame: Vec<f64> = (0..n)
        .map(|m| wa(m) * amplitude * (2.0 * PI * (k0 * m) as f64 / n as f64).sin())
        .collect();
    let cos: Vec<f64> = (0..n)
        .map(|m| (2.0 * PI * m as f64 / n as f64).cos())
        .collect();
    let sin: Vec<f64> = (0..n)
        .map(|m| (2.0 * PI * m as f64 / n as f64).sin())
        .collect();
    let spectrum: Vec<(f64, f64)> = (0..BINS)
        .map(|k| {
            frame
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (m, x)| {
                    let at = (k * m) % n;
                    (re + x * cos[at], im - x * sin[at])
                })
        })
        .collect();
    let power: Vec<f64> = spectrum.iter().map(|(re, im)| re * re + im * im).collect();
    let half = (8.0 * smoothing / 100.0_f64).round() as usize;
    let beta = 1.0 + 3.0 * sensitivity / 100.0;
    let theta = 10.0_f64.powf(threshold_db / 10.0);
    let floor = 10.0_f64.powf(-30.0 / 20.0);
    let gains: Vec<f64> = (0..BINS)
        .map(|k| {
            let (from, to) = (k.saturating_sub(half), (k + half).min(BINS - 1));
            let q =
                (power[from..=to].iter().sum::<f64>() / (to - from + 1) as f64).clamp(1e-12, 1e12);
            let r = q / (beta * theta * noise);
            floor + (1.0 - floor) * r * r / (1.0 + r * r)
        })
        .collect();
    // The inverse of the half spectrum, each bin under its gain.
    let z: Vec<f64> = (0..n)
        .map(|m| {
            let mut sum = 0.0;
            for k in 0..BINS {
                let (re, im) = (spectrum[k].0 * gains[k], spectrum[k].1 * gains[k]);
                let at = (k * m) % n;
                let term = re * cos[at] - im * sin[at];
                sum += if k == 0 || k == n / 2 {
                    term
                } else {
                    2.0 * term
                };
            }
            sum / n as f64
        })
        .collect();
    // The frames are all the same: a sample of the hop sums the three frames over it.
    (0..HOP)
        .map(|j| (1..4).map(|q| ws(j + q * HOP) * z[j + q * HOP]).sum())
        .collect()
}

#[test]
fn threshold_sensitivity_and_frequency_smoothing_shape_the_gain_as_written() {
    let (k0, amplitude) = (100, 0.01_f64);
    let tone = sine(
        RATE,
        k0 as f64 * f64::from(RATE) / FFT as f64,
        amplitude as f32,
        RATE as usize,
    );
    // The noise put where the mask of the defaults meets the tone at its bin (r = 1 there).
    let power_at_k0 = (amplitude
        * 0.5
        * (0..FFT)
            .map(|m| f64::from(analysis_window()[m]))
            .sum::<f64>())
    .powi(2);
    let noise = power_at_k0 / (2.5 * 10.0);
    for (threshold, sensitivity, smoothing) in [
        (10.0, 50.0, 0.0),
        (10.0, 0.0, 0.0),
        (20.0, 100.0, 0.0),
        (0.0, 50.0, 25.0),
        (10.0, 100.0, 100.0),
    ] {
        let settings = with(|s| {
            s.mode = Mode::Manual;
            s.profile = Some(vec![noise as f32; BINS]);
            s.threshold_db = threshold as f32;
            s.sensitivity = sensitivity as f32;
            s.frequency_smoothing = smoothing as f32;
            s.time_smoothing = false;
        });
        let output = run_mono(&settings, &tone);
        let expected = by_hand(amplitude, k0, noise, threshold, sensitivity, smoothing);
        // A sample t of the output is the hop's sample (t − 1536) mod 512, frames starting on
        // multiples of 512.
        let mut largest = 0.0_f64;
        let start = 3 * FFT;
        for (t, &sample) in output.iter().enumerate().skip(start).take(4 * HOP) {
            let j = (t - LATENCY) % HOP;
            largest = largest.max((f64::from(sample) - expected[j]).abs());
        }
        let peak = expected.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
        println!("T {threshold} S {sensitivity} FS {smoothing}: peak {peak:.6}, error {largest:e}");
        assert!(
            largest <= 1e-5 * amplitude,
            "{threshold} {sensitivity} {smoothing}: {largest}"
        );
    }
}

/// Noise alone, after 2 s, at three values of a parameter.
fn residuals(param: Param, values: [f32; 3]) -> [f64; 3] {
    let frames = 5 * RATE as usize;
    let noise = white(321, 0.01, frames);
    let range = 2 * RATE as usize..frames - LATENCY;
    values.map(|value| {
        let output = run_mono(&with(|s| s.set(param, value)), &noise);
        left(&noise, &output, LATENCY, std::slice::from_ref(&range))
    })
}

#[test]
fn more_threshold_and_more_sensitivity_leave_less_noise() {
    for (param, values) in [
        (Param::ThresholdDb, [0.0, 5.0, 10.0]),
        (Param::Sensitivity, [0.0, 50.0, 100.0]),
    ] {
        let left = residuals(param, values);
        println!("{param:?} {values:?}: {left:.2?} dB");
        assert!(
            left[0] > left[1] + 0.5 && left[1] > left[2] + 0.1,
            "{param:?}: {left:?}"
        );
    }
}

/// The noise rises 20 dB at 3 s: a longer Attack, or a lower Ratio (a slower signal profile),
/// follows it more slowly, so that more of it is let through in the half second after.
#[test]
fn a_longer_attack_and_a_lower_ratio_follow_a_louder_noise_more_slowly() {
    let rate = RATE as usize;
    let mut noise = white(331, 0.001, 3 * rate);
    noise.extend(white(332, 0.01, 2 * rate));
    let range = 3 * rate + rate / 4..3 * rate + rate;
    let after = |edit: &dyn Fn(&mut Settings)| {
        let mut settings = Settings::default();
        edit(&mut settings);
        let output = run_mono(&settings, &noise);
        left(&noise, &output, LATENCY, std::slice::from_ref(&range))
    };
    let attacks = [100.0, 500.0, 2000.0].map(|ms| after(&|s| s.attack_ms = ms));
    let ratios = [20.0, 4.0, 1.0].map(|ratio| after(&|s| s.ratio = ratio));
    println!("attack 100, 500, 2000 ms: {attacks:.2?} dB; ratio 20, 4, 1: {ratios:.2?} dB");
    assert!(
        attacks[0] < attacks[1] && attacks[1] < attacks[2],
        "{attacks:?}"
    );
    assert!(ratios[0] < ratios[1] && ratios[1] < ratios[2], "{ratios:?}");
}

/// A run whose settling reaches past its end compares nothing: the check says so.
#[test]
#[should_panic(expected = "nothing to compare")]
fn a_settling_longer_than_the_run_is_refused() {
    let run = Run::new(vec![white(341, 0.01, 2 * RATE as usize)]);
    let _ = assert_settle(Spectral::default_node, &mono(), &run, 1_000);
}

/// The pre-roll of 10 s (T6) for slow settings: after it, a start in the middle differs from an
/// earlier one by at most −60 dBFS, within 10 to 12 s, from a start in a word and in a pause
/// (the review measured −53 and −24 dBFS before the profile's rise was sped up after a reset).
#[test]
fn ten_seconds_of_pre_roll_bring_slow_settings_within_minus_60_dbfs() {
    let rate = RATE as usize;
    let frames = 40 * rate;
    let input = sum(&voice(RATE, 0.1, frames), &white(351, 0.01, frames));
    for (attack, ratio) in [(1000.0, 4.0), (2000.0, 1.0), (5000.0, 1.0)] {
        let settings = with(|s| {
            s.attack_ms = attack;
            s.ratio = ratio;
        });
        let spec = mono().with_max_block(frames);
        let mut early = Spectral::new(&settings);
        early.prepare(&spec).unwrap();
        let whole = render(&mut early, &spec, &Run::new(vec![input.clone()]), |left| {
            left
        })
        .remove(0);
        for from in [512 * 47 * 20, 512 * 1003] {
            let mut late = Spectral::new(&settings);
            late.prepare(&spec).unwrap();
            late.align(from as u64);
            let tail = render(
                &mut late,
                &spec,
                &Run::new(vec![input[from..].to_vec()]),
                |left| left,
            )
            .remove(0);
            let largest = (10 * rate..12 * rate)
                .map(|n| (whole[from + n] - tail[n]).abs())
                .fold(0.0_f32, f32::max);
            let dbfs = 20.0 * f64::from(largest.max(1e-30)).log10();
            println!("attack {attack} ratio {ratio} from {from}: {dbfs:.1} dBFS");
            if attack <= 2000.0 {
                assert!(dbfs <= -60.0, "{attack} {ratio} from {from}: {dbfs} dBFS");
            }
        }
    }
}

/// After more than [`DENOISE_FORGET`] of digital silence the gate starts again as new: an
/// export from the start and a playback started inside the gap give the same bits from the
/// second clip on.
#[test]
fn after_a_long_gap_a_start_inside_it_hears_what_an_export_hears() {
    let rate = RATE as usize;
    let clip = |seed: u64| sum(&voice(RATE, 0.1, 3 * rate), &white(seed, 0.01, 3 * rate));
    let mut input = clip(361);
    input.extend(std::iter::repeat_n(0.0, 12 * rate));
    let second = input.len();
    input.extend(clip(362));
    let spec = mono().with_max_block(input.len());
    let mut early = Spectral::default_node();
    early.prepare(&spec).unwrap();
    let whole = render(&mut early, &spec, &Run::new(vec![input.clone()]), |left| {
        left
    })
    .remove(0);
    for from in [8 * rate + 77, second - rate / 3] {
        let mut late = Spectral::default_node();
        late.prepare(&spec).unwrap();
        late.align(from as u64);
        let tail = render(
            &mut late,
            &spec,
            &Run::new(vec![input[from..].to_vec()]),
            |left| left,
        )
        .remove(0);
        for t in second..input.len() {
            assert_eq!(
                whole[t].to_bits(),
                tail[t - from].to_bits(),
                "from {from}, at {t}"
            );
        }
    }
    // A shorter gap keeps the noise (the test of the gap of 0.25 s), and so differs.
    const { assert!(columbina_audio::constants::DENOISE_FORGET <= 12.0) };
}
