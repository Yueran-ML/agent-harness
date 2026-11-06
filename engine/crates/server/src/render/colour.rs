//! Colour management end to end: media of the spaces the author asked for first (the HLG and PQ
//! of phones, Apple Log, DJI D-Log) and of others (S-Log3, `LogC4`), written by the ffmpeg of the
//! command line from codes made by OCIO's Python bindings (`tests/colour/make.py`), read in as
//! media, and drawn through their input transforms into a timeline of `DaVinci` Wide Gamut /
//! Intermediate, as OCIO has them; a project of `DaVinci YRGB` left as phase 1 drew it; and an
//! export tagged as its output space says, its light of HDR measured. Without ffmpeg or a card
//! the tests are skipped, unless `COLUMBINA_REQUIRE_FFMPEG` and `COLUMBINA_REQUIRE_GPU` are 1.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use columbina_compose::{Compositor, Gpu, Target};
use columbina_model::color_management::{self as cm, MediaInput, Preset, Science};
use columbina_model::project::{Clip, Timeline, Track, TrackKind};
use columbina_model::{ColorOverride, ColorSettings, Identity, Media, Project, Rate, Time};
use serde_json::Value;

use super::*;
use crate::play::sources::Sources;

const REFERENCE: &str = include_str!("../../tests/colour/reference.json");

/// The ffmpeg of the command line, where there is one; else the test is skipped, unless it is
/// required.
fn ffmpeg() -> Option<&'static str> {
    let found = crate::test_media::ffmpeg_program();
    assert!(
        found.is_some() || std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref() != Ok("1"),
        "COLUMBINA_REQUIRE_FFMPEG is 1, and there is no ffmpeg"
    );
    found
}

/// A file as media of a project, as it is taken in.
fn media_of(path: &Path) -> Media {
    Media {
        id: "m1".into(),
        path: path.to_string_lossy().into_owned(),
        identity: Identity {
            size: 1,
            modified_us: 0,
            digest: "0".repeat(64),
        },
        facts: columbina_media::probe(path).unwrap(),
        proxy: None,
        name: "Sample".into(),
        bin: None,
        mark_in: None,
        mark_out: None,
        color_override: ColorOverride::default(),
        audio_config: None,
    }
}

/// A project of one timeline of the size of the picture of `media`, of one clip of it.
fn project_of(media: Media, color: ColorSettings) -> Project {
    let video = media.facts.video.clone().unwrap();
    let scale = media.facts.duration.scale;
    Project {
        schema: columbina_model::SCHEMA,
        next_id: 20,
        bins: Vec::new(),
        media: vec![media],
        timelines: vec![Timeline {
            id: "t1".into(),
            name: "Timeline".into(),
            width: video.width,
            height: video.height,
            rate: Rate { num: 25, den: 1 },
            sample_rate: 48000,
            tracks: vec![Track {
                color: None,
                id: "V1".into(),
                name: "Picture".into(),
                kind: TrackKind::Video,
                enabled: true,
                locked: false,
                language: String::new(),
                cues: Vec::new(),
                clips: vec![Clip::of_media("c1", "m1", Time { ticks: 0, scale }, 0, 5)],
                transitions: Vec::new(),
                audio: columbina_model::TrackAudio::default(),
            }],
            grade: columbina_model::Grade::default(),
            buses: columbina_model::audio::first_buses(),
            vcas: Vec::new(),
            track_groups: Vec::new(),
            audio: columbina_model::TimelineAudio::default(),
        }],
        groups: Vec::new(),
        color,
    }
}

/// The side of a patch, in pixels.
const PATCH: u32 = 8;

/// A picture of 4:4:4 of 10 bits, a patch of `PATCH` pixels square for each of `codes` (Y', Cb,
/// Cr), written by ffmpeg into a lossless FFV1 in Matroska tagged BT.2020 of limited range, with
/// the transfer `transfer`: five frames alike.
fn written(directory: &Path, name: &str, codes: &[[u16; 3]], transfer: &str) -> Option<PathBuf> {
    let program = ffmpeg()?;
    let width = PATCH * u32::try_from(codes.len()).unwrap();
    let mut planes: Vec<u8> = Vec::new();
    // Each plane in turn: Y', then Cb, then Cr.
    for plane in [0, 1, 2] {
        for _ in 0..PATCH {
            for x in 0..width {
                let pixel = codes[(x / PATCH) as usize];
                planes.extend_from_slice(&pixel[plane].to_le_bytes());
            }
        }
    }
    let raw = directory.join(format!("{name}.yuv"));
    std::fs::write(&raw, planes.repeat(5)).unwrap();
    let path = directory.join(format!("{name}.mkv"));
    let made = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
        .args(["-pix_fmt", "yuv444p10le", "-s", &format!("{width}x{PATCH}")])
        .args(["-r", "25", "-i"])
        .arg(&raw)
        // The tags as the frames' own (the options of the command line are not kept by FFV1).
        .arg("-vf")
        .arg(format!(
            "setparams=color_primaries=bt2020:color_trc={transfer}:colorspace=bt2020nc:range=tv"
        ))
        .args(["-c:v", "ffv1"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    Some(path)
}

/// The canvas of a preview, in floats: R, G, B of each pixel.
fn floats(target: &Target) -> Vec<[f32; 3]> {
    target
        .read_floats()
        .unwrap()
        .chunks(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// Custom settings of RCM: a timeline of `DaVinci` Wide Gamut / Intermediate, its output
/// bypassed (so that what is drawn is the timeline), no DRT.
fn into_davinci_wide_gamut() -> ColorSettings {
    ColorSettings {
        science: Science::YrgbManaged,
        preset: Preset::Custom,
        timeline: "DaVinci Wide Gamut/DaVinci Intermediate".into(),
        output: cm::BYPASS.into(),
        ..ColorSettings::default()
    }
}

/// Draw the first frame of `project` as the monitor and an export draw it (its scene, graded
/// and given its colour management by [`Grading`]), into a preview of half floats.
fn drawn(gpu: &Arc<Gpu>, project: &Project) -> Vec<[f32; 3]> {
    let project = Arc::new(project.clone());
    let timeline = &project.timelines[0];
    let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
    let (mut scene, clips) = scene(&project, timeline, 0, &sources).unwrap();
    let mut grading = Grading::new(LutPlaces::with_library(Path::new("."), vec![]));
    grading.update(gpu, &project, 0);
    grading.grade(&mut scene, &clips, 0);
    let target = Target::preview(gpu, timeline.width, timeline.height).unwrap();
    Compositor::new(Arc::clone(gpu))
        .render(&scene, &target)
        .unwrap();
    floats(&target)
}

#[test]
fn media_of_each_space_is_drawn_into_the_timeline_as_ocio_has_it() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    assert_eq!(
        reference["ocio"],
        columbina_compose::ocio::config().map(|_| "2.6.0").unwrap()
    );
    let directory = tempfile::tempdir().unwrap();
    for space in reference["spaces"].as_array().unwrap() {
        let name = space["name"].as_str().unwrap();
        let codes: Vec<[u16; 3]> = serde_json::from_value(space["codes"].clone()).unwrap();
        let expected: Vec<[f32; 3]> = serde_json::from_value(space["expected"].clone()).unwrap();
        let ideal: Vec<[f32; 3]> = serde_json::from_value(space["ideal"].clone()).unwrap();
        let file = name.replace(['/', ' ', '.'], "_");
        let Some(path) = written(
            directory.path(),
            &file,
            &codes,
            space["transfer"].as_str().unwrap(),
        ) else {
            return;
        };
        let mut media = media_of(&path);
        let settings = into_davinci_wide_gamut();
        // HLG and PQ by what the file says; a camera's log by hand, as no tag says it.
        let input = cm::media_input(&settings, &media).unwrap().0;
        if name.starts_with("Rec.2100") {
            assert_eq!(
                input,
                MediaInput::Space(name.into()),
                "{name}: by its metadata"
            );
        } else {
            media.color_override.input = Some(name.into());
        }
        let shown = drawn(&gpu, &project_of(media, settings));
        let width = PATCH as usize * codes.len();
        let (mut worst, mut worst_ideal) = (0.0f32, 0.0f32);
        for (patch, (wanted, ideal)) in expected.iter().zip(&ideal).enumerate() {
            // The middle of the patch, in its middle row.
            let got = shown[(PATCH as usize / 2) * width + patch * PATCH as usize + 4];
            for c in 0..3 {
                worst = worst.max((got[c] - wanted[c]).abs());
                worst_ideal = worst_ideal.max((got[c] - ideal[c]).abs());
            }
        }
        eprintln!(
            "{name}: from its codes {worst:.2e}, from the light they were made of {worst_ideal:.2e}"
        );
        // DaVinci Intermediate from 0 to 1 in half floats (a step of at most 4.9e-4), against
        // OCIO's processor: within 1.5e-3 of the codes of the file decoded; within 3e-3 of the
        // light they were made of, rounded to codes of 10 bits.
        assert!(worst < 1.5e-3, "{name}: {worst}");
        assert!(worst_ideal < 3e-3, "{name}: {worst_ideal}");
    }
    assert_eq!(gpu.uncaught_errors().0, 0);
}

#[test]
fn a_project_of_davinci_yrgb_is_drawn_as_phase_1_drew_it() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let codes: Vec<[u16; 3]> =
        serde_json::from_value(reference["spaces"][1]["codes"].clone()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let Some(path) = written(directory.path(), "pq", &codes, "smpte2084") else {
        return;
    };
    // Media of PQ, in a project that is not managed: shown as its codes are, as before.
    let project = Arc::new(project_of(media_of(&path), ColorSettings::default()));
    let mut grading = Grading::new(LutPlaces::with_library(Path::new("."), vec![]));
    grading.update(&gpu, &project, 0);
    let timeline = &project.timelines[0];
    let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
    let (mut scene, clips) = scene(&project, timeline, 0, &sources).unwrap();
    let before = format!(
        "{:?}",
        scene.layers.iter().map(|l| &l.color).collect::<Vec<_>>()
    );
    grading.grade(&mut scene, &clips, 0);
    // No input transform, no output transform, decoding held to 0..1: as it was.
    for layer in &scene.layers {
        assert!(layer.color.input.is_identity() && !layer.color.unclamped);
    }
    assert!(scene.output.is_identity());
    let after = format!(
        "{:?}",
        scene.layers.iter().map(|l| &l.color).collect::<Vec<_>>()
    );
    assert_eq!(before, after);
    assert_eq!(grading.output(), managed::Output::default());
}

/// What `ffprobe` says of the picture of a file, flat.
fn probed(path: &Path) -> String {
    let ffprobe = ffmpeg().unwrap().replace("ffmpeg", "ffprobe");
    let output = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args([
            "stream=codec_name,profile,pix_fmt,color_range,color_space,color_transfer,\
             color_primaries:stream_side_data",
            "-of",
            "flat",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Export `project` as `codec` to `name` in `directory`, and say what ffprobe says of it.
fn exported(directory: &Path, project: Project, codec: VideoCodec, name: &str) -> String {
    let request = Request {
        timeline: "t1".into(),
        path: directory.join(name),
        codec: Some(codec),
        quality: Some(VideoQuality::Medium),
        video: None,
        audio: None,
        range: None,
        subtitle_tracks: vec![],
        replace: true,
        revision: None,
    };
    let mut worker = super::tests::worker(directory);
    worker.plan = Plan::new(project, request, directory).unwrap();
    worker.run().unwrap();
    probed(&worker.plan.path)
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each output space in turn, then a refusal"
)]
fn an_export_says_its_output_space_and_the_light_of_its_hdr() {
    let Some(_gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let pq = &reference["spaces"][1];
    assert_eq!(pq["name"], "Rec.2100 ST2084");
    let directory = tempfile::tempdir().unwrap();
    let codes: Vec<[u16; 3]> = serde_json::from_value(pq["codes"].clone()).unwrap();
    let Some(path) = written(directory.path(), "pq", &codes, "smpte2084") else {
        return;
    };
    // The preset HDR Rec.2020: a timeline of PQ mastered for 1000 nits, put out as it is.
    let hdr = ColorSettings {
        science: Science::YrgbManaged,
        preset: Preset::HdrRec2020,
        output: "Rec.2100 ST2084".into(),
        ..ColorSettings::default()
    };
    let said = exported(
        directory.path(),
        project_of(media_of(&path), hdr.clone()),
        VideoCodec::Prores,
        "hdr.mov",
    );
    let has = |said: &str, line: &str| said.contains(line);
    for line in [
        "color_primaries=\"bt2020\"",
        "color_transfer=\"smpte2084\"",
        "color_space=\"bt2020nc\"",
        "color_range=\"tv\"",
        "Mastering display metadata",
        "max_luminance=\"10000000/10000\"",
        "Content light level metadata",
    ] {
        assert!(has(&said, line), "{line}: {said}");
    }
    // The light measured: the brightest patch (2.0 of linear light, 200 nits), left as it is by
    // the Input DRT of the preset (which maps only above 500 nits, half the 1000 of the
    // timeline), within a code of 10 bits of PQ (about 1%).
    let number = |key: &str| -> u32 {
        said.lines()
            .find_map(|line| line.split_once(&format!("{key}=")))
            .map(|(_, value)| value.trim().parse().unwrap())
            .unwrap()
    };
    let (cll, fall) = (number("max_content"), number("max_average"));
    eprintln!("MaxCLL {cll}, MaxFALL {fall}");
    assert!(
        (197..=203).contains(&cll) && fall > 0 && fall < cll,
        "{said}"
    );
    // HEVC of HDR is Main 10, where NVENC opens.
    if columbina_media::encoders_for(Codec::Hevc).contains(&columbina_media::EncoderPath::Nvenc) {
        let said = exported(
            directory.path(),
            project_of(media_of(&path), hdr.clone()),
            VideoCodec::Hevc,
            "hdr.mp4",
        );
        assert!(
            has(&said, "profile=\"Main 10\"")
                && has(&said, "color_transfer=\"smpte2084\"")
                && has(&said, "max_luminance=\"10000000/10000\""),
            "{said}"
        );
    }
    // HLG, out of the same timeline: tagged so, no mastering display.
    let hlg = ColorSettings {
        output: "Rec.2100 HLG".into(),
        ..hdr.clone()
    };
    let said = exported(
        directory.path(),
        project_of(media_of(&path), hlg),
        VideoCodec::Prores,
        "hlg.mov",
    );
    assert!(has(&said, "color_transfer=\"arib-std-b67\""), "{said}");
    assert!(!has(&said, "Mastering display"), "{said}");
    // SDR, as every export was tagged before.
    let said = exported(
        directory.path(),
        project_of(media_of(&path), ColorSettings::default()),
        VideoCodec::Prores,
        "sdr.mov",
    );
    assert!(
        has(&said, "color_primaries=\"bt709\"") && has(&said, "color_transfer=\"bt709\""),
        "{said}"
    );
    // H.264 cannot be HDR.
    let refused = Plan::new(
        project_of(media_of(&path), hdr),
        Request {
            timeline: "t1".into(),
            path: directory.path().join("hdr264.mp4"),
            codec: Some(VideoCodec::H264),
            quality: Some(VideoQuality::Medium),
            video: None,
            audio: None,
            range: None,
            subtitle_tracks: vec![],
            replace: true,
            revision: None,
        },
        directory.path(),
    )
    .err()
    .unwrap();
    assert_eq!(refused.data.unwrap()["reason"], "hdr_codec");
}
