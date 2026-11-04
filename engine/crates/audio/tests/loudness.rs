//! The loudness of BS.1770 (plan T9): the cases of EBU Tech 3341 and 3342, made here (the EBU set
//! may not be passed on, convention 6), each also measured by ffmpeg's `ebur128` (convention 5:
//! I, S, M and LRA within 0.1 LU, the true peak within 0.2 dB) when there is an ffmpeg, and by
//! the crate `ebur128` over noise; the tap the same however the sound is cut, and taking no
//! memory on the thread of the mix.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::type_complexity,
    reason = "samples and levels of tests"
)]

use std::f64::consts::TAU;

use columbina_audio::analysis::analyze;
use columbina_audio::loudness::{
    cell_frames, energy, loudness_tap, lufs, weight, Cell, KFilter, Loudness, OFFSET,
};
use columbina_audio::testing::{noise, realtime, rt_violations, CountingAllocator, Rng};
use columbina_audio::{Layout, Role};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const RATE: u32 = 48_000;

/// The gain of `db` dBFS.
fn gain(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// A tone of 1 kHz, its phase running on through `segments` of (seconds, dBFS of its peak);
/// a level of `None` is silence.
fn tone(frequency: f64, segments: &[(f64, Option<f64>)]) -> Vec<f32> {
    let mut signal = Vec::new();
    for &(seconds, level) in segments {
        let frames = (seconds * f64::from(RATE)).round() as usize;
        let amplitude = level.map_or(0.0, gain);
        let start = signal.len();
        signal
            .extend((start..start + frames).map(|n| {
                (amplitude * (TAU * frequency * n as f64 / f64::from(RATE)).sin()) as f32
            }));
    }
    signal
}

/// The same signal in two channels.
fn stereo(signal: &[f32]) -> Vec<Vec<f32>> {
    vec![signal.to_vec(), signal.to_vec()]
}

fn slices(channels: &[Vec<f32>]) -> Vec<&[f32]> {
    channels.iter().map(Vec::as_slice).collect()
}

/// Our reading of `channels` of `layout`.
fn ours(
    layout: Layout,
    channels: &[Vec<f32>],
    true_peak: bool,
) -> columbina_audio::analysis::Analysis {
    analyze(RATE, layout, true_peak, &slices(channels))
}

// --- ffmpeg ---------------------------------------------------------------------------------

/// What ffmpeg's `ebur128` reads: the summary, and the loudest M and S of its log.
#[derive(Debug)]
struct Ffmpeg {
    integrated: f64,
    range: f64,
    true_peak: Option<f64>,
    momentary_max: f64,
    short_term_max: f64,
}

/// The ffmpeg of the tests: `FFMPEG`, else the LGPL build of the toolchain (convention 5), else
/// the one on the path; `None` when none runs.
fn ffmpeg_program() -> Option<String> {
    let lgpl = r"F:\agent harness\_review\toolchain\ffmpeg-lgpl-8.1\bin\ffmpeg.exe";
    let program = std::env::var("FFMPEG").unwrap_or_else(|_| {
        if std::path::Path::new(lgpl).exists() {
            lgpl.to_owned()
        } else {
            "ffmpeg".to_owned()
        }
    });
    std::process::Command::new(&program)
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| program)
}

/// A WAV of 32-bit floats of `channels`.
fn wav(channels: &[Vec<f32>]) -> Vec<u8> {
    let frames = channels[0].len();
    let count = channels.len();
    let data = u32::try_from(frames * count * 4).unwrap();
    let mut wav = Vec::with_capacity(44 + data as usize);
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
    wav
}

/// The number after `key` in `line`.
fn after(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    rest.split_whitespace().next()?.parse().ok()
}

/// ffmpeg's reading of `channels`, with the true peak if `true_peak`; `None` without ffmpeg.
fn ffmpeg(channels: &[Vec<f32>], true_peak: bool, name: &str) -> Option<Ffmpeg> {
    let program = ffmpeg_program()?;
    let path = std::env::temp_dir().join(format!(
        "columbina-loudness-{name}-{}.wav",
        std::process::id()
    ));
    std::fs::write(&path, wav(channels)).unwrap();
    let filter = if true_peak {
        "ebur128=peak=true:framelog=verbose"
    } else {
        "ebur128=framelog=verbose"
    };
    let output = std::process::Command::new(program)
        .args(["-hide_banner", "-nostats", "-v", "verbose", "-i"])
        .arg(&path)
        .args(["-af", filter, "-f", "null", "-"])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&output.stderr);
    let summary = &text[text.rfind("Summary:").expect("a summary")..];
    let mut momentary_max = f64::NEG_INFINITY;
    let mut short_term_max = f64::NEG_INFINITY;
    for line in text.lines().filter(|line| line.contains(" t: ")) {
        momentary_max = momentary_max.max(after(line, "M:").unwrap());
        short_term_max = short_term_max.max(after(line, "S:").unwrap());
    }
    let read = |key: &str| {
        summary
            .lines()
            .find_map(|line| line.trim().strip_prefix(key).map(str::to_owned))
            .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
    };
    Some(Ffmpeg {
        integrated: read("I:").unwrap(),
        range: read("LRA:").unwrap(),
        true_peak: read("Peak:"),
        momentary_max,
        short_term_max,
    })
}

/// Checks `ours` against ffmpeg's reading of the same sound (convention 5), if there is an
/// ffmpeg: I, LRA, the loudest M and S within 0.1 LU, the true peak within 0.2 dB.
fn beside_ffmpeg(
    name: &str,
    channels: &[Vec<f32>],
    ours: &columbina_audio::analysis::Analysis,
    true_peak: bool,
) {
    let Some(theirs) = ffmpeg(channels, true_peak, name) else {
        eprintln!("{name}: no ffmpeg, not compared");
        return;
    };
    eprintln!("{name}: ffmpeg {theirs:?}");
    let close = |what: &str, a: f64, b: f64, within: f64| {
        assert!(
            (a - b).abs() <= within,
            "{name}: {what} ours {a}, ffmpeg {b}"
        );
    };
    close("I", ours.integrated.unwrap(), theirs.integrated, 0.1);
    close("LRA", ours.range.unwrap_or(0.0), theirs.range, 0.1);
    close("max M", ours.momentary_max, theirs.momentary_max, 0.1);
    close("max S", ours.short_term_max, theirs.short_term_max, 0.1);
    if true_peak {
        close("TP", ours.true_peak, theirs.true_peak.unwrap(), 0.2);
    }
}

fn assert_near(what: &str, value: f64, expected: f64, within: f64) {
    assert!(
        (value - expected).abs() <= within,
        "{what}: {value}, expected {expected} ± {within}"
    );
}

// --- The K filter and the weights -----------------------------------------------------------

#[test]
fn the_k_filter_at_48_khz_has_the_coefficients_of_bs_1770() {
    // BS.1770-4, annex 1, tables 1 and 2: the coefficients at 48 kHz, read from the standard,
    // not from the formula.
    let filter = KFilter::new(48_000);
    let shelf = (
        [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
        ],
        [1.0, -1.690_659_293_182_41, 0.732_480_774_215_85],
    );
    let high_pass = (
        [1.0, -2.0, 1.0],
        [1.0, -1.990_047_454_833_98, 0.990_072_250_366_21],
    );
    for (got, want) in [(filter.shelf, shelf), (filter.high_pass, high_pass)] {
        for (g, w) in got.0.iter().chain(&got.1).zip(want.0.iter().chain(&want.1)) {
            assert!((g - w).abs() < 1e-8, "{got:?} against {want:?}");
        }
    }
}

#[test]
fn the_k_filter_lifts_the_highs_by_four_db_and_cuts_the_lows() {
    let filter = KFilter::new(48_000);
    let db = |f: f64| 10.0 * filter.power_at(f, 48_000).log10();
    assert_near("10 kHz", db(10_000.0), 4.0, 0.1);
    assert!(db(20.0) < -10.0, "{}", db(20.0));
    // At 1 kHz the filter gives 0.691 dB, which the offset takes back.
    assert_near("1 kHz", db(1_000.0), -OFFSET, 0.02);
}

#[test]
fn the_lfe_is_left_out_and_the_surrounds_weigh_more() {
    assert_eq!(weight(Role::Lfe), 0.0);
    assert_eq!(weight(Role::LeftSurround), 1.41);
    assert_eq!(weight(Role::RightSurround), 1.41);
    for role in [
        Role::Left,
        Role::Right,
        Role::Centre,
        Role::Mono,
        Role::LeftRear,
        Role::Surround,
        Role::Discrete(3),
    ] {
        assert_eq!(weight(role), 1.0, "{role:?}");
    }
    assert_near("lufs of energy", lufs(energy(-23.0)), -23.0, 1e-12);
    assert_eq!(lufs(0.0), f64::NEG_INFINITY);
    assert_eq!(cell_frames(48_000), 480);
    assert_eq!(cell_frames(44_100), 441);
}

// --- Tech 3341 ------------------------------------------------------------------------------

#[test]
fn a_stereo_tone_at_minus_23_reads_minus_23_and_mono_minus_26() {
    // Tech 3341 case 1, and the plan's mono tone.
    let signal = tone(1_000.0, &[(20.0, Some(-23.0))]);
    let channels = stereo(&signal);
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -23.0, 0.1);
    assert_near("M", reading.momentary, -23.0, 0.1);
    assert_near("S", reading.short_term, -23.0, 0.1);
    assert_near("max M", reading.momentary_max, -23.0, 0.1);
    beside_ffmpeg("case1", &channels, &reading, false);

    let mono = vec![signal];
    let reading = ours(Layout::Mono, &mono, false);
    assert_near("mono I", reading.integrated.unwrap(), -26.0, 0.1);
    assert_near("mono M", reading.momentary, -26.0, 0.1);
    assert_near("mono S", reading.short_term, -26.0, 0.1);
    beside_ffmpeg("mono", &mono, &reading, false);
}

#[test]
fn tech_3341_case_2_reads_minus_33() {
    let channels = stereo(&tone(1_000.0, &[(20.0, Some(-33.0))]));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -33.0, 0.1);
    assert_near("M", reading.momentary, -33.0, 0.1);
    assert_near("S", reading.short_term, -33.0, 0.1);
    beside_ffmpeg("case2", &channels, &reading, false);
}

#[test]
fn tech_3341_case_3_the_relative_gate_leaves_the_quiet_out() {
    // 10 s at −36, 60 s at −23, 10 s at −36: ungated it would read −24.5.
    let channels = stereo(&tone(
        1_000.0,
        &[
            (10.0, Some(-36.0)),
            (60.0, Some(-23.0)),
            (10.0, Some(-36.0)),
        ],
    ));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -23.0, 0.1);
    assert!(reading.integrated_ungated.unwrap() < -23.5, "{reading:?}");
    beside_ffmpeg("case3", &channels, &reading, false);
}

#[test]
fn tech_3341_case_4_the_absolute_gate_leaves_the_silence_out() {
    let channels = stereo(&tone(
        1_000.0,
        &[
            (10.0, Some(-72.0)),
            (10.0, Some(-36.0)),
            (60.0, Some(-23.0)),
            (10.0, Some(-36.0)),
            (10.0, Some(-72.0)),
        ],
    ));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -23.0, 0.1);
    beside_ffmpeg("case4", &channels, &reading, false);
}

#[test]
fn the_absolute_gate_leaves_out_what_is_under_minus_70() {
    // 10 s at −60 and 60 s at −75: gated by −70 alone, −60.0; without it the −75 blocks would
    // pass the relative gate (−67.7 − 10) and pull it down to about −67.7.
    let channels = stereo(&tone(1_000.0, &[(10.0, Some(-60.0)), (60.0, Some(-75.0))]));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -60.0, 0.1);
    // The fall from −60 is in the low tail of the range: the 10th percentile by the nearest rank,
    // as ffmpeg reads it (3.8 LU; round((n − 1)·p) would give 3.5).
    assert_near("LRA", reading.range.unwrap(), 3.8, 0.1);
    beside_ffmpeg("gate", &channels, &reading, false);
}

#[test]
fn the_windows_are_the_means_of_their_cells() {
    // Cells of energy 1, 2, 3, …: each window is the exact mean of its last cells.
    let mut loudness = Loudness::new();
    for n in 1..=500 {
        loudness.push(Cell {
            energy: f64::from(n),
            true_peak: 0.0,
            sample_peak: 0.0,
        });
    }
    let reading = loudness.reading();
    let mean = |from: u32, to: u32| f64::from(from + to) / 2.0;
    assert_eq!(reading.momentary, lufs(mean(461, 500)));
    assert_eq!(reading.short_term, lufs(mean(201, 500)));
    assert_eq!(reading.momentary_max, reading.momentary);
    assert_eq!(reading.mean, Some(lufs(mean(1, 500))));
    // Blocks of 40 cells every 10, from the 40th: their power mean, ungated.
    let blocks: Vec<f64> = (4..=50).map(|k| mean(10 * k - 39, 10 * k)).collect();
    let ungated = blocks.iter().sum::<f64>() / blocks.len() as f64;
    assert!((reading.integrated_ungated.unwrap() - lufs(ungated)).abs() < 1e-12);
    // Before a window fills, the silence before the start counts.
    let mut early = Loudness::new();
    early.push(Cell {
        energy: 40.0,
        true_peak: 0.0,
        sample_peak: 0.0,
    });
    assert_eq!(early.reading().momentary, lufs(1.0));
    assert_eq!(early.reading().integrated, None);
}

#[test]
fn tech_3341_case_5_reads_minus_23() {
    let channels = stereo(&tone(
        1_000.0,
        &[
            (20.0, Some(-26.0)),
            (20.1, Some(-20.0)),
            (20.0, Some(-26.0)),
        ],
    ));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -23.0, 0.1);
    beside_ffmpeg("case5", &channels, &reading, false);
}

#[test]
fn tech_3341_case_6_weighs_the_surrounds_of_five_channels() {
    // L and R at −28, C at −24, Ls and Rs at −30 dBFS, 20 s.
    let channels: Vec<Vec<f32>> = [-28.0, -28.0, -24.0, -30.0, -30.0]
        .iter()
        .map(|&level| tone(1_000.0, &[(20.0, Some(level))]))
        .collect();
    let reading = ours(Layout::Surround50, &channels, false);
    assert_near("I", reading.integrated.unwrap(), -23.0, 0.1);
    beside_ffmpeg("case6", &channels, &reading, false);
    // The LFE of 5.1 is not heard: a loud LFE changes nothing.
    let mut with_lfe = channels.clone();
    with_lfe.insert(3, tone(1_000.0, &[(20.0, Some(0.0))]));
    let reading = ours(Layout::Surround51, &with_lfe, false);
    assert_near("I with LFE", reading.integrated.unwrap(), -23.0, 0.1);
}

#[test]
fn tech_3341_case_9_short_term_holds_still_after_three_seconds() {
    let mut segments = Vec::new();
    for _ in 0..5 {
        segments.push((1.34, Some(-20.0)));
        segments.push((1.66, Some(-30.0)));
    }
    let signal = tone(1_000.0, &segments);
    let mut tap = loudness_tap(RATE, Layout::Stereo, false);
    let mut loudness = Loudness::new();
    let cell = cell_frames(RATE);
    for (index, chunk) in signal.chunks(cell).enumerate() {
        tap.0.take(&[chunk, chunk]);
        tap.1.drain(|c| loudness.push(c));
        if index >= 300 {
            assert_near("S", loudness.reading().short_term, -23.0, 0.1);
        }
    }
}

#[test]
fn tech_3341_case_12_momentary_holds_still_after_a_second() {
    let mut segments = Vec::new();
    for _ in 0..25 {
        segments.push((0.18, Some(-20.0)));
        segments.push((0.22, Some(-30.0)));
    }
    let signal = tone(1_000.0, &segments);
    let (mut tap, mut reader) = loudness_tap(RATE, Layout::Stereo, false);
    let mut loudness = Loudness::new();
    for (index, chunk) in signal.chunks(cell_frames(RATE)).enumerate() {
        tap.take(&[chunk, chunk]);
        reader.drain(|c| loudness.push(c));
        if index >= 100 {
            assert_near("M", loudness.reading().momentary, -23.0, 0.1);
        }
    }
}

#[test]
fn tech_3341_cases_10_and_13_read_the_loudest_window_wherever_the_tone_starts() {
    for i in [0, 1, 7, 19] {
        // Case 10: i × 0.15 s of silence, 3 s at −23, 1 s of silence: Max S −23.
        let silence = f64::from(i) * 0.15;
        let channels = stereo(&tone(
            1_000.0,
            &[(silence, None), (3.0, Some(-23.0)), (1.0, None)],
        ));
        let reading = ours(Layout::Stereo, &channels, false);
        assert_near("max S", reading.short_term_max, -23.0, 0.1);
        // Case 13: i × 20 ms of silence, 400 ms at −23, 1 s of silence: Max M −23.
        let silence = f64::from(i) * 0.02;
        let channels = stereo(&tone(
            1_000.0,
            &[(silence, None), (0.4, Some(-23.0)), (1.0, None)],
        ));
        let reading = ours(Layout::Stereo, &channels, false);
        assert_near("max M", reading.momentary_max, -23.0, 0.1);
    }
}

#[test]
fn tech_3341_cases_11_and_14_read_each_level() {
    for i in [1, 10, 19] {
        let level = -38.0 + f64::from(i);
        let silence = f64::from(i) * 0.15;
        let channels = stereo(&tone(
            1_000.0,
            &[(silence, None), (3.0, Some(level)), (3.0 - silence, None)],
        ));
        assert_near(
            "max S",
            ours(Layout::Stereo, &channels, false).short_term_max,
            level,
            0.1,
        );
        let silence = f64::from(i) * 0.02;
        let channels = stereo(&tone(
            1_000.0,
            &[(silence, None), (0.4, Some(level)), (0.4 - silence, None)],
        ));
        assert_near(
            "max M",
            ours(Layout::Stereo, &channels, false).momentary_max,
            level,
            0.1,
        );
    }
}

// --- Tech 3342 ------------------------------------------------------------------------------

#[test]
fn tech_3342_cases_1_to_4_read_their_loudness_range() {
    let cases: [(&str, &[(f64, Option<f64>)], f64); 4] = [
        ("lra1", &[(20.0, Some(-20.0)), (20.0, Some(-30.0))], 10.0),
        ("lra2", &[(20.0, Some(-20.0)), (20.0, Some(-15.0))], 5.0),
        ("lra3", &[(20.0, Some(-40.0)), (20.0, Some(-20.0))], 20.0),
        (
            "lra4",
            &[
                (20.0, Some(-50.0)),
                (20.0, Some(-35.0)),
                (20.0, Some(-20.0)),
                (20.0, Some(-35.0)),
                (20.0, Some(-50.0)),
            ],
            15.0,
        ),
    ];
    for (name, segments, expected) in cases {
        let channels = stereo(&tone(1_000.0, segments));
        let reading = ours(Layout::Stereo, &channels, false);
        assert_near(name, reading.range.unwrap(), expected, 1.0);
        beside_ffmpeg(name, &channels, &reading, false);
    }
}

#[test]
fn a_steady_tone_has_no_range_and_silence_has_no_loudness() {
    let channels = stereo(&tone(1_000.0, &[(10.0, Some(-20.0))]));
    let reading = ours(Layout::Stereo, &channels, false);
    assert_near("LRA", reading.range.unwrap(), 0.0, 0.05);
    let silence = vec![vec![0.0; 480_000]; 2];
    let reading = ours(Layout::Stereo, &silence, true);
    assert_eq!(reading.integrated, None);
    assert_eq!(reading.range, None);
    assert_eq!(reading.momentary_max, f64::NEG_INFINITY);
    assert_eq!(reading.true_peak, f64::NEG_INFINITY);
    // Shorter than a block: no integrated loudness, but a mean.
    let short = stereo(&tone(1_000.0, &[(0.2, Some(-23.0))]));
    let reading = ours(Layout::Stereo, &short, false);
    assert_eq!(reading.integrated, None);
    assert_eq!(reading.integrated_ungated, None);
    assert_near("mean", reading.mean.unwrap(), -23.0, 0.1);
}

// --- The true peak (Tech 3341 cases 15–23) ----------------------------------------------------

/// A sine of `fraction` of the rate, `amplitude` of full scale, from `phase` degrees, faded in
/// and out over 10 ms (Tech 3341 cases 15–19), 1 s.
fn peak_tone(fraction: f64, amplitude: f64, phase: f64) -> Vec<f32> {
    let frames = RATE as usize;
    let fade = 480;
    (0..frames)
        .map(|n| {
            let edge = (n.min(frames - 1 - n) as f64 / fade as f64).min(1.0);
            (edge * amplitude * (TAU * fraction * n as f64 + phase.to_radians()).sin()) as f32
        })
        .collect()
}

#[test]
fn tech_3341_cases_15_to_19_read_the_true_peak() {
    let cases = [
        ("tp15", 0.25, 0.50, 0.0, -6.0),
        ("tp16", 0.25, 0.50, 45.0, -6.0),
        ("tp17", 1.0 / 6.0, 0.50, 60.0, -6.0),
        ("tp18", 0.125, 0.50, 67.5, -6.0),
        ("tp19", 0.25, 1.41, 45.0, 3.0),
    ];
    for (name, fraction, amplitude, phase, expected) in cases {
        let channels = stereo(&peak_tone(fraction, amplitude, phase));
        let reading = ours(Layout::Stereo, &channels, true);
        let tp = reading.true_peak;
        assert!(
            tp <= expected + 0.2 && tp >= expected - 0.4,
            "{name}: {tp} dBTP, expected {expected} +0.2/−0.4"
        );
        if let Some(theirs) = ffmpeg(&channels, true, name) {
            assert_near(name, tp, theirs.true_peak.unwrap(), 0.2);
        }
    }
    // The plan's case: fs/4 at 45°, 0.5: the samples peak at −9.03 dBFS, the sound at −6.02.
    let reading = ours(Layout::Stereo, &stereo(&peak_tone(0.25, 0.5, 45.0)), true);
    assert_near("sample peak", reading.sample_peak, -9.03, 0.01);
    assert_near("true peak", reading.true_peak, -6.02, 0.2);
}

/// Tech 3341 cases 20–23: a sine of fs/6 at 0.5 with one period of fs/4 at 1.0 in it, the phase
/// running on, made at four times the rate, filtered under the Nyquist frequency of the rate and
/// taken one sample in four from `offset`.
fn burst(offset: usize) -> Vec<f32> {
    let over = 4;
    let frames = RATE as usize / 2 * over;
    let mut phase = 0.0;
    let start = frames / 2;
    let burst_len = 4 * over; // one period of fs/4 at 4 fs
    let high: Vec<f64> = (0..frames)
        .map(|n| {
            let (fraction, amplitude) = if (start..start + burst_len).contains(&n) {
                (0.25 / over as f64, 1.0)
            } else {
                (1.0 / 6.0 / over as f64, 0.5)
            };
            let x = amplitude * (TAU * phase).sin();
            phase += fraction;
            x
        })
        .collect();
    // A windowed sinc of 0.45 of the rate, at 4 fs.
    let half = 128_isize;
    let cutoff = 0.45 / over as f64;
    let taps: Vec<f64> = (-half..=half)
        .map(|k| {
            let k = k as f64;
            let sinc = if k == 0.0 {
                2.0 * cutoff
            } else {
                (TAU * cutoff * k).sin() / (std::f64::consts::PI * k)
            };
            let window = 0.5 + 0.5 * (std::f64::consts::PI * k / (half as f64 + 1.0)).cos();
            sinc * window
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    (offset..frames)
        .step_by(over)
        .map(|n| {
            let mut y = 0.0;
            for (i, tap) in taps.iter().enumerate() {
                let m = n as isize + i as isize - half;
                if (0..frames as isize).contains(&m) {
                    y += tap / sum * high[m as usize];
                }
            }
            y as f32
        })
        .collect()
}

#[test]
fn tech_3341_cases_20_to_23_read_a_burst_between_the_samples() {
    for offset in 0..4 {
        let channels = stereo(&burst(offset));
        let reading = ours(Layout::Stereo, &channels, true);
        let tp = reading.true_peak;
        assert!((-0.4..=0.2).contains(&tp), "offset {offset}: {tp} dBTP");
        if let Some(theirs) = ffmpeg(&channels, true, &format!("tp2{offset}")) {
            assert_near("beside ffmpeg", tp, theirs.true_peak.unwrap(), 0.2);
        }
    }
}

// --- Beside the crate ebur128 -----------------------------------------------------------------

#[test]
fn noise_in_steps_reads_as_the_crate_ebur128_reads_it() {
    let mut rng = Rng::new(7);
    for case in 0..6 {
        let mut channels = vec![Vec::new(), Vec::new()];
        for _ in 0..rng.range(2, 6) {
            let seconds = 1.0 + rng.unit() * 6.0;
            let level = -60.0 + rng.unit() * 55.0;
            let frames = (seconds * f64::from(RATE)) as usize;
            for (index, channel) in channels.iter_mut().enumerate() {
                let seed = rng.next_u64() + index as u64;
                channel.extend(noise(seed, gain(level) as f32, frames));
            }
        }
        let reading = ours(Layout::Stereo, &channels, false);
        let mut theirs =
            ebur128::EbuR128::new(2, RATE, ebur128::Mode::I | ebur128::Mode::LRA).unwrap();
        theirs.add_frames_planar_f32(&slices(&channels)).unwrap();
        let integrated = theirs.loudness_global().unwrap();
        assert_near(
            &format!("case {case}: I"),
            reading.integrated.unwrap(),
            integrated,
            0.02,
        );
    }
}

// --- The tap ----------------------------------------------------------------------------------

/// The cells that a tap publishes of `channels` fed in blocks of `blocks` (cycled).
fn cells(channels: &[Vec<f32>], blocks: &[usize], true_peak: bool) -> Vec<Cell> {
    let (mut tap, mut reader) = loudness_tap(RATE, Layout::Stereo, true_peak);
    let mut cells = Vec::new();
    let frames = channels[0].len();
    let mut at = 0;
    let mut sizes = blocks.iter().cycle();
    while at < frames {
        let end = frames.min(at + sizes.next().unwrap());
        tap.take(&[&channels[0][at..end], &channels[1][at..end]]);
        let lost = reader.drain(|cell| cells.push(cell));
        assert_eq!(lost, 0);
        at = end;
    }
    cells
}

#[test]
fn the_tap_publishes_the_same_cells_however_the_sound_is_cut() {
    let channels = vec![noise(1, 0.8, 48_000), noise(2, 0.3, 48_000)];
    // Sample by sample, then in blocks of up to 4096 (the reader reads after each, as it must
    // within 32 cells).
    let whole = cells(&channels, &[1], true);
    assert_eq!(whole.len(), 100);
    let mut rng = Rng::new(3);
    for _ in 0..4 {
        let blocks: Vec<usize> = (0..50).map(|_| rng.range(1, 4_096)).collect();
        let cut = cells(&channels, &blocks, true);
        if let Some(n) = (0..whole.len()).find(|&n| cut.get(n) != whole.get(n)) {
            panic!(
                "cell {n} of {}: {:?} against {:?}",
                cut.len(),
                cut.get(n),
                whole[n]
            );
        }
    }
    assert!(whole
        .iter()
        .all(|cell| cell.true_peak >= cell.sample_peak * 0.99));
}

#[test]
fn a_reader_late_by_more_than_the_tap_keeps_says_what_it_lost() {
    let (mut tap, mut reader) = loudness_tap(RATE, Layout::Stereo, false);
    let block = vec![0.1_f32; 480 * 40];
    tap.take(&[&block, &block]);
    let mut read = 0;
    let lost = reader.drain(|_| read += 1);
    assert_eq!((read, lost), (32, 8));
    tap.take(&[&block[..480], &block[..480]]);
    assert_eq!(reader.drain(|_| read += 1), 0);
    assert_eq!(read, 33);
    // A reader made later reads from then on.
    let mut late = tap.reader();
    tap.take(&[&block[..960], &block[..960]]);
    let mut count = 0;
    assert_eq!(late.drain(|_| count += 1), 0);
    assert_eq!(count, 2);
}

#[test]
fn a_reset_clears_the_tap_and_the_meter() {
    let channels = stereo(&tone(1_000.0, &[(1.0, Some(-10.0))]));
    let (mut tap, mut reader) = loudness_tap(RATE, Layout::Stereo, true);
    let mut loudness = Loudness::new();
    for (left, right) in channels[0].chunks(4_800).zip(channels[1].chunks(4_800)) {
        tap.take(&[left, right]);
        reader.drain(|cell| loudness.push(cell));
    }
    assert!(loudness.reading().integrated.is_some());
    loudness.reset();
    let reading = loudness.reading();
    assert_eq!(reading.integrated, None);
    assert_eq!(reading.momentary_max, f64::NEG_INFINITY);
    assert_eq!(reading.true_peak, f64::NEG_INFINITY);
    assert_eq!(reading.seconds, 0.0);
    // After a reset of the tap the filters start from silence: the same as a new tap.
    tap.reset();
    let (mut fresh, mut fresh_reader) = loudness_tap(RATE, Layout::Stereo, true);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    tap.take(&slices(&channels));
    fresh.take(&slices(&channels));
    reader.drain(|cell| a.push(cell));
    fresh_reader.drain(|cell| b.push(cell));
    assert_eq!(a, b);
}

#[test]
fn the_tap_and_the_meter_take_no_memory() {
    let channels = [noise(1, 0.5, 4_800), noise(2, 0.5, 4_800)];
    let (mut tap, mut reader) = loudness_tap(RATE, Layout::Surround51, true);
    let mut loudness = Loudness::new();
    let silence = vec![0.0; 4_800];
    let ((), counted) = realtime(|| {
        for _ in 0..50 {
            tap.take(&[
                &channels[0],
                &channels[1],
                &silence,
                &silence,
                &channels[0],
                &channels[1],
            ]);
            tap.take(&[&channels[0][..17]]);
            reader.drain(|cell| loudness.push(cell));
            let _ = loudness.reading();
        }
        tap.reset();
        loudness.reset();
    });
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn the_code_of_the_taps_holds_no_word_of_the_threads_of_the_sound() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["loudness.rs", "meter.rs"] {
        let source = std::fs::read_to_string(src.join(file)).unwrap();
        assert_eq!(rt_violations(&source), [], "{file}");
    }
}

#[test]
fn rates_other_than_48_khz_read_the_same() {
    for rate in [44_100, 96_000] {
        let frames = rate as usize * 10;
        let signal: Vec<f32> = (0..frames)
            .map(|n| (gain(-23.0) * (TAU * 1_000.0 * n as f64 / f64::from(rate)).sin()) as f32)
            .collect();
        let reading = analyze(rate, Layout::Stereo, true, &[&signal, &signal]);
        assert_near(
            &format!("{rate}: I"),
            reading.integrated.unwrap(),
            -23.0,
            0.1,
        );
        assert_near(&format!("{rate}: TP"), reading.true_peak, -23.0, 0.05);
    }
}
