//! Pictures in the sampling of chroma that they are stored in: 4:2:2 and 4:4:4 keep every
//! sample of chroma, in 16 bits, where a [`Picture`] halves them down to 4:2:0.

use std::fmt;

use columbina_model::ColorRange;
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::color::{self, YuvMatrix};
use crate::ffi::{self, ColorCodes, Planes, Scaler};
use crate::{MediaError, Picture};

/// How a [`NativePicture`] is laid out. A picture of Y'CbCr (all but [`PictureFormat::Rgba64`]
/// and [`PictureFormat::RgbaF16`]) has two planes, luma, then Cb and Cr interleaved (Cb first);
/// one of R'G'B'A has one, of the four values of each pixel. Each row of a plane starts 64 bytes
/// after a multiple of 64 from the start.
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
    /// R'G'B'A, as it is stored, never made Y'CbCr (of PNG, TIFF, `QuickTime` Animation, and
    /// the grey and the palettes of stills too, R' = G' = B' of grey): one plane of four
    /// little-endian words of 16 bits a pixel, R', G', B' and A, as libav's `rgba64le`, each the
    /// code of the file at full scale, `round(code · 65535 / (2^bits − 1))`, so that a code of 8
    /// bits is `code · 257` and the most of any depth is 65535: normalized, each is the code as
    /// a fraction of its most, exactly. Alpha is 65535 where the file has none.
    Rgba64,
    /// R'G'B'A of floats (of `OpenEXR`, and other formats of floats): one plane of four
    /// little-endian half floats (IEEE 754 binary16) a pixel, R', G', B' and A, as libav's
    /// `rgbaf16le`; values beyond 0..1, as of scene-linear light, are kept. Alpha is 1 where the
    /// file has none.
    RgbaF16,
}

impl PictureFormat {
    /// How far chroma is halved across and down, as powers of 2; nothing (0, 0) of R'G'B'A,
    /// which has no chroma.
    #[must_use]
    pub fn chroma_shift(self) -> (u32, u32) {
        match self {
            Self::Nv12 | Self::P010 => (1, 1),
            Self::P216 => (1, 0),
            Self::P416 | Self::Rgba64 | Self::RgbaF16 => (0, 0),
        }
    }

    /// The bytes of a sample, or of each of the four values of a pixel of R'G'B'A: 1 for NV12,
    /// else 2.
    #[must_use]
    pub fn bytes(self) -> usize {
        if self == Self::Nv12 {
            1
        } else {
            2
        }
    }

    /// The bytes of a pixel of the first plane: of luma, or of R'G'B'A (8).
    #[must_use]
    pub fn pixel_bytes(self) -> usize {
        if self.is_rgb() {
            8
        } else {
            self.bytes()
        }
    }

    /// Whether it is R'G'B'A, of one plane, rather than Y'CbCr.
    #[must_use]
    pub fn is_rgb(self) -> bool {
        matches!(self, Self::Rgba64 | Self::RgbaF16)
    }

    /// The format of libav that is laid out so.
    fn pixel(self) -> Pixel {
        match self {
            Self::Nv12 => Pixel::NV12,
            Self::P010 => Pixel::P010LE,
            Self::P216 => Pixel::P216LE,
            Self::P416 => Pixel::P416LE,
            Self::Rgba64 => Pixel::RGBA64LE,
            Self::RgbaF16 => Pixel::RGBAF16LE,
        }
    }
}

/// How the alpha of a [`NativePicture`] is to be taken: none, or with its colour kept apart from
/// it (straight) or multiplied by it (premultiplied), as `DaVinci Resolve`'s Alpha Mode of a clip
/// has it (RM20 p446). The file says which, where libav reads it (`AVFrame::alpha_mode`:
/// premultiplied of `OpenEXR`, straight of PNG); where it does not (`ProRes` 4444, whose frames
/// carry no word of it, TIFF, `QuickTime` Animation), alpha is straight, as libav's formats of
/// alpha are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alpha {
    /// No alpha: every pixel is opaque.
    #[default]
    Opaque,
    /// Alpha apart from the colour, which is the colour of the pixel where it is seen at all.
    Straight,
    /// The colour multiplied by alpha already.
    Premultiplied,
}

/// A decoded frame in memory, in the sampling of chroma that its file stores (4:2:0, 4:2:2 or
/// 4:4:4), made by [`crate::Converter::native`]: two planes, luma, then Cb and Cr interleaved,
/// as [`PictureFormat`] lays them. Unlike a [`Picture`], nothing of its chroma is thrown away:
/// a picture of 4:2:2 or 4:4:4 keeps every sample, in 16 bits; one of 4:2:0 is as a
/// [`Picture`] is (NV12 or P010), and [`NativePicture::into_picture`] makes it one.
///
/// Its codes are those of the file, of the range and matrix that it says ([`NativePicture::range`],
/// [`NativePicture::matrix`]). A picture stored as R'G'B' (or grey, or a palette) is not made
/// Y'CbCr: it is R'G'B'A ([`PictureFormat::Rgba64`], or [`PictureFormat::RgbaF16`] of floats),
/// in one plane (`luma`), and has no matrix.
///
/// Its alpha, if it has any ([`NativePicture::alpha`]), is the fourth value of each pixel of
/// R'G'B'A, or of Y'CbCr a plane of its own ([`NativePicture::alpha_plane`]).
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
    /// `bits` bits is `word >> (16 - bits)`. Where it was converted (from a format of another
    /// layout), the bits below are those of the conversion, not 0. Of R'G'B'A: of
    /// [`PictureFormat::Rgba64`] the bits of the codes of the file, from 8 (8 for fewer, and
    /// for a palette) to 16, which its words hold at full scale; 16 of
    /// [`PictureFormat::RgbaF16`].
    pub bits: u8,
    /// The matrix that its Y'CbCr is made R'G'B' with; BT.709, and not used, of R'G'B'A.
    pub matrix: YuvMatrix,
    /// The range of its codes: limited or full, never unspecified. Of R'G'B'A, full unless the
    /// file says limited; of floats always full.
    pub range: ColorRange,
    /// Where its chroma lies, as libav numbers it (`AVChromaLocation`): 0 unspecified, 1 left
    /// (the default of MPEG-2, H.264 and HEVC), 2 center, 3 top left, 4 top, 5 bottom left,
    /// 6 bottom. Only 4:2:0 and 4:2:2 are sited; 4:4:4 lies on luma; R'G'B'A has no chroma (0).
    pub chroma_location: u8,
    /// How its alpha is taken, or that it has none.
    pub alpha: Alpha,
    /// The plane of luma: `height` rows of `luma_stride` bytes. Of R'G'B'A, the one plane, of
    /// the four values of each pixel.
    pub luma: Vec<u8>,
    /// The plane of chroma: as many rows as [`NativePicture::chroma_size`] says, of
    /// `chroma_stride` bytes, each holding Cb and Cr of a sample of chroma, one after the other.
    /// Empty of R'G'B'A.
    pub chroma: Vec<u8>,
    /// The plane of alpha of a picture of Y'CbCr that has alpha: laid out as luma is (`height`
    /// rows of `luma_stride` bytes, a sample a pixel), each sample at full scale (a byte of
    /// NV12, else a little-endian word of 16 bits whose most, 65535, is opaque), whatever the
    /// bits of the file. Empty of a picture without alpha, and of R'G'B'A, whose alpha is the
    /// fourth value of each pixel.
    pub alpha_plane: Vec<u8>,
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
            .field("alpha", &self.alpha)
            .field("luma_stride", &self.luma_stride)
            .field("chroma_stride", &self.chroma_stride)
            .finish_non_exhaustive()
    }
}

impl NativePicture {
    /// The width and height of its plane of chroma in samples of chroma (each a Cb and a Cr):
    /// the size of the picture halved, rounded up, where its format halves it; nothing of
    /// R'G'B'A.
    #[must_use]
    pub fn chroma_size(&self) -> (u32, u32) {
        if self.format.is_rgb() {
            return (0, 0);
        }
        let (across, down) = self.format.chroma_shift();
        (
            self.width.div_ceil(1 << across),
            self.height.div_ceil(1 << down),
        )
    }

    /// The bytes of its planes, all together.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.luma.len() + self.chroma.len() + self.alpha_plane.len()
    }

    /// The four words of R', G', B' and A at the pixel `(x, y)` of a picture of R'G'B'A: codes
    /// at full scale of [`PictureFormat::Rgba64`], the bits of half floats of
    /// [`PictureFormat::RgbaF16`].
    ///
    /// # Panics
    /// The picture is of Y'CbCr, the pixel is outside it, or its plane is shorter than it says.
    #[must_use]
    pub fn rgba(&self, x: u32, y: u32) -> [u16; 4] {
        assert!(self.format.is_rgb(), "a picture of {:?}", self.format);
        assert!(x < self.width && y < self.height, "({x}, {y}) is outside");
        let at = y as usize * self.luma_stride + 8 * x as usize;
        let pixel = &self.luma[at..at + 8];
        std::array::from_fn(|value| u16::from_le_bytes([pixel[2 * value], pixel[2 * value + 1]]))
    }

    /// The alpha of the pixel `(x, y)`, from 0 (transparent) to 1 (opaque): 1 of a picture
    /// without alpha.
    ///
    /// # Panics
    /// The pixel is outside the picture, or its planes are shorter than it says.
    #[must_use]
    pub fn alpha_at(&self, x: u32, y: u32) -> f64 {
        assert!(x < self.width && y < self.height, "({x}, {y}) is outside");
        if self.alpha == Alpha::Opaque {
            return 1.0;
        }
        match self.format {
            PictureFormat::Rgba64 => f64::from(self.rgba(x, y)[3]) / 65535.0,
            PictureFormat::RgbaF16 => f64::from(float_of_half(self.rgba(x, y)[3])),
            PictureFormat::Nv12 => {
                f64::from(self.alpha_plane[y as usize * self.luma_stride + x as usize]) / 255.0
            }
            PictureFormat::P010 | PictureFormat::P216 | PictureFormat::P416 => {
                let at = y as usize * self.luma_stride + 2 * x as usize;
                let word = [self.alpha_plane[at], self.alpha_plane[at + 1]];
                f64::from(u16::from_le_bytes(word)) / 65535.0
            }
        }
    }

    /// The codes of Y', Cb and Cr at the pixel `(x, y)`, in [`NativePicture::bits`] bits: the
    /// sample of chroma that the pixel lies in, unblended.
    ///
    /// # Panics
    /// The picture is of R'G'B'A, the pixel is outside it, or its planes are shorter than it
    /// says.
    #[must_use]
    pub fn ycc(&self, x: u32, y: u32) -> [u16; 3] {
        assert!(!self.format.is_rgb(), "a picture of {:?}", self.format);
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
    /// the same planes of (but no alpha); else itself back.
    ///
    /// # Errors
    /// It is of 4:2:2 or 4:4:4, or of R'G'B'A.
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

    /// Shape the picture for a frame of `width` × `height` in `format`, without alpha, and give
    /// its planes to be written (of R'G'B'A, the plane of chroma is empty). The memory that it
    /// holds is kept where it is of the length needed. Of 4:2:0 both planes have the stride of
    /// a [`Picture`]'s.
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
        self.alpha = Alpha::Opaque;
        self.alpha_plane.clear();
        let (chroma_width, chroma_rows) = self.chroma_size();
        let bytes = format.bytes();
        let chroma_stride = if format.is_rgb() {
            0
        } else {
            (chroma_width as usize * 2 * bytes).next_multiple_of(64)
        };
        let luma_stride = if format.chroma_shift() == (1, 1) {
            chroma_stride
        } else {
            (width as usize * format.pixel_bytes()).next_multiple_of(64)
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

    /// Give a picture of Y'CbCr, shaped, a plane of alpha taken as `alpha` says, to be written:
    /// laid out as luma is, of its stride.
    fn shape_alpha(&mut self, alpha: Alpha) -> &mut [u8] {
        self.alpha = alpha;
        self.alpha_plane.resize(self.luma.len(), 0);
        &mut self.alpha_plane
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
/// each in a plane of their own, else converted by swscale (into Y'CbCr of its own sampling).
/// A frame of R'G'B', grey or a palette is taken as R'G'B'A as it is ([`fill_rgb`]); its alpha,
/// and the alpha of Y'CbCr in a plane of its own, is kept. `stream` is the bits of luma that the
/// stream says, `codes` what the frame says of its colours.
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
    if layout.rgb_like {
        return fill_rgb(source, frame, codes, into);
    }
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
            PictureFormat::P216 | PictureFormat::P416 => true,
            PictureFormat::P010 | PictureFormat::Rgba64 | PictureFormat::RgbaF16 => false,
        };
    let bits = match target {
        PictureFormat::Nv12 => 8,
        PictureFormat::P010 => 10,
        PictureFormat::P216 | PictureFormat::P416 => bits_of(layout.depth, stream),
        PictureFormat::Rgba64 | PictureFormat::RgbaF16 => 16,
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
    if layout.alpha_plane {
        let alpha = alpha_said(source);
        let stride = into.luma_stride;
        copy_alpha(
            source,
            layout.depth,
            target,
            stride,
            into.shape_alpha(alpha),
        );
    }
    Ok(())
}

/// How a frame that has alpha says its alpha is taken: straight unless it says premultiplied.
fn alpha_said(source: &ff::frame::Video) -> Alpha {
    if ffi::alpha_mode(source) == 1 {
        Alpha::Premultiplied
    } else {
        Alpha::Straight
    }
}

/// Write the plane of alpha of a frame of planar Y'CbCr with alpha (plane 3, samples of `depth`
/// bits at the bottom of a byte or a word) into the plane of alpha of a picture of `format`,
/// rows of `stride` bytes: as bytes of NV12 (of 8 bits, as it is), else as words at full scale.
fn copy_alpha(
    source: &ff::frame::Video,
    depth: u32,
    format: PictureFormat,
    stride: usize,
    into: &mut [u8],
) {
    let (width, height) = (source.width() as usize, source.height() as usize);
    let (data, from_stride) = (source.data(3), source.stride(3));
    for y in 0..height {
        let from = &data[y * from_stride..];
        let row = &mut into[y * stride..];
        if format == PictureFormat::Nv12 {
            row[..width].copy_from_slice(&from[..width]);
        } else if depth > 8 {
            for (pair, bytes) in row[..2 * width]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(from[..2 * width].as_chunks::<2>().0)
            {
                *pair = full_scale(u32::from(u16::from_le_bytes(*bytes)), depth).to_le_bytes();
            }
        } else {
            for (pair, &byte) in row[..2 * width]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(&from[..width])
            {
                *pair = full_scale(byte.into(), depth).to_le_bytes();
            }
        }
    }
}

/// A code of `depth` bits (from 1 to 16) at full scale in 16 bits: `round(code · 65535 /
/// (2^depth − 1))`, so that a code of 8 bits is `code · 257`, of 16 bits itself, and the most of
/// any depth 65535. Bits above `depth` are ignored.
#[allow(
    clippy::cast_possible_truncation,
    reason = "at most 65535, as the most of a code at full scale"
)]
fn full_scale(code: u32, depth: u32) -> u16 {
    let depth = depth.clamp(1, 16);
    let most = (1_u64 << depth) - 1;
    let code = u64::from(code) & most;
    ((code * 65535 * 2 + most) / (2 * most)) as u16
}

/// A float as the half float (IEEE 754 binary16) nearest to it, ties to even: beyond the
/// largest half (65504), infinity; a NaN stays a NaN.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "the fields of a float, each of a few bits"
)]
pub(crate) fn half_of(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        let nan = if mantissa == 0 {
            0
        } else {
            0x0200 | (mantissa >> 13) as u16
        };
        return sign | 0x7c00 | nan;
    }
    // The exponent of the half, biased.
    let biased = exponent - 127 + 15;
    if biased >= 0x1f {
        return sign | 0x7c00;
    }
    // What is shifted out is rounded, to even where it is half of the last bit kept.
    let rounded = |kept: u32, shift: u32, of: u32| -> u32 {
        let rest = of & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        kept + u32::from(rest > halfway || (rest == halfway && kept & 1 == 1))
    };
    if biased <= 0 {
        // A subnormal half (or 0): the mantissa with its leading 1, at 2^-24 a step.
        if biased < -10 {
            return sign;
        }
        let whole = mantissa | 0x0080_0000;
        let shift = (14 - biased) as u32;
        return sign | rounded(whole >> shift, shift, whole) as u16;
    }
    // A carry of the rounding into the exponent is right, to infinity too.
    let kept = ((biased as u32) << 10) | (mantissa >> 13);
    sign | rounded(kept, 13, mantissa) as u16
}

/// The value of a half float (IEEE 754 binary16) as a float, exactly.
#[must_use]
pub(crate) fn float_of_half(half: u16) -> f32 {
    let sign = if half & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((half >> 10) & 0x1f);
    let mantissa = f32::from(half & 0x03ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        0x1f if mantissa == 0.0 => sign * f32::INFINITY,
        0x1f => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// Write a frame of R'G'B', grey or a palette (see [`ffi::Layout::rgb_like`]) into a picture of
/// R'G'B'A as it is, never made Y'CbCr: integers at full scale ([`PictureFormat::Rgba64`]), so
/// that a red of 255 is 65535 and stays 1.0; floats as half floats ([`PictureFormat::RgbaF16`]),
/// beyond 0..1 too. Grey is R', G' and B' alike; a palette is looked up; alpha is kept (taken as
/// the frame says, [`alpha_said`]), and is opaque where there is none.
fn fill_rgb(
    source: &ff::frame::Video,
    frame: i64,
    codes: ColorCodes,
    into: &mut NativePicture,
) -> Result<(), MediaError> {
    let format = source.format();
    let described = ffi::components(format).ok_or_else(|| {
        MediaError::Decode("a frame of a format that libav does not describe".into())
    })?;
    let (width, height) = (source.width(), source.height());
    let target = if described.float {
        PictureFormat::RgbaF16
    } else {
        PictureFormat::Rgba64
    };
    // The component that each of R', G', B' and A is read from (alpha opaque where there is
    // none), and whether the colour is looked up in a palette by the first.
    let colours = if described.rgb { [0, 1, 2] } else { [0, 0, 0] };
    let alpha_of = if described.palette {
        None
    } else {
        Some(if described.rgb { 3 } else { 1 }).filter(|&alpha| {
            described.alpha && alpha < described.count && described.depths[alpha] > 0
        })
    };
    let palette = described.palette.then(|| ffi::palette(source));
    // A palette always has alpha; it is had where an entry is not opaque.
    let has_alpha = match &palette {
        Some(entries) => entries.iter().any(|entry| entry >> 24 != 0xff),
        None => alpha_of.is_some(),
    };
    let bits = if described.float {
        16
    } else if described.palette {
        8
    } else {
        let most = colours.iter().map(|&c| described.depths[c]).max();
        u8::try_from(most.unwrap_or(8).clamp(8, 16)).unwrap_or(16)
    };
    // A bit of `monow` is black where it is 1, white where it is 0.
    let inverted = format == Pixel::MonoWhite;
    let one = if described.float { 0x3c00 } else { 65535 };
    // The word of a value of a component of `depth` bits.
    let word = |value: u32, depth: u32| -> u16 {
        if described.float {
            if depth == 32 {
                half_of(f32::from_bits(value))
            } else {
                u16::try_from(value & 0xffff).unwrap_or(0)
            }
        } else if inverted {
            full_scale(1 - (value & 1), 1)
        } else {
            full_scale(value, depth)
        }
    };
    let planes = into.shape(frame, width, height, target, bits);
    let stride = planes.luma_stride;
    let plane = planes.luma;
    let mut values = vec![0_u32; width as usize];
    for y in 0..height {
        let row = &mut plane[y as usize * stride..][..8 * width as usize];
        let pixels = row.as_chunks_mut::<8>().0;
        if let Some(palette) = &palette {
            ffi::read_row(source, 0, y, &mut values);
            for (pixel, &index) in pixels.iter_mut().zip(&values) {
                let entry = palette[(index & 0xff) as usize];
                // A, R, G and B, a byte each from the top.
                let [blue, green, red, alpha] = entry.to_le_bytes();
                for (at, byte) in [red, green, blue, alpha].into_iter().enumerate() {
                    pixel[2 * at..2 * at + 2]
                        .copy_from_slice(&full_scale(byte.into(), 8).to_le_bytes());
                }
            }
            continue;
        }
        let mut read: Option<usize> = None;
        for (at, component) in colours.into_iter().map(Some).chain([alpha_of]).enumerate() {
            let Some(component) = component else {
                for pixel in pixels.iter_mut() {
                    pixel[2 * at..2 * at + 2].copy_from_slice(&u16::to_le_bytes(one));
                }
                continue;
            };
            if read != Some(component) {
                ffi::read_row(source, component, y, &mut values);
                read = Some(component);
            }
            let depth = described.depths[component];
            for (pixel, &value) in pixels.iter_mut().zip(&values) {
                pixel[2 * at..2 * at + 2].copy_from_slice(&word(value, depth).to_le_bytes());
            }
        }
    }
    into.matrix = YuvMatrix::Bt709;
    // `AVCOL_RANGE_MPEG`: R'G'B' of limited range, which few files are.
    into.range = if !described.float && codes.range == 1 {
        ColorRange::Limited
    } else {
        ColorRange::Full
    };
    into.chroma_location = 0;
    if has_alpha {
        into.alpha = alpha_said(source);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_of_every_depth_are_made_full_scale() {
        for code in 0..=255_u32 {
            assert_eq!(u32::from(full_scale(code, 8)), code * 257);
        }
        assert_eq!(full_scale(1023, 10), 65535);
        assert_eq!(full_scale(0, 10), 0);
        // 512 of 10 bits is 0.50049, which is 32800.5 of 16: rounded up.
        assert_eq!(full_scale(512, 10), 32800);
        assert_eq!(full_scale(4095, 12), 65535);
        assert_eq!(full_scale(1, 1), 65535);
        assert_eq!(full_scale(40000, 16), 40000);
        // Each code of 10 bits, back again by rounding.
        for code in 0..1024_u32 {
            let back = (f64::from(full_scale(code, 10)) * 1023.0 / 65535.0).round();
            assert!((back - f64::from(code)).abs() < 1e-9);
        }
    }

    #[test]
    fn every_half_float_is_a_float_and_back() {
        for half in 0..=u16::MAX {
            let value = float_of_half(half);
            if value.is_nan() {
                assert!(float_of_half(half_of(value)).is_nan());
                continue;
            }
            assert_eq!(half_of(value), half, "{half:#06x} is {value}");
        }
    }

    #[test]
    fn floats_are_made_the_nearest_half_ties_to_even() {
        assert_eq!(half_of(1.0), 0x3c00);
        assert_eq!(half_of(4.0), 0x4400);
        assert_eq!(half_of(-2.0), 0xc000);
        assert_eq!(half_of(65504.0), 0x7bff);
        assert_eq!(
            half_of(65520.0),
            0x7c00,
            "beyond the largest, rounded up to infinity"
        );
        assert_eq!(half_of(1e10), 0x7c00);
        assert_eq!(half_of(f32::NEG_INFINITY), 0xfc00);
        // 1 + 2^-11 is halfway between 1 and the half after it: to even, 1.
        assert_eq!(half_of(1.0 + 2f32.powi(-11)), 0x3c00);
        // 1 + 3·2^-11 is halfway between 1 + 2^-10 and 1 + 2^-9: to even, 1 + 2^-9.
        assert_eq!(half_of(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
        assert_eq!(half_of(1.0 + 2f32.powi(-11) + 2f32.powi(-20)), 0x3c01);
        // Subnormal halves, and what is too small for any.
        assert_eq!(half_of(2f32.powi(-24)), 0x0001);
        assert_eq!(half_of(2f32.powi(-25)), 0x0000, "halfway, to even");
        assert_eq!(half_of(1.5 * 2f32.powi(-25)), 0x0001);
        assert_eq!(half_of(3.0 * 2f32.powi(-25)), 0x0002, "halfway, to even");
        assert_eq!(half_of(2f32.powi(-30)), 0x0000);
        assert_eq!(half_of(-2f32.powi(-30)), 0x8000);
        assert_eq!(half_of(2f32.powi(-14) * (1.0 - 2f32.powi(-11))), 0x0400);
    }

    #[test]
    fn a_picture_of_r_g_b_a_is_one_plane_and_no_chroma() {
        let mut picture = NativePicture::default();
        let planes = picture.shape(0, 5, 3, PictureFormat::Rgba64, 8);
        assert_eq!((planes.luma_stride, planes.chroma.len()), (64, 0));
        assert_eq!(picture.luma.len(), 64 * 3);
        assert_eq!(picture.chroma_size(), (0, 0));
        picture.luma[8 * 4..8 * 5].copy_from_slice(&[1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(picture.rgba(4, 0), [1, 2, 3, 4]);
        assert_eq!(
            picture.alpha_at(4, 0),
            1.0,
            "opaque, whatever its fourth word"
        );
        picture.alpha = Alpha::Straight;
        assert!((picture.alpha_at(4, 0) - 4.0 / 65535.0).abs() < 1e-12);
        // Shaped for another frame, it has no alpha until it is given some.
        picture.shape(1, 9, 2, PictureFormat::P416, 12);
        assert_eq!(picture.alpha, Alpha::Opaque);
        let plane = picture.shape_alpha(Alpha::Premultiplied);
        assert_eq!(plane.len(), 64 * 2);
        assert_eq!(picture.bytes(), 3 * 64 * 2);
    }
}
