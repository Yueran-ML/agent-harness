//! What a file says of the colours of its picture, from the parameters of its stream and the
//! side data of its first frame; and how the Y'CbCr of its pictures is made R'G'B'.

use columbina_model::{
    ColorFacts, ColorRange, DisplayPrimaries, LightLevel, Luminance, MasteringDisplay,
};
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::ffi::{self, ColorCodes};

/// A number of H.273 as a byte; a number beyond one (an extension of libav's, from 256 on) is
/// taken as unspecified.
fn code(number: i32) -> u8 {
    u8::try_from(number).unwrap_or(columbina_model::color::UNSPECIFIED)
}

/// The range of `AVColorRange`: 1 is limited (MPEG), 2 full (JPEG). The formats of JPEG of
/// libav (`yuvj420p` and the like) are of full range whatever is said.
fn range(number: i32, format: Option<Pixel>) -> ColorRange {
    let jpeg = matches!(
        format,
        Some(
            Pixel::YUVJ420P | Pixel::YUVJ422P | Pixel::YUVJ444P | Pixel::YUVJ440P | Pixel::YUVJ411P
        )
    );
    match number {
        2 => ColorRange::Full,
        _ if jpeg => ColorRange::Full,
        1 => ColorRange::Limited,
        _ => ColorRange::Unspecified,
    }
}

/// The colours that numbers of libav say, without side data.
pub(crate) fn of_codes(codes: ColorCodes, format: Option<Pixel>) -> ColorFacts {
    ColorFacts {
        primaries: code(codes.primaries),
        transfer: code(codes.transfer),
        matrix: code(codes.matrix),
        range: range(codes.range, format),
        chroma_location: u8::try_from(codes.location).unwrap_or(0),
        mastering: None,
        light_level: None,
    }
}

/// The colours of a decoded frame: its numbers, and its side data of mastering and light.
pub(crate) fn of_frame(frame: &ff::frame::Video) -> ColorFacts {
    use ff::util::frame::side_data::Type;
    let format = Some(frame.format()).filter(|&format| !ffi::on_device(format));
    let mut color = of_codes(ffi::frame_colors(frame), format);
    color.mastering = frame
        .side_data(Type::MasteringDisplayMetadata)
        .and_then(|data| mastering(data.data()));
    color.light_level = frame
        .side_data(Type::ContentLightLevel)
        .and_then(|data| light_level(data.data()));
    color
}

/// The colours of a stream: its parameters and its side data. `format` is the format of its
/// pixels, if libav knows it.
pub(crate) fn of_stream(stream: &ff::format::stream::Stream, format: Option<Pixel>) -> ColorFacts {
    use ff::codec::packet::side_data::Type;
    let mut color = of_codes(ffi::stream_colors(stream), format);
    for data in stream.side_data() {
        match data.kind() {
            Type::MasteringDisplayMetadata => color.mastering = mastering(data.data()),
            Type::ContentLightLevel => color.light_level = light_level(data.data()),
            _ => {}
        }
    }
    color
}

/// Whether the side data of HDR may be in the frames of a stream rather than in its parameters:
/// it is of PQ or HLG, and its parameters lack the mastering display or the light levels.
pub(crate) fn wants_frame(color: &ColorFacts) -> bool {
    matches!(color.transfer, 16 | 18) && (color.mastering.is_none() || color.light_level.is_none())
}

/// The colours of a stream filled in from those of its first frame: what the stream leaves
/// unspecified and the frame says, and the side data that the frame has and the stream lacks.
pub(crate) fn merged(stream: &ColorFacts, frame: &ColorFacts) -> ColorFacts {
    let said = |of_stream: u8, of_frame: u8| {
        if of_stream == columbina_model::color::UNSPECIFIED {
            of_frame
        } else {
            of_stream
        }
    };
    ColorFacts {
        primaries: said(stream.primaries, frame.primaries),
        transfer: said(stream.transfer, frame.transfer),
        matrix: said(stream.matrix, frame.matrix),
        range: if stream.range == ColorRange::Unspecified {
            frame.range
        } else {
            stream.range
        },
        chroma_location: if stream.chroma_location == 0 {
            frame.chroma_location
        } else {
            stream.chroma_location
        },
        mastering: stream.mastering.or(frame.mastering),
        light_level: stream.light_level.or(frame.light_level),
    }
}

/// The words of 32 bits of side data, as the machine orders them, as libav writes its structs.
fn words(data: &[u8]) -> Vec<i32> {
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|word| i32::from_ne_bytes(*word))
        .collect()
}

/// `num / den` in units of `1 / unit`, rounded; none for a fraction that is not a number of
/// them.
fn units(num: i32, den: i32, unit: i64) -> Option<u32> {
    if den <= 0 || num < 0 {
        return None;
    }
    let scaled = (i64::from(num) * unit * 2 + i64::from(den)) / (2 * i64::from(den));
    u32::try_from(scaled).ok()
}

/// An `AVMasteringDisplayMetadata`: the primaries (red, green, blue) and the white point, each
/// x and y as a fraction of two `int`s, then the least and the most luminance as fractions, then
/// whether it has the primaries and whether it has the luminance; 22 `int`s. Chromaticities are
/// given in units of 0.00002 and luminance in units of 0.0001 cd/m², as ST 2086 counts them.
fn mastering(data: &[u8]) -> Option<MasteringDisplay> {
    let words = words(data);
    let words = words.get(..22)?;
    let fraction = |at: usize, unit| units(words[at], words[at + 1], unit);
    let point = |at: usize| Some([fraction(at, 50_000)?, fraction(at + 2, 50_000)?]);
    let primaries = (words[20] != 0)
        .then(|| {
            Some(DisplayPrimaries {
                red: point(0)?,
                green: point(4)?,
                blue: point(8)?,
                white: point(12)?,
            })
        })
        .flatten();
    let luminance = (words[21] != 0)
        .then(|| {
            Some(Luminance {
                min: fraction(16, 10_000)?,
                max: fraction(18, 10_000)?,
            })
        })
        .flatten();
    (primaries.is_some() || luminance.is_some()).then_some(MasteringDisplay {
        primaries,
        luminance,
    })
}

/// An `AVContentLightMetadata`: `MaxCLL` and `MaxFALL`, two `unsigned`s, in cd/m².
fn light_level(data: &[u8]) -> Option<LightLevel> {
    let words = words(data);
    let [max_cll, max_fall] = words.get(..2)? else {
        return None;
    };
    Some(LightLevel {
        max_cll: max_cll.cast_unsigned(),
        max_fall: max_fall.cast_unsigned(),
    })
}

/// The matrix that the Y'CbCr of a picture is made R'G'B' with: `Kr` and `Kb` of
/// `Y' = Kr·R' + (1 − Kr − Kb)·G' + Kb·B'`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum YuvMatrix {
    /// BT.601 (BT.470BG and SMPTE 170M): Kr 0.299, Kb 0.114.
    Bt601,
    /// BT.709: Kr 0.2126, Kb 0.0722.
    #[default]
    Bt709,
    /// BT.2020 of non-constant luminance: Kr 0.2627, Kb 0.0593. A picture of constant luminance
    /// (matrix 10), which hardly any camera writes, is taken as one of non-constant.
    Bt2020,
    /// SMPTE 240M: Kr 0.212, Kb 0.087.
    Smpte240m,
    /// FCC: Kr 0.30, Kb 0.11.
    Fcc,
}

impl YuvMatrix {
    /// `Kr` and `Kb`.
    #[must_use]
    pub fn coefficients(self) -> (f64, f64) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
            Self::Bt2020 => (0.2627, 0.0593),
            Self::Smpte240m => (0.212, 0.087),
            Self::Fcc => (0.30, 0.11),
        }
    }

    /// The matrix of a picture of `width` × `height` whose colours are `color`: the one that it
    /// says; where it says none, that of its primaries (BT.2020, BT.601 of 525 or 625 lines,
    /// BT.709); where it says neither, BT.601 for a picture of standard definition (no more than
    /// 1024 × 576) and BT.709 for a larger one, as players take it. A picture stored as R'G'B'
    /// (matrix 0) is made Y'CbCr by the [`crate::Converter`] with BT.709 in a
    /// [`crate::NativePicture`].
    #[must_use]
    pub fn of(color: &ColorFacts, width: u32, height: u32) -> Self {
        match color.matrix {
            0 | 1 => return Self::Bt709,
            4 => return Self::Fcc,
            5 | 6 => return Self::Bt601,
            7 => return Self::Smpte240m,
            9 | 10 => return Self::Bt2020,
            _ => {}
        }
        match color.primaries {
            1 => Self::Bt709,
            5..=7 => Self::Bt601,
            9 => Self::Bt2020,
            _ if width <= 1024 && height <= 576 => Self::Bt601,
            _ => Self::Bt709,
        }
    }
}

/// The range that the codes of a picture whose colours are `color` span: full where it says
/// so, else limited, the range of video ("Auto" of the data levels of a clip).
#[must_use]
pub fn levels(color: &ColorFacts) -> ColorRange {
    if color.range == ColorRange::Full {
        ColorRange::Full
    } else {
        ColorRange::Limited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[i32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_ne_bytes()).collect()
    }

    #[test]
    fn mastering_metadata_is_read_in_the_units_of_st_2086() {
        // As the decoder of HEVC writes the SEI of `G(13250,34500)B(7500,3000)R(34000,16000)
        // WP(15635,16450)L(10000000,1)`: chromaticities over 50000, luminance over 10000.
        let data = bytes(&[
            34000, 50000, 16000, 50000, // red
            13250, 50000, 34500, 50000, // green
            7500, 50000, 3000, 50000, // blue
            15635, 50000, 16450, 50000, // white
            1, 10000, 10_000_000, 10000, // luminance
            1, 1,
        ]);
        let read = mastering(&data).unwrap();
        assert_eq!(
            read.primaries,
            Some(DisplayPrimaries {
                red: [34000, 16000],
                green: [13250, 34500],
                blue: [7500, 3000],
                white: [15635, 16450],
            })
        );
        assert_eq!(
            read.luminance,
            Some(Luminance {
                min: 1,
                max: 10_000_000
            })
        );
        // As a container writes them, of other denominators: 0.708 and 1000 nits.
        let mut other = data.clone();
        other[..8].copy_from_slice(&bytes(&[708, 1000]));
        other[64..80].copy_from_slice(&bytes(&[5, 100, 1000, 1]));
        let read = mastering(&other).unwrap();
        assert_eq!(read.primaries.unwrap().red[0], 35400);
        assert_eq!(
            read.luminance,
            Some(Luminance {
                min: 500,
                max: 10_000_000
            })
        );
        // Without its primaries, or anything.
        let mut no_primaries = data.clone();
        no_primaries[80..84].copy_from_slice(&0_i32.to_ne_bytes());
        assert_eq!(mastering(&no_primaries).unwrap().primaries, None);
        let mut nothing = no_primaries;
        nothing[84..88].copy_from_slice(&0_i32.to_ne_bytes());
        assert_eq!(mastering(&nothing), None);
        assert_eq!(mastering(&data[..80]), None);
    }

    #[test]
    fn light_levels_are_read_as_two_numbers() {
        assert_eq!(
            light_level(&bytes(&[1000, 400])),
            Some(LightLevel {
                max_cll: 1000,
                max_fall: 400
            })
        );
        assert_eq!(light_level(&bytes(&[1000])), None);
    }

    #[test]
    fn a_matrix_unsaid_follows_the_primaries_then_the_size() {
        let color = |primaries, matrix| ColorFacts {
            primaries,
            matrix,
            ..ColorFacts::default()
        };
        assert_eq!(YuvMatrix::of(&color(1, 6), 1920, 1080), YuvMatrix::Bt601);
        assert_eq!(YuvMatrix::of(&color(2, 9), 720, 576), YuvMatrix::Bt2020);
        assert_eq!(YuvMatrix::of(&color(9, 2), 3840, 2160), YuvMatrix::Bt2020);
        assert_eq!(YuvMatrix::of(&color(6, 2), 1920, 1080), YuvMatrix::Bt601);
        assert_eq!(YuvMatrix::of(&color(2, 2), 720, 480), YuvMatrix::Bt601);
        assert_eq!(YuvMatrix::of(&color(2, 2), 1280, 720), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::of(&color(2, 0), 640, 480), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::Bt2020.coefficients(), (0.2627, 0.0593));
    }

    #[test]
    fn what_a_stream_leaves_unsaid_is_taken_from_its_first_frame() {
        let stream = ColorFacts {
            primaries: 9,
            transfer: 16,
            ..ColorFacts::default()
        };
        let frame = ColorFacts {
            primaries: 1,
            transfer: 1,
            matrix: 9,
            range: ColorRange::Limited,
            chroma_location: 3,
            light_level: Some(LightLevel {
                max_cll: 1000,
                max_fall: 400,
            }),
            mastering: None,
        };
        assert!(wants_frame(&stream));
        let both = merged(&stream, &frame);
        assert_eq!((both.primaries, both.transfer, both.matrix), (9, 16, 9));
        assert_eq!(both.range, ColorRange::Limited);
        assert_eq!(both.chroma_location, 3);
        assert_eq!(both.light_level, frame.light_level);
        assert!(!wants_frame(&ColorFacts {
            transfer: 1,
            ..ColorFacts::default()
        }));
    }

    #[test]
    fn codes_beyond_h273_are_unspecified_and_jpeg_is_full() {
        let codes = ColorCodes {
            primaries: 256,
            transfer: 18,
            matrix: 0,
            range: 0,
            location: 2,
        };
        let color = of_codes(codes, Some(Pixel::YUVJ422P));
        assert_eq!((color.primaries, color.transfer, color.matrix), (2, 18, 0));
        assert_eq!(color.range, ColorRange::Full);
        assert_eq!(
            of_codes(codes, Some(Pixel::YUV422P)).range,
            ColorRange::Unspecified
        );
        assert_eq!(levels(&of_codes(codes, None)), ColorRange::Limited);
    }
}
