//! Pictures in the sampling of chroma that they are stored in: 4:2:2 and 4:4:4 keep every
//! sample of chroma, in 16 bits, where a [`Picture`] halves them down to 4:2:0.

use std::fmt;

use columbina_model::ColorRange;
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::color::{self, YuvMatrix};
use crate::ffi::{self, ColorCodes, Planes, Scaler};
use crate::{MediaError, Picture};

/// How a [`NativePicture`] is laid out: always two planes, luma, then Cb and Cr interleaved
/// (Cb first), each row of a plane starting 64 bytes after a multiple of 64 from the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PictureFormat {
    /// 4:2:0 in 8 bits, a byte a sample: chroma at half the width and half the height.
    #[default]
    Nv12,
    /// 4:2:0 in 10 bits, each sample a little-endian word of 16 bits with its 10 bits at the
    /// top: chroma at half the width and half the height.
    P010,
    /// 4:2:2, each sample a little-endian word of 16 bits with its bits at the top (as libav's
    /// `p216le`; of 10 bits it is `p210le` bit for bit): chroma at half the width and the full
    /// height.
    P216,
    /// 4:4:4, each sample a little-endian word of 16 bits with its bits at the top (as libav's
    /// `p416le`): chroma at the full width and height.
    P416,
}

impl PictureFormat {
    /// How far chroma is halved across and down, as powers of 2.
    #[must_use]
    pub fn chroma_shift(self) -> (u32, u32) {
        match self {
            Self::Nv12 | Self::P010 => (1, 1),
            Self::P216 => (1, 0),
            Self::P416 => (0, 0),
        }
    }

    /// The bytes of a sample: 1 for NV12, else 2.
    #[must_use]
    pub fn bytes(self) -> usize {
        if self == Self::Nv12 {
            1
        } else {
            2
        }
    }

    /// The format of libav that is laid out so.
    fn pixel(self) -> Pixel {
        match self {
            Self::Nv12 => Pixel::NV12,
            Self::P010 => Pixel::P010LE,
            Self::P216 => Pixel::P216LE,
            Self::P416 => Pixel::P416LE,
        }
    }
}

/// A decoded frame in memory, in the sampling of chroma that its file stores (4:2:0, 4:2:2 or
/// 4:4:4), made by [`crate::Converter::native`]: two planes, luma, then Cb and Cr interleaved,
/// as [`PictureFormat`] lays them. Unlike a [`Picture`], nothing of its chroma is thrown away:
/// a picture of 4:2:2 or 4:4:4 keeps every sample, in 16 bits; one of 4:2:0 is as a
/// [`Picture`] is (NV12 or P010), and [`NativePicture::into_picture`] makes it one.
///
/// Its codes are those of the file, of the range and matrix that it says ([`NativePicture::range`],
/// [`NativePicture::matrix`]), but for a picture stored as R'G'B', which is made Y'CbCr of
/// BT.709 of limited range.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NativePicture {
    /// Its number, at the rate of its picture.
    pub frame: i64,
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    /// How its planes are laid out.
    pub format: PictureFormat,
    /// The bits of each sample that matter: 8 for NV12, 10 for P010, and for P216 and P416 the
    /// bits of the file (8, 10, 12 or 16), which are the top bits of each word of 16. A code of
    /// `bits` bits is `word >> (16 - bits)`. Where it was converted (from R'G'B', or from a
    /// format of another layout), the bits below are those of the conversion, not 0.
    pub bits: u8,
    /// The matrix that its Y'CbCr is made R'G'B' with.
    pub matrix: YuvMatrix,
    /// The range of its codes: limited or full, never unspecified.
    pub range: ColorRange,
    /// Where its chroma lies, as libav numbers it (`AVChromaLocation`): 0 unspecified, 1 left
    /// (the default of MPEG-2, H.264 and HEVC), 2 center, 3 top left, 4 top, 5 bottom left,
    /// 6 bottom. Only 4:2:0 and 4:2:2 are sited; 4:4:4 lies on luma.
    pub chroma_location: u8,
    /// The plane of luma: `height` rows of `luma_stride` bytes.
    pub luma: Vec<u8>,
    /// The plane of chroma: as many rows as [`NativePicture::chroma_size`] says, of
    /// `chroma_stride` bytes, each holding Cb and Cr of a sample of chroma, one after the other.
    pub chroma: Vec<u8>,
    /// The bytes from a row of luma to the next one.
    pub luma_stride: usize,
    /// The bytes from a row of chroma to the next one.
    pub chroma_stride: usize,
}

impl fmt::Debug for NativePicture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativePicture")
            .field("frame", &self.frame)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("bits", &self.bits)
            .field("matrix", &self.matrix)
            .field("range", &self.range)
            .field("chroma_location", &self.chroma_location)
            .field("luma_stride", &self.luma_stride)
            .field("chroma_stride", &self.chroma_stride)
            .finish_non_exhaustive()
    }
}

impl NativePicture {
    /// The width and height of its plane of chroma in samples of chroma (each a Cb and a Cr):
    /// the size of the picture halved, rounded up, where its format halves it.
    #[must_use]
    pub fn chroma_size(&self) -> (u32, u32) {
        let (across, down) = self.format.chroma_shift();
        (
            self.width.div_ceil(1 << across),
            self.height.div_ceil(1 << down),
        )
    }

    /// The codes of Y', Cb and Cr at the pixel `(x, y)`, in [`NativePicture::bits`] bits: the
    /// sample of chroma that the pixel lies in, unblended.
    ///
    /// # Panics
    /// The pixel is outside the picture, or its planes are shorter than it says.
    #[must_use]
    pub fn ycc(&self, x: u32, y: u32) -> [u16; 3] {
        assert!(x < self.width && y < self.height, "({x}, {y}) is outside");
        let (across, down) = self.format.chroma_shift();
        let (x, y) = (x as usize, y as usize);
        let (cx, cy) = (x >> across, y >> down);
        let word = |plane: &[u8], at: usize| -> u16 {
            if self.format == PictureFormat::Nv12 {
                u16::from(plane[at])
            } else {
                let bytes = [plane[2 * at], plane[2 * at + 1]];
                u16::from_le_bytes(bytes) >> (16 - u32::from(self.bits))
            }
        };
        let samples = self.format.bytes();
        let luma = word(&self.luma, y * self.luma_stride / samples + x);
        let row = cy * self.chroma_stride / samples;
        [
            luma,
            word(&self.chroma, row + 2 * cx),
            word(&self.chroma, row + 2 * cx + 1),
        ]
    }

    /// The picture as a [`Picture`], where it is of 4:2:0 (NV12 or P010), which a picture has
    /// the same planes of; else itself back.
    ///
    /// # Errors
    /// It is of 4:2:2 or 4:4:4.
    pub fn into_picture(self) -> Result<Picture, Self> {
        if !matches!(self.format, PictureFormat::Nv12 | PictureFormat::P010) {
            return Err(self);
        }
        Ok(Picture {
            frame: self.frame,
            width: self.width,
            height: self.height,
            bits: self.bits,
            luma: self.luma,
            chroma: self.chroma,
            luma_stride: self.luma_stride,
            chroma_stride: self.chroma_stride,
        })
    }

    /// Shape the picture for a frame of `width` × `height` in `format`, and give its planes to
    /// be written. The memory that it holds is kept where it is of the length needed. Of 4:2:0
    /// both planes have the stride of a [`Picture`]'s.
    pub(crate) fn shape(
        &mut self,
        frame: i64,
        width: u32,
        height: u32,
        format: PictureFormat,
        bits: u8,
    ) -> Planes<'_> {
        self.frame = frame;
        self.width = width;
        self.height = height;
        self.format = format;
        self.bits = bits;
        let (chroma_width, chroma_rows) = self.chroma_size();
        let bytes = format.bytes();
        let chroma_stride = (chroma_width as usize * 2 * bytes).next_multiple_of(64);
        let luma_stride = if format.chroma_shift() == (1, 1) {
            chroma_stride
        } else {
            (width as usize * bytes).next_multiple_of(64)
        };
        self.luma_stride = luma_stride;
        self.chroma_stride = chroma_stride;
        for (plane, length) in [
            (&mut self.luma, luma_stride * height as usize),
            (&mut self.chroma, chroma_stride * chroma_rows as usize),
        ] {
            if plane.len() != length {
                *plane = vec![0; length];
            }
        }
        Planes {
            luma: &mut self.luma,
            luma_stride,
            chroma: &mut self.chroma,
            chroma_stride,
        }
    }

    /// Say how its codes are made R'G'B', from what its frame says.
    pub(crate) fn describe(&mut self, codes: ColorCodes, format: Pixel, rgb: bool) {
        let said = color::of_codes(codes, Some(format));
        if rgb {
            self.matrix = YuvMatrix::Bt709;
            self.range = ColorRange::Limited;
        } else {
            self.matrix = YuvMatrix::of(&said, self.width, self.height);
            self.range = color::levels(&said);
        }
        self.chroma_location = said.chroma_location;
    }
}

/// The format of a picture that a frame of a card, copied out as `memory`, is laid out as
/// already, with the bits of each sample: two planes of luma and of Cb and Cr interleaved, of
/// little-endian words with their bits at the top.
pub(crate) fn laid_out_as(memory: Pixel) -> Option<(PictureFormat, u8)> {
    Some(match memory {
        Pixel::NV12 => (PictureFormat::Nv12, 8),
        Pixel::P010LE => (PictureFormat::P010, 10),
        Pixel::P210LE => (PictureFormat::P216, 10),
        Pixel::P212LE => (PictureFormat::P216, 12),
        Pixel::P216LE => (PictureFormat::P216, 16),
        Pixel::P410LE => (PictureFormat::P416, 10),
        Pixel::P412LE => (PictureFormat::P416, 12),
        Pixel::P416LE => (PictureFormat::P416, 16),
        _ => return None,
    })
}

/// The bits that matter of a sample of `depth` bits of a frame of a stream of `stream` bits:
/// those of the stream where it says fewer, as a card gives 12 bits in 16.
pub(crate) fn bits_of(depth: u32, stream: u8) -> u8 {
    let depth = u8::try_from(depth.min(16)).unwrap_or(16);
    if (1..depth).contains(&stream) {
        stream
    } else {
        depth
    }
}

/// Write a frame in memory into a picture in the sampling of chroma of the frame: its planes
/// copied where they are laid out as a picture's, its samples moved where its Y', Cb and Cr are
/// each in a plane of their own, else converted by swscale (R'G'B' into Y'CbCr of BT.709).
/// `stream` is the bits of luma that the stream says, `codes` what the frame says of its
/// colours.
pub(crate) fn fill(
    scaler: &mut Option<Scaler>,
    source: &ff::frame::Video,
    stream: u8,
    frame: i64,
    codes: ColorCodes,
    into: &mut NativePicture,
) -> Result<(), MediaError> {
    let format = source.format();
    let (width, height) = (source.width(), source.height());
    let layout = ffi::layout(format).ok_or_else(|| {
        MediaError::Decode("a frame of a format that libav does not describe".into())
    })?;
    if let Some((target, bits)) = laid_out_as(format) {
        let planes = into.shape(frame, width, height, target, bits_of(bits.into(), stream));
        copy_planes(source, target, planes);
        into.describe(codes, format, false);
        return Ok(());
    }
    let (across, down) = layout.log2_chroma;
    let target = if layout.rgb {
        PictureFormat::P416
    } else if across >= 1 && down >= 1 {
        if layout.depth > 8 {
            PictureFormat::P010
        } else {
            PictureFormat::Nv12
        }
    } else if across >= 1 {
        PictureFormat::P216
    } else {
        PictureFormat::P416
    };
    let sampled = layout.planar_yuv && target.chroma_shift() == (across, down);
    // Where libav has a format laid out as the picture with the bits of the frame (P010, P210,
    // P212, P410, P412, P216, P416), swscale writes it: it moves each code to the top of its
    // word, as `repack` does, and keeps the range, which it changes only of the formats of JPEG
    // (of 8 bits); and it does so some 100 times as fast as `repack` where the engine is built
    // without optimizing (60 ms for a frame of 1080p in 10 bits, 100 ms of 4:2:2, which the
    // tests of the transport, built so, cannot wait for).
    let same_bits = match (target, layout.depth) {
        (PictureFormat::P010, 10) => Some(Pixel::P010LE),
        (PictureFormat::P216, 10) => Some(Pixel::P210LE),
        (PictureFormat::P216, 12) => Some(Pixel::P212LE),
        (PictureFormat::P216, 16) => Some(Pixel::P216LE),
        (PictureFormat::P416, 10) => Some(Pixel::P410LE),
        (PictureFormat::P416, 12) => Some(Pixel::P412LE),
        (PictureFormat::P416, 16) => Some(Pixel::P416LE),
        _ => None,
    }
    .filter(|_| sampled);
    // The others whose chroma is sampled as the picture's are moved by `repack`: bytes of 8 bits
    // (of JPEG too, whose range is kept), and words of other bits of 4:2:2 and 4:4:4.
    let exact = sampled
        && same_bits.is_none()
        && match target {
            PictureFormat::Nv12 => layout.depth == 8,
            PictureFormat::P010 => false,
            PictureFormat::P216 | PictureFormat::P416 => true,
        };
    let bits = match target {
        PictureFormat::Nv12 => 8,
        PictureFormat::P010 => 10,
        PictureFormat::P216 | PictureFormat::P416 => bits_of(layout.depth, stream),
    };
    let mut planes = into.shape(frame, width, height, target, bits);
    if exact {
        repack(source, layout.depth, target, &mut planes);
    } else {
        let output = same_bits.unwrap_or_else(|| target.pixel());
        let scaler = scaler_for(scaler, format, width, height, output, layout.rgb)?;
        ffi::scale_into(scaler, source, &mut planes).map_err(MediaError::Decode)?;
    }
    into.describe(codes, format, layout.rgb);
    Ok(())
}

/// The scaler that was kept, if it was made for frames of this format and size into `output`,
/// else a new one in its place.
fn scaler_for(
    slot: &mut Option<Scaler>,
    format: Pixel,
    width: u32,
    height: u32,
    output: Pixel,
    rgb: bool,
) -> Result<&mut Scaler, MediaError> {
    let fits = slot.as_ref().is_some_and(|scaler| {
        let input = scaler.0.input();
        (input.format, input.width, input.height) == (format, width, height)
            && scaler.0.output().format == output
    });
    if !fits {
        let made = ff::software::scaling::Context::get(
            format,
            width,
            height,
            output,
            width,
            height,
            ff::software::scaling::Flags::BILINEAR,
        )
        .map_err(|error| {
            MediaError::Decode(format!(
                "frames of {} cannot be converted: {error}",
                format
                    .descriptor()
                    .map_or("a format", |described| described.name())
            ))
        })?;
        let mut made = Scaler(made);
        if rgb {
            made.rgb_to_bt709().map_err(MediaError::Decode)?;
        }
        *slot = Some(made);
    }
    slot.as_mut()
        .ok_or_else(|| MediaError::Decode("no scaler".into()))
}

/// Copy the two planes of a frame laid out as `format` already, a row at a time.
fn copy_planes(source: &ff::frame::Video, format: PictureFormat, planes: Planes<'_>) {
    let bytes = format.bytes();
    let (across, down) = format.chroma_shift();
    let (width, height) = (source.width() as usize, source.height() as usize);
    for (plane, into, stride, rows, row) in [
        (0, planes.luma, planes.luma_stride, height, width * bytes),
        (
            1,
            planes.chroma,
            planes.chroma_stride,
            height.div_ceil(1 << down),
            width.div_ceil(1 << across) * 2 * bytes,
        ),
    ] {
        let (data, from_stride) = (source.data(plane), source.stride(plane));
        for y in 0..rows {
            into[y * stride..y * stride + row]
                .copy_from_slice(&data[y * from_stride..y * from_stride + row]);
        }
    }
}

/// A sample of `depth` bits at the top of a word of 16, as its two little-endian bytes.
fn word(sample: u16, depth: u32) -> [u8; 2] {
    (sample << (16 - depth)).to_le_bytes()
}

/// Write a frame whose Y', Cb and Cr are each in a plane of their own, a byte or a
/// little-endian word a sample (`depth` bits at the bottom), and whose chroma is sampled as
/// `format`'s, into the planes of `format`: bytes as they are for NV12, else each sample at the
/// top of a word. Nothing is scaled: every code is kept.
fn repack(source: &ff::frame::Video, depth: u32, format: PictureFormat, planes: &mut Planes<'_>) {
    let (width, height) = (source.width() as usize, source.height() as usize);
    let (across, down) = format.chroma_shift();
    let (chroma_width, chroma_rows) = (width.div_ceil(1 << across), height.div_ceil(1 << down));
    let wide = depth > 8;
    // A sample of a row of the frame.
    let sample = |row: &[u8], x: usize| -> u16 {
        if wide {
            u16::from_le_bytes([row[2 * x], row[2 * x + 1]])
        } else {
            u16::from(row[x])
        }
    };
    let (luma, stride) = (source.data(0), source.stride(0));
    for y in 0..height {
        let from = &luma[y * stride..];
        let into = &mut planes.luma[y * planes.luma_stride..];
        match (format, wide) {
            (PictureFormat::Nv12, _) => into[..width].copy_from_slice(&from[..width]),
            (_, true) => {
                for (pair, bytes) in into[..2 * width]
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(from[..2 * width].as_chunks::<2>().0)
                {
                    *pair = word(u16::from_le_bytes(*bytes), depth);
                }
            }
            (_, false) => {
                for (pair, &byte) in into[..2 * width]
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(&from[..width])
                {
                    *pair = word(byte.into(), depth);
                }
            }
        }
    }
    let (blues, blue_stride) = (source.data(1), source.stride(1));
    let (reds, red_stride) = (source.data(2), source.stride(2));
    for y in 0..chroma_rows {
        let (blue, red) = (&blues[y * blue_stride..], &reds[y * red_stride..]);
        let into = &mut planes.chroma[y * planes.chroma_stride..];
        if format == PictureFormat::Nv12 {
            for (x, pair) in into[..2 * chroma_width]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                *pair = [blue[x], red[x]];
            }
        } else {
            for (x, four) in into[..4 * chroma_width]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
            {
                let [b0, b1] = word(sample(blue, x), depth);
                let [r0, r1] = word(sample(red, x), depth);
                *four = [b0, b1, r0, r1];
            }
        }
    }
}
