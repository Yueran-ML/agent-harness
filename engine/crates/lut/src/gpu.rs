//! A LUT for the card, as plain data: the WGSL of an operation that applies it, its parameters,
//! and the bytes of its textures. The compositor uploads the textures and wraps the rest in its
//! `ColorOp` (its `body`, `params` and `textures`, with [`WGSL`] as its `library`); this crate
//! knows nothing of the card.
//!
//! # Textures
//!
//! - The cube is a 3D texture of `size`³ texels, red along x, green along y, blue along z: the
//!   rows of the `.cube` in their order, so the bytes are the table as it is, with an alpha of 1.
//! - The 1D table is a **2D** texture of rows of at most [`SHAPER_WIDTH`] points (point `i` at
//!   `(i % width, i / width)`), not a 1D one: a card's 1D textures may be as short as 8192
//!   texels (the default limit of wgpu), and a 1D table may have 65536 points.
//!
//! # Format: `Rgba16Float`, by default
//!
//! The compositor binds the textures of an operation as filterable floats (it hands every
//! operation a linear sampler), which `Rgba32Float` is not without a feature that the
//! compositor does not ask for; and every frame is composed in `Rgba16Float` anyway, so a value
//! from the table is rounded to a half float as soon as it is written to the canvas. A half
//! keeps 11 bits of significand: a value within 2⁻¹² of itself (2.4·10⁻⁴ at 1, a quarter of a
//! code of 10 bits), which is the rounding the canvas makes in any case. It halves the memory
//! (65³: 2.2 MB rather than 4.4 MB; 256³: 134 MB rather than 268 MB) and what a frame reads.
//! Values beyond the largest half (65504) are clamped to it. [`TexelFormat::Rgba32Float`] is
//! there for a binding that is not filterable (the test of this crate holds the WGSL to the
//! reference with it, to 1e-4); with halves, the reference is [`crate::Lut::rounded_to_half`].
//!
//! # Interpolation
//!
//! Every point is loaded with `textureLoad` and weighed in the shader: hardware filtering
//! weighs with as few as 8 bits of fraction, and is trilinear only. Input is placed from 0 to
//! `size − 1` in the texels' own indices (not in normalized coordinates, so there is no half
//! texel to mind), clamped to the domain.
//!
//! The card computes in `f32` where the reference computes in `f64`: the input is placed among
//! the points to within about `size · 2⁻²⁴`, so the two agree to that times the slope of the
//! table (a few 10⁻⁶ for the LUTs of the test, a log shaper of 1024 points and cubes of 17 and
//! 33; a 1D table of random points, steep from point to point, would differ by more).

use std::fmt::Write as _;

use crate::{half, Domain, Interpolation, Lut};

/// The WGSL functions that the bodies call, one text for every LUT: `lut_1d`,
/// `lut_3d_tetrahedral`, `lut_3d_trilinear` (and `lut_place`, `lut_cell`, `LutCell`).
pub const WGSL: &str = include_str!("lut.wgsl");

/// The most points in a row of the texture of a 1D table.
pub const SHAPER_WIDTH: usize = 4096;

/// How the texels of a texture are kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TexelFormat {
    /// `Rgba16Float`: 8 bytes a texel.
    #[default]
    Rgba16Float,
    /// `Rgba32Float`: 16 bytes a texel.
    Rgba32Float,
}

impl TexelFormat {
    /// The bytes of a texel.
    #[must_use]
    pub fn texel_bytes(self) -> usize {
        match self {
            TexelFormat::Rgba16Float => 8,
            TexelFormat::Rgba32Float => 16,
        }
    }

    fn push(self, bytes: &mut Vec<u8>, [r, g, b]: [f32; 3]) {
        match self {
            TexelFormat::Rgba16Float => {
                for value in [r, g, b, 1.0] {
                    let bits = half::to_bits(value.clamp(-half::MAX, half::MAX));
                    bytes.extend_from_slice(&bits.to_le_bytes());
                }
            }
            TexelFormat::Rgba32Float => {
                for value in [r, g, b, 1.0] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}

/// The dimension of a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dimension {
    /// `texture_2d<f32>`: a 1D table, in rows.
    D2,
    /// `texture_3d<f32>`: a cube.
    D3,
}

/// The texels of a texture, to be written to it whole (`width × height × depth`, x fastest,
/// little-endian, each row `width × texel bytes` long, with no padding).
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub dimension: Dimension,
    pub format: TexelFormat,
    pub width: u32,
    pub height: u32,
    /// 1 for a 2D texture.
    pub depth: u32,
    pub bytes: Vec<u8>,
}

impl TextureData {
    /// The bytes of a row of texels.
    #[must_use]
    pub fn bytes_per_row(&self) -> u32 {
        // At most 4096 texels of 16 bytes.
        u32::try_from(self.width as usize * self.format.texel_bytes()).unwrap_or(u32::MAX)
    }
}

/// An operation of colour that applies a LUT, for the compositor to wrap.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuLut {
    /// WGSL statements that change `rgb` (a `var` of `vec3<f32>`), calling the functions of
    /// [`WGSL`]; `param($P + k)` is the `k`th of `params` and `$T0`, `$T1` the `textures`, as
    /// the compositor's operations write them. The same text for every LUT of the same shape
    /// (which tables it has, and the interpolation), so that LUTs share a shader.
    pub body: String,
    /// Two for each table: the lower bounds of its domain with the index of its last point,
    /// and one over the width of its domain.
    pub params: Vec<[f32; 4]>,
    /// The 1D table first, if there is one, then the cube.
    pub textures: Vec<TextureData>,
}

/// The body of a LUT with a 1D table (`shaper`), a cube, or both, as an operation of its own.
#[must_use]
pub fn body(shaper: bool, cube: bool, interpolation: Interpolation) -> String {
    body_at(shaper, cube, interpolation, 0, 0)
}

/// The body of a LUT whose textures are `$T{first_texture}` on and whose parameters are
/// `param($P + {first_param}u)` on: for a LUT put into an operation that reads textures and
/// parameters of its own before it (a corrector node, whose LUT comes after its Hue and
/// Saturation, p. 3331), with [`GpuLut::params`] and [`GpuLut::textures`] appended to the
/// operation's.
#[must_use]
pub fn body_at(
    shaper: bool,
    cube: bool,
    interpolation: Interpolation,
    first_texture: usize,
    first_param: usize,
) -> String {
    let mut body = String::new();
    let mut texture = first_texture;
    let mut param = first_param;
    let mut call = |function: &str| {
        let p = if param == 0 {
            "$P".to_owned()
        } else {
            format!("$P + {param}u")
        };
        let _ = writeln!(
            body,
            "rgb = {function}($T{texture}, rgb, param({p}), param($P + {}u));",
            param + 1
        );
        texture += 1;
        param += 2;
    };
    if shaper {
        call("lut_1d");
    }
    if cube {
        call(match interpolation {
            Interpolation::Tetrahedral => "lut_3d_tetrahedral",
            Interpolation::Trilinear => "lut_3d_trilinear",
        });
    }
    body
}

fn params_of(domain: &Domain, size: usize) -> [[f32; 4]; 2] {
    let mut lo = [0.0; 4];
    let mut inv = [0.0; 4];
    for c in 0..3 {
        lo[c] = domain.min[c];
        let width = f64::from(domain.max[c]) - f64::from(domain.min[c]);
        inv[c] = crate::to_f32(1.0 / width);
    }
    lo[3] = crate::to_f32(crate::to_f64(size - 1));
    [lo, inv]
}

fn dimension(n: usize) -> u32 {
    // Sizes are at most 65536 points.
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The texture of a 1D table.
#[must_use]
pub fn shaper_texture(table: &[[f32; 3]], format: TexelFormat) -> TextureData {
    let width = table.len().clamp(1, SHAPER_WIDTH);
    let height = table.len().div_ceil(width).max(1);
    let mut bytes = Vec::with_capacity(width * height * format.texel_bytes());
    let last = table.last().copied().unwrap_or([0.0; 3]);
    for i in 0..width * height {
        // The end of the last row is the last point again, never read.
        format.push(&mut bytes, table.get(i).copied().unwrap_or(last));
    }
    TextureData {
        dimension: Dimension::D2,
        format,
        width: dimension(width),
        height: dimension(height),
        depth: 1,
        bytes,
    }
}

/// The texture of a cube of `size` points on an edge.
#[must_use]
pub fn cube_texture(size: usize, table: &[[f32; 3]], format: TexelFormat) -> TextureData {
    let mut bytes = Vec::with_capacity(table.len() * format.texel_bytes());
    for row in table {
        format.push(&mut bytes, *row);
    }
    TextureData {
        dimension: Dimension::D3,
        format,
        width: dimension(size),
        height: dimension(size),
        depth: dimension(size),
        bytes,
    }
}

impl Lut {
    /// The operation that applies it on the card.
    #[must_use]
    pub fn gpu(&self, interpolation: Interpolation, format: TexelFormat) -> GpuLut {
        let mut params = Vec::new();
        let mut textures = Vec::new();
        if let Some(shaper) = self.shaper() {
            params.extend(params_of(&shaper.domain(), shaper.size()));
            textures.push(shaper_texture(shaper.table(), format));
        }
        if let Some(cube) = self.cube() {
            params.extend(params_of(&cube.domain(), cube.size()));
            textures.push(cube_texture(cube.size(), cube.table(), format));
        }
        GpuLut {
            body: body(
                self.shaper().is_some(),
                self.cube().is_some(),
                interpolation,
            ),
            params,
            textures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lut1d, Lut3d};

    #[test]
    fn bodies_are_of_the_shape_of_the_lut_only() {
        assert_eq!(
            body(true, true, Interpolation::Tetrahedral),
            "rgb = lut_1d($T0, rgb, param($P), param($P + 1u));\n\
             rgb = lut_3d_tetrahedral($T1, rgb, param($P + 2u), param($P + 3u));\n"
        );
        assert_eq!(
            body(false, true, Interpolation::Trilinear),
            "rgb = lut_3d_trilinear($T0, rgb, param($P), param($P + 1u));\n"
        );
        assert_eq!(
            body(true, false, Interpolation::Trilinear),
            "rgb = lut_1d($T0, rgb, param($P), param($P + 1u));\n"
        );
        assert_eq!(
            body_at(true, true, Interpolation::Tetrahedral, 2, 5),
            "rgb = lut_1d($T2, rgb, param($P + 5u), param($P + 6u));\n\
             rgb = lut_3d_tetrahedral($T3, rgb, param($P + 7u), param($P + 8u));\n"
        );
    }

    #[test]
    fn a_long_shaper_is_laid_in_rows_and_a_cube_as_it_is() {
        let shaper = Lut1d::from_fn(5000, Domain::uniform(-1.0, 3.0), |x| x).unwrap();
        let cube = Lut3d::from_fn(3, Domain::default(), |x| x).unwrap();
        let lut = Lut::new(None, Some(shaper), Some(cube)).unwrap();
        let gpu = lut.gpu(Interpolation::Tetrahedral, TexelFormat::Rgba32Float);
        let [shaper, cube] = &gpu.textures[..] else {
            panic!("two textures")
        };
        assert_eq!((shaper.width, shaper.height, shaper.depth), (4096, 2, 1));
        assert_eq!(shaper.bytes.len(), 4096 * 2 * 16);
        assert_eq!(shaper.bytes_per_row(), 4096 * 16);
        let texel = |data: &TextureData, i: usize| -> [f32; 4] {
            let at = i * 16;
            [0, 1, 2, 3].map(|k| {
                f32::from_le_bytes(data.bytes[at + 4 * k..at + 4 * k + 4].try_into().unwrap())
            })
        };
        // Point 4096 is the first of the second row; 4999 the last, and past it again.
        let step = 4.0 / 4999.0;
        assert!((texel(shaper, 4096)[0] - (-1.0 + 4096.0 * step)).abs() < 1e-5);
        assert_eq!(texel(shaper, 4999), [3.0, 3.0, 3.0, 1.0]);
        assert_eq!(texel(shaper, 8191), [3.0, 3.0, 3.0, 1.0]);
        assert_eq!((cube.width, cube.height, cube.depth), (3, 3, 3));
        // r + 3g + 9b: (1, 0, 2) is texel 19.
        assert_eq!(texel(cube, 19), [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(
            gpu.params,
            [
                [-1.0, -1.0, -1.0, 4999.0],
                [0.25, 0.25, 0.25, 0.0],
                [0.0, 0.0, 0.0, 2.0],
                [1.0, 1.0, 1.0, 0.0]
            ]
        );
    }

    #[test]
    fn halves_are_rounded_and_clamped() {
        let cube = Lut3d::new(2, Domain::default(), vec![[0.1, 1e6, -1e6]; 8]).unwrap();
        let data = cube_texture(2, cube.table(), TexelFormat::Rgba16Float);
        assert_eq!(data.bytes.len(), 8 * 8);
        let words: Vec<u16> = data
            .bytes
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(&words[..4], [half::to_bits(0.1), 0x7bff, 0xfbff, 0x3c00]);
        let small = shaper_texture(&[[0.0; 3], [1.0; 3]], TexelFormat::Rgba16Float);
        assert_eq!((small.width, small.height), (2, 1));
    }
}
