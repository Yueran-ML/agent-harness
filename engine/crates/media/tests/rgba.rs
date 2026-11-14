//! Pictures of R'G'B' kept as R'G'B'A, never made Y'CbCr: stills of PNG and TIFF code for code
//! at full scale (pure red is 65535, not the 253 of 255 that Y'CbCr of 16 bits made of it), grey
//! and palettes, 16 bits as they are, `OpenEXR` as half floats beyond 1.0 too; and alpha, of
//! R'G'B'A and of `ProRes` 4444, taken as the file says.

mod support;

use std::path::Path;

use columbina_media::{
    probe, Alpha, ColorRange, Converter, Hardware, NativePicture, PictureFormat, VideoDecoder,
};
use support::{media, Sample};

/// The first frame of a sample as a native picture, and its path; None where the sample cannot
/// be made and is not required.
fn native(sample: Sample) -> Option<(std::path::PathBuf, NativePicture)> {
    let made = media(sample)?;
    let facts = probe(&made.path).unwrap();
    assert!(facts.video.is_some(), "{sample:?} has a picture");
    let mut opened = VideoDecoder::open(&made.path, Hardware::Off).unwrap();
    let frame = opened.next_decoded().unwrap().expect("a first frame");
    let picture = Converter::new().native(&frame).unwrap();
    Some((made.path, picture))
}

/// A code of 8 bits at full scale in 16.
fn full(code: u8) -> u16 {
    u16::from(code) * 257
}

/// The value of a half float (IEEE 754 binary16).
fn float_of_half(half: u16) -> f32 {
    let sign = if half & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((half >> 10) & 0x1f);
    let mantissa = f32::from(half & 0x03ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        0x1f => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// Floats of 32 bits, little-endian, as the `ffmpeg` of the command line gives a frame of
/// `gbrpf32le` or `gbrapf32le`: planes of G, B, R (and A).
fn floats(raw: &[u8]) -> Vec<f32> {
    raw.as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect()
}

/// Check that a still of the ramp ([`support::ramp`]) is R'G'B'A of its own codes, at full
/// scale, opaque.
fn assert_ramp(sample: Sample) {
    let Some((_, picture)) = native(sample) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.bits, picture.range, picture.alpha),
        (PictureFormat::Rgba64, 8, ColorRange::Full, Alpha::Opaque),
        "{sample:?}"
    );
    assert_eq!((picture.width, picture.height), (256, 4));
    assert!(picture.chroma.is_empty() && picture.alpha_plane.is_empty());
    for y in 0..4 {
        for x in 0..256 {
            let [red, green, blue] = support::ramp(x, y);
            assert_eq!(
                picture.rgba(x, y),
                [full(red), full(green), full(blue), 65535],
                "{sample:?} at ({x}, {y})"
            );
        }
    }
    // Pure red, green, blue and white, as 1.0 exactly.
    for (y, pure) in [[65535, 0, 0], [0, 65535, 0], [0, 0, 65535], [65535; 3]]
        .into_iter()
        .enumerate()
    {
        let y = u32::try_from(y).unwrap();
        assert_eq!(picture.rgba(255, y)[..3], pure, "{sample:?}");
    }
    assert_eq!(picture.alpha_at(255, 0), 1.0);
}

#[test]
fn a_png_of_r_g_b_is_kept_code_for_code() {
    assert_ramp(Sample::RampPng);
}

#[test]
fn a_tiff_of_r_g_b_is_kept_code_for_code() {
    assert_ramp(Sample::RampTiff);
    let Some(made) = media(Sample::RampTiff) else {
        return;
    };
    let video = probe(&made.path).unwrap().video.unwrap();
    assert!(video.still, "a TIFF is a still, as a PNG is");
}

#[test]
fn the_red_png_is_red_exactly() {
    let Some((_, picture)) = native(Sample::RedPng) else {
        return;
    };
    assert_eq!(picture.format, PictureFormat::Rgba64);
    assert_eq!(picture.rgba(10, 10), [65535, 0, 0, 65535]);
}

#[test]
fn grey_is_r_g_b_alike() {
    let Some((_, picture)) = native(Sample::GreyPng) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.bits, picture.alpha),
        (PictureFormat::Rgba64, 8, Alpha::Opaque)
    );
    for x in 0..256 {
        let code = full(u8::try_from(x).unwrap());
        assert_eq!(picture.rgba(x, 1), [code, code, code, 65535], "at {x}");
    }
}

#[test]
fn sixteen_bits_are_kept_as_they_are() {
    let Some((_, picture)) = native(Sample::Rgb48Png) else {
        return;
    };
    assert_eq!((picture.format, picture.bits), (PictureFormat::Rgba64, 16));
    for y in 0..2 {
        for x in 0..256 {
            let expected = [256 * x + 255 * y, 65535 - 256 * x, 16 * x, 65535]
                .map(|word| u16::try_from(word).unwrap());
            assert_eq!(picture.rgba(x, y), expected, "at ({x}, {y})");
        }
    }
}

#[test]
fn a_palette_is_looked_up() {
    let Some((path, picture)) = native(Sample::PalettePng) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.bits, picture.alpha),
        (PictureFormat::Rgba64, 8, Alpha::Opaque),
        "every entry of its palette is opaque"
    );
    // A palette is looked up by libav as it is by the converter: compared with the frame that
    // the `ffmpeg` of the command line gives in `rgba`.
    let raw = support::first_frame_raw(&path, "rgba").unwrap();
    let mut colours = std::collections::HashSet::new();
    for (at, pixel) in raw.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (
            u32::try_from(at % 64).unwrap(),
            u32::try_from(at / 64).unwrap(),
        );
        assert_eq!(picture.rgba(x, y), pixel.map(full), "at ({x}, {y})");
        colours.insert(*pixel);
    }
    assert!(colours.len() > 8, "a picture of {} colours", colours.len());
}

#[test]
fn straight_alpha_of_a_png_is_kept() {
    let Some((_, picture)) = native(Sample::AlphaPng) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.alpha),
        (PictureFormat::Rgba64, Alpha::Straight)
    );
    for x in 0..256 {
        let alpha = u8::try_from(x).unwrap();
        // Straight: the colour is the colour, however transparent.
        assert_eq!(
            picture.rgba(x, 0),
            [full(200), full(100), full(50), full(alpha)]
        );
        assert!((picture.alpha_at(x, 0) - f64::from(alpha) / 255.0).abs() < 1e-12);
    }
}

#[test]
fn open_exr_is_kept_in_half_floats_beyond_1() {
    let Some((path, picture)) = native(Sample::HotExr) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.bits, picture.range, picture.alpha),
        (PictureFormat::RgbaF16, 16, ColorRange::Full, Alpha::Opaque)
    );
    let raw = floats(&support::first_frame_raw(&path, "gbrpf32le").unwrap());
    let size = 64 * 48;
    assert_eq!(raw.len(), 3 * size);
    let mut brightest = 0.0_f32;
    for at in 0..size {
        let (x, y) = (
            u32::try_from(at % 64).unwrap(),
            u32::try_from(at / 64).unwrap(),
        );
        let [red, green, blue, alpha] = picture.rgba(x, y).map(float_of_half);
        // Planes of G, B and R; each half float within half of its last place.
        for (got, wanted) in [
            (red, raw[2 * size + at]),
            (green, raw[at]),
            (blue, raw[size + at]),
        ] {
            assert!(
                (got - wanted).abs() <= wanted.abs() / 2048.0 + 1e-7,
                "{got} for {wanted} at ({x}, {y})"
            );
            brightest = brightest.max(got);
        }
        assert_eq!(alpha, 1.0);
    }
    assert!(
        brightest > 3.9,
        "the brightest is {brightest}, not beyond 1"
    );
}

#[test]
fn alpha_of_open_exr_is_premultiplied() {
    let Some((path, picture)) = native(Sample::AlphaExr) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.alpha),
        (PictureFormat::RgbaF16, Alpha::Premultiplied)
    );
    let raw = floats(&support::first_frame_raw(&path, "gbrapf32le").unwrap());
    let size = 64 * 48;
    let got = picture.rgba(5, 5).map(float_of_half);
    let at = 5 * 64 + 5;
    let wanted = [
        raw[2 * size + at],
        raw[at],
        raw[size + at],
        raw[3 * size + at],
    ];
    for (got, wanted) in got.into_iter().zip(wanted) {
        assert!(
            (got - wanted).abs() <= wanted.abs() / 2048.0 + 1e-7,
            "{got} for {wanted}"
        );
    }
    // Red at alpha of about a half (as libav makes 128 of 8 bits a float), premultiplied by it:
    // R' as much as alpha.
    assert!((picture.alpha_at(5, 5) - 0.5).abs() < 0.01);
    assert!((got[0] - got[3]).abs() < 0.01, "{got:?}");
}

#[test]
fn alpha_of_prores_4444_is_a_plane_of_its_own_and_straight() {
    let Some((path, picture)) = native(Sample::ProresAlpha) else {
        return;
    };
    assert_eq!(
        (picture.format, picture.bits, picture.alpha),
        (PictureFormat::P416, 12, Alpha::Straight)
    );
    // Planes of Y', Cb, Cr and A of 12 bits, as the command line decodes them.
    let raw: Vec<u16> = support::first_frame_raw(&path, "yuva444p12le")
        .unwrap()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let size = 256 * 64;
    assert_eq!(raw.len(), 4 * size);
    assert_eq!(picture.alpha_plane.len(), picture.luma.len());
    for y in 0..64 {
        for x in 0..256 {
            let at = (y * 256 + x) as usize;
            assert_eq!(
                picture.ycc(x, y),
                [raw[at], raw[size + at], raw[2 * size + at]],
                "at ({x}, {y})"
            );
            // Alpha at full scale of its 12 bits.
            let alpha = f64::from(raw[3 * size + at]) / 4095.0;
            assert!(
                (picture.alpha_at(x, y) - alpha).abs() <= 0.5 / 65535.0,
                "at ({x}, {y})"
            );
        }
    }
    // A gradient from transparent to opaque.
    assert_eq!(picture.alpha_at(0, 0), 0.0);
    assert_eq!(picture.alpha_at(255, 0), 1.0);
    assert!((picture.alpha_at(128, 0) - 0.5).abs() < 0.01);
}

#[test]
fn a_picture_without_alpha_of_prores_4444_is_opaque() {
    let Some((_, picture)) = native(Sample::Prores4444) else {
        return;
    };
    assert_eq!(picture.alpha, Alpha::Opaque);
    assert_eq!(picture.alpha_plane, [0_u8; 0]);
}

#[test]
fn each_frame_of_a_converter_is_shaped_anew() {
    // A converter that wrote a picture with alpha writes one without it in its place: no plane
    // of alpha is left over.
    let (Some(with), Some(without)) = (media(Sample::ProresAlpha), media(Sample::Prores4444))
    else {
        return;
    };
    let first = |path: &Path| {
        let mut decoder = VideoDecoder::open(path, Hardware::Off).unwrap();
        decoder.next_decoded().unwrap().unwrap()
    };
    let mut converter = Converter::new();
    let mut picture = NativePicture::default();
    converter
        .native_into(&first(&with.path), &mut picture)
        .unwrap();
    assert_eq!(picture.alpha, Alpha::Straight);
    converter
        .native_into(&first(&without.path), &mut picture)
        .unwrap();
    assert_eq!(picture.alpha, Alpha::Opaque);
    assert_eq!(picture.alpha_plane, [0_u8; 0]);
    let Some(still) = media(Sample::AlphaPng) else {
        return;
    };
    converter
        .native_into(&first(&still.path), &mut picture)
        .unwrap();
    assert_eq!(
        (picture.format, picture.alpha, picture.chroma.len()),
        (PictureFormat::Rgba64, Alpha::Straight, 0)
    );
}
