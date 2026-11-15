//! The planes of a picture on the card, and the shader that turns them into R'G'B'
//! (`yuv.wgsl`): shared by the compositor, which draws pictures as layers, and by the view, which
//! draws one fitted into its window.
//!
//! A picture is a [`NativePicture`] of the media crate, in the sampling of chroma that its file
//! stores: 4:2:0 in 8 bits (NV12) or 10 (P010), 4:2:2 (P216) or 4:4:4 (P416) in words of 16 bits
//! whose top [`NativePicture::bits`] matter; or R'G'B'A as its file stores it, in words of 16
//! bits at full scale (Rgba64) or half floats (`RgbaF16`), which are drawn as they are. Its
//! planes are uploaded as they are, a texel a sample, and the shader reads each sample of chroma
//! where it lies (see `sample_at` of `yuv.wgsl`, and [`siting`]), so that nothing of 4:2:2 and
//! 4:4:4 is thrown away. What the codes mean, their matrix and range, is a [`YuvCoding`]: a
//! picture says its own ([`YuvCoding::of_native`]). Its alpha, if it has any, is taken as it
//! says ([`Alpha`]), from a plane of its own (Y'CbCr) or from its fourth value (R'G'B'A).

use columbina_media::{Alpha, NativePicture, Picture, PictureFormat, YuvMatrix};

use crate::color::{ColorRange, YuvCoding};
use crate::geometry::quarter_turns;

/// The shader, for planes of floats (8-bit and 16-bit normalized textures, and half floats); see
/// [`Texel::shader`] for planes of integers.
pub const SHADER: &str = include_str!("yuv.wgsl");

/// The bytes of the uniform of the shader (`Params` in `yuv.wgsl`): the view's and the picture's
/// part, then the compositor's, which ends with the crop of the layer and its composite mode.
pub const PARAMS_SIZE: u64 = 208;

/// The floats of the first part of the uniform, the view's and the picture's
/// ([`picture_params`]); the compositor's part follows.
pub const PICTURE_PARAMS: usize = 28;

/// How the texels of a plane are read in the shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texel {
    /// Normalized, as floats from 0 to 1; or half floats, as they are.
    Float,
    /// 16-bit integers, where the card has no 16-bit normalized formats.
    Uint,
}

impl Texel {
    /// How the planes of a picture are read: as floats, unless its samples are words of 16 bits
    /// (`wide`: all but NV12) and the card has no 16-bit normalized formats. See
    /// [`Layout::texel`], which knows that half floats are read as floats on any card.
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

/// The shape of the planes of a picture, the bits of its samples that matter, where its chroma
/// lies and how its alpha is taken: what its textures are made for, and how the shader reads
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    pub format: PictureFormat,
    /// 8 for NV12, else the top bits of each word of 16 that matter (10 for P010); of R'G'B'A
    /// the bits of the codes of the file, which its words hold at full scale.
    pub bits: u8,
    /// Where its chroma lies, as libav numbers it ([`NativePicture::chroma_location`]).
    pub chroma_location: u8,
    /// How its alpha is taken, or that it has none.
    pub alpha: Alpha,
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
            chroma_location: picture.chroma_location,
            alpha: picture.alpha,
        }
    }

    /// Whether its samples (or the values of its pixels of R'G'B'A) are words of 16 bits: all
    /// but NV12.
    #[must_use]
    pub fn wide(self) -> bool {
        self.format.bytes() == 2
    }

    /// Whether it has a plane of alpha: of Y'CbCr that has alpha.
    #[must_use]
    pub fn alpha_plane(self) -> bool {
        self.alpha != Alpha::Opaque && !self.format.is_rgb()
    }

    /// How its planes are read on a card that has 16-bit normalized formats or not: as
    /// [`Texel::for_picture`] says, but half floats always as floats.
    #[must_use]
    pub fn texel(self, sixteen_norm: bool) -> Texel {
        if self.format == PictureFormat::RgbaF16 {
            Texel::Float
        } else {
            Texel::for_picture(self.wide(), sixteen_norm)
        }
    }

    /// The width and height of its plane of chroma, in samples (each a Cb and a Cr): half the
    /// width, rounded up, of 4:2:0 and 4:2:2, and half the height too of 4:2:0; nothing of
    /// R'G'B'A.
    #[must_use]
    pub fn chroma_size(self) -> (u32, u32) {
        chroma_size(self.width, self.height, self.format)
    }
}

/// The width and height of the plane of chroma of a picture of `width` × `height` in `format`.
fn chroma_size(width: u32, height: u32, format: PictureFormat) -> (u32, u32) {
    if format.is_rgb() {
        return (0, 0);
    }
    let (across, down) = format.chroma_shift();
    (width.div_ceil(1 << across), height.div_ceil(1 << down))
}

/// Where a sample of chroma of a picture of `format` lies, said `location` as libav numbers it
/// (`AVChromaLocation`), in samples of luma from the sample of luma at its own place (twice its
/// place, where chroma is halved): across and down.
///
/// Of 4:2:0, "left" (1, the default of MPEG-2, H.264 and HEVC, and what is taken where nothing
/// is said, 0) lies on the first column of its pair and halfway between its rows, (0, 0.5);
/// "center" (2, of JPEG and MPEG-1) halfway between its columns too, (0.5, 0.5); "top left" (3,
/// of interlaced 4:2:0 of DV and the like) on the first of each, (0, 0); "top" (4) (0.5, 0);
/// "bottom left" (5) (0, 1); "bottom" (6) (0.5, 1). Of 4:2:2 only how far across matters: 0 of
/// the sitings of the left (co-sited, as BT.601 and BT.709 lay it), 0.5 of the others (of JPEG).
/// Of 4:4:4 and R'G'B'A, nothing.
#[must_use]
pub fn siting(format: PictureFormat, location: u8) -> [f32; 2] {
    let (across, down) = format.chroma_shift();
    let centred = matches!(location, 2 | 4 | 6);
    let x = if across > 0 && centred { 0.5 } else { 0.0 };
    let y = if down == 0 {
        0.0
    } else {
        match location {
            3 | 4 => 0.0,
            5 | 6 => 1.0,
            _ => 0.5,
        }
    };
    [x, y]
}

/// A picture of 4:2:0 as a [`Picture`] holds it (NV12 or P010), as a picture of its own
/// sampling with the same planes, said to be of BT.709 and limited range, its chroma sited left,
/// opaque: what phase 1 took every picture to be.
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
        ..NativePicture::default()
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

/// The formats of the three planes of a picture of `format` read as `texel`s: luma (of R'G'B'A,
/// its one plane), chroma and alpha. A plane that the picture does not have is bound all the
/// same, as a texel of a format that the card has and the shader reads as `texel`s: the chroma
/// of R'G'B'A, and the alpha of a picture without it.
#[must_use]
pub fn texture_formats(format: PictureFormat, texel: Texel) -> [wgpu::TextureFormat; 3] {
    match format {
        PictureFormat::Rgba64 => match texel {
            Texel::Float => [
                wgpu::TextureFormat::Rgba16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
                wgpu::TextureFormat::R16Unorm,
            ],
            Texel::Uint => [
                wgpu::TextureFormat::Rgba16Uint,
                wgpu::TextureFormat::Rg16Uint,
                wgpu::TextureFormat::R16Uint,
            ],
        },
        PictureFormat::RgbaF16 => [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rg8Unorm,
            wgpu::TextureFormat::R8Unorm,
        ],
        _ => {
            let (luma, chroma) = plane_formats(format.bytes() == 2, texel);
            // Alpha is laid out as luma.
            [luma, chroma, luma]
        }
    }
}

/// Check that the planes of a picture hold it, and that its bits are of its format, and give
/// its layout.
///
/// # Errors
/// It has no pixels, or bits that its format cannot hold (NV12 has 8, P010 10, P216, P416 and
/// Rgba64 from 8 to 16, `RgbaF16` 16), or a plane is shorter than its rows, or its rows than its
/// samples; or it says it has alpha and has no plane of it (of Y'CbCr).
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
        PictureFormat::P216 | PictureFormat::P416 | PictureFormat::Rgba64 => {
            (8..=16).contains(&picture.bits)
        }
        PictureFormat::RgbaF16 => picture.bits == 16,
    };
    if !bits_held {
        return Err(format!(
            "a picture of {:?} cannot be of {} bits",
            picture.format, picture.bits
        ));
    }
    let bytes = picture.format.bytes();
    let (chroma_width, chroma_rows) = layout.chroma_size();
    let mut planes = vec![(
        "luma",
        &picture.luma,
        picture.luma_stride,
        width * picture.format.pixel_bytes(),
        height,
    )];
    if !picture.format.is_rgb() {
        planes.push((
            "chroma",
            &picture.chroma,
            picture.chroma_stride,
            chroma_width as usize * 2 * bytes,
            chroma_rows as usize,
        ));
    }
    if layout.alpha_plane() {
        planes.push((
            "alpha",
            &picture.alpha_plane,
            picture.luma_stride,
            width * bytes,
            height,
        ));
    }
    for (name, plane, stride, row, rows) in planes {
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

/// The textures of the planes of a picture: of luma (of R'G'B'A, its one plane), of chroma and
/// of alpha, each of a texel where the picture has no such plane.
#[derive(Debug)]
pub struct PlaneTextures {
    pub luma: wgpu::Texture,
    pub chroma: wgpu::Texture,
    pub alpha: wgpu::Texture,
}

impl PlaneTextures {
    /// The views of the three, in the order of their bindings (1, 2 and 3 of `yuv.wgsl`).
    #[must_use]
    pub fn views(&self) -> [wgpu::TextureView; 3] {
        [&self.luma, &self.chroma, &self.alpha]
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()))
    }
}

/// Textures for the planes of a picture of `width` × `height` in `format`, with a plane of alpha
/// or not (`alpha_plane`, of Y'CbCr: [`Layout::alpha_plane`]), to be uploaded into. Those of a
/// 4K picture of P416 take some 50 MB, of R'G'B'A some 66 MB, so they are kept and used again.
#[must_use]
pub fn plane_textures(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: PictureFormat,
    alpha_plane: bool,
    texel: Texel,
) -> PlaneTextures {
    let [luma_format, chroma_format, alpha_format] = texture_formats(format, texel);
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
    let (chroma_width, chroma_height) = if format.is_rgb() {
        (1, 1)
    } else {
        chroma_size(width, height, format)
    };
    let (alpha_width, alpha_height) = if alpha_plane { (width, height) } else { (1, 1) };
    PlaneTextures {
        luma: texture("columbina luma", width, height, luma_format),
        chroma: texture(
            "columbina chroma",
            chroma_width,
            chroma_height,
            chroma_format,
        ),
        alpha: texture("columbina alpha", alpha_width, alpha_height, alpha_format),
    }
}

/// Upload the planes of a picture, which [`check`] passed, into textures of its layout: those
/// that it has (not the chroma of R'G'B'A, nor alpha that it does not have).
///
/// # Errors
/// A stride beyond what the card takes.
pub fn upload(
    queue: &wgpu::Queue,
    textures: &PlaneTextures,
    picture: &NativePicture,
) -> Result<(), String> {
    let layout = Layout::of(picture);
    let mut planes = vec![(&textures.luma, &picture.luma, picture.luma_stride)];
    if !picture.format.is_rgb() {
        planes.push((&textures.chroma, &picture.chroma, picture.chroma_stride));
    }
    if layout.alpha_plane() {
        planes.push((&textures.alpha, &picture.alpha_plane, picture.luma_stride));
    }
    for (texture, data, stride) in planes {
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

/// Where [`picture_params`] says whether R'G'B' decoded keeps what is beyond 0..1 (`kind.w`).
pub const UNCLAMPED_PARAM: usize = 27;

/// Say in the parameters of a picture whether its R'G'B' decoded keeps what is beyond 0..1
/// ([`crate::LayerColor::unclamped`]): the shader's `rgb_of` holds Y'CbCr and integer R'G'B' to
/// 0..1 where it does not, as phase 1 did.
pub fn set_unclamped(params: &mut [f32], unclamped: bool) {
    params[UNCLAMPED_PARAM] = if unclamped { 1.0 } else { 0.0 };
}

/// Where [`picture_params`] says how much of a second picture is mixed into the picture
/// (`siting.z`): 0, of a picture that is drawn alone.
pub const BETWEEN_PARAM: usize = 22;

/// Say in the parameters of a picture how much of a second picture of the same layout is mixed
/// into it, from 0 to 1: read by the shader of a layer of two pictures alone (`between.wgsl`,
/// which the compositor makes of `yuv.wgsl` for a clip that is retimed by Frame Blend), and
/// left at 0 for every other.
pub fn set_between(params: &mut [f32], weight: f32) {
    params[BETWEEN_PARAM] = weight;
}

/// The black and the range from black to white of R'G'B' of `bits` bits at full scale, from 0
/// to 1: 0 and 1 of full range; of limited range, `16 · 2^(bits − 8)` and `219 · 2^(bits − 8)`
/// of the most, `2^bits − 1` (16/255 and 219/255 of 8 bits). Of floats, always 0 and 1.
#[must_use]
pub fn rgb_levels(coding: YuvCoding, format: PictureFormat, bits: u8) -> [f64; 2] {
    if coding.range == ColorRange::Full || format == PictureFormat::RgbaF16 {
        return [0.0, 1.0];
    }
    let [black, range, _, _] = levels(coding, bits);
    let most = 2f64.powi(i32::from(bits)) - 1.0;
    [black / most, range / most]
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
    let [black, luma_range, middle, chroma_range] = if layout.format.is_rgb() {
        let [black, range] = rgb_levels(coding, layout.format, layout.bits);
        [black, range, 0.0, 1.0]
    } else {
        levels(coding, layout.bits)
    }
    .map(|level| level as f32);
    let matrix = coding.coefficients().map(|weight| weight as f32);
    // A sample of chroma for every second sample of luma across (4:2:0, 4:2:2) and down
    // (4:2:0): 0.5 of a sample of chroma for a sample of luma; else 1.
    let (across, down) = layout.format.chroma_shift();
    let step = |shift: u32| if shift == 0 { 1.0 } else { 0.5 };
    let [sited_x, sited_y] = siting(layout.format, layout.chroma_location);
    let kind = match layout.format {
        PictureFormat::Rgba64 => 1.0,
        PictureFormat::RgbaF16 => 2.0,
        _ => 0.0,
    };
    let alpha = match layout.alpha {
        Alpha::Opaque => 0.0,
        Alpha::Straight => 1.0,
        Alpha::Premultiplied => 2.0,
    };
    // A texel of R'G'B'A, or of alpha, from 0 to 1: normalized and float texels are; words of
    // 16 bits read as integers are at full scale.
    let unit = if texel == Texel::Uint && layout.format != PictureFormat::RgbaF16 {
        1.0 / 65535.0
    } else {
        1.0
    };
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
        sited_x,
        sited_y,
        0.0,
        0.0,
        kind,
        alpha,
        unit,
        0.0,
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

    /// A layout of chroma sited left, without alpha.
    fn layout(width: u32, height: u32, format: PictureFormat, bits: u8) -> Layout {
        Layout {
            width,
            height,
            format,
            bits,
            chroma_location: 1,
            alpha: Alpha::Opaque,
        }
    }

    #[test]
    fn chroma_is_sited_where_the_file_says_left_where_it_says_nothing() {
        use PictureFormat::{Nv12, Rgba64, P010, P216, P416};
        // 4:2:0: unspecified and left alike; center, top left, top, bottom left, bottom.
        for (location, place) in [
            (0, [0.0, 0.5]),
            (1, [0.0, 0.5]),
            (2, [0.5, 0.5]),
            (3, [0.0, 0.0]),
            (4, [0.5, 0.0]),
            (5, [0.0, 1.0]),
            (6, [0.5, 1.0]),
        ] {
            assert_eq!(siting(Nv12, location), place, "{location}");
            assert_eq!(siting(P010, location), place, "{location}");
        }
        // 4:2:2: across only, left or centred.
        for (location, across) in [(0, 0.0), (1, 0.0), (2, 0.5), (3, 0.0), (4, 0.5), (5, 0.0)] {
            assert_eq!(siting(P216, location), [across, 0.0], "{location}");
        }
        assert_eq!(siting(P416, 2), [0.0, 0.0]);
        assert_eq!(siting(Rgba64, 2), [0.0, 0.0]);
        // Given to the card after the matrix.
        let mut centred = layout(8, 8, Nv12, 8);
        centred.chroma_location = 2;
        let values = picture_params([0.0; 4], centred, Texel::Float, YuvCoding::default(), 0);
        assert_eq!(values[20..24], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(values[24..28], [0.0, 0.0, 1.0, 0.0], "Y'CbCr without alpha");
    }

    #[test]
    fn how_much_of_a_second_picture_is_mixed_in_is_said_after_its_siting_and_is_nothing_at_first() {
        let alone = layout(8, 8, PictureFormat::Nv12, 8);
        let plain = picture_params([0.0; 4], alone, Texel::Float, YuvCoding::default(), 0);
        assert_eq!(plain[BETWEEN_PARAM], 0.0);
        let mut mixed = plain;
        set_between(&mut mixed, 0.25);
        assert_eq!(mixed[20..24], [0.0, 0.5, 0.25, 0.0]);
        // Nothing else of it is changed.
        assert_eq!(mixed[..BETWEEN_PARAM], plain[..BETWEEN_PARAM]);
        assert_eq!(mixed[BETWEEN_PARAM + 1..], plain[BETWEEN_PARAM + 1..]);
    }

    #[test]
    fn r_g_b_a_is_told_from_y_cb_cr_with_its_alpha_and_levels() {
        let full = YuvCoding {
            range: ColorRange::Full,
            ..YuvCoding::default()
        };
        let mut still = layout(6, 4, PictureFormat::Rgba64, 8);
        still.alpha = Alpha::Straight;
        let values = picture_params([0.0; 4], still, Texel::Float, full, 0);
        assert_eq!(values[4..8], [6.0, 4.0, 0.0, 0.0]);
        // Black 0 and white 1, of full range.
        assert_eq!(values[9..11], [0.0, 1.0]);
        assert_eq!(values[24..28], [1.0, 1.0, 1.0, 0.0]);
        // As integers, a word at full scale is a 65535th.
        let integers = picture_params([0.0; 4], still, Texel::Uint, full, 0);
        assert_eq!(integers[26], 1.0 / 65535.0);
        // Of limited range: 16/255 and 219/255 of 8 bits, 64/1023 and 876/1023 of 10.
        let limited = picture_params([0.0; 4], still, Texel::Float, YuvCoding::default(), 0);
        assert!((limited[9] - 16.0 / 255.0).abs() < 1e-7);
        assert!((limited[10] - 219.0 / 255.0).abs() < 1e-7);
        still.bits = 10;
        let ten = picture_params([0.0; 4], still, Texel::Float, YuvCoding::default(), 0);
        assert!((ten[9] - 64.0 / 1023.0).abs() < 1e-7);
        // Floats are drawn as they are, of any range, read as floats on any card.
        let mut hot = layout(6, 4, PictureFormat::RgbaF16, 16);
        hot.alpha = Alpha::Premultiplied;
        assert_eq!(hot.texel(false), Texel::Float);
        assert_eq!(still.texel(false), Texel::Uint);
        let floats = picture_params([0.0; 4], hot, Texel::Float, YuvCoding::default(), 0);
        assert_eq!(floats[9..11], [0.0, 1.0]);
        assert_eq!(floats[24..28], [2.0, 2.0, 1.0, 0.0]);
        // Alpha of Y'CbCr is a plane of its own; of R'G'B'A, not.
        let mut prores = layout(6, 4, PictureFormat::P416, 12);
        assert!(!prores.alpha_plane());
        prores.alpha = Alpha::Straight;
        assert!(prores.alpha_plane() && !hot.alpha_plane());
        let alpha = picture_params([0.0; 4], prores, Texel::Uint, full, 0);
        assert_eq!(alpha[24..27], [0.0, 1.0, 1.0 / 65535.0]);
    }

    #[test]
    fn the_planes_of_r_g_b_a_and_of_alpha_are_checked() {
        let mut still = NativePicture {
            width: 5,
            height: 3,
            format: PictureFormat::Rgba64,
            bits: 8,
            luma: vec![0; 64 * 3],
            luma_stride: 64,
            ..NativePicture::default()
        };
        assert_eq!(check(&still).unwrap().chroma_size(), (0, 0));
        still.luma_stride = 32;
        assert!(
            check(&still).is_err(),
            "rows of 32 bytes hold no 5 pixels of 8"
        );
        still.luma_stride = 64;
        still.bits = 7;
        assert!(check(&still).is_err());
        still.format = PictureFormat::RgbaF16;
        still.bits = 16;
        assert!(check(&still).is_ok());
        still.bits = 8;
        assert!(check(&still).is_err(), "half floats are of 16 bits");
        // Y'CbCr with alpha has a plane of it, laid out as luma.
        let mut prores = nv12();
        prores.alpha = Alpha::Straight;
        assert!(check(&prores).is_err(), "no plane of alpha");
        prores.alpha_plane = vec![0; 64 * 8];
        assert!(check(&prores).unwrap().alpha_plane());
        prores.alpha_plane.truncate(64 * 7);
        assert!(check(&prores).is_err());
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
