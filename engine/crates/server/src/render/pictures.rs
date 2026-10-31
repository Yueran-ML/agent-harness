//! Real media read and drawn as the engine draws them, end to end: each picture decoded by its
//! own range and matrix and its chroma kept at its own resolution, the same on the card and on
//! the processor, and a preview and an export of it agree.
//!
//! The samples are decoded by the readers of the timeline and of an export ([`Sources`]), put in
//! a scene by [`super::scene`] (the export's, whose layers the timeline monitor makes alike, with
//! [`super::layer_color`]), and composed on the card. What is drawn is held to R'G'B' worked out
//! here from the codes of the picture, by the matrix and range that each sample is known to be
//! of (not by what the picture says), at pixels about which the picture is of one colour, so that
//! how its chroma is blended does not matter. Without ffmpeg or a card the tests are skipped,
//! unless `COLUMBINA_REQUIRE_FFMPEG` and `COLUMBINA_REQUIRE_GPU` are 1.

use std::path::Path;
use std::sync::Arc;

use columbina_compose::{ColorRange, Compositor, Source, Target, YuvCoding, YuvMatrix};
use columbina_media::{Hardware, NativePicture, Picture, PictureFormat};
use columbina_model::project::{Clip, Timeline, Track, TrackKind};
use columbina_model::{Identity, Media, Project, Rate, Time};

use crate::play::sources::Sources;
use crate::test_media::{self, Sample};

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
        color_override: columbina_model::ColorOverride::default(),
    }
}

/// A project of one timeline of the size of the picture of `media`, of one clip of it from its
/// first frame.
fn project_of(media: Media) -> Project {
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
                id: "V1".into(),
                name: "Picture".into(),
                kind: TrackKind::Video,
                enabled: true,
                locked: false,
                language: String::new(),
                cues: Vec::new(),
                clips: vec![Clip::of_media("c1", "m1", Time { ticks: 0, scale }, 0, 5)],
                transitions: Vec::new(),
            }],
            grade: columbina_model::Grade::default(),
        }],
        groups: Vec::new(),
        color: columbina_model::ColorSettings::default(),
    }
}

/// The codes of Y', Cb and Cr at a pixel, of `bits` bits, as the card reads them: what is below
/// the top bits of a word of 16 a fraction of a code. Its chroma is that of the sample of chroma
/// it lies in.
fn codes_at(picture: &NativePicture, x: u32, y: u32) -> [f64; 3] {
    let (across, down) = picture.format.chroma_shift();
    let (x, y) = (x as usize, y as usize);
    let (cx, cy) = (x >> across, y >> down);
    let read = |plane: &[u8], row: usize, at: usize| -> f64 {
        if picture.format == PictureFormat::Nv12 {
            f64::from(plane[row + at])
        } else {
            let word = u16::from_le_bytes([plane[row + 2 * at], plane[row + 2 * at + 1]]);
            f64::from(word) / f64::from(1u32 << (16 - picture.bits))
        }
    };
    let chroma_row = cy * picture.chroma_stride;
    [
        read(&picture.luma, y * picture.luma_stride, x),
        read(&picture.chroma, chroma_row, 2 * cx),
        read(&picture.chroma, chroma_row, 2 * cx + 1),
    ]
}

/// R'G'B' from 0 to 1 of codes of `bits` bits of `coding`, worked out here.
fn rgb_of(codes: [f64; 3], coding: YuvCoding, bits: u8) -> [f64; 3] {
    let bits = i32::from(bits);
    let (black, luma_range, middle, chroma_range) = if coding.range == ColorRange::Full {
        let most = 2f64.powi(bits) - 1.0;
        (0.0, most, 2f64.powi(bits - 1), most)
    } else {
        let step = 2f64.powi(bits - 8);
        (16.0 * step, 219.0 * step, 128.0 * step, 224.0 * step)
    };
    let (kr, kb) = coding.matrix.coefficients();
    let kg = 1.0 - kr - kb;
    let y = (codes[0] - black) / luma_range;
    let cb = (codes[1] - middle) / chroma_range;
    let cr = (codes[2] - middle) / chroma_range;
    [
        y + 2.0 * (1.0 - kr) * cr,
        y - 2.0 * kb * (1.0 - kb) / kg * cb - 2.0 * kr * (1.0 - kr) / kg * cr,
        y + 2.0 * (1.0 - kb) * cb,
    ]
    .map(|value| value.clamp(0.0, 1.0))
}

/// Pixels about which, three pixels each way, every pixel has the codes of its own: of one
/// colour, its chroma of whichever samples about it alike.
fn flat_pixels(picture: &NativePicture) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    for y in (4..picture.height.saturating_sub(4)).step_by(5) {
        for x in (4..picture.width.saturating_sub(4)).step_by(5) {
            // The codes are exact fractions of words: compared bit for bit.
            let codes = |x, y| codes_at(picture, x, y).map(f64::to_bits);
            let here = codes(x, y);
            let flat = (y - 3..=y + 3).all(|v| (x - 3..=x + 3).all(|u| codes(u, v) == here));
            if flat {
                found.push((x, y));
            }
        }
    }
    found
}

/// The R'G'B' from 0 to 1 of a pixel of an export of P010 (BT.709, limited range).
fn exported(read: &Picture, x: u32, y: u32) -> [f64; 3] {
    let (x, y) = (x as usize, y as usize);
    let word =
        |plane: &[u8], at: usize| f64::from(u16::from_le_bytes([plane[at], plane[at + 1]]) >> 6);
    let chroma = (y / 2) * read.chroma_stride + (x / 2) * 4;
    rgb_of(
        [
            word(&read.luma, y * read.luma_stride + 2 * x),
            word(&read.chroma, chroma),
            word(&read.chroma, chroma + 2),
        ],
        YuvCoding {
            matrix: YuvMatrix::Bt709,
            range: ColorRange::Limited,
        },
        10,
    )
}

#[track_caller]
fn assert_near(pixel: &[u8], wanted: [f64; 3], most: f64, what: &str) {
    for channel in 0..3 {
        assert!(
            (f64::from(pixel[channel]) - wanted[channel] * 255.0).abs() <= most,
            "{what}: {:?}, not {:?}",
            &pixel[..3],
            wanted.map(|value| value * 255.0)
        );
    }
}

/// What a sample is known to be, as a reader gives it.
struct Known {
    format: PictureFormat,
    bits: u8,
    coding: YuvCoding,
}

/// Draw the first frame of the media at `path` as the engine draws it, on the card and on the
/// processor alike, and hold it to what it is known to be: its picture of its own sampling and
/// coding, a preview of the colours of its codes by that coding, and an export that agrees with
/// the preview. The R'G'B'A of the preview is given.
fn drawn_as_known(path: &Path, known: &Known) -> Vec<u8> {
    let gpu = columbina_compose::gpu_for_tests().expect("a card");
    let project = project_of(media_of(path));
    let timeline = &project.timelines[0];
    let (width, height) = (timeline.width, timeline.height);
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let mut previews: Vec<Vec<u8>> = Vec::new();
    for hardware in [Hardware::Off, Hardware::Auto] {
        let what = format!("{} decoded with {hardware:?}", path.display());
        let sources = Sources::new(hardware, Arc::new(|_, _| {}));
        let (scene, _) = super::scene(&project, timeline, 0, &sources).unwrap();
        let layer = &scene.layers[0];
        let Source::Picture(picture, _) = &layer.source else {
            panic!("{what}: not a picture");
        };
        assert_eq!(
            (picture.format, picture.bits),
            (known.format, known.bits),
            "{what}"
        );
        assert_eq!(layer.color.coding, known.coding, "{what}");
        let preview = Target::preview(&gpu, width, height).unwrap();
        compositor.render(&scene, &preview).unwrap();
        let shown = preview.read_rgba().unwrap();
        let export = Target::export(&gpu, width, height, 10).unwrap();
        compositor.render(&scene, &export).unwrap();
        let mut read = Picture::default();
        export.read_picture(&mut read).unwrap();
        // R'G'B'A is drawn as it is, at every pixel; Y'CbCr where its chroma does not matter.
        let flat = if picture.format.is_rgb() {
            (0..height)
                .step_by(3)
                .flat_map(|y| (0..width).step_by(3).map(move |x| (x, y)))
                .collect()
        } else {
            flat_pixels(picture)
        };
        assert!(
            flat.len() >= 8,
            "{what}: {} pixels of one colour about",
            flat.len()
        );
        for &(x, y) in &flat {
            let pixel = &shown[(y * width + x) as usize * 4..][..4];
            let wanted = if picture.format.is_rgb() {
                let words = picture.rgba(x, y);
                [0, 1, 2].map(|channel| f64::from(words[channel]) / 65535.0)
            } else {
                rgb_of(codes_at(picture, x, y), known.coding, known.bits)
            };
            let at = format!("{what}, at ({x}, {y})");
            // A half float rounded to 8 bits: within a step of the colour of the codes.
            assert_near(pixel, wanted, 1.0, &at);
            // The export's codes of 10 bits within a step and a half of the preview.
            let from_export = exported(&read, x, y);
            assert_near(pixel, from_export, 1.5, &format!("{at}, the export"));
        }
        previews.push(shown);
    }
    // The card keeps the codes of the file as the processor does.
    assert!(
        previews[0] == previews[1],
        "{}: drawn otherwise on the card",
        path.display()
    );
    previews.pop().unwrap()
}

fn coding(matrix: YuvMatrix, range: ColorRange) -> YuvCoding {
    YuvCoding { matrix, range }
}

#[test]
fn a_picture_of_full_range_is_drawn_by_its_full_range() {
    let Some(made) = test_media::media(Sample::Bt709Full) else {
        return;
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    drawn_as_known(
        &made.path,
        &Known {
            format: PictureFormat::Nv12,
            bits: 8,
            coding: coding(YuvMatrix::Bt709, ColorRange::Full),
        },
    );
    // Its limited twin, of the same pictures said to be of limited range, is drawn by that.
    let Some(made) = test_media::media(Sample::Bt709Limited) else {
        return;
    };
    drawn_as_known(
        &made.path,
        &Known {
            format: PictureFormat::Nv12,
            bits: 8,
            coding: coding(YuvMatrix::Bt709, ColorRange::Limited),
        },
    );
}

/// A file of SD (720×576) of H.264 that says nothing of its colours, made in `dir`: of BT.601,
/// as players take a picture of its size. None where there is no ffmpeg with libx264.
fn standard_definition(dir: &Path) -> Option<std::path::PathBuf> {
    let program = test_media::ffmpeg_program()?;
    let path = dir.join("sd-601.mp4");
    let made = std::process::Command::new(program)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=720x576:rate=25:duration=0.2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "10",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .status()
        .ok()?;
    assert!(
        made.success() || std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref() != Ok("1"),
        "ffmpeg could not make a file of SD"
    );
    made.success().then_some(path)
}

#[test]
fn a_picture_of_sd_that_says_nothing_is_drawn_by_bt_601() {
    let dir = tempfile::tempdir().unwrap();
    let Some(path) = standard_definition(dir.path()) else {
        return;
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let color = media_of(&path).facts.video.unwrap().color.unwrap();
    assert_eq!(color.matrix, 2, "it says no matrix");
    drawn_as_known(
        &path,
        &Known {
            format: PictureFormat::Nv12,
            bits: 8,
            coding: coding(YuvMatrix::Bt601, ColorRange::Limited),
        },
    );
}

#[test]
fn a_still_of_r_g_b_is_drawn_in_its_own_colour() {
    let Some(made) = test_media::media(Sample::RedPng) else {
        return;
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    // Kept R'G'B'A as it is read (T5b), its codes at full scale, of full range; not made
    // Y'CbCr, as it was until then.
    let shown = drawn_as_known(
        &made.path,
        &Known {
            format: PictureFormat::Rgba64,
            bits: 8,
            coding: coding(YuvMatrix::Bt709, ColorRange::Full),
        },
    );
    // Red exactly: 255, 0, 0. (It was R' 253 until T5b: its file, `v1-red.png`, was red made
    // Y'CbCr by the filter that made it and back, 253, and so was the still as it was drawn.)
    for pixel in shown.as_chunks::<4>().0 {
        assert_eq!(pixel, &[255, 0, 0, 255], "pure red");
    }
}

#[test]
fn a_picture_of_4_2_2_in_10_bits_keeps_its_chroma() {
    let Some(made) = test_media::media(Sample::H264High422) else {
        return;
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    // 640×360, saying no matrix: BT.601, as players take it.
    drawn_as_known(
        &made.path,
        &Known {
            format: PictureFormat::P216,
            bits: 10,
            coding: coding(YuvMatrix::Bt601, ColorRange::Limited),
        },
    );
}

#[test]
fn a_picture_of_4_4_4_in_12_bits_keeps_its_chroma() {
    let Some(made) = test_media::media(Sample::Hevc444) else {
        return;
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    drawn_as_known(
        &made.path,
        &Known {
            format: PictureFormat::P416,
            bits: 12,
            coding: coding(YuvMatrix::Bt601, ColorRange::Limited),
        },
    );
}
