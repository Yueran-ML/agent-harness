//! The media of the tests, made by the `ffmpeg` of the command line and kept in
//! `target/test-media`, so that each is made once.
//!
//! That `ffmpeg` is any build, a full one of the GPL too: it only makes files, and nothing is
//! linked with it. It is `COLUMBINA_FFMPEG` if that is set, else the first `ffmpeg` on `PATH`
//! (put the directory of the DLLs of libav at the end of `PATH`, since the LGPL build that holds
//! them has an `ffmpeg` without the encoders of x264 and x265). Where there is none, or it
//! cannot make a file, a test that needs the file is skipped, unless `COLUMBINA_REQUIRE_FFMPEG`
//! is `1`: then it fails, as the tests of Python do. A few samples need an encoder that a full
//! build has and others may not ([`Sample::optional`]); they are skipped without one, always.
//!
//! Every frame of a picture carries its number in its top left corner: three blocks of 64×64
//! pixels, each a hexadecimal digit `d` of the number, the least significant leftmost, whose
//! luma is `16 + 14·d` in 8 bits and `64 + 56·d` in 10 bits. [`number_in`] reads it back.
//!
//! This file is a module of the tests of the server too.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// What is made, and how it is named; a new recipe of a file needs a new name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// 1080p H.264, 8 bits, 25 frames a second, 4 s (100 frames), a key frame every 50 frames
    /// and B-frames, with AAC sound in stereo at 48 kHz.
    Fhd,
    /// 2160p HEVC Main 10, 60 frames a second, 3 s (180 frames), a key frame every 120 frames
    /// and three B-frames between others.
    Uhd,
    /// 1080p ProRes 422 HQ, 10 bits 4:2:2, 25 frames a second, 2 s (50 frames).
    Prores,
    /// 640×360 H.264, 30 frames a second, 2 s (60 frames), to be shown turned a quarter turn
    /// counterclockwise (`-display_rotation 90`): as a portrait, 360×640.
    Portrait,
    /// Sound only: WAV, 16 bits in stereo at 48 kHz, 2 s.
    Wav,
    /// 1080p HEVC Main 10, 25 frames a second, 1 s (25 frames): the size of [`Sample::Fhd`] in
    /// 10 bits.
    Fhd10,
    /// MPEG-TS of H.264: 640×360, 25 frames a second, 4 s (100 frames), a key frame every 25
    /// and B-frames; its timestamps start at 1.4 s, as those of MPEG-TS do.
    TsH264,
    /// MPEG-TS of MPEG-2: as [`Sample::TsH264`], a key frame every 15.
    TsMpeg2,
    /// MPEG-PS (`.mpg`) of MPEG-2: as [`Sample::TsMpeg2`].
    PsMpeg2,
    /// M2TS, as AVCHD cameras write it, of H.264: as [`Sample::TsH264`].
    M2ts,
    /// Matroska: 4 s of picture (100 frames) and 6 s of sound; Matroska says how long the file
    /// lasts, and not how long each stream does.
    MkvLongSound,
    /// Matroska: 4 s of picture (100 frames) from 1 s on, and 5 s of sound from 0.
    MkvLateStart,
    /// MP4 of H.264 whose frames come at changing times: frame `n` at `(n ± 0.3) / 25` s,
    /// in a time base of milliseconds; 100 frames.
    Vfr,
    /// Matroska of H.264 whose every fourth frame has the timestamp of the one before it; 100
    /// frames.
    SameTimes,
    /// MP4 of EVC (MPEG-5), which the LGPL build of libav cannot decode, with AAC sound.
    Evc,
    /// `WebM` of VP9 for 2 s, and Opus for 3 s.
    Webm,
    /// Sound only: AAC in MP4 (`.m4a`), pink noise at 48 kHz in stereo, 3 s, coded without
    /// perceptual noise substitution; noise, so that a sample is told from the samples near it.
    Noise,
}

impl Sample {
    /// The samples that every test machine can make.
    pub const ALL: [Sample; 15] = [
        Sample::Fhd,
        Sample::Uhd,
        Sample::Prores,
        Sample::Portrait,
        Sample::Wav,
        Sample::Fhd10,
        Sample::TsH264,
        Sample::TsMpeg2,
        Sample::PsMpeg2,
        Sample::M2ts,
        Sample::MkvLongSound,
        Sample::MkvLateStart,
        Sample::Vfr,
        Sample::SameTimes,
        Sample::Noise,
    ];

    /// How many frames its picture has, if it has one that is decoded.
    pub fn frames(self) -> Option<i64> {
        match self {
            Sample::Fhd
            | Sample::TsH264
            | Sample::TsMpeg2
            | Sample::PsMpeg2
            | Sample::M2ts
            | Sample::MkvLongSound
            | Sample::MkvLateStart
            | Sample::Vfr
            | Sample::SameTimes => Some(100),
            Sample::Uhd => Some(180),
            Sample::Prores | Sample::Webm => Some(50),
            Sample::Portrait => Some(60),
            Sample::Fhd10 => Some(25),
            Sample::Wav | Sample::Evc | Sample::Noise => None,
        }
    }

    /// Whether it needs an encoder that a full build of ffmpeg may not have, so that it is
    /// skipped without one, even where ffmpeg is required.
    pub fn optional(self) -> bool {
        matches!(self, Sample::Evc | Sample::Webm)
    }

    fn file_name(self) -> &'static str {
        match self {
            Sample::Fhd => "v1-fhd-h264-aac.mp4",
            Sample::Uhd => "v1-uhd-hevc-main10.mp4",
            Sample::Prores => "v1-fhd-prores-hq.mov",
            Sample::Portrait => "v1-portrait-h264-rotated.mp4",
            Sample::Wav => "v1-sound.wav",
            Sample::Fhd10 => "v1-fhd-hevc-main10.mp4",
            Sample::TsH264 => "v1-ts-h264.ts",
            Sample::TsMpeg2 => "v1-ts-mpeg2.ts",
            Sample::PsMpeg2 => "v1-ps-mpeg2.mpg",
            Sample::M2ts => "v1-avchd-h264.m2ts",
            Sample::MkvLongSound => "v1-mkv-sound-longer.mkv",
            Sample::MkvLateStart => "v1-mkv-picture-from-1s.mkv",
            Sample::Vfr => "v1-vfr-jitter.mp4",
            Sample::SameTimes => "v1-same-timestamps.mkv",
            Sample::Evc => "v1-evc-aac.mp4",
            Sample::Webm => "v1-vp9-opus.webm",
            Sample::Noise => "v1-noise-aac.m4a",
        }
    }
}

/// A file that was made, and what made its picture.
#[derive(Clone, Debug)]
pub struct Made {
    pub path: PathBuf,
    /// The encoder of its picture, as ffmpeg names it, or `pcm_s16le`.
    pub encoder: String,
}

/// A sample, made if it was not made before. None if there is no ffmpeg that can make it and
/// it is not required: the test is then to be skipped.
///
/// # Panics
/// If ffmpeg is required (`COLUMBINA_REQUIRE_FFMPEG=1`) and a sample that is not optional
/// cannot be made.
pub fn media(sample: Sample) -> Option<Made> {
    static MAKING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = MAKING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = directory();
    let path = directory.join(sample.file_name());
    let note = directory.join(format!("{}.encoder", sample.file_name()));
    if path.is_file() {
        let encoder = std::fs::read_to_string(&note).unwrap_or_default();
        return Some(Made { path, encoder });
    }
    let Some(ffmpeg) = ffmpeg() else {
        return skip(sample, "there is no ffmpeg on PATH or in COLUMBINA_FFMPEG");
    };
    std::fs::create_dir_all(&directory).unwrap();
    let mut failures = Vec::new();
    for recipe in recipes(sample, &ffmpeg.encoders) {
        let part = directory.join(format!(
            "part-{}-{}",
            std::process::id(),
            sample.file_name()
        ));
        match recipe.make(&ffmpeg.program, &part, &directory) {
            Ok(()) => {
                std::fs::write(&note, &recipe.encoder).unwrap();
                std::fs::rename(&part, &path).unwrap();
                eprintln!("made {} with {}", path.display(), recipe.encoder);
                return Some(Made {
                    path,
                    encoder: recipe.encoder,
                });
            }
            Err(why) => {
                let _ = std::fs::remove_file(&part);
                failures.push(format!("{}: {why}", recipe.encoder));
            }
        }
    }
    skip(
        sample,
        &format!(
            "{} cannot be made: {}",
            sample.file_name(),
            if failures.is_empty() {
                "ffmpeg has none of its encoders".to_owned()
            } else {
                failures.join("; ")
            }
        ),
    )
}

/// Skip a test, or fail it where ffmpeg is required and the sample is not optional.
fn skip<T>(sample: Sample, why: &str) -> Option<T> {
    assert!(
        sample.optional() || std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref() != Ok("1"),
        "COLUMBINA_REQUIRE_FFMPEG is 1, and {why}"
    );
    eprintln!("skipped: {why}");
    None
}

/// `target/test-media`.
fn directory() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("the target directory")
        .join("test-media")
}

struct Ffmpeg {
    program: String,
    /// What `ffmpeg -encoders` says.
    encoders: String,
}

fn ffmpeg() -> Option<&'static Ffmpeg> {
    static FOUND: OnceLock<Option<Ffmpeg>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let program = std::env::var("COLUMBINA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
            let output = Command::new(&program)
                .args(["-hide_banner", "-encoders"])
                .output()
                .ok()?;
            output.status.success().then(|| Ffmpeg {
                program,
                encoders: String::from_utf8_lossy(&output.stdout).into_owned(),
            })
        })
        .as_ref()
}

/// One way to make a sample: arguments of ffmpeg for each step, the last of which writes the
/// file, as `{out}`, and those before it a file of their own in the same directory, as
/// `{tmp}`.
struct Recipe {
    encoder: String,
    steps: Vec<Vec<String>>,
}

impl Recipe {
    fn make(&self, program: &str, out: &Path, directory: &Path) -> Result<(), String> {
        let extension = out.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let tmp = directory.join(format!("tmp-{}.{extension}", std::process::id()));
        for step in &self.steps {
            let args: Vec<String> = step
                .iter()
                .map(|arg| {
                    arg.replace("{out}", out.to_str().unwrap())
                        .replace("{tmp}", tmp.to_str().unwrap())
                })
                .collect();
            let output = Command::new(program)
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(&args)
                .output()
                .map_err(|error| error.to_string())?;
            if !output.status.success() {
                let _ = std::fs::remove_file(&tmp);
                let text = String::from_utf8_lossy(&output.stderr);
                return Err(text.lines().last().unwrap_or("it failed").to_owned());
            }
        }
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }
}

/// The filter graph of a picture with its number in each frame, as `[v]`, with `then` (filters
/// that begin with a comma) after the number is drawn.
fn numbered(
    width: u32,
    height: u32,
    rate: u32,
    seconds: u32,
    bits: u8,
    chroma: &str,
    then: &str,
) -> String {
    let (base, step, middle, deep) = if bits > 8 {
        (64, 56, 512, true)
    } else {
        (16, 14, 128, false)
    };
    let format = format!("yuv{chroma}p{}", if deep { "10le" } else { "" });
    let overlay = format!("yuv{chroma}{}", if deep { "p10" } else { "" });
    format!(
        "testsrc2=size={width}x{height}:rate={rate}:duration={seconds},format={format}[bg];\
         color=black:size=192x64:rate={rate}:duration={seconds},format={format},\
         geq=lum='{base}+{step}*mod(floor(N/pow(16,floor(X/64))),16)':cb={middle}:cr={middle}[digits];\
         [bg][digits]overlay=0:0:format={overlay}{then}[v]"
    )
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&arg| arg.to_owned()).collect()
}

/// The encoders of H.264 that ffmpeg has, best first, each with its arguments.
fn h264(encoders: &str, gop: u32) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    if encoders.contains(" libx264 ") {
        found.push((
            "libx264".to_owned(),
            strings(&[
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "14",
                "-x264-params",
                &format!("keyint={gop}:min-keyint={gop}:scenecut=0:bframes=2"),
                "-pix_fmt",
                "yuv420p",
            ]),
        ));
    }
    if encoders.contains(" h264_nvenc ") {
        found.push((
            "h264_nvenc".to_owned(),
            strings(&[
                "-c:v",
                "h264_nvenc",
                "-rc",
                "constqp",
                "-qp",
                "18",
                "-g",
                &gop.to_string(),
                "-bf",
                "2",
                "-pix_fmt",
                "yuv420p",
            ]),
        ));
    }
    found
}

/// The encoders of HEVC Main 10 that ffmpeg has, best first, each with its arguments.
fn hevc10(encoders: &str, gop: u32) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    if encoders.contains(" hevc_nvenc ") {
        found.push((
            "hevc_nvenc".to_owned(),
            strings(&[
                "-c:v",
                "hevc_nvenc",
                "-profile:v",
                "main10",
                "-pix_fmt",
                "p010le",
                "-g",
                &gop.to_string(),
                "-bf",
                "3",
                "-rc",
                "constqp",
                "-qp",
                "24",
            ]),
        ));
    }
    if encoders.contains(" libx265 ") {
        found.push((
            "libx265".to_owned(),
            strings(&[
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                &format!("keyint={gop}:min-keyint={gop}:scenecut=0:bframes=3:log-level=error"),
            ]),
        ));
    }
    found
}

/// A recipe of one step for each encoder: `graph` (whose picture is `[v]`, and whose sound, if
/// it has any, `[a]`), the encoder's arguments, and `after` them.
fn one_step(
    encoders: Vec<(String, Vec<String>)>,
    graph: &str,
    sound: bool,
    after: &[&str],
) -> Vec<Recipe> {
    encoders
        .into_iter()
        .map(|(encoder, codec)| {
            let mut step = strings(&["-filter_complex", graph, "-map", "[v]"]);
            if sound {
                step.extend(strings(&["-map", "[a]"]));
            }
            step.extend(codec);
            step.extend(strings(after));
            step.push("{out}".into());
            Recipe {
                encoder,
                steps: vec![step],
            }
        })
        .collect()
}

/// The encoder of its own name, with its arguments, if ffmpeg has it.
fn named(encoders: &str, name: &str, args: &[&str]) -> Vec<(String, Vec<String>)> {
    if encoders.contains(&format!(" {name} ")) {
        let mut codec = strings(&["-c:v", name]);
        codec.extend(strings(args));
        vec![(name.to_owned(), codec)]
    } else {
        Vec::new()
    }
}

/// Stereo sound at 48 kHz for `seconds`, as `[a]`.
fn sound(seconds: u32) -> String {
    format!(
        "sine=frequency=440:sample_rate=48000:duration={seconds},aformat=channel_layouts=stereo[a]"
    )
}

#[allow(clippy::too_many_lines)]
fn recipes(sample: Sample, encoders: &str) -> Vec<Recipe> {
    let small = |then: &str| numbered(640, 360, 25, 4, 8, "420", then);
    let mpeg2 = || {
        named(
            encoders,
            "mpeg2video",
            &["-g", "15", "-bf", "2", "-q:v", "2"],
        )
    };
    match sample {
        Sample::Fhd => one_step(
            h264(encoders, 50),
            &format!("{};{}", numbered(1920, 1080, 25, 4, 8, "420", ""), sound(4)),
            true,
            &["-c:a", "aac", "-b:a", "128k"],
        ),
        Sample::Uhd => one_step(
            hevc10(encoders, 120),
            &numbered(3840, 2160, 60, 3, 10, "420", ""),
            false,
            &[],
        ),
        Sample::Fhd10 => one_step(
            hevc10(encoders, 25),
            &numbered(1920, 1080, 25, 1, 10, "420", ""),
            false,
            &[],
        ),
        Sample::Prores => one_step(
            named(
                encoders,
                "prores_ks",
                &["-profile:v", "3", "-pix_fmt", "yuv422p10le"],
            ),
            &numbered(1920, 1080, 25, 2, 10, "422", ""),
            false,
            &[],
        ),
        Sample::Portrait => h264(encoders, 30)
            .into_iter()
            .map(|(encoder, codec)| {
                let mut first = strings(&[
                    "-filter_complex",
                    &numbered(640, 360, 30, 2, 8, "420", ""),
                    "-map",
                    "[v]",
                ]);
                first.extend(codec);
                first.push("{tmp}".into());
                let second = strings(&[
                    "-display_rotation:v:0",
                    "90",
                    "-i",
                    "{tmp}",
                    "-c",
                    "copy",
                    "{out}",
                ]);
                Recipe {
                    encoder,
                    steps: vec![first, second],
                }
            })
            .collect(),
        Sample::Wav => vec![Recipe {
            encoder: "pcm_s16le".into(),
            steps: vec![strings(&[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                "{out}",
            ])],
        }],
        Sample::Noise => vec![Recipe {
            encoder: "aac".into(),
            steps: vec![strings(&[
                "-f",
                "lavfi",
                "-i",
                "anoisesrc=color=pink:sample_rate=48000:amplitude=0.3:duration=3:seed=11",
                "-ac",
                "2",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                // No noise made up by the decoder (PNS), which is not the same after a seek.
                "-aac_pns",
                "0",
                "{out}",
            ])],
        }],
        Sample::TsH264 => one_step(h264(encoders, 25), &small(""), false, &["-f", "mpegts"]),
        Sample::M2ts => one_step(
            h264(encoders, 25),
            &small(""),
            false,
            &["-f", "mpegts", "-mpegts_m2ts_mode", "1"],
        ),
        Sample::TsMpeg2 => one_step(mpeg2(), &small(""), false, &["-f", "mpegts"]),
        Sample::PsMpeg2 => one_step(mpeg2(), &small(""), false, &["-f", "vob"]),
        Sample::MkvLongSound => one_step(
            h264(encoders, 25),
            &format!("{};{}", small(""), sound(6)),
            true,
            &["-c:a", "aac"],
        ),
        Sample::MkvLateStart => one_step(
            h264(encoders, 25),
            &format!("{};{}", small(",setpts=PTS+1/TB"), sound(5)),
            true,
            &["-c:a", "aac"],
        ),
        Sample::Vfr => one_step(
            h264(encoders, 25),
            &small(",settb=1/1000,setpts='(N+0.3*(mod(N\\,3)-1)*gt(N\\,0))/(25*TB)'"),
            false,
            &[
                "-fps_mode",
                "passthrough",
                "-enc_time_base:v",
                "1/1000",
                "-video_track_timescale",
                "1000",
            ],
        ),
        Sample::SameTimes => one_step(
            h264(encoders, 25),
            &small(",setpts='(N-eq(mod(N\\,4)\\,3))/(25*TB)'"),
            false,
            &["-fps_mode", "passthrough"],
        ),
        Sample::Evc => one_step(
            named(encoders, "libxeve", &[]),
            &format!("{};{}", small(""), sound(4)),
            true,
            &["-c:a", "aac"],
        ),
        Sample::Webm => one_step(
            named(
                encoders,
                "libvpx-vp9",
                &[
                    "-deadline",
                    "realtime",
                    "-cpu-used",
                    "8",
                    "-crf",
                    "20",
                    "-b:v",
                    "0",
                ],
            ),
            &format!("{};{}", numbered(640, 360, 25, 2, 8, "420", ""), sound(3)),
            true,
            &["-c:a", "libopus"],
        ),
    }
}

/// The number that a frame carries, read from its plane of luma (`stride` bytes a row, 8 bits
/// a sample, or 16 with the value in the high 10 bits, as P010 has it): the mean of the middle
/// 48×48 pixels of each block is taken as its digit. None if a block is far from any digit.
pub fn number_in(luma: &[u8], stride: usize, bits: u8) -> Option<i64> {
    let mut number = 0;
    for block in 0..3 {
        let mut sum = 0_u64;
        for y in 8..56 {
            let row = &luma[y * stride..];
            for x in (block * 64 + 8)..(block * 64 + 56) {
                sum += if bits > 8 {
                    u64::from(u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) >> 6)
                } else {
                    u64::from(row[x])
                };
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = sum as f64 / (48.0 * 48.0);
        let (base, step) = if bits > 8 { (64.0, 56.0) } else { (16.0, 14.0) };
        let digit = (mean - base) / step;
        if !(-0.3..=15.3).contains(&digit) || (digit - digit.round()).abs() > 0.3 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let digit = digit.round() as i64;
        number += digit << (4 * block);
    }
    Some(number)
}
