//! The EBU loudness test set itself (convention 6): it may not be passed on, so it never enters
//! the repository; a developer who has it puts it outside and names its folder in
//! `COLUMBINA_EBU_SET`. Without it, these tests pass without reading anything; they are not run
//! in CI. The cases are told by the names of the files (`…3341-<case>[-<file>]…`,
//! `…3342-<case>…`; Tech 3341 V4 and Tech 3342 V4 give what each must read).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "samples of a file"
)]

use std::path::{Path, PathBuf};

use columbina_audio::analysis::analyze;
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
fn case(name: &str, document: &str) -> Option<(u32, Option<u32>)> {
    let rest = &name[name.find(&format!("{document}-"))? + document.len() + 1..];
    let mut numbers = rest
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().ok());
    let case = numbers.next()??;
    // A second number right after a dash names the file of a case of many (10-1 … 10-20).
    let second = rest
        .strip_prefix(&case.to_string())
        .and_then(|after| after.strip_prefix('-'))
        .and_then(|after| after.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok());
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

#[test]
fn the_ebu_set_reads_what_tech_3341_and_3342_require() {
    let mut checked = 0;
    for path in files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let tech3341 = case(&name, "3341");
        let tech3342 = case(&name, "3342");
        if tech3341.is_none() && tech3342.is_none() {
            continue;
        }
        let wav = read_wav(&path);
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
            Some((10, Some(_))) => near("max S", reading.short_term_max, -23.0),
            Some((13, Some(_))) => near("max M", reading.momentary_max, -23.0),
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
        checked += 1;
    }
    eprintln!("{checked} files of the EBU set checked");
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
    assert_eq!(case("1kHz Sine -20 LUFS-16bit.wav", "3341"), None);
}
