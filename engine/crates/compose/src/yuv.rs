//! The planes of a picture of Y'CbCr on the card, and the shader that turns them into R'G'B'
//! (`yuv.wgsl`): shared by the compositor, which draws pictures as layers, and by the view, which
//! draws one fitted into its window.
//!
//! A picture is a [`NativePicture`] of the media crate, in the sampling of chroma that its file
//! stores: 4:2:0 in 8 bits (NV12) or 10 (P010), 4:2:2 (P216) or 4:4:4 (P416) in words of 16 bits
//! whose top [`NativePicture::bits`] matter. Its planes are uploaded as they are, a texel a
//! sample, and the shader reads each sample of chroma where it lies (see `ycc_at` of
//! `yuv.wgsl`), so that nothing of 4:2:2 and 4:4:4 is thrown away. What the codes mean, their
//! matrix and range, is a [`YuvCoding`]: a picture says its own ([`YuvCoding::of_native`]).

use columbina_media::{NativePicture, Picture, PictureFormat, YuvMatrix};

use crate::color::{ColorRange, YuvCoding};
use crate::geometry::quarter_turns;

/// The shader, for planes of floats (8-bit and 16-bit normalized textures); see [`Texel::shader`]
/// for planes of integers.
pub const SHADER: &str = include_str!("yuv.wgsl");

/// The bytes of the uniform of the shader (`Params` in `yuv.wgsl`).
pub const PARAMS_SIZE: u64 = 144;

/// The floats of the first part of the uniform, the view's and the picture's
/// ([`picture_params`]); the compositor's part follows.
pub const PICTURE_PARAMS: usize = 20;

/// How the texels of a plane are read in the shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texel {
    /// Normalized, as floats from 0 to 1.
    Float,
    /// 16-bit integers, where the card has no 16-bit normalized formats.
    Uint,
}

impl Texel {
    /// How the planes of a picture are read: as floats, unless its samples are words of 16 bits
    /// (`wide`: all but NV12) and the card has no 16-bit normalized formats.
    #[must_use]
    pub fn for_picture(wide: bool, sixteen_norm: bool) -> Texel {
        if wide && !sixteen_norm {
            Texel::Uint
        } else {
            Texel::Float
        }
    }

    /// The shader that reads texels of this kind.
    #[must_use]
    pub fn shader(self) -> String {
        match self {
            Texel::Float => SHADER.to_owned(),
            Texel::Uint => SHADER.replacen("alias Texel = f32;", "alias Texel = u32;", 1),
        }
    }

    /// How a plane of texels of this kind is bound.
    #[must_use]
    pub fn sample_type(self) -> wgpu::TextureSampleType {
        match self {
            Texel::Float => wgpu::TextureSampleType::Float { filterable: false },
            Texel::Uint => wgpu::TextureSampleType::Uint,
        }
    }
}

/// The shape of the planes of a picture, and the bits of its samples that matter: what its
/// textures are made for, and how the shader reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    pub format: PictureFormat,
    /// 8 for NV12, else the top bits of each word of 16 that matter (10 for P010).
    pub bits: u8,
}

impl Layout {
    /// The layout of a picture, as it says it is.
    #[must_use]
    pub fn of(picture: &NativePicture) -> Layout {
        Layout {
            width: picture.width,
            height: picture.height,
            format: picture.format,
            bits: picture.bits,
        }
    }

    /// Whether its samples are words of 16 bits: all but NV12.
    #[must_use]
    pub fn wide(self) -> bool {
        self.format.bytes() == 2
    }

    /// The width and height of its plane of chroma, in samples (each a Cb and a Cr): half the
    /// width, rounded up, of 4:2:0 and 4:2:2, and half the height too of 4:2:0.
    #[must_use]
    pub fn chroma_size(self) -> (u32, u32) {
        chroma_size(self.width, self.height, self.format)
    }
}

/// The width and height of the plane of chroma of a picture of `width` × `height` in `format`.
fn chroma_size(width: u32, height: u32, format: PictureFormat) -> (u32, u32) {
    let (across, down) = format.chroma_shift();
    (width.div_ceil(1 << across), height.div_ceil(1 << down))
}

/// A picture of 4:2:0 as a [`Picture`] holds it (NV12 or P010), as a picture of its own
/// sampling with the same planes, said to be of BT.709 and limited range, its chroma sited left:
/// what phase 1 took every picture to be.
#[must_use]
pub fn native_of(picture: Picture) -> NativePicture {
    let ten = picture.bits > 8;
    NativePicture {
        frame: picture.frame,
        width: picture.width,
        height: picture.height,
        format: if ten {
            PictureFormat::P010
        } else {
            PictureFormat::Nv12
        },
        bits: if ten { 10 } else { 8 },
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Limited,
        chroma_location: 1,
        luma: picture.luma,
        chroma: picture.chroma,
        luma_stride: picture.luma_stride,
        chroma_stride: picture.chroma_stride,
    }
}

/// The formats of the planes of luma and of chroma: of bytes for NV12, else of words of 16
/// bits (`wide`).
#[must_use]
pub fn plane_formats(wide: bool, texel: Texel) -> (wgpu::TextureFormat, wgpu::TextureFormat) {
    match (wide, texel) {
        (false, _) => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
        (true, Texel::Float) => (
            wgpu::TextureFormat::R16Unorm,
            wgpu::TextureFormat::Rg16Unorm,
        ),
        (true, Texel::Uint) => (wgpu::TextureFormat::R16Uint, wgpu::TextureFormat::Rg16Uint),
    }
}

/// Check that the planes of a picture hold it, and that its bits are of its format, and give
/// its layout.
///
/// # Errors
/// It has no pixels, or bits that its format cannot hold (NV12 has 8, P010 10, P216 and P416
/// from 8 to 16), or a plane is shorter than its rows, or its rows than its samples.
pub fn check(picture: &NativePicture) -> Result<Layout, String> {
    let layout = Layout::of(picture);
    let (width, height) = (picture.width as usize, picture.height as usize);
    if width == 0 || height == 0 {
        return Err(format!(
            "a picture of {}×{} has no pixels",
            picture.width, picture.height
        ));
    }
    let bits_held = match picture.format {
        PictureFormat::Nv12 => picture.bits == 8,
        PictureFormat::P010 => picture.bits == 10,
        PictureFormat::P216 | PictureFormat::P416 => (8..=16).contains(&picture.bits),
    };
    if !bits_held {
        return Err(format!(
            "a picture of {:?} cannot be of {} bits",
            picture.format, picture.bits
        ));
    }
    let bytes = picture.format.bytes();
    let (chroma_width, chroma_rows) = layout.chroma_size();
    for (name, plane, stride, row, rows) in [
        (
            "luma",
            &picture.luma,
            picture.luma_stride,
            width * bytes,
            height,
        ),
        (
            "chroma",
            &picture.chroma,
            picture.chroma_stride,
            chroma_width as usize * 2 * bytes,
            chroma_rows as usize,
        ),
    ] {
        let needed = stride
            .checked_mul(rows - 1)
            .and_then(|before| before.checked_add(row));
        if stride < row || needed.is_none_or(|needed| plane.len() < needed) {
            return Err(format!(
                "the {name} plane of a picture of {}×{} of {:?} in {} bits is {} bytes in rows of \
                 {stride}",
                picture.width,
                picture.height,
                picture.format,
                picture.bits,
                plane.len()
            ));
        }
    }
    Ok(layout)
}

/// Textures for the planes of a picture of `width` × `height` in `format`, to be uploaded into.
/// Those of a 4K picture of P416 take some 50 MB, so they are kept and used again.
#[must_use]
pub fn plane_textures(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: PictureFormat,
    texel: Texel,
) -> (wgpu::Texture, wgpu::Texture) {
    let (luma_format, chroma_format) = plane_formats(format.bytes() == 2, texel);
    let texture = |label, width, height, format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let (chroma_width, chroma_height) = chroma_size(width, height, format);
    (
        texture("columbina luma", width, height, luma_format),
        texture(
            "columbina chroma",
            chroma_width,
            chroma_height,
            chroma_format,
        ),
    )
}

/// Upload the planes of a picture, which [`check`] passed, into textures of its layout.
///
/// # Errors
/// A stride beyond what the card takes.
pub fn upload(
    queue: &wgpu::Queue,
    luma: &wgpu::Texture,
    chroma: &wgpu::Texture,
    picture: &NativePicture,
) -> Result<(), String> {
    for (texture, data, stride) in [
        (luma, &picture.luma, picture.luma_stride),
        (chroma, &picture.chroma, picture.chroma_stride),
    ] {
        let size = texture.size();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(u32::try_from(stride).map_err(|_| "a stride too long")?),
                rows_per_image: Some(size.height),
            },
            size,
        );
    }
    Ok(())
}

/// The codes of black, of the range of luma from black to white, of no colour, and of the
/// range of chroma, in codes of `bits` bits: those of 8 bits (16, 219, 128 and 224) times
/// `2^(bits - 8)` of limited range, and `0`, `2^bits - 1`, `2^(bits - 1)` and `2^bits - 1` of
/// full range. Of 8 and 10 bits they are [`YuvCoding::levels`].
#[must_use]
pub fn levels(coding: YuvCoding, bits: u8) -> [f64; 4] {
    let bits = i32::from(bits);
    if coding.range == ColorRange::Full {
        let most = 2f64.powi(bits) - 1.0;
        [0.0, most, 2f64.powi(bits - 1), most]
    } else {
        [16.0, 219.0, 128.0, 224.0].map(|level| level * 2f64.powi(bits - 8))
    }
}

/// The first part of the uniform of the shader, for a picture of `layout`, read as `texel`s,
/// coded as `coding` says, turned `rotation` degrees clockwise to be shown, and drawn (by the
/// view) into `dest`: x, y, width and height of the target, in pixels.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sizes of at most 16384, and codes and weights, as the card takes them"
)]
pub fn picture_params(
    dest: [f32; 4],
    layout: Layout,
    texel: Texel,
    coding: YuvCoding,
    rotation: i16,
) -> [f32; PICTURE_PARAMS] {
    let (chroma_width, chroma_height) = layout.chroma_size();
    // What a texel is multiplied by to be a code of `bits` bits: a byte as 0..=255; a word whose
    // top `bits` are the code, as 0..=65535 / 2^(16 - bits) normalized, or as the integer /
    // 2^(16 - bits) (of P010, 65535 / 64 and 1 / 64).
    let below = 2f64.powi(16 - i32::from(layout.bits));
    let scale = match (layout.wide(), texel) {
        (false, _) => 255.0,
        (true, Texel::Float) => (65535.0 / below) as f32,
        (true, Texel::Uint) => (1.0 / below) as f32,
    };
    let [black, luma_range, middle, chroma_range] =
        levels(coding, layout.bits).map(|level| level as f32);
    let matrix = coding.coefficients().map(|weight| weight as f32);
    // A sample of chroma for every second sample of luma across (4:2:0, 4:2:2) and down
    // (4:2:0): 0.5 of a sample of chroma for a sample of luma; else 1.
    let (across, down) = layout.format.chroma_shift();
    let step = |shift: u32| if shift == 0 { 1.0 } else { 0.5 };
    [
        dest[0],
        dest[1],
        dest[2],
        dest[3],
        layout.width as f32,
        layout.height as f32,
        chroma_width as f32,
        chroma_height as f32,
        scale,
        black,
        luma_range,
        quarter_turns(rotation) as f32,
        middle,
        chroma_range,
        step(across),
        step(down),
        matrix[0],
        matrix[1],
        matrix[2],
        matrix[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_of_sixteen_bits_are_read_as_integers_only_without_normalized_formats() {
        assert_eq!(Texel::for_picture(false, false), Texel::Float);
        assert_eq!(Texel::for_picture(false, true), Texel::Float);
        assert_eq!(Texel::for_picture(true, true), Texel::Float);
        assert_eq!(Texel::for_picture(true, false), Texel::Uint);
        // The line itself, not the comment above it that says how it is changed.
        assert!(Texel::Uint.shader().contains("\nalias Texel = u32;"));
        assert!(!Texel::Float.shader().contains("\nalias Texel = u32;"));
        assert!(Texel::Float.shader().contains("\nalias Texel = f32;"));
    }

    fn layout(width: u32, height: u32, format: PictureFormat, bits: u8) -> Layout {
        Layout {
            width,
            height,
            format,
            bits,
        }
    }

    #[test]
    fn the_codes_of_black_and_white_are_those_of_limited_range_by_default() {
        let limited = YuvCoding::default();
        let nv12 = layout(5, 3, PictureFormat::Nv12, 8);
        let eight = picture_params([0.0; 4], nv12, Texel::Float, limited, 90);
        assert_eq!(
            eight[4..16],
            [5.0, 3.0, 3.0, 2.0, 255.0, 16.0, 219.0, 1.0, 128.0, 224.0, 0.5, 0.5]
        );
        let p010 = layout(4, 4, PictureFormat::P010, 10);
        let ten = picture_params([1.0, 2.0, 3.0, 4.0], p010, Texel::Float, limited, 0);
        assert_eq!(ten[..4], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ten[8..14], [65535.0 / 64.0, 64.0, 876.0, 0.0, 512.0, 896.0]);
        let integers = picture_params([0.0; 4], p010, Texel::Uint, limited, -90);
        assert_eq!(integers[8], 1.0 / 64.0);
        assert_eq!(integers[11], 3.0);
        // As the levels of a coding of 8 and 10 bits say.
        for (bits, ten) in [(8, false), (10, true)] {
            for range in [ColorRange::Limited, ColorRange::Full] {
                let coding = YuvCoding {
                    range,
                    ..YuvCoding::default()
                };
                assert_eq!(levels(coding, bits), coding.levels(ten));
            }
        }
    }

    #[test]
    fn words_of_more_bits_are_read_by_their_bits_and_their_chroma_where_it_lies() {
        let limited = YuvCoding::default();
        let full = YuvCoding {
            range: ColorRange::Full,
            ..YuvCoding::default()
        };
        // 4:2:2 of 10 bits is read as P010 is, its chroma at half the width and the full height,
        // a sample of chroma for each row of luma.
        let p216 = picture_params(
            [0.0; 4],
            layout(7, 5, PictureFormat::P216, 10),
            Texel::Float,
            limited,
            0,
        );
        assert_eq!(
            p216[4..16],
            [
                7.0,
                5.0,
                4.0,
                5.0,
                65535.0 / 64.0,
                64.0,
                876.0,
                0.0,
                512.0,
                896.0,
                0.5,
                1.0
            ]
        );
        // 4:4:4 of 12 bits: a sample of chroma for each of luma; a word is 16 codes of 12 bits.
        let p416 = picture_params(
            [0.0; 4],
            layout(7, 5, PictureFormat::P416, 12),
            Texel::Float,
            limited,
            0,
        );
        assert_eq!(
            p416[4..16],
            [
                7.0,
                5.0,
                7.0,
                5.0,
                65535.0 / 16.0,
                256.0,
                3504.0,
                0.0,
                2048.0,
                3584.0,
                1.0,
                1.0
            ]
        );
        let integers = picture_params(
            [0.0; 4],
            layout(7, 5, PictureFormat::P416, 12),
            Texel::Uint,
            full,
            0,
        );
        assert_eq!(
            integers[8..14],
            [1.0 / 16.0, 0.0, 4095.0, 0.0, 2048.0, 4095.0]
        );
        // All 16 bits, and a still made 16 bits of 8 (its codes are the top byte).
        let sixteen = picture_params(
            [0.0; 4],
            layout(2, 2, PictureFormat::P416, 16),
            Texel::Float,
            full,
            0,
        );
        assert_eq!(
            sixteen[8..14],
            [65535.0, 0.0, 65535.0, 0.0, 32768.0, 65535.0]
        );
        let still = picture_params(
            [0.0; 4],
            layout(2, 2, PictureFormat::P416, 8),
            Texel::Float,
            limited,
            0,
        );
        assert_eq!(
            still[8..14],
            [65535.0 / 256.0, 16.0, 219.0, 0.0, 128.0, 224.0]
        );
    }

    #[test]
    fn the_matrix_and_the_range_of_a_picture_are_given_to_the_card() {
        let coded = |matrix, range| {
            picture_params(
                [0.0; 4],
                layout(4, 4, PictureFormat::P010, 10),
                Texel::Float,
                YuvCoding { matrix, range },
                0,
            )
        };
        let full = coded(YuvMatrix::Bt2020, ColorRange::Full);
        assert_eq!(full[9..14], [0.0, 1023.0, 0.0, 512.0, 1023.0]);
        let weights = YuvCoding {
            matrix: YuvMatrix::Bt2020,
            range: ColorRange::Full,
        }
        .coefficients();
        for (given, weight) in full[16..].iter().zip(weights) {
            assert!((f64::from(*given) - weight).abs() < 1e-6, "{full:?}");
        }
        let sd = coded(YuvMatrix::Bt601, ColorRange::Limited);
        assert_eq!(sd[9..14], [64.0, 876.0, 0.0, 512.0, 896.0]);
        assert!((sd[16] - 1.402).abs() < 1e-6);
    }

    fn nv12() -> NativePicture {
        native_of(Picture {
            frame: 0,
            width: 16,
            height: 8,
            bits: 8,
            luma: vec![0; 64 * 8],
            chroma: vec![0; 64 * 4],
            luma_stride: 64,
            chroma_stride: 64,
        })
    }

    #[test]
    fn a_picture_whose_planes_do_not_hold_it_is_refused() {
        let good = nv12();
        assert_eq!(check(&good), Ok(layout(16, 8, PictureFormat::Nv12, 8)));
        assert!(!check(&good).unwrap().wide());
        let mut short = good.clone();
        short.chroma.truncate(64 * 3);
        assert!(check(&short).is_err());
        let mut narrow = good.clone();
        narrow.luma_stride = 8;
        assert!(check(&narrow).is_err());
        let mut empty = good.clone();
        empty.height = 0;
        assert!(check(&empty).is_err());
        let mut ten = good.clone();
        ten.format = PictureFormat::P010;
        ten.bits = 10;
        // Rows of 32 bytes of 16 pixels of 2 bytes: they hold it.
        assert!(check(&ten).unwrap().wide());
        ten.width = 33;
        assert!(check(&ten).is_err());
        // Bits that a format does not have.
        let mut wrong = good.clone();
        wrong.bits = 10;
        assert!(check(&wrong).is_err());
        ten.width = 16;
        ten.bits = 12;
        assert!(check(&ten).is_err());
    }

    #[test]
    fn the_chroma_of_4_2_2_and_4_4_4_is_checked_at_its_own_size() {
        // 4:2:2 of 16 × 8: chroma of 8 × 8 samples, of 2 words each, rows of 32 bytes.
        let mut p216 = nv12();
        p216.format = PictureFormat::P216;
        p216.bits = 10;
        p216.chroma = vec![0; 64 * 8];
        assert_eq!(check(&p216).unwrap().chroma_size(), (8, 8));
        p216.chroma.truncate(64 * 7 + 31);
        assert!(check(&p216).is_err(), "a row short");
        // 4:4:4 of 16 × 8: chroma of 16 × 8, rows of 64 bytes.
        let mut p416 = nv12();
        p416.format = PictureFormat::P416;
        p416.bits = 12;
        p416.chroma = vec![0; 64 * 8];
        assert_eq!(check(&p416).unwrap().chroma_size(), (16, 8));
        p416.chroma_stride = 48;
        assert!(check(&p416).is_err(), "rows narrower than their samples");
        p416.chroma_stride = 64;
        p416.bits = 7;
        assert!(check(&p416).is_err());
        p416.bits = 16;
        assert!(check(&p416).is_ok());
    }

    #[test]
    fn a_picture_of_phase_1_is_one_of_bt_709_and_limited_range_of_the_same_planes() {
        let native = nv12();
        assert_eq!(native.format, PictureFormat::Nv12);
        assert_eq!(
            YuvCoding::of_native(&native),
            YuvCoding {
                matrix: YuvMatrix::Bt709,
                range: ColorRange::Limited
            }
        );
        // Drawn as one of the default coding, whose range, not said, is taken as limited.
        assert_eq!(
            levels(YuvCoding::of_native(&native), 8),
            levels(YuvCoding::default(), 8)
        );
        assert_eq!(native.into_picture().unwrap().luma.len(), 64 * 8);
    }
}
