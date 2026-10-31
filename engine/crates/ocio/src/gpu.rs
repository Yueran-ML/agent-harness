//! A processor as an operation of colour for the card: plain data that the compositor wraps in
//! its `ColorOp` (as it wraps a LUT of `columbina-lut`).
//!
//! # Two paths
//!
//! 1. **Translated** (the default): OCIO's own shader, which computes every transform (the camera
//!    curves, PQ and HLG, the matrices, the ACES 2.0 output transforms) the way its processor
//!    does, in Vulkan GLSL, made WGSL by naga (see `translate.rs`). Its tables (OCIO's 1D LUTs,
//!    and the tables of the ACES 2.0 output transforms) are textures; the operation's body calls
//!    the main function of its library with the compositor's textures and sampler:
//!    `rgb = ocio_…_main(vec4<f32>(rgb, 1.0), $T0, $S).rgb;`. Its library is named with a prefix
//!    of the processor's own (from OCIO's cache identifier), so libraries of different processors
//!    do not meet in one shader, and the same processor's are the same text, put in once.
//! 2. **Baked**, where the shader cannot be translated: the processor on the processor, sampled
//!    into a cube of `columbina-lut` ([`Bake`]), after a shaper where the input is linear. A cube
//!    loses what is between its points, and clips what is beyond its domain (as Resolve says of
//!    LUTs, p. 225); it is the fallback, and [`GpuOp::path`] says which path an operation took.
//!
//! # Textures
//!
//! [`TextureData`] of `columbina-lut`: OCIO's 1D tables are 2D textures of at most
//! [`MAX_TEXTURE_WIDTH`] texels a row (as OCIO lays them out, so its shader reads them as it
//! wrote them), its 3D tables 3D textures, red along x. `Rgba16Float` by default, as the
//! compositor binds filterable floats; a table of one channel has it in R, G and B.
//!
//! The textures are sampled through the compositor's one sampler (linear, clamped to the edge):
//! OCIO samples its tables that it wants read to the nearest texel at the centres of their
//! texels, where a linear sampler gives the texel itself.

use columbina_lut::gpu::{Dimension, TexelFormat, TextureData};
use columbina_lut::{half, Domain, Interpolation, Lut, Lut3d};

use crate::translate::{self, Resource, TranslateError};
use crate::{Error, Processor};

/// The most texels in a row of the texture of a 1D table.
pub const MAX_TEXTURE_WIDTH: u32 = 4096;

/// OCIO's shader of a processor, as it hands it over (Vulkan GLSL 4.6).
#[derive(Clone, Debug)]
pub struct Glsl {
    pub text: String,
    /// The uniforms it declares: only OCIO's dynamic properties make them, and none of the
    /// processors of the engine has one.
    pub uniforms: usize,
    pub tables: Vec<Table1d>,
    pub cubes: Vec<Table3d>,
}

/// A 1D table of OCIO's, laid out as a 2D texture.
#[derive(Clone, Debug)]
pub struct Table1d {
    /// The name the shader reads it by (OCIO's name of its sampler).
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// 1 (red) or 3 (RGB).
    pub channels: u32,
    /// Whether OCIO reads it to the nearest texel rather than linearly.
    pub nearest: bool,
    pub values: Vec<f32>,
}

/// A 3D table of OCIO's.
#[derive(Clone, Debug)]
pub struct Table3d {
    pub name: String,
    pub edge: u32,
    pub nearest: bool,
    /// RGB, red fastest.
    pub values: Vec<f32>,
}

impl From<crate::ffi::RawShaderInfo> for Glsl {
    fn from(raw: crate::ffi::RawShaderInfo) -> Glsl {
        Glsl {
            text: raw.text,
            uniforms: raw.uniforms,
            tables: raw
                .textures
                .into_iter()
                .map(|t| Table1d {
                    name: t.sampler,
                    width: t.width,
                    height: t.height,
                    channels: t.channels,
                    nearest: t.nearest,
                    values: t.values,
                })
                .collect(),
            cubes: raw
                .textures_3d
                .into_iter()
                .map(|t| Table3d {
                    name: t.sampler,
                    edge: t.edge,
                    nearest: t.nearest,
                    values: t.values,
                })
                .collect(),
        }
    }
}

/// Which path an operation took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuPath {
    /// OCIO's shader, translated.
    Translated,
    /// A cube baked on the processor: because the shader could not be translated (why), or
    /// because it was asked for ([`Processor::baked`]).
    Baked(Option<TranslateError>),
}

/// An operation of colour for the compositor: `body` changes `rgb` (a `var` of `vec3<f32>`),
/// reading `param($P + k)` (the `k`th of `params`), `$T0`, `$T1`… (`textures`) and `$S` (the
/// compositor's sampler); `library` is put once before it.
#[derive(Clone, Debug)]
pub struct GpuOp {
    pub body: String,
    pub library: String,
    pub params: Vec<[f32; 4]>,
    pub textures: Vec<TextureData>,
    pub path: GpuPath,
}

/// How a processor is baked when its shader cannot be translated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bake {
    pub shaper: Shaper,
    /// The points on an edge of the cube.
    pub size: usize,
    pub interpolation: Interpolation,
}

impl Default for Bake {
    /// A cube of 65 with tetrahedral interpolation (what HDR needs at the least, research §3.1),
    /// over 0 to 1.
    fn default() -> Bake {
        Bake {
            shaper: Shaper::Range { min: 0.0, max: 1.0 },
            size: 65,
            interpolation: Interpolation::Tetrahedral,
        }
    }
}

/// What the input of a baked processor is spread over before its cube.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shaper {
    /// Nothing: the cube spans `min` to `max` (a log or video encoding: 0 to 1).
    Range { min: f32, max: f32 },
    /// Linear light: its log₂ from 2⁻¹² to 2⁸ onto 0 to 1, below 2⁻¹² a straight line of the
    /// same slope, so that 0 is in the cube too (and what is below 0 is clipped to it).
    Log,
}

/// The log shaper: its least stop, its stops, and where 0 falls.
const LOG_LEAST: f64 = -12.0;
const LOG_STOPS: f64 = 20.0;

impl Shaper {
    fn domain(self) -> Domain {
        match self {
            Shaper::Range { min, max } => Domain::uniform(min, max),
            #[allow(clippy::cast_possible_truncation)]
            Shaper::Log => Domain::uniform(log_encode(0.0) as f32, 1.0),
        }
    }

    fn decode(self, u: f64) -> f64 {
        match self {
            Shaper::Range { .. } => u,
            Shaper::Log => log_decode(u),
        }
    }

    /// WGSL statements that shape `rgb`, the same as [`Shaper::decode`] undoes.
    fn wgsl(self) -> String {
        match self {
            Shaper::Range { .. } => String::new(),
            Shaper::Log => {
                let least = LOG_LEAST.exp2();
                format!(
                    "{{\n        let least = vec3<f32>({least:?});\n        \
                     let above = (log2(max(rgb, least)) - ({LOG_LEAST:?})) / {LOG_STOPS:?};\n        \
                     let below = (rgb - least) / ({:?});\n        \
                     rgb = select(below, above, rgb >= least);\n    }}\n",
                    least * std::f64::consts::LN_2 * LOG_STOPS
                )
            }
        }
    }
}

fn log_encode(x: f64) -> f64 {
    let least = LOG_LEAST.exp2();
    if x >= least {
        (x.log2() - LOG_LEAST) / LOG_STOPS
    } else {
        (x - least) / (least * std::f64::consts::LN_2 * LOG_STOPS)
    }
}

fn log_decode(u: f64) -> f64 {
    let least = LOG_LEAST.exp2();
    if u >= 0.0 {
        (u * LOG_STOPS + LOG_LEAST).exp2()
    } else {
        least + u * least * std::f64::consts::LN_2 * LOG_STOPS
    }
}

/// What to make of a processor for the card.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuOptions {
    pub format: TexelFormat,
    /// How to bake it if it cannot be translated.
    pub bake: Bake,
}

impl Processor {
    /// The operation of the card that does what the processor does: OCIO's shader translated,
    /// or, where that fails, a cube baked as `options.bake` says.
    ///
    /// # Errors
    /// OCIO's, or a cube that cannot be made (a table of no finite values).
    pub fn gpu(&self, options: &GpuOptions) -> Result<GpuOp, Error> {
        match self.translated(options.format)? {
            Ok(op) => Ok(op),
            Err(reason) => {
                let mut op = self.baked(&options.bake, options.format)?;
                op.path = GpuPath::Baked(Some(reason));
                Ok(op)
            }
        }
    }

    /// OCIO's shader translated into WGSL, or why it could not be.
    ///
    /// # Errors
    /// OCIO's, when it cannot make the shader at all.
    pub fn translated(&self, format: TexelFormat) -> Result<Result<GpuOp, TranslateError>, Error> {
        let prefix = prefix(&self.cache_id());
        let glsl = self.glsl(&prefix)?;
        Ok(from_glsl(&glsl, &format!("{prefix}main"), format))
    }

    /// The processor sampled into a cube on the processor.
    ///
    /// # Errors
    /// OCIO's, or a cube that cannot be made.
    #[allow(clippy::cast_precision_loss)]
    pub fn baked(&self, bake: &Bake, format: TexelFormat) -> Result<GpuOp, Error> {
        let domain = bake.shaper.domain();
        let size = bake.size;
        let mut inputs = Vec::with_capacity(size * size * size);
        let last = (size.max(2) - 1) as f64;
        let at = |i: usize| {
            let (min, max) = (f64::from(domain.min[0]), f64::from(domain.max[0]));
            #[allow(clippy::cast_possible_truncation)]
            let x = bake.shaper.decode(min + (max - min) * i as f64 / last) as f32;
            x
        };
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    inputs.push([at(r), at(g), at(b)]);
                }
            }
        }
        self.apply(&mut inputs)?;
        for value in inputs.iter_mut().flatten() {
            if !value.is_finite() {
                *value = if value.is_nan() {
                    0.0
                } else {
                    value.signum() * f32::MAX
                };
            }
        }
        let cube = Lut3d::new(size, domain, inputs).map_err(|e| Error::Shader(e.to_string()))?;
        let lut = Lut::new(None, None, Some(cube)).map_err(|e| Error::Shader(e.to_string()))?;
        let gpu = lut.gpu(bake.interpolation, format);
        Ok(GpuOp {
            body: format!("{}{}", bake.shaper.wgsl(), gpu.body),
            library: columbina_lut::gpu::WGSL.to_owned(),
            params: gpu.params,
            textures: gpu.textures,
            path: GpuPath::Baked(None),
        })
    }
}

/// The prefix of the names of a processor's shader: `ocio_` and a hash of its cache identifier.
fn prefix(cache_id: &str) -> String {
    // FNV-1a, 64 bits: names, not security.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in cache_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("ocio_{hash:016x}_")
}

/// The operation of OCIO's shader `glsl`, whose main function is `function`.
///
/// # Errors
/// Why it could not be translated.
pub fn from_glsl(
    glsl: &Glsl,
    function: &str,
    format: TexelFormat,
) -> Result<GpuOp, TranslateError> {
    if glsl.uniforms > 0 {
        return Err(TranslateError::Uniforms(glsl.uniforms));
    }
    let split = format == TexelFormat::Rgba16Float;
    let translated = translate::translate(&glsl.text, function, split)?;
    let mut textures = Vec::new();
    let mut names = Vec::new();
    let mut arguments = Vec::new();
    for resource in &translated.resources {
        match resource {
            Resource::Sampler(_) => arguments.push("$S".to_owned()),
            Resource::Texture(name) => {
                let index = if let Some(index) = names.iter().position(|n| n == name) {
                    index
                } else {
                    let (base, part) = match name.strip_suffix(translate::LOW) {
                        Some(base) => (base, Part::Low),
                        None if split => (name.as_str(), Part::High),
                        None => (name.as_str(), Part::Whole),
                    };
                    let data = if let Some(table) = glsl.tables.iter().find(|t| t.name == base) {
                        texture_1d(table, format, part)
                    } else if let Some(cube) = glsl.cubes.iter().find(|t| t.name == base) {
                        texture_3d(cube, format, part)
                    } else {
                        return Err(TranslateError::Texture(name.clone()));
                    };
                    textures.push(data);
                    names.push(name.clone());
                    names.len() - 1
                };
                arguments.push(format!("$T{index}"));
            }
        }
    }
    let mut call = format!("rgb = {function}(vec4<f32>(rgb, 1.0)");
    for argument in arguments {
        call.push_str(", ");
        call.push_str(&argument);
    }
    call.push_str(").rgb;");
    Ok(GpuOp {
        body: call,
        library: translated.library,
        params: Vec::new(),
        textures,
        path: GpuPath::Translated,
    })
}

/// Which part of a table's values a texture keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The values as they are (`Rgba32Float`).
    Whole,
    /// The half float nearest each value.
    High,
    /// What that half float lacks of the value, as a half float.
    Low,
}

fn push(bytes: &mut Vec<u8>, format: TexelFormat, part: Part, rgb: [f32; 3]) {
    for value in [rgb[0], rgb[1], rgb[2], 1.0] {
        let value = match part {
            Part::Whole | Part::High => value,
            Part::Low if value.abs() > half::MAX => 0.0,
            Part::Low => value - half::round(value),
        };
        match format {
            TexelFormat::Rgba16Float => {
                let bits = half::to_bits(value.clamp(-half::MAX, half::MAX));
                bytes.extend_from_slice(&bits.to_le_bytes());
            }
            TexelFormat::Rgba32Float => bytes.extend_from_slice(&value.to_le_bytes()),
        }
    }
}

fn texture_1d(table: &Table1d, format: TexelFormat, part: Part) -> TextureData {
    let texels = table.width as usize * table.height as usize;
    let mut bytes = Vec::with_capacity(texels * format.texel_bytes());
    let channels = table.channels.max(1) as usize;
    for i in 0..texels {
        let at = |c: usize| {
            table
                .values
                .get(i * channels + c.min(channels - 1))
                .copied()
                .unwrap_or(0.0)
        };
        push(&mut bytes, format, part, [at(0), at(1), at(2)]);
    }
    TextureData {
        dimension: Dimension::D2,
        format,
        width: table.width,
        height: table.height,
        depth: 1,
        bytes,
    }
}

fn texture_3d(cube: &Table3d, format: TexelFormat, part: Part) -> TextureData {
    let texels = (cube.edge as usize).pow(3);
    let mut bytes = Vec::with_capacity(texels * format.texel_bytes());
    for i in 0..texels {
        let at = |c: usize| cube.values.get(i * 3 + c).copied().unwrap_or(0.0);
        push(&mut bytes, format, part, [at(0), at(1), at(2)]);
    }
    TextureData {
        dimension: Dimension::D3,
        format,
        width: cube.edge,
        height: cube.edge,
        depth: cube.edge,
        bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_shaper_is_undone_by_its_inverse() {
        for x in [0.0, 1e-5, 0.000_244_140_625, 0.18, 1.0, 100.0, 256.0] {
            let u = log_encode(x);
            assert!((log_decode(u) - x).abs() <= 1e-12 * x.max(1.0), "{x}");
        }
        assert!((log_encode(256.0) - 1.0).abs() < 1e-12);
        assert!(log_encode(0.0) < 0.0);
    }

    #[test]
    fn prefixes_are_names_of_wgsl() {
        let p = prefix("$abc");
        assert!(p.starts_with("ocio_") && p.ends_with('_') && p.len() == 22);
        assert_ne!(p, prefix("$abd"));
    }
}
