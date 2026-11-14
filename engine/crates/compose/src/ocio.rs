//! The transforms of `OpenColorIO` ([`columbina_ocio`]) as operations of a stage of colour
//! ([`ColorOp`]) on one card: the input transforms of colour management (the space of media into
//! the timeline's) and its output transforms (the timeline's into the output's, an ACES output
//! transform among them).
//!
//! The ocio crate gives an operation as plain data ([`GpuOp`]): its WGSL, its parameters and the
//! bytes of its textures (OCIO's tables, each of half floats in two textures that are added, so
//! that a table keeps about 22 bits of its values, `columbina_ocio::gpu`). Here those textures
//! are put on the card and the operation is wrapped as the compositor takes it, once for each
//! transform: [`CardOps`] keeps what it made by OCIO's identifier of what a processor does
//! ([`Processor::cache_id`]), so that two processors that do the same (a space into itself, the
//! same pair made again when a setting of the project changes) share their textures and their
//! code, and the compositor's shader of it.
//!
//! The configs are made once for the process ([`config`], [`builtin_config`]): opening the
//! studio config and adding our fragment to it takes tens of milliseconds, and a config is cheap
//! to share between threads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use columbina_lut::gpu::{Dimension, TexelFormat, TextureData};
pub use columbina_ocio::gpu::{GpuOp, GpuOptions, GpuPath};
pub use columbina_ocio::{catalogue, Config, Direction, Error, Processor, Referred};

use crate::color::{ColorOp, OpTexture, TextureKind};
use crate::Gpu;

/// The config of the engine's colour management, [`Config::columbina`]: the built-in ACES
/// studio config with the spaces of the catalogue that it lacks. Made the first time it is asked
/// for, and kept for the process.
///
/// # Errors
/// OCIO's, where the config cannot be made (which no build that passed its tests does).
pub fn config() -> Result<Config, Error> {
    static CONFIG: OnceLock<Result<Config, Error>> = OnceLock::new();
    CONFIG.get_or_init(Config::columbina).clone()
}

/// One of OCIO's built-in configs by its URI (the ACES 1.3 and 2.0 studio configs that the ODTs
/// of ACES are of, `columbina_model::color_management::AcesVersion::config`), each made once for
/// the process.
///
/// # Errors
/// OCIO's, for a URI that it does not know.
pub fn builtin_config(uri: &str) -> Result<Config, Error> {
    static CONFIGS: OnceLock<Mutex<HashMap<String, Config>>> = OnceLock::new();
    let configs = CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(config) = configs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(uri)
    {
        return Ok(config.clone());
    }
    let made = Config::open(uri)?;
    configs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(uri.to_owned(), made.clone());
    Ok(made)
}

/// What the operations of OCIO are made with: OCIO's own shader translated, its tables in pairs
/// of half floats (which the compositor binds, as filterable floats), and where a shader cannot
/// be translated, a cube of 65 baked from the processor.
#[must_use]
pub fn options() -> GpuOptions {
    GpuOptions {
        format: TexelFormat::Rgba16Float,
        ..GpuOptions::default()
    }
}

/// The operations of OCIO's processors on one card, each made once for what it does.
///
/// Shared between the threads that make the colour management of a project (a monitor, and
/// the thread that makes a new one ready while the monitor shows the old): it is `Sync`, and a
/// processor's operation is made outside its lock, so that one thread making an ACES 2.0 output
/// transform does not hold up another that finds what it asks for made.
pub struct CardOps {
    gpu: Arc<Gpu>,
    /// By OCIO's identifier of a processor: its operation, or none where it does nothing.
    made: Mutex<HashMap<String, Option<ColorOp>>>,
}

impl std::fmt::Debug for CardOps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CardOps")
            .field("made", &self.made().len())
            .finish_non_exhaustive()
    }
}

impl CardOps {
    #[must_use]
    pub fn new(gpu: Arc<Gpu>) -> CardOps {
        CardOps {
            gpu,
            made: Mutex::new(HashMap::new()),
        }
    }

    /// The card that the operations are of.
    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    fn made(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<ColorOp>>> {
        self.made.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many processors' operations were made, for the tests.
    #[must_use]
    pub fn count(&self) -> usize {
        self.made().len()
    }

    /// The operation of `processor` on the card: none where it does nothing (a space into
    /// itself). Made the first time that what it does is asked for, and then given again.
    ///
    /// # Errors
    /// OCIO's, where its shader cannot be made nor a cube baked of it.
    pub fn op(&self, processor: &Processor) -> Result<Option<ColorOp>, Error> {
        let key = processor.cache_id();
        if let Some(made) = self.made().get(&key) {
            return Ok(made.clone());
        }
        let op = if processor.is_noop() {
            None
        } else {
            Some(op_of(&self.gpu, &processor.gpu(&options())?))
        };
        Ok(self.made().entry(key).or_insert(op).clone())
    }
}

/// An operation of OCIO's as the compositor takes it, its textures put on `gpu`.
#[must_use]
#[allow(
    clippy::needless_update,
    reason = "an operation may have more than these, which an operation of OCIO has none of"
)]
pub fn op_of(gpu: &Gpu, op: &GpuOp) -> ColorOp {
    ColorOp {
        body: op.body.as_str().into(),
        library: op.library.as_str().into(),
        params: op.params.clone(),
        textures: op.textures.iter().map(|data| upload(gpu, data)).collect(),
        ..ColorOp::default()
    }
}

/// A texture of an operation's table, put on the card: 2D (OCIO's 1D tables, in rows) or 3D,
/// sampled by the compositor's sampler.
#[must_use]
pub fn upload(gpu: &Gpu, data: &TextureData) -> OpTexture {
    let (dimension, kind) = match data.dimension {
        Dimension::D2 => (wgpu::TextureDimension::D2, TextureKind::D2),
        Dimension::D3 => (wgpu::TextureDimension::D3, TextureKind::D3),
    };
    let size = wgpu::Extent3d {
        width: data.width,
        height: data.height,
        depth_or_array_layers: data.depth,
    };
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("columbina ocio"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format: match data.format {
            TexelFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            TexelFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue().write_texture(
        texture.as_image_copy(),
        &data.bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.bytes_per_row()),
            rows_per_image: Some(data.height),
        },
        size,
    );
    OpTexture {
        kind,
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configs_are_made_once_and_know_the_catalogue() {
        let made = config().unwrap();
        for space in catalogue::spaces() {
            assert!(
                made.color_space(&space.ocio).unwrap().is_some(),
                "{}",
                space.name
            );
        }
        let aces = builtin_config("ocio://studio-config-v2.2.0_aces-v1.3_ocio-v2.4").unwrap();
        assert!(aces.displays().iter().any(|d| d.contains("Rec.709")));
        assert!(builtin_config("ocio://no-such-config").is_err());
    }
}
