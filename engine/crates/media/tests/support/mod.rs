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
    /// As `MkvLateStart`, with PCM stereo whose levels change at 1 s: audio before and after
    /// the first picture is distinguishable when testing waveform and mix alignment.
    MkvOffsetAudio,
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
    /// MP4 of H.264 at 30 frames a second in a time base of 1/90000, 4 s (120 frames), a key
    /// frame every 30 and B-frames, whose frame `10k + 4` comes 0.55 of a frame late: it is
    /// numbered `10k + 5`, as the frame after it is, and no frame is numbered `10k + 4`.
    Collide,
    /// MP4 of H.264 cut from 1.3 s of a file of 6 s (25 frames a second, a key frame every 50,
    /// B-frames) without coding it again (`-ss 1.3 -c copy`): an edit list starts it after its
    /// first key frame, whose frames before 1.3 s are decoded and dropped.
    EditList,
    /// MP4 of H.264 at 30 frames a second in a time base of 1/600, as a phone writes it, with
    /// B-frames: 121 numbers of frames (4.033 s), and frame 60 dropped, so that no frame has the
    /// number 60 and the picture has 120 frames, one fewer than its duration makes room for.
    Dropped,
    /// Sound only: WAV, 16 bits in stereo at 48 kHz, 4 s, silent but for a beep of 10 ms at
    /// 1 kHz, half of full scale, starting at each whole second (0, 1, 2 and 3 s), at its
    /// peak from its first sample: where a beep is heard tells where the sound is.
    Clicks,
    /// 640×360 H.264, 8 bits 4:2:0, 25 frames a second, 1 s, of `testsrc2`, said to be BT.709
    /// (primaries, transfer and matrix 1-1-1) of limited range.
    Bt709Limited,
    /// As [`Sample::Bt709Limited`], said to be of full range (its frames are `yuvj420p`).
    Bt709Full,
    /// 640×360 HEVC Main 10, 1 s, said to be BT.2020 and PQ (9-16-9) of limited range, with
    /// a mastering display and light levels in SEI (in its frames, not its container):
    /// `G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)`, `MaxCLL` 1000 and
    /// `MaxFALL` 400.
    Pq,
    /// 640×360 HEVC Main 10, 1 s, said to be BT.2020 and HLG (9-18-9).
    Hlg,
    /// 640×360 H.264 High 4:2:2, 10 bits, 1 s, of `testsrc2` made in 4:2:2.
    H264High422,
    /// 640×360 `DNxHR` HQX, 10 bits 4:2:2, in `QuickTime`, 1 s.
    Dnxhr,
    /// 640×360 HEVC of 4:4:4 in 12 bits (Main 4:4:4 12), 1 s, of `testsrc2` made in 4:4:4.
    Hevc444,
    /// 640×360 `ProRes` 4444, 1 s, made of 10 bits 4:4:4 (decoded as 12 bits).
    Prores4444,
    /// A still PNG of 64×48 in R'G'B' of 8 bits, every pixel pure red.
    RedPng,
    /// A still PNG of 256×4 in R'G'B' of 8 bits, of every code: row 0 red (R' = x, G' = B' =
    /// 0, x from 0 to 255 across), row 1 green, row 2 blue, row 3 grey (R' = G' = B' = x); pure
    /// red, green, blue and white at x = 255. See [`ramp`].
    RampPng,
    /// As [`Sample::RampPng`], a TIFF.
    RampTiff,
    /// A still PNG of 256×2 of grey of 8 bits (`gray`): the code x across.
    GreyPng,
    /// A still PNG of 256×2 in R'G'B' of 16 bits (`rgb48be`): R' = 256·x + 255·y, G' = 65535 −
    /// 256·x, B' = 16·x.
    Rgb48Png,
    /// A still PNG of 256×2 in R'G'B'A of 8 bits: R' 200, G' 100, B' 50, alpha x (straight, as
    /// PNG has it).
    AlphaPng,
    /// A still PNG of a palette (`pal8`) of 64×48, of `testsrc2`, every entry opaque.
    PalettePng,
    /// A still `OpenEXR` of 64×48 of floats (`gbrpf32le`): `testsrc2` four times as bright
    /// (exposure +2), up to 4.0.
    HotExr,
    /// A still `OpenEXR` of 64×48 of floats with alpha (`gbrapf32le`): red, alpha about a half,
    /// premultiplied (as `OpenEXR` has it, and as libav makes it for the encoder).
    AlphaExr,
    /// 256×64 `ProRes` 4444 with alpha, 1 s at 25 frames a second, made of `yuva444p10le` and
    /// decoded as 12 bits: the colour 0x3080C0, its alpha `x · 1023 / 255` of 10 bits, from 0 at
    /// the left to 1023 at the right.
    ProresAlpha,
    /// 64×32 H.264, lossless 4:2:0 of 8 bits, one frame, its chroma said (in its VUI) to be
    /// sited as libav numbers it (`AVChromaLocation`, 1 left to 6 bottom): luma 128, Cb a
    /// checkerboard of its samples (192 where x + y of the sample is even, else 64), Cr
    /// stripes across (160 where x of the sample is even, else 96). See [`checkerboard`].
    Sited420(u8),
    /// As [`Sample::Sited420`], 4:2:2 in FFV1 in Matroska, said to be sited left (1) or center
    /// (2).
    Sited422(u8),
    /// MXF (`OP1a`) of `DNxHR` HQX, 10 bits 4:2:2, 640×360 at 25 frames a second, 1 s, of
    /// `testsrc2`, said by MXF (not by its stream, which has no word of it) to be BT.2020 and
    /// HLG (9-18-9) of limited range.
    Mxf,
}

/// The codes of R', G' and B' of [`Sample::RampPng`] (and [`Sample::RampTiff`]) at `(x, y)`.
pub fn ramp(x: u32, y: u32) -> [u8; 3] {
    let code = u8::try_from(x).unwrap();
    match y {
        0 => [code, 0, 0],
        1 => [0, code, 0],
        2 => [0, 0, code],
        _ => [code; 3],
    }
}

/// The codes of Cb and Cr of a sample of chroma at `(x, y)` of the plane of chroma of
/// [`Sample::Sited420`] and [`Sample::Sited422`].
pub fn checkerboard(x: u32, y: u32) -> [u8; 2] {
    [
        if (x + y).is_multiple_of(2) { 192 } else { 64 },
        if x.is_multiple_of(2) { 160 } else { 96 },
    ]
}

/// The name of a siting of chroma, as `ffmpeg` names it (`-chroma_sample_location`).
fn siting(location: u8) -> &'static str {
    match location {
        1 => "left",
        2 => "center",
        3 => "topleft",
        4 => "top",
        5 => "bottomleft",
        6 => "bottom",
        _ => panic!("no siting {location}"),
    }
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
            | Sample::MkvOffsetAudio
            | Sample::Vfr
            | Sample::SameTimes => Some(100),
            Sample::Uhd => Some(180),
            Sample::Collide | Sample::Dropped => Some(120),
            Sample::Prores | Sample::Webm => Some(50),
            Sample::Portrait => Some(60),
            Sample::Fhd10
            | Sample::Bt709Limited
            | Sample::Bt709Full
            | Sample::Pq
            | Sample::Hlg
            | Sample::H264High422
            | Sample::Dnxhr
            | Sample::Hevc444
            | Sample::Prores4444
            | Sample::ProresAlpha
            | Sample::Mxf => Some(25),
            Sample::RedPng
            | Sample::RampPng
            | Sample::RampTiff
            | Sample::GreyPng
            | Sample::Rgb48Png
            | Sample::AlphaPng
            | Sample::PalettePng
            | Sample::HotExr
            | Sample::AlphaExr
            | Sample::Sited420(_)
            | Sample::Sited422(_) => Some(1),
            Sample::Wav | Sample::Evc | Sample::Noise | Sample::EditList | Sample::Clicks => None,
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
            Sample::Uhd => "v2-uhd-hevc-main10.mp4",
            Sample::Prores => "v1-fhd-prores-hq.mov",
            Sample::Portrait => "v1-portrait-h264-rotated.mp4",
            Sample::Wav => "v1-sound.wav",
            Sample::Fhd10 => "v2-fhd-hevc-main10.mp4",
            Sample::TsH264 => "v1-ts-h264.ts",
            Sample::TsMpeg2 => "v1-ts-mpeg2.ts",
            Sample::PsMpeg2 => "v1-ps-mpeg2.mpg",
            Sample::M2ts => "v1-avchd-h264.m2ts",
            Sample::MkvLongSound => "v1-mkv-sound-longer.mkv",
            Sample::MkvLateStart => "v1-mkv-picture-from-1s.mkv",
            Sample::MkvOffsetAudio => "v1-mkv-waveform-from-1s.mkv",
            Sample::Vfr => "v1-vfr-jitter.mp4",
            Sample::SameTimes => "v1-same-timestamps.mkv",
            Sample::Evc => "v1-evc-aac.mp4",
            Sample::Webm => "v1-vp9-opus.webm",
            Sample::Noise => "v1-noise-aac.m4a",
            Sample::Collide => "v1-collide-h264.mp4",
            Sample::EditList => "v1-editlist-h264.mp4",
            Sample::Dropped => "v1-dropped-h264.mp4",
            Sample::Clicks => "v1-clicks.wav",
            Sample::Bt709Limited => "v1-bt709-limited-h264.mp4",
            Sample::Bt709Full => "v1-bt709-full-h264.mp4",
            Sample::Pq => "v1-pq-hevc-main10.mp4",
            Sample::Hlg => "v1-hlg-hevc-main10.mp4",
            Sample::H264High422 => "v1-h264-high422-10bit.mp4",
            Sample::Dnxhr => "v1-dnxhr-hqx.mov",
            Sample::Hevc444 => "v1-hevc-444-12bit.mp4",
            Sample::Prores4444 => "v1-prores-4444.mov",
            Sample::RedPng => "v2-red.png",
            Sample::RampPng => "v1-ramp.png",
            Sample::RampTiff => "v1-ramp.tiff",
            Sample::GreyPng => "v1-grey.png",
            Sample::Rgb48Png => "v1-rgb48.png",
            Sample::AlphaPng => "v1-alpha.png",
            Sample::PalettePng => "v1-palette.png",
            Sample::HotExr => "v1-hot.exr",
            Sample::AlphaExr => "v1-alpha.exr",
            Sample::ProresAlpha => "v1-prores-4444-alpha.mov",
            Sample::Sited420(location) => match location {
                1 => "v1-sited-420-left.mp4",
                2 => "v1-sited-420-center.mp4",
                3 => "v1-sited-420-topleft.mp4",
                4 => "v1-sited-420-top.mp4",
                5 => "v1-sited-420-bottomleft.mp4",
                6 => "v1-sited-420-bottom.mp4",
                _ => panic!("no siting {location}"),
            },
            Sample::Sited422(location) => match location {
                1 => "v1-sited-422-left.mkv",
                2 => "v1-sited-422-center.mkv",
                _ => panic!("no siting {location} of 4:2:2 here"),
            },
            Sample::Mxf => "v1-dnxhr-hqx-hlg.mxf",
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
    // Cargo defines this for integration tests, but not for the server's unit tests.
    let temporary = option_env!("CARGO_TARGET_TMPDIR")
        .unwrap_or(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/tmp"));
    Path::new(temporary)
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
///
/// x265 comes first, as it does for H.264: its groups of frames are the same wherever it runs,
/// while NVENC is on few machines and none of those of the CI. Its groups are closed
/// (`open-gop=0`), as those of NVENC are: by default x265 makes open ones, whose key frame is
/// decoded before the B-frames shown just before it, so that MP4 indexes it three frames
/// earlier than it is shown, and the tests that know where the key frames are would not hold.
fn hevc10(encoders: &str, gop: u32) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
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
                &format!(
                    "keyint={gop}:min-keyint={gop}:scenecut=0:open-gop=0:bframes=3:log-level=error"
                ),
            ]),
        ));
    }
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
    found
}

/// The ffmpeg that makes the media, if there is one: its program.
pub fn ffmpeg_program() -> Option<&'static str> {
    ffmpeg().map(|found| found.program.as_str())
}

/// The first frame of the picture of a file, decoded by the ffmpeg of the command line into
/// raw planes of `pixel_format` (as `yuv422p10le`): what a decoder of libav should give. None
/// where there is no ffmpeg.
///
/// # Panics
/// If ffmpeg cannot decode it.
pub fn first_frame_raw(path: &Path, pixel_format: &str) -> Option<Vec<u8>> {
    let program = ffmpeg_program()?;
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pixel_format,
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(output.stdout)
}

/// `testsrc2` of 640×360 at 25 frames a second for 1 s, made in `format`, its frames said to be
/// of the colours `params` (of the filter `setparams`), as `[v]`.
fn tagged(format: &str, params: &str) -> String {
    format!("testsrc2=size=640x360:rate=25:duration=1,format={format},setparams={params}[v]")
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

/// A still: the first frame of `graph` (whose picture is `[v]`) by `encoder`, in `format`.
fn still(encoders: &str, encoder: &str, graph: &str, format: &str) -> Vec<Recipe> {
    one_step(
        named(encoders, encoder, &["-pix_fmt", format, "-frames:v", "1"]),
        graph,
        false,
        &[],
    )
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
        Sample::Clicks => vec![Recipe {
            encoder: "pcm_s16le".into(),
            steps: vec![strings(&[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=exprs='if(lt(mod(t,1),0.01),0.5*cos(2*PI*1000*t),0)':s=48000:d=4",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                "{out}",
            ])],
        }],
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
        Sample::MkvOffsetAudio => one_step(
            h264(encoders, 25),
            &format!(
                "{};aevalsrc=exprs='if(lt(t,1),0.75,0.25)|if(lt(t,1),-0.25,-0.75)':s=48000:d=5[a]",
                small(",setpts=PTS+1/TB")
            ),
            true,
            &["-c:a", "pcm_f32le"],
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
        Sample::Collide => one_step(
            h264(encoders, 30),
            &numbered(
                640,
                360,
                30,
                4,
                8,
                "420",
                ",settb=1/90000,setpts='(N+0.55*eq(mod(N\\,10)\\,4))/(30*TB)'",
            ),
            false,
            &[
                "-fps_mode",
                "passthrough",
                "-enc_time_base:v",
                "1/90000",
                "-video_track_timescale",
                "90000",
            ],
        ),
        Sample::Dropped => one_step(
            h264(encoders, 30),
            &numbered(
                640,
                360,
                30,
                5,
                8,
                "420",
                ",trim=end_frame=121,select='not(eq(n\\,60))',settb=1/600",
            ),
            false,
            &[
                "-fps_mode",
                "passthrough",
                "-enc_time_base:v",
                "1/600",
                "-video_track_timescale",
                "600",
            ],
        ),
        Sample::EditList => h264(encoders, 50)
            .into_iter()
            .map(|(encoder, codec)| {
                let mut first = strings(&[
                    "-filter_complex",
                    &numbered(640, 360, 25, 6, 8, "420", ""),
                    "-map",
                    "[v]",
                ]);
                first.extend(codec);
                first.push("{tmp}".into());
                let second = strings(&["-ss", "1.3", "-i", "{tmp}", "-c", "copy", "{out}"]);
                Recipe {
                    encoder,
                    steps: vec![first, second],
                }
            })
            .collect(),
        Sample::Bt709Limited | Sample::Bt709Full => one_step(
            h264(encoders, 25),
            &tagged(
                "yuv420p",
                &format!(
                    "color_primaries=bt709:color_trc=bt709:colorspace=bt709:range={}",
                    if sample == Sample::Bt709Full {
                        "pc"
                    } else {
                        "tv"
                    }
                ),
            ),
            false,
            &[],
        ),
        Sample::Pq | Sample::Hlg => {
            let (transfer, extra) = if sample == Sample::Pq {
                (
                    "smpte2084",
                    "master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):\
                     max-cll=1000,400:",
                )
            } else {
                ("arib-std-b67", "")
            };
            one_step(
                named(
                    encoders,
                    "libx265",
                    &[
                        "-preset",
                        "ultrafast",
                        "-crf",
                        "20",
                        "-x265-params",
                        &format!("{extra}log-level=error"),
                    ],
                ),
                &tagged(
                    "yuv420p10le",
                    &format!(
                        "color_primaries=bt2020:color_trc={transfer}:colorspace=bt2020nc:range=tv"
                    ),
                ),
                false,
                &[],
            )
        }
        Sample::H264High422 => one_step(
            named(
                encoders,
                "libx264",
                &[
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "10",
                    "-profile:v",
                    "high422",
                ],
            ),
            &tagged("yuv422p10le", "range=tv"),
            false,
            &[],
        ),
        Sample::Dnxhr => one_step(
            named(encoders, "dnxhd", &["-profile:v", "dnxhr_hqx"]),
            &tagged("yuv422p10le", "range=tv"),
            false,
            &[],
        ),
        Sample::Hevc444 => one_step(
            named(
                encoders,
                "libx265",
                &[
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "10",
                    "-x265-params",
                    "log-level=error",
                ],
            ),
            &tagged("yuv444p12le", "range=tv"),
            false,
            &[],
        ),
        Sample::Prores4444 => one_step(
            named(encoders, "prores_ks", &["-profile:v", "4"]),
            &tagged("yuv444p10le", "range=tv"),
            false,
            &[],
        ),
        // Made R'G'B' by the filter itself: `color` makes Y'CbCr unless it is asked for R'G'B',
        // and red made Y'CbCr and back is 253 (as `v1-red.png`, made so, was).
        Sample::RedPng => still(
            encoders,
            "png",
            "color=c=red:size=64x48,format=rgb24[v]",
            "rgb24",
        ),
        Sample::RampPng | Sample::RampTiff => still(
            encoders,
            if sample == Sample::RampPng {
                "png"
            } else {
                "tiff"
            },
            "color=c=black:size=256x4,format=gbrp,geq=r='X*(eq(Y,0)+eq(Y,3))':\
             g='X*(eq(Y,1)+eq(Y,3))':b='X*(eq(Y,2)+eq(Y,3))'[v]",
            "rgb24",
        ),
        Sample::GreyPng => still(
            encoders,
            "png",
            "color=c=black:size=256x2,format=gray,geq=lum='X'[v]",
            "gray",
        ),
        Sample::Rgb48Png => still(
            encoders,
            "png",
            "color=c=black:size=256x2,format=gbrp16le,\
             geq=r='256*X+255*Y':g='65535-256*X':b='16*X'[v]",
            "rgb48be",
        ),
        Sample::AlphaPng => still(
            encoders,
            "png",
            "color=c=black:size=256x2,format=gbrap,geq=r=200:g=100:b=50:a='X'[v]",
            "rgba",
        ),
        Sample::PalettePng => still(
            encoders,
            "png",
            "testsrc2=size=64x48,format=pal8[v]",
            "pal8",
        ),
        Sample::HotExr => still(
            encoders,
            "exr",
            "testsrc2=size=64x48,format=gbrpf32le,exposure=exposure=2[v]",
            "gbrpf32le",
        ),
        Sample::AlphaExr => still(
            encoders,
            "exr",
            "color=c=red@0.5:size=64x48,format=rgba,format=gbrapf32le[v]",
            "gbrapf32le",
        ),
        Sample::ProresAlpha => one_step(
            named(
                encoders,
                "prores_ks",
                &["-profile:v", "4", "-alpha_bits", "16"],
            ),
            "color=c=0x3080C0:size=256x64:rate=25:duration=1,format=yuva444p10le,\
             geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='X*1023/255'[v]",
            false,
            &[],
        ),
        Sample::Sited420(location) => one_step(
            named(
                encoders,
                "libx264",
                &[
                    "-qp",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                    "-chroma_sample_location",
                    siting(location),
                    "-frames:v",
                    "1",
                ],
            ),
            "color=c=gray:size=64x32,format=yuv420p,\
             geq=lum='128':cb='if(mod(X+Y,2),64,192)':cr='if(mod(X,2),96,160)'[v]",
            false,
            &[],
        ),
        Sample::Sited422(location) => one_step(
            named(
                encoders,
                "ffv1",
                &[
                    "-pix_fmt",
                    "yuv422p",
                    "-chroma_sample_location",
                    siting(location),
                    "-frames:v",
                    "1",
                ],
            ),
            "color=c=gray:size=64x32,format=yuv422p,\
             geq=lum='128':cb='if(mod(X+Y,2),64,192)':cr='if(mod(X,2),96,160)'[v]",
            false,
            &[],
        ),
        Sample::Mxf => one_step(
            named(encoders, "dnxhd", &["-profile:v", "dnxhr_hqx"]),
            &tagged(
                "yuv422p10le",
                "color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc:range=tv",
            ),
            false,
            &["-f", "mxf"],
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
    number_in_blocks(luma, stride, bits, 64)
}

/// As [`number_in`], of a picture scaled so that its blocks are `size` pixels wide and high (32
/// in a picture of 4K made 1920 wide): the middle three quarters of each block are read.
pub fn number_in_blocks(luma: &[u8], stride: usize, bits: u8, size: usize) -> Option<i64> {
    let (from, to) = (size / 8, size - size / 8);
    let mut number = 0;
    for block in 0..3 {
        let mut sum = 0_u64;
        for y in from..to {
            let row = &luma[y * stride..];
            for x in (block * size + from)..(block * size + to) {
                sum += if bits > 8 {
                    u64::from(u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) >> 6)
                } else {
                    u64::from(row[x])
                };
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = sum as f64 / ((to - from) * (to - from)) as f64;
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
