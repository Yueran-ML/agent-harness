//! Offline analysis, Normalize Audio Levels and Level Matcher (plan T9; p. 3840–3841, p. 3804):
//! the gain of each clip, Independent and Relative, by loudness and by peak; the table of the
//! standards.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    reason = "samples and levels of tests"
)]

use std::f64::consts::TAU;

use columbina_audio::analysis::{
    analyze, level_match, normalize, normalize_clips, Analysis, Analyzer, Level, SetLevel,
};
use columbina_audio::constants::LOUDNESS_DEFAULT_TOLERANCE;
use columbina_audio::standards::{self, by_id, Measure, Verdict, STANDARDS};
use columbina_audio::testing::{noise, Rng};
use columbina_audio::Layout;

const RATE: u32 = 48_000;

fn gain(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

/// Pink-ish noise (white through a one-pole low pass) scaled to read `lufs`, `seconds` long, in
/// two channels.
fn program(seed: u64, lufs: f64, seconds: f64) -> Vec<Vec<f32>> {
    let frames = (seconds * f64::from(RATE)) as usize;
    let mut channels: Vec<Vec<f32>> = (0..2)
        .map(|channel| {
            let mut state = 0.0_f32;
            noise(seed + channel, 1.0, frames)
                .into_iter()
                .map(|x| {
                    state = 0.9 * state + 0.1 * x;
                    state
                })
                .collect()
        })
        .collect();
    let measured = analyze(RATE, Layout::Stereo, false, &slices(&channels))
        .integrated
        .unwrap();
    let scale = gain(lufs - measured);
    for channel in &mut channels {
        for x in channel.iter_mut() {
            *x *= scale;
        }
    }
    channels
}

fn slices(channels: &[Vec<f32>]) -> Vec<&[f32]> {
    channels.iter().map(Vec::as_slice).collect()
}

fn of(channels: &[Vec<f32>]) -> Analysis {
    analyze(RATE, Layout::Stereo, true, &slices(channels))
}

/// The clips heard one after another, as Relative hears them.
fn together(clips: &[&[Vec<f32>]]) -> Analysis {
    let mut analyzer = Analyzer::new(RATE, Layout::Stereo, true);
    for clip in clips {
        analyzer.take(&slices(clip));
    }
    analyzer.finish()
}

fn assert_near(what: &str, value: f64, expected: f64, within: f64) {
    assert!(
        (value - expected).abs() <= within,
        "{what}: {value}, expected {expected} ± {within}"
    );
}

#[test]
fn independent_brings_each_clip_to_the_target_and_relative_moves_all_by_one() {
    let a = program(1, -30.0, 10.0);
    let b = program(3, -20.0, 10.0);
    let each = [of(&a), of(&b)];
    let both = together(&[&a, &b]);
    let level = Level::of(by_id("ebu-r128").unwrap());
    let independent = normalize_clips(level, SetLevel::Independent, -23.0, &each, &both);
    assert_near("a", independent[0].unwrap(), 7.0, 0.1);
    assert_near("b", independent[1].unwrap(), -3.0, 0.1);
    let relative = normalize_clips(level, SetLevel::Relative, -23.0, &each, &both);
    assert_eq!(relative[0], relative[1]);
    let expected = -23.0 - both.integrated.unwrap();
    assert_near("relative", relative[0].unwrap(), expected, 1e-12);
    // The two together read nearer the louder: about −22.6 LUFS, so about −0.4 dB each.
    assert_near("together", both.integrated.unwrap(), -22.6, 0.3);
    // The gain applied reads the target.
    let mut louder = a.clone();
    let g = gain(independent[0].unwrap());
    for channel in &mut louder {
        for x in channel.iter_mut() {
            *x *= g;
        }
    }
    assert_near("after", of(&louder).integrated.unwrap(), -23.0, 0.05);
}

#[test]
fn the_peak_modes_bring_the_highest_sample_or_true_peak_to_the_level() {
    // fs/4 at 45°: the samples at 0.5 · sin 45°, the sound at 0.5.
    let tone: Vec<f32> = (0..48_000)
        .map(|n| (0.5 * (TAU * 0.25 * n as f64 + TAU / 8.0).sin()) as f32)
        .collect();
    let quiet: Vec<f32> = tone.iter().map(|x| x * 0.1).collect();
    let loud = vec![tone.clone(), tone];
    let quiet = vec![quiet.clone(), quiet];
    let each = [of(&loud), of(&quiet)];
    let both = together(&[&loud, &quiet]);
    let sample = normalize_clips(Level::SamplePeak, SetLevel::Independent, -9.0, &each, &both);
    assert_near("loud", sample[0].unwrap(), -9.0 + 9.03, 0.01);
    assert_near("quiet", sample[1].unwrap(), -9.0 + 29.03, 0.01);
    let relative = normalize_clips(Level::SamplePeak, SetLevel::Relative, -9.0, &each, &both);
    assert_near("relative", relative[1].unwrap(), -9.0 + 9.03, 0.01);
    let true_peak = normalize(Level::TruePeak, -1.0, &each[0]).unwrap();
    assert_near("true peak", true_peak, -1.0 + 6.02, 0.2);
    assert!(Level::TruePeak.needs_true_peak());
    assert!(!Level::SamplePeak.needs_true_peak());
}

#[test]
fn silence_and_short_clips_are_left_as_they_are() {
    let silence = vec![vec![0.0; 48_000]; 2];
    let analysis = of(&silence);
    for level in [
        Level::SamplePeak,
        Level::TruePeak,
        Level::Loudness(Measure::Gated),
        Level::Loudness(Measure::Ungated),
    ] {
        assert_eq!(normalize(level, -23.0, &analysis), None, "{level:?}");
    }
    let short = program(5, -20.0, 10.0)
        .into_iter()
        .map(|mut channel| {
            channel.truncate(9_600);
            channel
        })
        .collect::<Vec<_>>();
    let analysis = of(&short);
    assert_eq!(
        normalize(Level::Loudness(Measure::Gated), -23.0, &analysis),
        None
    );
    assert!(normalize(Level::SamplePeak, -9.0, &analysis).is_some());
}

#[test]
fn bs_1770_1_measures_without_the_gates() {
    // 10 s at −20 LUFS then 10 s of silence: gated −20, ungated 3 dB lower.
    let mut clip = program(7, -20.0, 10.0);
    for channel in &mut clip {
        channel.resize(20 * 48_000, 0.0);
    }
    let analysis = of(&clip);
    let gated = normalize(Level::of(by_id("bs1770-4").unwrap()), -23.0, &analysis).unwrap();
    let ungated = normalize(Level::of(by_id("bs1770-1").unwrap()), -23.0, &analysis).unwrap();
    assert_near("gated", gated, -3.0, 0.1);
    assert_near("ungated", ungated, 0.0, 0.15);
}

#[test]
fn level_matcher_adds_the_difference_of_loudness() {
    let reference = of(&program(11, -18.0, 5.0));
    let clip = of(&program(13, -27.0, 5.0));
    assert_near("match", level_match(&reference, &clip).unwrap(), 9.0, 0.1);
    // A clip shorter than a block falls back to the ungated mean of both.
    let short: Vec<Vec<f32>> = program(17, -30.0, 5.0)
        .into_iter()
        .map(|mut channel| {
            channel.truncate(14_400);
            channel
        })
        .collect();
    let short = of(&short);
    assert_eq!(short.integrated, None);
    let matched = level_match(&reference, &short).unwrap();
    assert_near(
        "short",
        matched,
        reference.mean.unwrap() - short.mean.unwrap(),
        1e-12,
    );
    assert_near("short, about", matched, 12.0, 0.6);
    let silence = of(&vec![vec![0.0; 48_000]; 2]);
    assert_eq!(level_match(&reference, &silence), None);
}

#[test]
fn an_analysis_is_the_same_however_the_render_is_cut() {
    let clip = program(19, -25.0, 4.0);
    let whole = of(&clip);
    let mut rng = Rng::new(23);
    let mut analyzer = Analyzer::new(RATE, Layout::Stereo, true);
    let mut at = 0;
    let frames = clip[0].len();
    while at < frames {
        let end = frames.min(at + rng.range(1, 40_000));
        analyzer.take(&[&clip[0][at..end], &clip[1][at..end]]);
        at = end;
    }
    let cut = analyzer.finish();
    assert_eq!(cut, whole);
    assert!(whole.true_peak >= whole.sample_peak);
}

#[test]
fn the_end_of_the_sound_counts_for_its_peaks() {
    // A last sample louder than all, in no complete cell, read by both peaks.
    let mut clip = vec![vec![0.01_f32; 48_000 + 100]; 2];
    clip[0][48_099] = 0.9;
    let analysis = of(&clip);
    assert_near(
        "sample peak",
        analysis.sample_peak,
        20.0 * 0.9_f64.log10(),
        1e-6,
    );
    assert!(analysis.true_peak >= analysis.sample_peak - 1e-6);
}

#[test]
fn the_table_holds_the_eleven_of_resolve_then_the_presets() {
    let resolve: Vec<&str> = STANDARDS
        .iter()
        .filter(|standard| standard.resolve)
        .map(|standard| standard.name)
        .collect();
    assert_eq!(
        resolve,
        [
            "BS.1770-1",
            "BS.1770-4",
            "ATSC A/85",
            "EBU R128",
            "OP-59",
            "TR-B32",
            "AGCOM 219",
            "NETFLIX",
            "YouTube",
            "Disney 2.0",
            "Disney 5.1",
        ]
    );
    assert!(STANDARDS[..11].iter().all(|standard| standard.resolve));
    let numbers = |id: &str| {
        let standard = by_id(id).unwrap();
        (standard.target, standard.tolerance, standard.true_peak)
    };
    assert_eq!(numbers("ebu-r128"), (Some(-23.0), Some(1.0), Some(-1.0)));
    assert_eq!(numbers("atsc-a85"), (Some(-24.0), Some(2.0), Some(-2.0)));
    assert_eq!(numbers("op-59"), (Some(-24.0), Some(1.0), Some(-2.0)));
    assert_eq!(numbers("tr-b32"), (Some(-24.0), Some(1.0), Some(-1.0)));
    assert_eq!(numbers("netflix"), (Some(-27.0), Some(2.0), Some(-2.0)));
    assert_eq!(
        numbers("apple-podcasts"),
        (Some(-16.0), Some(1.0), Some(-1.0))
    );
    assert_eq!(numbers("spotify"), (Some(-14.0), None, Some(-1.0)));
    assert_eq!(
        numbers("aes-streaming-dialogue"),
        (Some(-18.0), None, Some(-1.0))
    );
    assert_eq!(numbers("bs1770-4"), (None, None, None));
    assert_eq!(by_id("bs1770-1").unwrap().measure, Measure::Ungated);
    assert!(STANDARDS
        .iter()
        .filter(|standard| standard.id != "bs1770-1")
        .all(|standard| standard.measure == Measure::Gated));
    for unofficial in [
        "youtube",
        "agcom-219",
        "disney-2.0",
        "disney-5.1",
        "web-video",
    ] {
        assert!(!by_id(unofficial).unwrap().official, "{unofficial}");
    }
    assert!(!by_id("disney-5.1").unwrap().verified);
    assert!(by_id("netflix").unwrap().dialogue);
    assert_eq!(by_id(standards::DEFAULT).unwrap().name, "EBU R128");
    assert_eq!(by_id("nothing"), None);
    let mut ids: Vec<&str> = STANDARDS.iter().map(|standard| standard.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), STANDARDS.len());
}

#[test]
fn the_verdict_colours_by_the_tolerance_of_each_standard() {
    let r128 = by_id("ebu-r128").unwrap();
    assert_eq!(r128.verdict(-23.0, -20.0), Verdict::Within);
    assert_eq!(r128.verdict(-24.0, -20.0), Verdict::Within);
    assert_eq!(r128.verdict(-24.1, -20.0), Verdict::Low);
    assert_eq!(r128.verdict(-21.96, -20.0), Verdict::Within);
    assert_eq!(r128.verdict(-21.9, -20.0), Verdict::High);
    let atsc = by_id("atsc-a85").unwrap();
    assert_eq!(atsc.verdict(-22.0, -23.0), Verdict::Within);
    assert_eq!(atsc.verdict(-21.9, -23.0), Verdict::High);
    // BS.1770 takes the project's Target Loudness, and the default tolerance.
    let bs = by_id("bs1770-4").unwrap();
    assert_eq!(bs.target_or(-16.0), -16.0);
    let edge = -16.0 - f64::from(LOUDNESS_DEFAULT_TOLERANCE);
    assert_eq!(bs.verdict(edge, -16.0), Verdict::Within);
    assert_eq!(bs.verdict(edge - 0.1, -16.0), Verdict::Low);
}
