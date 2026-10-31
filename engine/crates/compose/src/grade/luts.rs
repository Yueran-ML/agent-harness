//! The LUTs that nodes name, read by the LUT crate (`columbina_lut`) and put on the card: the
//! [`Luts`] of a compositor.
//!
//! A name is read when it is first met, or when [`CardLuts::prepare`] is called for a grade
//! (as a grade changes, not every frame): the cache of the LUT crate reads a file once, and
//! again only once it changes. The textures of a LUT are made once for each content (by the
//! digest of its file) and kept, so that the same LUT in many nodes, or under two names, is
//! on the card once. A name that cannot be read (no such file, a file that is not a LUT) leaves
//! its node without a LUT, and the error is kept for the Color page to show, as `DaVinci`
//! marks a node whose LUT is missing (p. 3413).
//!
//! The operation of a LUT is the body of the LUT crate (the same text for every LUT of the same
//! shape and interpolation, so that swapping LUTs makes no new shader), its parameters, and its
//! textures; [`super::graph`] puts it into its node after Hue and Saturation and before the HSL
//! curves (p. 3331), with [`columbina_lut::gpu::WGSL`] as its library.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use columbina_lut::gpu::{Dimension, GpuLut, TexelFormat, TextureData, WGSL};
use columbina_lut::{Interpolation, Lut, LutCache, Roots};
use columbina_model::grade::{Grade, NodeKind};

use super::Luts;
use crate::{ColorOp, Gpu, OpTexture, TextureKind};

/// How the texels of LUTs are kept on the card: half floats, which the compositor can filter,
/// and which its canvas is of.
const FORMAT: TexelFormat = TexelFormat::Rgba16Float;

/// A LUT as a node reads it.
#[derive(Clone)]
struct Named {
    lut: Arc<Lut>,
    op: ColorOp,
}

/// The LUTs of the grades of a compositor, on its card.
pub struct CardLuts {
    gpu: Arc<Gpu>,
    cache: Arc<LutCache>,
    roots: Roots,
    interpolation: Interpolation,
    /// Each name met: its LUT and operation, or why it could not be read.
    named: Mutex<HashMap<String, Result<Named, String>>>,
    /// The textures of each content of a LUT, by its digest.
    textures: Mutex<HashMap<String, Vec<OpTexture>>>,
}

impl std::fmt::Debug for CardLuts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CardLuts")
            .field("roots", &self.roots)
            .field("interpolation", &self.interpolation)
            .finish_non_exhaustive()
    }
}

impl CardLuts {
    /// The LUTs of `roots` (the folder of the project, and the library) read through `cache`,
    /// on `gpu`, a cube read between its points as `DaVinci`'s projects do at first: trilinear
    /// (p. 152). The project's setting (T1) is to choose it with
    /// [`CardLuts::set_interpolation`].
    #[must_use]
    pub fn new(gpu: Arc<Gpu>, cache: Arc<LutCache>, roots: Roots) -> CardLuts {
        CardLuts {
            gpu,
            cache,
            roots,
            interpolation: Interpolation::Trilinear,
            named: Mutex::new(HashMap::new()),
            textures: Mutex::new(HashMap::new()),
        }
    }

    /// How cubes are read between their points.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// Read cubes between their points by `interpolation` from now on.
    pub fn set_interpolation(&mut self, interpolation: Interpolation) {
        if interpolation != self.interpolation {
            self.interpolation = interpolation;
            self.lock_named().clear();
        }
    }

    /// Look for relative names in `roots` from now on.
    pub fn set_roots(&mut self, roots: Roots) {
        if roots != self.roots {
            self.roots = roots;
            self.lock_named().clear();
        }
    }

    /// Read again the LUTs that the correctors of `grade` name (those whose file changed, or
    /// that could not be read, are read anew): to be called as the grade changes.
    pub fn prepare(&self, grade: &Grade) {
        for node in &grade.nodes {
            if let NodeKind::Corrector(corrector) = &node.kind {
                if let Some(name) = &corrector.lut {
                    let read = self.read(name);
                    self.lock_named().insert(name.clone(), read);
                }
            }
        }
    }

    /// Why the LUT `name` could not be read, if it was read and could not be.
    #[must_use]
    pub fn error(&self, name: &str) -> Option<String> {
        match self.lock_named().get(name) {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        }
    }

    /// The LUT `name`, as it was read, for the reference on the processor.
    #[must_use]
    pub fn table(&self, name: &str) -> Option<Arc<Lut>> {
        self.named(name).map(|named| named.lut)
    }

    fn lock_named(&self) -> std::sync::MutexGuard<'_, HashMap<String, Result<Named, String>>> {
        self.named.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn named(&self, name: &str) -> Option<Named> {
        if let Some(seen) = self.lock_named().get(name) {
            return seen.clone().ok();
        }
        let read = self.read(name);
        self.lock_named().insert(name.to_owned(), read.clone());
        read.ok()
    }

    fn read(&self, name: &str) -> Result<Named, String> {
        let loaded = self
            .cache
            .load_named(&self.roots, name)
            .map_err(|error| error.to_string())?;
        let GpuLut {
            body,
            params,
            textures: data,
        } = loaded.lut.gpu(self.interpolation, FORMAT);
        let textures = {
            let mut kept = self.textures.lock().unwrap_or_else(PoisonError::into_inner);
            kept.entry(loaded.digest.clone())
                .or_insert_with(|| data.iter().map(|data| self.upload(data)).collect())
                .clone()
        };
        Ok(Named {
            lut: loaded.lut,
            op: ColorOp {
                body: body.into(),
                library: WGSL.into(),
                params,
                textures,
            },
        })
    }

    fn upload(&self, data: &TextureData) -> OpTexture {
        let (dimension, kind) = match data.dimension {
            Dimension::D2 => (wgpu::TextureDimension::D2, TextureKind::D2),
            Dimension::D3 => (wgpu::TextureDimension::D3, TextureKind::D3),
        };
        let size = wgpu::Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: data.depth,
        };
        let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("columbina lut"),
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
        self.gpu.queue().write_texture(
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
}

impl Luts for CardLuts {
    fn lut(&self, name: &str) -> Option<ColorOp> {
        self.named(name).map(|named| named.op)
    }
}
