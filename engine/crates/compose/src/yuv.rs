//! The planes of a picture of Y'CbCr on the card, and the shader that turns them into R'G'B'
//! (`yuv.wgsl`): shared by the compositor, which draws pictures as layers, and by the view, which
//! draws one fitted into its window.

use columbina_media::Picture;

use crate::color::YuvCoding;
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
    /// How the planes of a picture are read: as floats, unless they are of 16 bits and the card
    /// has no 16-bit normalized formats.
    #[must_use]
    pub fn for_picture(ten: bool, sixteen_norm: bool) -> Texel {
        if ten && !sixteen_norm {
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

/// The formats of the planes of luma and of chroma.
#[must_use]
pub fn plane_formats(ten: bool, texel: Texel) -> (wgpu::TextureFormat, wgpu::TextureFormat) {
    match (ten, texel) {
        (false, _) => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
        (true, Texel::Float) => (
            wgpu::TextureFormat::R16Unorm,
            wgpu::TextureFormat::Rg16Unorm,
        ),
        (true, Texel::Uint) => (wgpu::TextureFormat::R16Uint, wgpu::TextureFormat::Rg16Uint),
    }
}

// Pictures are 4:2:0 here, in two planes, 8 bits (NV12) or 10 in 16 (P010), as the decoder
// gives them in a `Picture`. The pictures of their own sampling that the media crate makes
// (`NativePicture`: P216, 4:2:2, and P416, 4:4:4, in words of 16 bits whose top `bits` matter)
// come in by the shape of their planes, and nothing after a pixel is made R'G'B' changes:
// - [`check`] and [`plane_textures`]: a chroma plane of ⌈w/2⌉ × h (4:2:2) or w × h (4:4:4),
//   of `R16Unorm`/`Rg16Unorm` (or the `Uint` ones), as for P010;
// - [`picture_params`]: what a word is multiplied by to be a code, `65535 / 2^(16 - bits)`
//   for floats (or `1 / 2^(16 - bits)` for integers), and the levels of `bits` bits (black
//   `16 · 2^(bits - 8)`, and so on, or `0 .. 2^bits - 1` of full range), where they are of 8
//   or 10 bits now;
// - `ycc_at` of `yuv.wgsl`: the place of a chroma texel for a luma one, now
//   `(x · 0.5, (y − 0.5) · 0.5)`, is `(x · 0.5, y)` for 4:2:2 and `(x, y)` for 4:4:4, a factor
//   that the uniform would carry (the second half of `Params::planes` holds the size of the
//   chroma plane already).

/// Check that the planes of a picture hold it, and say whether it is of 10 bits (P010).
///
/// # Errors
/// It has no pixels, or a plane is shorter than its rows, or its rows than its pixels.
pub fn check(picture: &Picture) -> Result<bool, String> {
    let (width, height) = (picture.width as usize, picture.height as usize);
    if width == 0 || height == 0 {
        return Err(format!(
            "a picture of {}×{} has no pixels",
            picture.width, picture.height
        ));
    }
    let ten = picture.bits > 8;
    let bytes = if ten { 2 } else { 1 };
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
            width.div_ceil(2) * 2 * bytes,
            height.div_ceil(2),
        ),
    ] {
        let needed = stride
            .checked_mul(rows - 1)
            .and_then(|before| before.checked_add(row));
        if stride < row || needed.is_none_or(|needed| plane.len() < needed) {
            return Err(format!(
                "the {name} plane of a picture of {}×{} in {} bits is {} bytes in rows of {stride}",
                picture.width,
                picture.height,
                picture.bits,
                plane.len()
            ));
        }
    }
    Ok(ten)
}

/// Textures for the planes of a picture of `width` × `height`, to be uploaded into.
#[must_use]
pub fn plane_textures(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    ten: bool,
    texel: Texel,
) -> (wgpu::Texture, wgpu::Texture) {
    let (luma_format, chroma_format) = plane_formats(ten, texel);
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
    (
        texture("columbina luma", width, height, luma_format),
        texture(
            "columbina chroma",
            width.div_ceil(2),
            height.div_ceil(2),
            chroma_format,
        ),
    )
}

/// Upload the planes of a picture, which [`check`] passed, into textures of its shape.
///
/// # Errors
/// A stride beyond what the card takes.
pub fn upload(
    queue: &wgpu::Queue,
    luma: &wgpu::Texture,
    chroma: &wgpu::Texture,
    picture: &Picture,
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

/// The first part of the uniform of the shader, for a picture of `size` pixels as stored, in
/// 10 bits or 8, read as `texel`s, coded as `coding` says, turned `rotation` degrees clockwise
/// to be shown, and drawn (by the view) into `dest`: x, y, width and height of the target, in
/// pixels.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sizes of at most 16384, and codes and weights, as the card takes them"
)]
pub fn picture_params(
    dest: [f32; 4],
    size: (u32, u32),
    ten: bool,
    texel: Texel,
    coding: YuvCoding,
    rotation: i16,
) -> [f32; PICTURE_PARAMS] {
    let (width, height) = size;
    // What a texel is multiplied by to be a code: 8 bits as 0..=255; P010, whose 10 bits are
    // the high bits of 16, as 0..=65535 / 64 normalized, or as the integer / 64.
    let scale = match (ten, texel) {
        (false, _) => 255.0,
        (true, Texel::Float) => 65535.0 / 64.0,
        (true, Texel::Uint) => 1.0 / 64.0,
    };
    let [black, luma_range, middle, chroma_range] = coding.levels(ten).map(|level| level as f32);
    let matrix = coding.coefficients().map(|weight| weight as f32);
    [
        dest[0],
        dest[1],
        dest[2],
        dest[3],
        width as f32,
        height as f32,
        width.div_ceil(2) as f32,
        height.div_ceil(2) as f32,
        scale,
        black,
        luma_range,
        quarter_turns(rotation) as f32,
        middle,
        chroma_range,
        0.0,
        0.0,
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

    #[test]
    fn the_codes_of_black_and_white_are_those_of_limited_range_by_default() {
        let limited = YuvCoding::default();
        let eight = picture_params([0.0; 4], (5, 3), false, Texel::Float, limited, 90);
        assert_eq!(
            eight[4..16],
            [5.0, 3.0, 3.0, 2.0, 255.0, 16.0, 219.0, 1.0, 128.0, 224.0, 0.0, 0.0]
        );
        let ten = picture_params([1.0, 2.0, 3.0, 4.0], (4, 4), true, Texel::Float, limited, 0);
        assert_eq!(ten[..4], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ten[8..14], [65535.0 / 64.0, 64.0, 876.0, 0.0, 512.0, 896.0]);
        let integers = picture_params([0.0; 4], (4, 4), true, Texel::Uint, limited, -90);
        assert_eq!(integers[8], 1.0 / 64.0);
        assert_eq!(integers[11], 3.0);
    }

    #[test]
    fn the_matrix_and_the_range_of_a_picture_are_given_to_the_card() {
        use crate::color::{ColorRange, YuvMatrix};
        let coded = |matrix, range| {
            picture_params(
                [0.0; 4],
                (4, 4),
                true,
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

    #[test]
    fn a_picture_whose_planes_do_not_hold_it_is_refused() {
        let good = Picture {
            frame: 0,
            width: 16,
            height: 8,
            bits: 8,
            luma: vec![0; 64 * 8],
            chroma: vec![0; 64 * 4],
            luma_stride: 64,
            chroma_stride: 64,
        };
        assert_eq!(check(&good), Ok(false));
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
        ten.bits = 10;
        // Rows of 32 bytes of 16 pixels of 2 bytes: they hold it.
        assert_eq!(check(&ten), Ok(true));
        ten.width = 33;
        assert!(check(&ten).is_err());
    }
}
