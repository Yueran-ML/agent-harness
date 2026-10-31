//! The colours that files say, and pictures that keep the chroma of 4:2:2 and 4:4:4.
//!
//! Every picture is checked against the first frame as the `ffmpeg` of the command line decodes
//! it, code for code: decoding H.264, HEVC, `ProRes` and `DNxHR` is exact, so a picture that
//! lost or moved any of its chroma would differ. Those decoded on the graphics card where it
//! can (`Hardware::Auto`) take the GPU: run them one at a time (`--test-threads=1`).

mod support;

use std::path::Path;

use columbina_media::{
    input_space, probe, Basis, ColorRange, Converter, Decoded, Hardware, InputSpace, NativePicture,
    PictureFormat, VideoDecoder, VideoFacts, YuvMatrix,
};
use columbina_model::{DisplayPrimaries, LightLevel, Luminance};
use support::{media, Sample};

/// The picture of a sample, as `probe` reads it, and the path of the sample; None where the
/// sample cannot be made and is not required.
fn probed(sample: Sample) -> Option<(std::path::PathBuf, VideoFacts)> {
    let made = media(sample)?;
    let facts = probe(&made.path).unwrap();
    Some((made.path, facts.video.expect("a picture")))
}

/// The first frame of a file, decoded with or without the graphics card.
fn first(path: &Path, hardware: Hardware) -> Decoded {
    let mut decoder = VideoDecoder::open(path, hardware).unwrap();
    decoder.next_decoded().unwrap().expect("a first frame")
}

/// The first frame of a file as a native picture.
fn native(path: &Path, hardware: Hardware) -> NativePicture {
    Converter::new().native(&first(path, hardware)).unwrap()
}

/// Little-endian words of 16 bits.
fn words(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

/// Check that a picture holds, code for code, the planar frame `raw` of `bits` bits whose
/// chroma is halved `shift` times across and down (as the `ffmpeg` of the command line gives
/// it), and that its chroma has detail at its own resolution: some sample of chroma differs
/// from the one below it (and, for 4:4:4, from the one beside it), which 4:2:0 (or 4:2:2)
/// would have made alike.
fn assert_holds(picture: &NativePicture, raw: &[u8], bits: u8, shift: (u32, u32)) {
    let (width, height) = (picture.width as usize, picture.height as usize);
    let (chroma_width, chroma_height) = (width >> shift.0, height >> shift.1);
    assert_eq!(
        picture.chroma_size(),
        (
            u32::try_from(chroma_width).unwrap(),
            u32::try_from(chroma_height).unwrap()
        )
    );
    assert_eq!(picture.bits, bits);
    let samples: Vec<u16> = if bits > 8 {
        words(raw)
    } else {
        raw.iter().map(|&byte| u16::from(byte)).collect()
    };
    let luma_size = width * height;
    let chroma_size = chroma_width * chroma_height;
    assert_eq!(samples.len(), luma_size + 2 * chroma_size);
    let (luma, chroma) = samples.split_at(luma_size);
    let (blue, red) = chroma.split_at(chroma_size);
    let mut differs_down = false;
    let mut differs_across = false;
    for y in 0..height {
        for x in 0..width {
            let at = (y >> shift.1) * chroma_width + (x >> shift.0);
            let expected = [luma[y * width + x], blue[at], red[at]];
            let (px, py) = (u32::try_from(x).unwrap(), u32::try_from(y).unwrap());
            assert_eq!(picture.ycc(px, py), expected, "at ({x}, {y})");
        }
    }
    for y in 0..chroma_height - 1 {
        for x in 0..chroma_width - 1 {
            let at = y * chroma_width + x;
            differs_down |=
                blue[at] != blue[at + chroma_width] || red[at] != red[at + chroma_width];
            differs_across |= blue[at] != blue[at + 1] || red[at] != red[at + 1];
        }
    }
    assert!(differs_down, "the chroma has no detail down");
    if shift.0 == 0 {
        assert!(differs_across, "the chroma has no detail across");
    }
}

#[test]
fn bt709_of_limited_range_is_read_and_is_rec709() {
    let Some((path, video)) = probed(Sample::Bt709Limited) else {
        return;
    };
    let color = video.color.clone().unwrap();
    assert_eq!((color.primaries, color.transfer, color.matrix), (1, 1, 1));
    assert_eq!(color.range, ColorRange::Limited);
    assert_eq!(color.chroma_location, 1);
    assert_eq!((color.mastering, color.light_level), (None, None));
    assert_eq!(
        input_space(&video, None, InputSpace::REC709),
        (InputSpace::REC709, Basis::Metadata)
    );
    let picture = native(&path, Hardware::Off);
    assert_eq!(picture.format, PictureFormat::Nv12);
    assert_eq!(
        (picture.matrix, picture.range),
        (YuvMatrix::Bt709, ColorRange::Limited)
    );
}

#[test]
fn bt709_of_full_range_is_read_and_its_codes_are_kept_full() {
    let Some((path, video)) = probed(Sample::Bt709Full) else {
        return;
    };
    let color = video.color.clone().unwrap();
    assert_eq!((color.primaries, color.transfer, color.matrix), (1, 1, 1));
    assert_eq!(color.range, ColorRange::Full);
    assert_eq!(
        input_space(&video, None, InputSpace::REC709),
        (InputSpace::REC709, Basis::Metadata)
    );
    for hardware in [Hardware::Off, Hardware::Auto] {
        let picture = native(&path, hardware);
        assert_eq!(picture.format, PictureFormat::Nv12);
        assert_eq!(
            (picture.matrix, picture.range),
            (YuvMatrix::Bt709, ColorRange::Full)
        );
        let raw = support::first_frame_raw(&path, "yuvj420p").unwrap();
        assert_holds_420(&picture, &raw);
    }
}

/// As [`assert_holds`], of a picture of NV12 and a frame of `yuv420p`, without asking for
/// detail.
fn assert_holds_420(picture: &NativePicture, raw: &[u8]) {
    let (width, height) = (picture.width as usize, picture.height as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let (luma, chroma) = raw.split_at(width * height);
    let (blue, red) = chroma.split_at(chroma_width * chroma_height);
    for y in 0..height {
        for x in 0..width {
            let at = (y / 2) * chroma_width + x / 2;
            let expected = [luma[y * width + x], blue[at], red[at]].map(u16::from);
            let (px, py) = (u32::try_from(x).unwrap(), u32::try_from(y).unwrap());
            assert_eq!(picture.ycc(px, py), expected, "at ({x}, {y})");
        }
    }
}

#[test]
fn pq_is_read_with_the_mastering_display_and_light_levels_of_its_frames() {
    let Some((path, video)) = probed(Sample::Pq) else {
        return;
    };
    let color = video.color.clone().unwrap();
    assert_eq!((color.primaries, color.transfer, color.matrix), (9, 16, 9));
    assert_eq!(color.range, ColorRange::Limited);
    let mastering = color.mastering.expect("a mastering display");
    assert_eq!(
        mastering.primaries,
        Some(DisplayPrimaries {
            red: [34000, 16000],
            green: [13250, 34500],
            blue: [7500, 3000],
            white: [15635, 16450],
        })
    );
    assert_eq!(
        mastering.luminance,
        Some(Luminance {
            min: 1,
            max: 10_000_000
        })
    );
    assert_eq!(
        color.light_level,
        Some(LightLevel {
            max_cll: 1000,
            max_fall: 400
        })
    );
    assert_eq!(
        input_space(&video, None, InputSpace::REC709),
        (InputSpace::REC2100_PQ, Basis::Metadata)
    );
    // The frame says it too, on the card as on the processor.
    for hardware in [Hardware::Off, Hardware::Auto] {
        let decoded = first(&path, hardware);
        let said = decoded.color();
        assert_eq!((said.primaries, said.transfer, said.matrix), (9, 16, 9));
        assert_eq!(said.mastering, color.mastering);
        assert_eq!(said.light_level, color.light_level);
        let picture = Converter::new().native(&decoded).unwrap();
        assert_eq!(picture.format, PictureFormat::P010);
        assert_eq!(
            (picture.matrix, picture.range),
            (YuvMatrix::Bt2020, ColorRange::Limited)
        );
    }
}

#[test]
fn hlg_is_read_and_is_rec2100_hlg() {
    let Some((_, video)) = probed(Sample::Hlg) else {
        return;
    };
    let color = video.color.clone().unwrap();
    assert_eq!((color.primaries, color.transfer, color.matrix), (9, 18, 9));
    assert_eq!((color.mastering, color.light_level), (None, None));
    assert_eq!(
        input_space(&video, None, InputSpace::REC709),
        (InputSpace::REC2100_HLG, Basis::Metadata)
    );
}

#[test]
fn a_file_that_says_nothing_of_its_colours_is_rec709() {
    let Some((_, video)) = probed(Sample::Fhd) else {
        return;
    };
    let color = video.color.clone().unwrap();
    assert_eq!((color.primaries, color.transfer), (2, 2));
    assert_eq!(
        input_space(&video, None, InputSpace::REC709).0,
        InputSpace::REC709
    );
}

#[test]
fn prores_422_keeps_every_sample_of_its_chroma() {
    let Some((path, video)) = probed(Sample::Prores) else {
        return;
    };
    assert_eq!(video.bits, 10);
    let raw = support::first_frame_raw(&path, "yuv422p10le").unwrap();
    for hardware in [Hardware::Off, Hardware::Auto] {
        let picture = native(&path, hardware);
        assert_eq!(picture.format, PictureFormat::P216);
        assert_holds(&picture, &raw, 10, (1, 0));
    }
}

#[test]
fn h264_high_422_of_10_bits_keeps_every_sample_of_its_chroma() {
    let Some((path, _)) = probed(Sample::H264High422) else {
        return;
    };
    let raw = support::first_frame_raw(&path, "yuv422p10le").unwrap();
    for hardware in [Hardware::Off, Hardware::Auto] {
        let picture = native(&path, hardware);
        assert_eq!(picture.format, PictureFormat::P216);
        assert_holds(&picture, &raw, 10, (1, 0));
    }
}

#[test]
fn dnxhr_hqx_keeps_every_sample_of_its_chroma() {
    let Some((path, _)) = probed(Sample::Dnxhr) else {
        return;
    };
    let raw = support::first_frame_raw(&path, "yuv422p10le").unwrap();
    let picture = native(&path, Hardware::Auto);
    assert_eq!(picture.format, PictureFormat::P216);
    assert_holds(&picture, &raw, 10, (1, 0));
}

#[test]
fn hevc_444_of_12_bits_keeps_every_sample_of_its_chroma_on_the_card_too() {
    let Some((path, video)) = probed(Sample::Hevc444) else {
        return;
    };
    assert_eq!(video.bits, 12);
    let raw = support::first_frame_raw(&path, "yuv444p12le").unwrap();
    for hardware in [Hardware::Off, Hardware::Auto] {
        let picture = native(&path, hardware);
        assert_eq!(picture.format, PictureFormat::P416, "{hardware:?}");
        assert_holds(&picture, &raw, 12, (0, 0));
    }
}

#[test]
fn prores_4444_keeps_every_sample_of_its_chroma() {
    let Some((path, video)) = probed(Sample::Prores4444) else {
        return;
    };
    let raw = support::first_frame_raw(&path, "yuv444p12le").unwrap();
    assert_eq!(video.bits, 12);
    let picture = native(&path, Hardware::Off);
    assert_eq!(picture.format, PictureFormat::P416);
    assert_holds(&picture, &raw, 12, (0, 0));
}

#[test]
fn a_native_picture_of_4_2_0_is_the_picture_that_the_converter_makes() {
    for sample in [Sample::Bt709Limited, Sample::Fhd10, Sample::Pq] {
        let Some(made) = media(sample) else {
            continue;
        };
        for hardware in [Hardware::Off, Hardware::Auto] {
            let decoded = first(&made.path, hardware);
            let mut converter = Converter::new();
            let picture = converter.picture(&decoded).unwrap();
            let native = converter.native(&decoded).unwrap();
            assert_eq!(native.into_picture().unwrap(), picture, "{sample:?}");
        }
    }
}

#[test]
fn a_picture_of_4_2_2_is_no_picture_of_4_2_0() {
    let Some(made) = media(Sample::Prores) else {
        return;
    };
    let picture = native(&made.path, Hardware::Off);
    let back = picture.clone().into_picture().unwrap_err();
    assert_eq!(back, picture);
}

#[test]
fn a_picture_of_4_2_0_and_4_2_2_says_where_its_chroma_lies() {
    // Each file says a siting of its own; its chroma, coded losslessly, is the checkerboard as
    // it was made, whatever the siting (which is where the samples lie, not what they are).
    let sitings = (1..=6)
        .map(|location| (Sample::Sited420(location), location, (1, 1)))
        .chain((1..=2).map(|location| (Sample::Sited422(location), location, (1, 0))));
    for (sample, location, shift) in sitings {
        let Some((path, video)) = probed(sample) else {
            return;
        };
        assert_eq!(video.color.unwrap().chroma_location, location, "{sample:?}");
        let picture = native(&path, Hardware::Off);
        assert_eq!(picture.chroma_location, location, "{sample:?}");
        assert_eq!(picture.format.chroma_shift(), shift, "{sample:?}");
        let (width, height) = (picture.width, picture.height);
        for y in 0..height {
            for x in 0..width {
                let [blue, red] = support::checkerboard(x >> shift.0, y >> shift.1);
                let expected = [128, u16::from(blue), u16::from(red)];
                assert_eq!(picture.ycc(x, y), expected, "{sample:?} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn mxf_is_read_and_its_colours_are_those_that_mxf_says() {
    assert!(columbina_media::DEMUXERS.contains(&"mxf"));
    let Some((path, video)) = probed(Sample::Mxf) else {
        return;
    };
    let color = video.color.clone().expect("what MXF says");
    // BT.2020, HLG and the matrix of BT.2020: of the descriptor of MXF, as the stream of
    // DNxHR says nothing of them.
    assert_eq!(
        (color.primaries, color.transfer, color.matrix, color.range),
        (9, 18, 9, ColorRange::Limited)
    );
    let (space, basis) = input_space(&video, None, InputSpace::REC709);
    assert_eq!((space, basis), (InputSpace::REC2100_HLG, Basis::Metadata));
    assert!(!video.still);
    assert_eq!(video.bits, 10);
    let raw = support::first_frame_raw(&path, "yuv422p10le").unwrap();
    let picture = native(&path, Hardware::Off);
    assert_eq!(picture.format, PictureFormat::P216);
    assert_eq!(
        (picture.matrix, picture.range),
        (YuvMatrix::Bt2020, ColorRange::Limited)
    );
    assert_holds(&picture, &raw, 10, (1, 0));
}
