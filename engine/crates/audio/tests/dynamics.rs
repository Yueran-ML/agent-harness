//! The Dynamics of a strip (plan T8): the static curves against their formulas, the levels of
//! steady tones, the times of attack, hold and release, the sidechain, the linking of the
//! channels, bypass, the independence of the blocks, no allocation, no zipper, latency and
//! settling, and the words of the thread of the mix.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "the numbers of tests"
)]

use columbina_audio::constants::{DYNAMICS_HOLD_BIN, DYNAMICS_TIME_CONSTANT_FRACTION};
use columbina_audio::fx::dynamics::{
    compressor_gain_db, curve, expander_gain_db, gate_gain_db, limiter_gain_db, mixed_gain_db,
    Dynamics, Param, Settings, PARAMS,
};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, realtime, render, rt_violations, sine,
    CountingAllocator, Run,
};
use columbina_audio::{db_to_gain, Automated, Automation, Block, Layout, Node, Planar, Spec};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const RATE: u32 = 48_000;

fn ms(ms: f64) -> usize {
    (ms * f64::from(RATE) / 1000.0).round() as usize
}

fn amplitude(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

fn peak_db(signal: &[f32]) -> f64 {
    let peak = signal.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
    20.0 * f64::from(peak).log10()
}

/// The output of a node of `settings` through `run`, in blocks of 480.
fn process(settings: &Settings, spec: &Spec, run: &Run) -> Vec<Vec<f32>> {
    let mut node = Dynamics::new(settings);
    node.prepare(spec).unwrap();
    render(&mut node, spec, run, |_| 480)
}

fn mono(settings: &Settings, input: Vec<f32>) -> Vec<f32> {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    process(settings, &spec, &Run::new(vec![input])).remove(0)
}

/// Dynamics on, changed by `change`.
fn on(change: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings {
        on: true,
        ..Settings::default()
    };
    change(&mut settings);
    settings
}

/// A steady level: samples of `db` dBFS of alternating sign, whose peak never dips.
fn level(db: f64, frames: usize) -> Vec<f32> {
    let a = amplitude(db);
    (0..frames)
        .map(|n| if n.is_multiple_of(2) { a } else { -a })
        .collect()
}

/// Steps of steady levels: `(dB, seconds)` one after the other.
fn steps(parts: &[(f64, f64)]) -> Vec<f32> {
    parts
        .iter()
        .flat_map(|&(db, seconds)| level(db, ms(seconds * 1000.0)))
        .collect()
}

/// The gain of each sample, dB.
fn gains(input: &[f32], output: &[f32]) -> Vec<f64> {
    input
        .iter()
        .zip(output)
        .map(|(x, y)| 20.0 * (f64::from(*y) / f64::from(*x)).abs().log10())
        .collect()
}

/// The samples from `from` until the gain has gone `DYNAMICS_TIME_CONSTANT_FRACTION` of the way
/// from `start` to `end`.
fn time_to_63(gains: &[f64], from: usize, start: f64, end: f64) -> usize {
    let goal = start + DYNAMICS_TIME_CONSTANT_FRACTION * (end - start);
    let rising = end > start;
    gains[from..]
        .iter()
        .position(|&g| if rising { g >= goal } else { g <= goal })
        .expect("the gain gets there")
        + 1
}

// --- The static curves -----------------------------------------------------------------------

#[test]
fn the_static_curves_are_their_formulas() {
    // The plan's examples (T8).
    assert_eq!(compressor_gain_db(-5.0, -15.0, 2.0, 0.0), -5.0);
    assert_eq!(compressor_gain_db(-20.0, -15.0, 2.0, 0.0), 0.0);
    assert_eq!(gate_gain_db(-50.0, -40.0, 18.0), -18.0);
    assert_eq!(gate_gain_db(-30.0, -40.0, 18.0), 0.0);
    assert_eq!(expander_gain_db(-45.0, -35.0, 2.0, 18.0), -10.0);
    assert_eq!(expander_gain_db(-60.0, -35.0, 2.0, 18.0), -18.0);
    assert_eq!(expander_gain_db(-20.0, -35.0, 2.0, 18.0), 0.0);
    assert_eq!(limiter_gain_db(-10.0, -21.0), -11.0);
    assert_eq!(limiter_gain_db(-30.0, -21.0), 0.0);
    // A knee 10 dB wide: half the ratio's slope at the threshold, and meeting the straight lines
    // at its edges.
    let knee = |level| compressor_gain_db(level, -15.0, 2.0, 10.0);
    assert_eq!(knee(-20.0), 0.0);
    assert!((knee(-15.0) - -0.625).abs() < 1e-12);
    assert!((knee(-10.0) - -2.5).abs() < 1e-12);
    assert!((knee(0.0) - compressor_gain_db(0.0, -15.0, 2.0, 0.0)).abs() < 1e-12);
    // Mix: 50 % of a gain of −15 dB is 20·log10(0.5 + 0.5·10^(−15/20)).
    let half = 20.0 * (0.5 + 0.5 * 10.0_f64.powf(-0.75)).log10();
    assert!((mixed_gain_db(-15.0, 50.0) - half).abs() < 1e-12);
    assert_eq!(mixed_gain_db(-15.0, 0.0), -15.0);
    assert_eq!(mixed_gain_db(-15.0, 100.0), 0.0);
}

#[test]
fn the_curve_of_the_window_puts_the_modules_one_after_the_other() {
    let inputs = [-60.0, -45.0, -30.0, -20.0, -15.0, -10.0, -5.0, 0.0];
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -40.0;
        s.expander.ratio = 1.5;
        s.expander.range_db = 12.0;
        s.compressor.on = true;
        s.compressor.threshold_db = -25.0;
        s.compressor.ratio = 4.0;
        s.compressor.knee_db = 6.0;
        s.compressor.mix = 20.0;
        s.limiter.on = true;
        s.limiter.threshold_db = -12.0;
        s.makeup_db = 3.0;
    });
    let drawn = curve(&settings, &inputs.map(|x: f64| x as f32));
    for (input, drawn) in inputs.iter().zip(&drawn) {
        let mut level = *input;
        level += expander_gain_db(level, -40.0, 1.5, 12.0);
        level += mixed_gain_db(compressor_gain_db(level, -25.0, 4.0, 6.0), 20.0);
        level += limiter_gain_db(level, -12.0);
        let expected = level + 3.0;
        assert!(
            (f64::from(*drawn) - expected).abs() < 1e-4,
            "{input}: {drawn}, {expected}"
        );
    }
    // Off, the input; the default strip has everything off.
    let off = Settings {
        on: false,
        ..settings
    };
    assert_eq!(curve(&off, &[-5.0, -40.0]), [-5.0, -40.0]);
    assert_eq!(curve(&Settings::default(), &[-5.0]), [-5.0]);
    // The plan's compressor: −5 → −10, −20 → −20.
    let compressor = on(|s| s.compressor.on = true);
    assert_eq!(curve(&compressor, &[-5.0, -20.0]), [-10.0, -20.0]);
}

#[test]
fn a_steady_level_is_given_the_gain_of_the_curve() {
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -40.0;
        s.expander.ratio = 1.5;
        s.expander.range_db = 12.0;
        s.compressor.on = true;
        s.compressor.threshold_db = -25.0;
        s.compressor.ratio = 4.0;
        s.compressor.knee_db = 6.0;
        s.compressor.mix = 20.0;
        s.limiter.on = true;
        s.limiter.threshold_db = -12.0;
        s.makeup_db = 3.0;
    });
    for db in [-60.0, -45.0, -38.0, -30.0, -24.0, -20.0, -10.0, -3.0, 6.0] {
        let input = level(db, ms(2000.0));
        let output = mono(&settings, input.clone());
        let n = input.len() - 1;
        let measured = 20.0 * f64::from(output[n] / input[n]).abs().log10();
        let drawn = f64::from(curve(&settings, &[db as f32])[0]) - db;
        assert!(
            (measured - drawn).abs() < 0.005,
            "{db} dBFS: gain {measured}, the curve's {drawn}"
        );
    }
}

// --- Steady tones (plan T8) ------------------------------------------------------------------

/// The peak of the last half second of a 1 kHz tone of `db` dBFS through `settings`.
fn tone(settings: &Settings, db: f64) -> f64 {
    let output = mono(settings, sine(RATE, 1000.0, amplitude(db), ms(2000.0)));
    peak_db(&output[ms(1500.0)..])
}

#[test]
fn a_compressor_at_minus_15_and_2_to_1_takes_minus_5_to_minus_10() {
    let settings = on(|s| s.compressor.on = true);
    let loud = tone(&settings, -5.0);
    assert!((loud - -10.0).abs() <= 0.05, "{loud}");
    let quiet = tone(&settings, -20.0);
    assert!((quiet - -20.0).abs() <= 0.05, "{quiet}");
}

#[test]
fn a_gate_at_minus_40_with_a_range_of_18_takes_minus_50_to_minus_68() {
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.gate = true;
        s.expander.threshold_db = -40.0;
        s.expander.range_db = 18.0;
    });
    let gated = tone(&settings, -50.0);
    assert!((gated - -68.0).abs() <= 0.1, "{gated}");
    // Open, the gate starts to shut between the peaks of the wave (no hold) and opens again at
    // each: a ripple of hundredths of a dB.
    let open = tone(&settings, -30.0);
    assert!((open - -30.0).abs() <= 0.05, "{open}");
}

#[test]
fn an_expander_at_minus_35_and_1_to_2_takes_minus_45_to_minus_55() {
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -35.0;
        s.expander.ratio = 2.0;
    });
    let expanded = tone(&settings, -45.0);
    assert!((expanded - -55.0).abs() <= 0.1, "{expanded}");
}

#[test]
fn the_limiter_of_dynamics_at_minus_21_holds_minus_10_to_minus_21() {
    let settings = on(|s| s.limiter.on = true);
    let limited = tone(&settings, -10.0);
    assert!(limited <= -21.0 + 0.1, "{limited}");
    assert!(limited >= -21.5, "{limited}");
}

// --- Times -----------------------------------------------------------------------------------

#[test]
fn the_compressor_attacks_holds_and_releases_in_the_times_set() {
    let settings = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -30.0;
        s.compressor.ratio = 10.0;
        s.compressor.attack_ms = 10.0;
        s.compressor.hold_ms = 50.0;
        s.compressor.release_ms = 200.0;
    });
    let input = steps(&[(-40.0, 0.5), (-5.0, 1.0), (-40.0, 2.0)]);
    let output = mono(&settings, input.clone());
    let gains = gains(&input, &output);
    let reduction = -(25.0 * 0.9);
    let up = ms(500.0);
    assert!(gains[up - 1].abs() < 1e-5, "{}", gains[up - 1]);
    let attack = time_to_63(&gains, up, 0.0, reduction);
    assert!(
        attack.abs_diff(ms(10.0)) <= ms(10.0) / 20,
        "attack {attack}"
    );
    let down = ms(1500.0);
    let held = gains[down - 1];
    assert!((held - reduction).abs() < 1e-4, "{held}");
    for (n, g) in gains[down..down + ms(50.0)].iter().enumerate() {
        assert!((g - held).abs() < 1e-4, "moved in the hold at {n}: {g}");
    }
    // The hold is kept in bins: the release starts from H to H + one bin after the drop.
    let bin = DYNAMICS_HOLD_BIN as usize;
    let starts = gains[down..]
        .iter()
        .position(|g| (g - held).abs() >= 1e-4)
        .unwrap();
    assert!(
        (ms(50.0)..=ms(50.0) + bin).contains(&starts),
        "the release starts {starts} after the drop"
    );
    let release = time_to_63(&gains, down + starts, held, 0.0);
    assert!(
        release.abs_diff(ms(200.0)) <= ms(200.0) / 20,
        "release {release}"
    );
}

#[test]
fn the_gate_opens_in_its_attack_stays_open_in_its_hold_and_shuts_in_its_release() {
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.gate = true;
        s.expander.threshold_db = -40.0;
        s.expander.range_db = 30.0;
        s.expander.attack_ms = 5.0;
        s.expander.hold_ms = 100.0;
        s.expander.release_ms = 300.0;
    });
    let input = steps(&[(-60.0, 0.5), (-20.0, 1.0), (-60.0, 2.0)]);
    let output = mono(&settings, input.clone());
    let gains = gains(&input, &output);
    let up = ms(500.0);
    assert!(
        (gains[up - 1] - -30.0).abs() < 1e-4,
        "shut: {}",
        gains[up - 1]
    );
    let attack = time_to_63(&gains, up, -30.0, 0.0);
    assert!(attack.abs_diff(ms(5.0)) <= ms(5.0) / 20, "attack {attack}");
    let down = ms(1500.0);
    for (n, g) in gains[down..down + ms(100.0)].iter().enumerate() {
        assert!(g.abs() < 1e-4, "shut in the hold at {n}: {g}");
    }
    // The hold is kept in bins (up to one bin longer): within the ±5 % of the release below.
    let release = time_to_63(&gains, down + ms(100.0), 0.0, -30.0);
    assert!(
        release.abs_diff(ms(300.0)) <= ms(300.0) / 20,
        "release {release}"
    );
}

#[test]
fn the_limiter_attacks_holds_and_releases_in_the_times_set() {
    let settings = on(|s| {
        s.limiter.on = true;
        s.limiter.threshold_db = -20.0;
        s.limiter.attack_ms = 2.0;
        s.limiter.hold_ms = 30.0;
        s.limiter.release_ms = 100.0;
    });
    let input = steps(&[(-40.0, 0.5), (-5.0, 1.0), (-40.0, 1.0)]);
    let output = mono(&settings, input.clone());
    let gains = gains(&input, &output);
    let attack = time_to_63(&gains, ms(500.0), 0.0, -15.0);
    assert!(attack.abs_diff(ms(2.0)) <= ms(2.0) / 20, "attack {attack}");
    let down = ms(1500.0);
    for g in &gains[down..down + ms(30.0)] {
        assert!((g - -15.0).abs() < 1e-4, "{g}");
    }
    let release = time_to_63(&gains, down + ms(30.0), -15.0, 0.0);
    assert!(
        release.abs_diff(ms(100.0)) <= ms(100.0) / 20,
        "release {release}"
    );
}

// --- The sidechain ---------------------------------------------------------------------------

fn keyed() -> Settings {
    on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -30.0;
        s.compressor.ratio = 4.0;
        s.compressor.sidechain = true;
    })
}

#[test]
fn a_sidechain_ducks_the_tone_while_its_source_plays() {
    // The plan's case: a 1 kHz tone at −20 dBFS, the source noise at −5 dBFS in [1 s, 2 s).
    let frames = ms(3000.0);
    let target = sine(RATE, 1000.0, amplitude(-20.0), frames);
    let mut source = vec![0.0; frames];
    source[ms(1000.0)..ms(2000.0)].copy_from_slice(&noise(7, amplitude(-5.0), ms(1000.0)));
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![target.clone()]).with_sidechain(vec![source]);
    let output = process(&keyed(), &spec, &run).remove(0);
    // Before the source, the tone is untouched, to the bit.
    assert_eq!(output[..ms(1000.0)], target[..ms(1000.0)]);
    for window in output[ms(1100.0)..ms(2000.0)].chunks(ms(10.0)) {
        let ducked = peak_db(window);
        assert!(ducked <= -20.0 - 15.0, "{ducked}");
    }
    let back = peak_db(&output[ms(2900.0)..]);
    assert!((back - -20.0).abs() <= 0.1, "{back}");
}

#[test]
fn keyed_by_its_sidechain_the_compressor_does_not_hear_its_own_sound() {
    let loud = sine(RATE, 1000.0, amplitude(-5.0), ms(1000.0));
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![loud.clone()]).with_sidechain(vec![vec![0.0; loud.len()]]);
    let output = process(&keyed(), &spec, &run).remove(0);
    assert_eq!(output, loud);
    // With no sidechain given (no source chosen), it hears its own sound.
    let output = mono(&keyed(), loud.clone());
    let compressed = peak_db(&output[ms(500.0)..]);
    assert!((compressed - -23.75).abs() < 0.1, "{compressed}");
}

#[test]
fn listening_lets_the_sidechain_out() {
    let frames = ms(200.0);
    let target = sine(RATE, 1000.0, 0.5, frames);
    let source = noise(3, 0.25, frames);
    let listen = {
        let mut settings = keyed();
        settings.compressor.listen = true;
        settings
    };
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let run = Run::new(vec![target.clone()]).with_sidechain(vec![source.clone()]);
    assert_eq!(process(&listen, &spec, &run)[0], source);
    // A stereo strip with a mono source hears it on both sides; Listen with the sidechain off
    // does nothing.
    let spec = Spec::new(RATE, 512, Layout::Stereo).with_sidechain(1);
    let run = Run::new(vec![target.clone(), target.clone()]).with_sidechain(vec![source.clone()]);
    let output = process(&listen, &spec, &run);
    assert_eq!(output, [source.clone(), source.clone()]);
    let mut unkeyed = listen;
    unkeyed.compressor.sidechain = false;
    unkeyed.compressor.on = false;
    assert_eq!(process(&unkeyed, &spec, &run), [target.clone(), target]);
}

// --- Linked channels -------------------------------------------------------------------------

#[test]
fn the_channels_are_linked_and_given_one_gain() {
    let frames = ms(1000.0);
    let left = sine(RATE, 1000.0, amplitude(-5.0), frames);
    let right = sine(RATE, 700.0, amplitude(-30.0), frames);
    let settings = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -20.0;
        s.compressor.ratio = 4.0;
    });
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    let output = process(
        &settings,
        &spec,
        &Run::new(vec![left.clone(), right.clone()]),
    );
    for n in ms(500.0)..frames {
        if left[n].abs() > 1e-3 && right[n].abs() > 1e-4 {
            let a = f64::from(output[0][n]) / f64::from(left[n]);
            let b = f64::from(output[1][n]) / f64::from(right[n]);
            assert!((a - b).abs() < 1e-5 * a, "{n}: {a}, {b}");
        }
    }
    // The quiet side is lowered with the loud one, by about (−5 + 20)·(1 − 1/4).
    let quiet = peak_db(&output[1][ms(500.0)..]);
    assert!((quiet - (-30.0 - 11.25)).abs() < 0.15, "{quiet}");
}

#[test]
fn linked_dynamics_do_not_hear_the_lfe_but_lower_it() {
    let frames = ms(1000.0);
    let settings = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -20.0;
        s.compressor.ratio = 4.0;
    });
    let spec = Spec::new(RATE, 512, Layout::Surround51);
    let quiet = sine(RATE, 1000.0, amplitude(-40.0), frames);
    let loud = sine(RATE, 60.0, amplitude(-5.0), frames);
    // A loud LFE alone: nothing heard above the threshold, nothing changed.
    let mut input = vec![quiet.clone(); 6];
    input[3].clone_from(&loud);
    let output = process(&settings, &spec, &Run::new(input.clone()));
    assert_eq!(output, input);
    // A loud centre: every channel lowered, the LFE as much as the others.
    let mut input = vec![quiet.clone(); 6];
    input[2] = sine(RATE, 1000.0, amplitude(-5.0), frames);
    input[3].clone_from(&quiet);
    let output = process(&settings, &spec, &Run::new(input.clone()));
    for channel in [0, 3, 5] {
        let lowered = peak_db(&output[channel][ms(500.0)..]);
        assert!(
            (lowered - (-40.0 - 11.25)).abs() < 0.15,
            "{channel}: {lowered}"
        );
    }
}

// --- Bypass ----------------------------------------------------------------------------------

fn everything_on() -> Settings {
    on(|s| {
        s.expander.on = true;
        s.compressor.on = true;
        s.compressor.threshold_db = -30.0;
        s.compressor.ratio = 6.0;
        s.limiter.on = true;
        s.makeup_db = 6.0;
    })
}

#[test]
fn off_or_with_nothing_on_it_lets_the_sound_through_to_the_bit() {
    let mut signal = noise(11, 1.0, ms(500.0));
    signal[10] = -0.0;
    signal[11] = 0.0;
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    let off = Settings {
        on: false,
        ..everything_on()
    };
    assert_eq!(bits(&mono(&off, signal.clone())), bits(&signal));
    let nothing = on(|_| {});
    assert_eq!(bits(&mono(&nothing, signal.clone())), bits(&signal));
    let all_dry = on(|s| {
        s.compressor.on = true;
        s.compressor.mix = 100.0;
    });
    assert_eq!(bits(&mono(&all_dry, signal.clone())), bits(&signal));
    // Turned off while it plays: through to the bit once the 10 ms have passed.
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let run = Run::new(vec![signal.clone()]).with_change(ms(100.0), Param::On.id(), 0.0);
    let output = process(&everything_on(), &spec, &run).remove(0);
    assert_ne!(output[..ms(100.0)], signal[..ms(100.0)]);
    assert_eq!(bits(&output[ms(110.0)..]), bits(&signal[ms(110.0)..]));
}

// --- The blocks, the memory, the zipper ------------------------------------------------------

/// Every module on, gating, the sidechain keying and listened to in turn, lanes of automation,
/// and changes along the way.
fn busy_run(channels: usize, frames: usize) -> Run {
    let input = (0..channels)
        .map(|c| {
            let tone = sine(RATE, 300.0 + 200.0 * c as f64, 0.6, frames);
            let hiss = noise(20 + c as u64, 0.2, frames);
            tone.iter()
                .zip(&hiss)
                .enumerate()
                .map(|(n, (t, h))| {
                    if (n / 3000).is_multiple_of(3) {
                        0.01 * t
                    } else {
                        t + h
                    }
                })
                .collect()
        })
        .collect();
    let mut key = noise(5, 0.9, frames);
    for (n, sample) in key.iter_mut().enumerate() {
        if !(n / 4000).is_multiple_of(2) {
            *sample = 0.0;
        }
    }
    let threshold = (0..frames)
        .map(|n| -40.0 + 30.0 * (n as f32 / frames as f32))
        .collect();
    let switching = (0..frames)
        .map(|n| {
            if (n / 5000).is_multiple_of(2) {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    Run::new(input)
        .with_sidechain(vec![key])
        .with_automation(Param::CompressorThresholdDb.id(), threshold)
        .with_automation(Param::LimiterOn.id(), switching)
        .with_change(1_000, Param::ExpanderMode.id(), 1.0)
        .with_change(2_500, Param::CompressorRatio.id(), 8.0)
        .with_change(4_000, Param::SidechainOn.id(), 1.0)
        .with_change(6_000, Param::MakeupDb.id(), 9.0)
        .with_change(7_000, Param::SidechainListen.id(), 1.0)
        .with_change(7_700, Param::SidechainListen.id(), 0.0)
        .with_change(9_000, Param::ExpanderHoldMs.id(), 40.0)
        .with_change(11_000, Param::On.id(), 0.0)
        .with_change(13_000, Param::On.id(), 1.0)
        .with_change(15_000, Param::ExpanderMode.id(), 0.0)
        .with_change(16_000, Param::CompressorKneeDb.id(), 12.0)
        .with_change(17_000, Param::CompressorMix.id(), 40.0)
}

fn busy() -> Settings {
    on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -30.0;
        s.expander.attack_ms = 0.5;
        s.expander.release_ms = 60.0;
        s.compressor.on = true;
        s.compressor.attack_ms = 3.0;
        s.compressor.hold_ms = 5.0;
        s.compressor.release_ms = 70.0;
        s.limiter.on = true;
        s.limiter.threshold_db = -6.0;
    })
}

#[test]
fn the_output_does_not_depend_on_the_blocks() {
    let spec = Spec::new(RATE, 2048, Layout::Stereo).with_sidechain(1);
    let output = chunk_invariance(
        || Dynamics::new(&busy()),
        &spec,
        &busy_run(2, 20_000),
        &[1, 2, 3],
    );
    assert_ne!(output[0], busy_run(2, 20_000).input[0]);
    let spec = Spec::new(RATE, 1024, Layout::Surround51);
    let run = Run::new((0..6).map(|c| noise(40 + c, 0.8, 12_000)).collect());
    let run = run
        .with_change(3_000, Param::CompressorOn.id(), 0.0)
        .with_change(6_000, Param::CompressorOn.id(), 1.0);
    chunk_invariance(|| Dynamics::new(&everything_on()), &spec, &run, &[4]);
}

#[test]
fn the_thread_of_the_mix_takes_no_memory() {
    const BLOCK: usize = 480;
    let spec = Spec::new(RATE, 512, Layout::Stereo).with_sidechain(1);
    let mut node = Dynamics::new(&busy());
    node.prepare(&spec).unwrap();
    let ball = node.ball();
    let mut input = Planar::new(2, 512);
    input.channel_mut(0).copy_from_slice(&noise(1, 0.8, 512));
    input.channel_mut(1).copy_from_slice(&noise(2, 0.8, 512));
    let mut key = Planar::new(1, 512);
    key.channel_mut(0).copy_from_slice(&noise(3, 0.9, 512));
    let mut output = Planar::new(2, 512);
    let lane: Vec<f32> = (0..BLOCK).map(|n| -40.0 + n as f32 / 20.0).collect();
    let ((), counted) = realtime(|| {
        for n in 0..2_000_usize {
            if n.is_multiple_of(7) {
                node.set(Param::ALL[n % PARAMS].id(), (n % 5) as f32 * 0.3);
            }
            if n == 1_000 {
                node.reset();
            }
            let lanes = [Automated {
                param: Param::CompressorThresholdDb.id(),
                values: &lane,
            }];
            let automation = if n.is_multiple_of(3) {
                Automation::new(&lanes)
            } else {
                Automation::NONE
            };
            let block = Block::new(0, RATE, BLOCK).with_automation(automation);
            key.read(BLOCK, |key| {
                input.read(BLOCK, |input| {
                    output.write(BLOCK, |output| {
                        node.process(&block.with_sidechain(key), input, output);
                    });
                });
            });
            let _ = ball.try_read();
        }
    });
    assert!(counted.none(), "{counted:?}");
}

/// The largest step between two samples of `signal`.
fn largest_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .fold(0.0, |step, pair| step.max((pair[1] - pair[0]).abs()))
}

/// A tone at −6 dBFS through `settings`, `param` set to `value` at 0.5 s: the largest step in
/// the 30 ms after the change is no more than 1.1 times the larger of the steady ones before and
/// after.
fn no_zipper(settings: &Settings, changes: &[(usize, Param, f32)], sidechain: bool) {
    let frames = ms(1500.0);
    let tone = sine(RATE, 1000.0, amplitude(-6.0), frames);
    let mut spec = Spec::new(RATE, 512, Layout::Mono);
    let mut run = Run::new(vec![tone]);
    if sidechain {
        spec = spec.with_sidechain(1);
        let key: Vec<f32> = sine(RATE, 1000.0, 0.9, frames + 7)[7..].to_vec();
        run = run.with_sidechain(vec![key]);
    }
    for &(at, param, value) in changes {
        run = run.with_change(at, param.id(), value);
    }
    let output = process(settings, &spec, &run).remove(0);
    let first = changes.iter().map(|c| c.0).min().unwrap();
    let last = changes.iter().map(|c| c.0).max().unwrap();
    let before = largest_step(&output[first - ms(100.0)..first]);
    let after = largest_step(&output[frames - ms(100.0)..]);
    let moving = largest_step(&output[first - 1..last + ms(30.0)]);
    assert!(
        moving <= 1.1 * before.max(after),
        "{changes:?}: {moving}, steady {before} and {after}"
    );
}

#[test]
fn changes_while_it_plays_make_no_zipper() {
    let half = ms(500.0);
    let compressor = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -30.0;
        s.compressor.ratio = 4.0;
    });
    no_zipper(&on(|_| {}), &[(half, Param::MakeupDb, 12.0)], false);
    no_zipper(&on(|_| {}), &[(half, Param::CompressorOn, 1.0)], false);
    no_zipper(&compressor, &[(half, Param::On, 0.0)], false);
    no_zipper(&compressor, &[(half, Param::CompressorRatio, 10.0)], false);
    no_zipper(&compressor, &[(half, Param::CompressorKneeDb, 20.0)], false);
    no_zipper(&compressor, &[(half, Param::CompressorMix, 100.0)], false);
    no_zipper(&compressor, &[(half, Param::LimiterOn, 1.0)], false);
    no_zipper(&compressor, &[(half, Param::SidechainOn, 1.0)], true);
    let mut listened = compressor;
    listened.compressor.sidechain = true;
    no_zipper(&listened, &[(half, Param::SidechainListen, 1.0)], true);
    let expander = on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -3.0;
        s.expander.ratio = 2.0;
        s.expander.range_db = 30.0;
    });
    no_zipper(&expander, &[(half, Param::ExpanderMode, 1.0)], false);
    no_zipper(&expander, &[(half, Param::ExpanderRangeDb, 3.0)], false);
    // A threshold swept down by 1 dB every 10 ms.
    let sweep: Vec<_> = (0..30)
        .map(|k| {
            (
                half + k * ms(10.0),
                Param::CompressorThresholdDb,
                -10.0 - k as f32,
            )
        })
        .collect();
    no_zipper(
        &on(|s| {
            s.compressor.on = true;
            s.compressor.threshold_db = -10.0;
            s.compressor.ratio = 4.0;
        }),
        &sweep,
        false,
    );
}

// --- Latency and settling --------------------------------------------------------------------

#[test]
fn it_reports_no_latency_and_has_none() {
    let spec = Spec::new(RATE, 512, Layout::Stereo);
    assert_eq!(assert_latency(|| Dynamics::new(&everything_on()), &spec), 0);
    let spec = spec.with_sidechain(1);
    assert_eq!(assert_latency(|| Dynamics::new(&keyed()), &spec), 0);
}

#[test]
fn started_in_the_middle_it_settles_as_it_reports() {
    let spec = Spec::new(RATE, 512, Layout::Stereo).with_sidechain(1);
    let settings = on(|s| {
        s.expander.on = true;
        s.expander.gate = true;
        s.expander.threshold_db = -30.0;
        s.expander.range_db = 40.0;
        s.expander.attack_ms = 1.0;
        s.expander.hold_ms = 20.0;
        s.expander.release_ms = 80.0;
        s.compressor.on = true;
        s.compressor.threshold_db = -25.0;
        s.compressor.ratio = 6.0;
        s.compressor.attack_ms = 5.0;
        s.compressor.release_ms = 60.0;
        s.limiter.on = true;
        s.limiter.threshold_db = -10.0;
        s.limiter.release_ms = 50.0;
    });
    let frames = ms(12000.0);
    let input: Vec<Vec<f32>> = (0..2)
        .map(|c| {
            noise(60 + c, 1.0, frames)
                .iter()
                .enumerate()
                .map(|(n, x)| {
                    if (n / ms(700.0)).is_multiple_of(2) {
                        x * 0.9
                    } else {
                        x * 0.003
                    }
                })
                .collect()
        })
        .collect();
    let key = noise(70, 0.5, frames);
    let run = Run::new(input)
        .with_sidechain(vec![key])
        .with_change(ms(300.0), Param::CompressorOn.id(), 0.0)
        .with_change(ms(5500.0), Param::CompressorOn.id(), 1.0)
        .with_change(ms(6000.0), Param::SidechainOn.id(), 1.0);
    let settle = {
        let mut node = Dynamics::new(&settings);
        node.prepare(&spec).unwrap();
        node.settle() as usize
    };
    assert!(settle > ms(2000.0) && settle < ms(6000.0), "{settle}");
    // Started loud and compressing, and started in a quiet part with the gate holding.
    for from in [ms(350.0), ms(1450.0), ms(2120.0)] {
        assert_settle(|| Dynamics::new(&settings), &spec, &run, from);
    }
    // Off, it settles at once but for the ramp.
    let mut off = Dynamics::default();
    off.prepare(&spec).unwrap();
    assert_eq!(off.settle() as usize, ms(10.0));
}

// --- Parameters, the window, the code --------------------------------------------------------

#[test]
fn the_parameters_have_their_ids_paths_ranges_and_defaults() {
    for (index, param) in Param::ALL.into_iter().enumerate() {
        assert_eq!(param.id().0 as usize, index);
        assert_eq!(Param::from_id(param.id()), Some(param));
    }
    assert_eq!(
        Param::from_id(columbina_audio::ParamId(PARAMS as u32)),
        None
    );
    let mut paths: Vec<_> = Param::ALL.iter().map(|p| p.path()).collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), PARAMS);
    let defaults = Settings::default();
    assert_eq!(defaults.value(Param::ExpanderThresholdDb), -35.0);
    assert_eq!(defaults.value(Param::ExpanderRatio), 1.1);
    assert_eq!(defaults.value(Param::CompressorThresholdDb), -15.0);
    assert_eq!(defaults.value(Param::CompressorAttackMs), 1.4);
    assert_eq!(defaults.value(Param::CompressorReleaseMs), 93.0);
    assert_eq!(defaults.value(Param::LimiterThresholdDb), -21.0);
    assert_eq!(defaults.value(Param::LimiterAttackMs), 0.71);
    assert_eq!(defaults.value(Param::LimiterReleaseMs), 90.0);
    assert!(Param::ALL
        .iter()
        .filter(|p| p.is_switch())
        .all(|&p| defaults.value(p) == 0.0));
    // Set and read back, clamped.
    let mut node = Dynamics::default();
    node.set(Param::CompressorRatio.id(), 40.0);
    node.set(Param::ExpanderReleaseMs.id(), 1.0);
    node.set(Param::MakeupDb.id(), -6.0);
    node.set(Param::LimiterOn.id(), 0.7);
    node.set(Param::CompressorThresholdDb.id(), f32::NAN);
    node.set(columbina_audio::ParamId(999), 1.0);
    let set = node.settings();
    assert_eq!(set.compressor.ratio, 10.0);
    assert_eq!(set.expander.release_ms, 50.0);
    assert!((set.makeup_db - -6.0).abs() < 1e-5);
    assert!(set.limiter.on);
    assert_eq!(set.compressor.threshold_db, -15.0);
    // The settings as parameters make the same node.
    let busy = busy();
    let mut node = Dynamics::default();
    for (param, value) in busy.params() {
        node.set(param, value);
    }
    let mut settings = node.settings();
    settings.makeup_db = busy.makeup_db;
    assert_eq!(settings, busy);
}

#[test]
fn it_takes_any_layout_with_the_same_out_and_no_empty_one() {
    let mut node = Dynamics::default();
    for layout in [
        Layout::Mono,
        Layout::Stereo,
        Layout::Surround51,
        Layout::Surround71,
        Layout::Adaptive(24),
    ] {
        assert!(node.prepare(&Spec::new(RATE, 64, layout)).is_ok());
        assert!(node
            .prepare(&Spec::new(RATE, 64, layout).with_sidechain(6))
            .is_ok());
    }
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Empty).with_output(Layout::Stereo))
        .is_err());
    assert!(node
        .prepare(&Spec::new(RATE, 64, Layout::Stereo).with_output(Layout::Mono))
        .is_err());
}

#[test]
fn the_ball_shows_the_level_and_the_reduction() {
    let settings = on(|s| s.compressor.on = true);
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let mut node = Dynamics::new(&settings);
    node.prepare(&spec).unwrap();
    let ball = node.ball();
    render(
        &mut node,
        &spec,
        &Run::new(vec![level(-5.0, ms(500.0))]),
        |_| 480,
    );
    let seen = ball.read();
    assert!((seen.input_db - -5.0).abs() < 1e-4, "{seen:?}");
    assert!((seen.compressor_db - -5.0).abs() < 1e-3, "{seen:?}");
    assert!((seen.output_db - -10.0).abs() < 1e-3, "{seen:?}");
    assert_eq!((seen.expander_db, seen.limiter_db), (0.0, 0.0));
}

#[test]
fn the_code_of_dynamics_does_not_lock_allocate_or_log_on_the_thread_of_the_mix() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fx/dynamics.rs");
    let source = std::fs::read_to_string(path).unwrap();
    assert_eq!(rt_violations(&source), []);
}

#[test]
fn a_makeup_gain_of_6_db_doubles_the_sound() {
    let settings = on(|s| s.makeup_db = 6.0);
    let output = mono(&settings, level(-20.0, 1000));
    let gain = f64::from(output[998]) / f64::from(amplitude(-20.0));
    assert!((gain - f64::from(db_to_gain(6.0))).abs() < 1e-6, "{gain}");
}

// --- Review (P1–P3) --------------------------------------------------------------------------

/// The largest difference, from `settle` samples after the late start on, between a node that
/// processed `run` from its start and one started at `from`.
fn difference_after(
    settings: &Settings,
    spec: &Spec,
    run: &Run,
    from: usize,
    settle: usize,
) -> f32 {
    let mut early = Dynamics::new(settings);
    early.prepare(spec).unwrap();
    early.align(0);
    let whole = render(&mut early, spec, run, |left| left);
    let mut late = Dynamics::new(settings);
    late.prepare(spec).unwrap();
    late.align(from as u64);
    let tail = render(&mut late, spec, &run.from(from), |left| left);
    let mut largest = 0.0_f32;
    for (whole, tail) in whole.iter().zip(&tail) {
        for (a, b) in whole[from..].iter().zip(tail).skip(settle) {
            largest = largest.max((a - b).abs());
        }
    }
    largest
}

/// A 1 kHz tone from 0 dBFS, falling `db_per_second`.
fn decaying(db_per_second: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            let t = n as f64 / f64::from(RATE);
            let a = 10_f64.powf(-db_per_second * t / 20.0);
            (a * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32
        })
        .collect()
}

#[test]
fn a_hold_keeps_no_phase_of_its_own_and_a_late_start_settles() {
    // The reviewer's case: a tone falling 4 dB/s, a hold of 500 ms.
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let run = Run::new(vec![decaying(4.0, ms(8000.0))]);
    let compressor = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -50.0;
        s.compressor.ratio = 4.0;
        s.compressor.attack_ms = 1.0;
        s.compressor.hold_ms = 500.0;
        s.compressor.release_ms = 50.0;
    });
    let expander = on(|s| {
        s.expander.on = true;
        s.expander.threshold_db = -10.0;
        s.expander.ratio = 2.0;
        s.expander.range_db = 40.0;
        s.expander.attack_ms = 1.0;
        s.expander.hold_ms = 500.0;
        s.expander.release_ms = 50.0;
    });
    for settings in [compressor, expander] {
        for from in [ms(1000.0), ms(1234.0), ms(2100.0)] {
            assert_settle(|| Dynamics::new(&settings), &spec, &run, from);
        }
    }
}

#[test]
fn listen_is_not_automated() {
    assert!(!Param::SidechainListen.is_automatable());
    assert!(Param::ALL
        .iter()
        .filter(|&&p| p != Param::SidechainListen)
        .all(|p| p.is_automatable()));
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let tone = sine(RATE, 1000.0, 0.5, 4800);
    let key = noise(3, 0.25, 4800);
    let run = Run::new(vec![tone]).with_sidechain(vec![key]);
    let plain = process(&keyed(), &spec, &run);
    let lane = run.with_automation(Param::SidechainListen.id(), vec![1.0; 4800]);
    assert_eq!(process(&keyed(), &spec, &lane), plain);
}

#[test]
fn settle_for_the_slowest_settings_covers_a_lane() {
    // The Compressor off as set, on by a lane: the graph asks for the settling of it on.
    let frames = ms(10000.0);
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let input: Vec<f32> = noise(9, 0.9, frames)
        .iter()
        .enumerate()
        .map(|(n, x)| {
            if (n / ms(700.0)).is_multiple_of(2) {
                *x
            } else {
                x * 0.01
            }
        })
        .collect();
    let set = on(|s| {
        s.compressor.threshold_db = -40.0;
        s.compressor.ratio = 8.0;
        s.compressor.release_ms = 300.0;
    });
    let run = Run::new(vec![input]).with_automation(Param::CompressorOn.id(), vec![1.0; frames]);
    let mut node = Dynamics::new(&set);
    node.prepare(&spec).unwrap();
    assert_eq!(node.settle(), Dynamics::settle_for(&set, RATE));
    assert_eq!(node.settle() as usize, ms(10.0));
    let mut slowest = set;
    slowest.compressor.on = true;
    let settle = Dynamics::settle_for(&slowest, RATE) as usize;
    assert!(settle > ms(5000.0), "{settle}");
    let floor = db_to_gain(-90.0);
    for from in [ms(300.0), ms(500.0)] {
        let largest = difference_after(&set, &spec, &run, from, settle);
        assert!(largest < floor, "{from}: {largest}");
    }
}

/// The largest change of the gain from one sample to the next, in the units of `unit`, through
/// a steady level in which `param` is set to `value` at 0.5 s; and the whole change.
fn largest_change(
    settings: &Settings,
    param: Param,
    value: f32,
    unit: fn(f64) -> f64,
) -> (f64, f64) {
    let input = level(-20.0, ms(1000.0));
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let run = Run::new(vec![input.clone()]).with_change(ms(500.0), param.id(), value);
    let output = process(settings, &spec, &run).remove(0);
    let gain: Vec<f64> = input
        .iter()
        .zip(&output)
        .map(|(x, y)| unit((f64::from(*y) / f64::from(*x)).abs()))
        .collect();
    let largest = gain[ms(500.0) - 1..]
        .windows(2)
        .fold(0.0_f64, |m, w| m.max((w[1] - w[0]).abs()));
    (largest, (gain[gain.len() - 1] - gain[ms(500.0) - 1]).abs())
}

#[test]
fn the_numbers_ramp_over_10_ms() {
    let linear = |g: f64| g;
    let db = |g: f64| 20.0 * g.log10();
    let ramp = ms(10.0) as f64;
    // Make Up: in the linear gain.
    let (step, total) = largest_change(&on(|_| {}), Param::MakeupDb, 12.0, linear);
    assert!(total > 2.9, "{total}");
    assert!(step <= total / ramp + 1e-6, "{step} of {total}");
    // Mix: after the detector, in the linear gain.
    let compressing = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -40.0;
        s.compressor.ratio = 10.0;
    });
    let (step, total) = largest_change(&compressing, Param::CompressorMix, 100.0, linear);
    assert!(total > 0.8, "{total}");
    assert!(step <= total / ramp + 1e-6, "{step} of {total}");
    // Range: a gate shut, opening from 18 to 3 dB below, no faster than the ramp.
    let gating = on(|s| {
        s.expander.on = true;
        s.expander.gate = true;
        s.expander.threshold_db = -10.0;
    });
    let (step, total) = largest_change(&gating, Param::ExpanderRangeDb, 3.0, db);
    assert!((total - 15.0).abs() < 1e-3, "{total}");
    assert!(step <= total / ramp + 1e-4, "{step} of {total}");
}

#[test]
fn a_lane_of_nan_keeps_the_value_set() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let settings = on(|s| {
        s.compressor.on = true;
        s.expander.on = true;
    });
    let tone = sine(RATE, 1000.0, 0.5, 4800);
    let run = Run::new(vec![tone]);
    let plain = process(&settings, &spec, &run);
    let lanes = run
        .with_automation(Param::CompressorThresholdDb.id(), vec![f32::NAN; 4800])
        .with_automation(Param::ExpanderOn.id(), vec![f32::NAN; 4800]);
    assert_eq!(process(&settings, &spec, &lanes), plain);
}

#[test]
fn one_infinite_sample_does_not_mute_the_track() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let settings = on(|s| s.compressor.on = true);
    let tone = sine(RATE, 1000.0, 0.5, ms(4000.0));
    let mut broken = tone.clone();
    broken[100] = f32::INFINITY;
    let clean = process(&settings, &spec, &Run::new(vec![tone]));
    let out = process(&settings, &spec, &Run::new(vec![broken]));
    let mut node = Dynamics::new(&settings);
    node.prepare(&spec).unwrap();
    let after = 101 + node.settle() as usize;
    let floor = db_to_gain(-90.0);
    for (a, b) in clean[0][after..].iter().zip(&out[0][after..]) {
        assert!((a - b).abs() < floor, "{a}, {b}");
    }
}

#[test]
fn a_block_without_its_sidechain_is_heard_as_one_with_none() {
    let settings = on(|s| {
        s.compressor.on = true;
        s.compressor.threshold_db = -30.0;
        s.compressor.ratio = 4.0;
        s.compressor.sidechain = true;
        s.compressor.listen = true;
    });
    let tone = sine(RATE, 1000.0, 0.5, 4800);
    let mono_spec = Spec::new(RATE, 4800, Layout::Mono);
    let unkeyed = process(&settings, &mono_spec, &Run::new(vec![tone.clone()]));
    let spec = mono_spec.with_sidechain(1);
    let mut node = Dynamics::new(&settings);
    node.prepare(&spec).unwrap();
    let mut input = Planar::new(1, 4800);
    input.channel_mut(0).copy_from_slice(&tone);
    let mut output = Planar::new(1, 4800);
    for k in 0..10 {
        let block = Block::new(k * 480, RATE, 480);
        let at = k as usize * 480;
        input.read_range(at, 480, |i| {
            output.write_range(at, 480, |o| node.process(&block, i, o));
        });
    }
    assert_eq!(output.channel(0), &unkeyed[0][..]);
    assert!(peak_db(&output.channel(0)[2400..]) < -10.0);
}

/// The deepest dip of the gain of a steady loud level when `param` is set to `value` at 0.5 s.
fn dip(settings: &Settings, param: Param, value: f32) -> f64 {
    let input = level(-6.0, ms(1000.0));
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let run = Run::new(vec![input.clone()]).with_change(ms(500.0), param.id(), value);
    let output = process(settings, &spec, &run).remove(0);
    gains(&input, &output)[ms(500.0)..]
        .iter()
        .fold(0.0_f64, |low, &g| low.min(g))
}

#[test]
fn a_module_turned_on_starts_from_the_gain_of_what_it_hears() {
    let open = |gate: bool| {
        on(move |s| {
            s.expander.gate = gate;
            s.expander.threshold_db = -40.0;
            s.expander.range_db = 40.0;
            s.expander.attack_ms = 100.0;
        })
    };
    // A loud sound, above the threshold: a gate turned on stays open.
    let low = dip(&open(true), Param::ExpanderOn, 1.0);
    assert!(low > -0.01, "{low}");
    // Expander to Gate: the gate starts where the expander was.
    let mut expanding = open(false);
    expanding.expander.on = true;
    let low = dip(&expanding, Param::ExpanderMode, 1.0);
    assert!(low > -0.01, "{low}");
}

#[test]
fn the_ball_starts_silent_and_shows_the_sidechain_listened_to() {
    let spec = Spec::new(RATE, 512, Layout::Mono).with_sidechain(1);
    let mut settings = keyed();
    settings.compressor.listen = true;
    let mut node = Dynamics::new(&settings);
    node.prepare(&spec).unwrap();
    let ball = node.ball();
    assert_eq!(ball.read().input_db, -200.0);
    assert_eq!(ball.read().output_db, -200.0);
    let run = Run::new(vec![level(-20.0, 4800)]).with_sidechain(vec![level(-6.0, 4800)]);
    render(&mut node, &spec, &run, |_| 480);
    let seen = ball.read();
    assert!((seen.input_db - -20.0).abs() < 1e-3, "{seen:?}");
    assert!((seen.output_db - -6.0).abs() < 1e-3, "{seen:?}");
}

/// The hold is kept in bins of [`DYNAMICS_HOLD_BIN`] laid on the timeline ([`Node::align`]): a
/// node started at a sample that is not a multiple of 32 after another lays its bins where the
/// other does. Unaligned, a gate shutting on a tone falling through its threshold shut up to 31
/// samples apart in the two, and they differed by up to −52 dBFS after the reported settling.
#[test]
fn a_gate_shutting_in_its_hold_settles_whatever_the_phase_of_the_bins() {
    let spec = Spec::new(RATE, 512, Layout::Mono);
    let run = Run::new(vec![decaying(4.0, ms(8000.0))]);
    let gate = on(|s| {
        s.expander.on = true;
        s.expander.gate = true;
        s.expander.threshold_db = -20.0;
        s.expander.range_db = 40.0;
        s.expander.attack_ms = 1.0;
        s.expander.hold_ms = 200.0;
        s.expander.release_ms = 50.0;
    });
    for from in [ms(1000.0) + 7, ms(1000.0) + 16, ms(1000.0) + 31] {
        assert_settle(|| Dynamics::new(&gate), &spec, &run, from);
    }
}
