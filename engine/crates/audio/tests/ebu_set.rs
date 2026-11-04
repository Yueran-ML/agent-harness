//! The EBU loudness test set itself (convention 6): it may not be passed on, so it never enters
//! the repository; a developer who has it puts it outside and names its folder in
//! `COLUMBINA_EBU_SET`. Without it, these tests pass without reading anything; they are not run
//! in CI. The cases are told by the names of the files (`…3341-<case>[-<file>]…`,
//! `…3342-<case>…`; Tech 3341 V4 and Tech 3342 V4 give what each must read).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "samples of a file"
)]

use std::path::{Path, PathBuf};

use columbina_audio::analysis::analyze;
use columbina_audio::loudness::{loudness_tap, Loudness};
use columbina_audio::Layout;

/// A WAV read: its rate and its channels, planar.
struct Wav {
    rate: u32,
    channels: Vec<Vec<f32>>,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads a WAV of PCM of 16, 24 or 32 bits or of 32-bit floats.
fn read_wav(path: &Path) -> Wav {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF", "{}", path.display());
    let mut at = 12;
    let (mut format, mut count, mut rate, mut bits) = (0, 0, 0, 0);
    let mut data = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(&bytes, at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " {
            format = u16_at(&bytes, body);
            count = usize::from(u16_at(&bytes, body + 2));
            rate = u32_at(&bytes, body + 4);
            bits = u16_at(&bytes, body + 14);
            if format == 0xFFFE {
                format = u16_at(&bytes, body + 24);
            }
        } else if id == b"data" {
            data = Some(&bytes[body..(body + size).min(bytes.len())]);
        }
        at = body + size + size % 2;
    }
    let data = data.expect("a data chunk");
    let width = usize::from(bits / 8);
    let frames = data.len() / (width * count);
    let mut channels = vec![Vec::with_capacity(frames); count];
    for frame in 0..frames {
        for (channel, samples) in channels.iter_mut().enumerate() {
            let at = (frame * count + channel) * width;
            let sample = match (format, bits) {
                (3, 32) => f32::from_le_bytes(data[at..at + 4].try_into().unwrap()),
                (1, 16) => f32::from(i16::from_le_bytes([data[at], data[at + 1]])) / 32_768.0,
                (1, 24) => {
                    let value = i32::from_le_bytes([0, data[at], data[at + 1], data[at + 2]]) >> 8;
                    value as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as f32
                        / 2_147_483_648.0
                }
                other => panic!("{}: format {other:?}", path.display()),
            };
            samples.push(sample);
        }
    }
    Wav { rate, channels }
}

/// The files of the set, if `COLUMBINA_EBU_SET` names a folder.
fn files() -> Vec<PathBuf> {
    let Ok(folder) = std::env::var("COLUMBINA_EBU_SET") else {
        eprintln!("COLUMBINA_EBU_SET is not set: the EBU set is not read");
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        })
        .collect();
    files.sort();
    files
}

/// The case of `document` (3341 or 3342) that the name of a file gives, and the number after it.
/// A year after the number of the document (`3341-2011-8_…`, a file of 2011) is passed over.
fn case(name: &str, document: &str) -> Option<(u32, Option<u32>)> {
    let mut rest = &name[name.find(&format!("{document}-"))? + document.len() + 1..];
    let number = |text: &str| -> Option<(u32, usize)> {
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        text[..digits].parse().ok().map(|value| (value, digits))
    };
    let (mut case, mut digits) = number(rest)?;
    if case >= 1_990 {
        rest = rest[digits..].strip_prefix('-')?;
        (case, digits) = number(rest)?;
    }
    // A second number right after a dash names the file of a case of many (10-1 … 10-20).
    let second = rest[digits..]
        .strip_prefix('-')
        .and_then(number)
        .map(|(value, _)| value);
    Some((case, second))
}

fn layout(channels: usize) -> Layout {
    match channels {
        1 => Layout::Mono,
        2 => Layout::Stereo,
        5 => Layout::Surround50,
        6 => Layout::Surround51,
        n => Layout::discrete(n),
    }
}

/// The momentary and the short-term loudness after each cell of 10 ms of `wav`.
fn cells(wav: &Wav) -> Vec<(f64, f64)> {
    let (mut tap, mut reader) = loudness_tap(wav.rate, layout(wav.channels.len()), false);
    let mut loudness = Loudness::new();
    let cell = tap.cell();
    let frames = wav.channels[0].len();
    let mut read = Vec::new();
    for at in (0..frames).step_by(cell) {
        let end = frames.min(at + cell);
        let slices: Vec<&[f32]> = wav.channels.iter().map(|c| &c[at..end]).collect();
        tap.take(&slices);
        reader.drain(|c| {
            loudness.push(c);
            let reading = loudness.reading();
            read.push((reading.momentary, reading.short_term));
        });
    }
    read
}

/// Tech 3341 cases 11 and 14 (meters that read live): segments of `seconds`, each a tone of
/// its own level after a silence that grows; the loudest S (case 11) or M (case 14) of each
/// segment is a whole level, one LU over the one before.
fn rising_levels(name: &str, wav: &Wav, seconds: f64, short_term: bool) {
    let read = cells(wav);
    let per = (seconds / 0.01).round() as usize;
    let maxima: Vec<f64> = read
        .chunks(per)
        .filter(|chunk| chunk.len() == per)
        .map(|chunk| {
            chunk
                .iter()
                .map(|&(m, s)| if short_term { s } else { m })
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();
    assert!(maxima.len() >= 2, "{name}: {} segments", maxima.len());
    for (index, max) in maxima.iter().enumerate() {
        assert!(
            (max - max.round()).abs() <= 0.1,
            "{name}: segment {index} reads {max}"
        );
        if index > 0 {
            let step = max - maxima[index - 1];
            assert!(
                (step - 1.0).abs() <= 0.1,
                "{name}: segment {index} rises {step}"
            );
        }
    }
}

/// Checks a file of the set; says whether it was one of its cases.
fn check(name: &str, wav: &Wav) -> bool {
    let tech3341 = case(name, "3341");
    let tech3342 = case(name, "3342");
    if tech3341.is_none() && tech3342.is_none() {
        return false;
    }
    let slices: Vec<&[f32]> = wav.channels.iter().map(Vec::as_slice).collect();
    let true_peak = matches!(tech3341, Some((15..=23, _)));
    let reading = analyze(wav.rate, layout(wav.channels.len()), true_peak, &slices);
    let near = |what: &str, value: f64, expected: f64| {
        assert!(
            (value - expected).abs() <= 0.1,
            "{name}: {what} {value}, expected {expected} ± 0.1"
        );
    };
    match tech3341 {
        Some((2, _)) => near("I", reading.integrated.unwrap(), -33.0),
        Some((1 | 3..=8, _)) => near("I", reading.integrated.unwrap(), -23.0),
        Some((9, _)) => {
            // S holds at −23 once 3 s are heard.
            for (index, &(_, s)) in cells(wav).iter().enumerate().skip(300) {
                near(&format!("S at cell {index}"), s, -23.0);
            }
        }
        Some((10, Some(_))) => near("max S", reading.short_term_max, -23.0),
        Some((11, _)) => rising_levels(name, wav, 6.0, true),
        Some((12, _)) => {
            // M holds at −23 once a second is heard.
            for (index, &(m, _)) in cells(wav).iter().enumerate().skip(100) {
                near(&format!("M at cell {index}"), m, -23.0);
            }
        }
        Some((13, Some(_))) => near("max M", reading.momentary_max, -23.0),
        Some((14, _)) => rising_levels(name, wav, 0.8, false),
        Some((15..=18, _)) => {
            let tp = reading.true_peak;
            assert!((-6.4..=-5.8).contains(&tp), "{name}: {tp} dBTP");
        }
        Some((19, _)) => {
            let tp = reading.true_peak;
            assert!((2.6..=3.2).contains(&tp), "{name}: {tp} dBTP");
        }
        Some((20..=23, _)) => {
            let tp = reading.true_peak;
            assert!((-0.4..=0.2).contains(&tp), "{name}: {tp} dBTP");
        }
        _ => {}
    }
    if let Some((case, _)) = tech3342 {
        let expected = match case {
            1 => Some(10.0),
            2 | 5 => Some(5.0),
            3 => Some(20.0),
            4 | 6 => Some(15.0),
            _ => None,
        };
        if let Some(expected) = expected {
            let range = reading.range.unwrap();
            assert!(
                (range - expected).abs() <= 1.0,
                "{name}: LRA {range}, expected {expected} ± 1"
            );
        }
    }
    eprintln!("{name}: {reading:?}");
    true
}

#[test]
fn the_ebu_set_reads_what_tech_3341_and_3342_require() {
    let mut checked = 0;
    for path in files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if check(&name, &read_wav(&path)) {
            checked += 1;
        }
    }
    eprintln!("{checked} files of the EBU set checked");
}

/// A stereo tone of 1 kHz at 48 kHz through `segments` of (seconds, dBFS or silence).
fn tone(segments: &[(f64, Option<f64>)]) -> Wav {
    let mut signal = Vec::new();
    for &(seconds, level) in segments {
        let frames = (seconds * 48_000.0).round() as usize;
        let amplitude = level.map_or(0.0, |db| 10.0_f64.powf(db / 20.0));
        let start = signal.len();
        signal.extend((start..start + frames).map(|n| {
            (amplitude * (std::f64::consts::TAU * 1_000.0 * n as f64 / 48_000.0).sin()) as f32
        }));
    }
    Wav {
        rate: 48_000,
        channels: vec![signal.clone(), signal],
    }
}

#[test]
fn the_checks_of_cases_9_11_12_and_14_hold_for_the_signals_of_the_tables() {
    // The signals as Tech 3341 V4 describes them, made here: the checks that the files of the
    // set will meet, met first by these.
    let mut case9 = Vec::new();
    let mut case12 = Vec::new();
    for _ in 0..5 {
        case9.extend([(1.34, Some(-20.0)), (1.66, Some(-30.0))]);
    }
    for _ in 0..25 {
        case12.extend([(0.18, Some(-20.0)), (0.22, Some(-30.0))]);
    }
    let mut case11 = Vec::new();
    let mut case14 = Vec::new();
    for i in 1..=20 {
        let i = f64::from(i);
        case11.extend([
            (i * 0.15, None),
            (3.0, Some(-38.0 + i)),
            (3.0 - i * 0.15, None),
        ]);
        case14.extend([
            (i * 0.02, None),
            (0.4, Some(-38.0 + i)),
            (0.4 - i * 0.02, None),
        ]);
    }
    assert!(check("seq-3341-9-24bit.wav", &tone(&case9)));
    assert!(check("seq-3341-11-24bit.wav", &tone(&case11)));
    assert!(check("seq-3341-12-24bit.wav", &tone(&case12)));
    assert!(check("seq-3341-14-24bit.wav", &tone(&case14)));
    assert!(check(
        "seq-3341-2011-8_seq-3342-6-24bit-v02.wav",
        // I −23 with the −38 under the relative gate; LRA 15 with it over the gate of the range.
        &tone(&[(20.0, Some(-38.0)), (20.0, Some(-23.0))])
    ));
}

#[test]
#[should_panic(expected = "rises")]
fn the_check_of_case_11_finds_a_level_out_of_step() {
    let mut segments = Vec::new();
    for i in 1..=4 {
        let level = if i == 3 { -33.0 } else { -38.0 + f64::from(i) };
        let i = f64::from(i);
        segments.extend([(i * 0.15, None), (3.0, Some(level)), (3.0 - i * 0.15, None)]);
    }
    check("seq-3341-11-24bit.wav", &tone(&segments));
}

#[test]
fn the_names_of_the_files_give_their_cases() {
    assert_eq!(case("seq-3341-1-16bit.wav", "3341"), Some((1, Some(16))));
    assert_eq!(case("seq-3341-10-7-24bit.wav", "3341"), Some((10, Some(7))));
    assert_eq!(
        case("seq-3341-7_seq-3342-5-24bit.wav", "3342"),
        Some((5, Some(24)))
    );
    assert_eq!(
        case("seq-3341-7_seq-3342-5-24bit.wav", "3341"),
        Some((7, None))
    );
    assert_eq!(
        case("seq-3341-2011-8_seq-3342-6-24bit-v02.wav", "3341"),
        Some((8, None))
    );
    assert_eq!(
        case("seq-3341-2011-8_seq-3342-6-24bit-v02.wav", "3342"),
        Some((6, Some(24)))
    );
    assert_eq!(case("1kHz Sine -20 LUFS-16bit.wav", "3341"), None);
}
