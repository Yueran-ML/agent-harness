//! The LUTs that nodes name, read by the LUT crate (`columbina_lut`) and put on the card: the
//! [`Luts`] of a compositor.
//!
//! A name is read when it is first met, or when [`CardLuts::prepare`] is called for a grade
//! (as a grade changes, not every frame): the cache of the LUT crate reads a file once, and
//! again only once it changes. The textures of a LUT are made once for each content (by the
//! digest of its file) and kept, so that the same LUT in many nodes, or under two names, is
//! on the card once; and those used longest ago are let go while all take more than a budget
//! ([`TEXTURE_BUDGET`] at first, [`CardLuts::set_budget`]), as the cache of the LUT crate does
//! its tables (`columbina_lut::LutCache`), to be made again if they are asked for again. The
//! textures go from the card once no grade made ready with them holds them still. A name that cannot be read (no such file, a file that is not a LUT) leaves
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

/// How many bytes of textures of LUTs a [`CardLuts`] keeps at first: some 300 cubes of 65³, or
/// 2000 of 33³.
pub const TEXTURE_BUDGET: u64 = 256 << 20;

/// A LUT as a node reads it: its table, its code, and the digest that its textures are kept
/// by.
#[derive(Clone)]
struct Named {
    lut: Arc<Lut>,
    digest: String,
    body: Arc<str>,
    params: Vec<[f32; 4]>,
}

/// Things kept by a key, each of so many bytes, those used longest ago let go while all take
/// more than a budget.
#[derive(Debug)]
struct Kept<T> {
    entries: HashMap<String, (T, u64, u64)>,
    clock: u64,
    bytes: u64,
}

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Kept {
            entries: HashMap::new(),
            clock: 0,
            bytes: 0,
        }
    }
}

impl<T: Clone> Kept<T> {
    /// What is kept by `key`, now the last used.
    fn get(&mut self, key: &str) -> Option<T> {
        self.clock += 1;
        let (value, _, used) = self.entries.get_mut(key)?;
        *used = self.clock;
        Some(value.clone())
    }

    /// Keep `value`, of `bytes`, by `key`, now the last used; and let go of the others used
    /// longest ago while all take more than `budget`.
    fn put(&mut self, key: &str, value: T, bytes: u64, budget: u64) {
        self.clock += 1;
        if let Some((_, was, _)) = self
            .entries
            .insert(key.to_owned(), (value, bytes, self.clock))
        {
            self.bytes -= was;
        }
        self.bytes += bytes;
        self.evict(budget, key);
    }

    /// Let go of those used longest ago but `keep` while all take more than `budget`.
    fn evict(&mut self, budget: u64, keep: &str) {
        while self.bytes > budget {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(key, _)| key.as_str() != keep)
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            if let Some((_, bytes, _)) = self.entries.remove(&oldest) {
                self.bytes -= bytes;
            }
        }
    }
}

/// The LUTs of the grades of a compositor, on its card.
pub struct CardLuts {
    gpu: Arc<Gpu>,
    cache: Arc<LutCache>,
    roots: Roots,
    interpolation: Interpolation,
    /// Each name met: its LUT and code, or why it could not be read.
    named: Mutex<HashMap<String, Result<Named, String>>>,
    /// The textures of each content of a LUT, by its digest.
    textures: Mutex<Kept<Vec<OpTexture>>>,
    budget: u64,
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
            textures: Mutex::new(Kept::default()),
            budget: TEXTURE_BUDGET,
        }
    }

    /// Keep textures of at most about `budget` bytes from now on (those of one LUT, however
    /// large, are kept while it is used).
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        let mut textures = self.lock_textures();
        textures.evict(budget, "");
    }

    /// The bytes of the textures kept, and how many LUTs they are of.
    #[must_use]
    pub fn texture_bytes(&self) -> (u64, usize) {
        let textures = self.lock_textures();
        (textures.bytes, textures.entries.len())
    }

    fn lock_textures(&self) -> std::sync::MutexGuard<'_, Kept<Vec<OpTexture>>> {
        self.textures.lock().unwrap_or_else(PoisonError::into_inner)
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
        self.textures_of(&loaded.digest, || data);
        Ok(Named {
            lut: loaded.lut,
            digest: loaded.digest,
            body: body.into(),
            params,
        })
    }

    /// The textures of the LUT whose content is of `digest`: those kept, or those of the data
    /// that `data` gives, put on the card and kept.
    fn textures_of(&self, digest: &str, data: impl FnOnce() -> Vec<TextureData>) -> Vec<OpTexture> {
        if let Some(textures) = self.lock_textures().get(digest) {
            return textures;
        }
        let data = data();
        let bytes = data.iter().map(|data| data.bytes.len() as u64).sum();
        let textures: Vec<OpTexture> = data.iter().map(|data| self.upload(data)).collect();
        self.lock_textures()
            .put(digest, textures.clone(), bytes, self.budget);
        textures
    }

    /// The operation of the LUT `named`, its textures kept or made again.
    fn op(&self, named: &Named) -> ColorOp {
        let textures = self.textures_of(&named.digest, || {
            named.lut.gpu(self.interpolation, FORMAT).textures
        });
        ColorOp {
            body: Arc::clone(&named.body),
            library: WGSL.into(),
            params: named.params.clone(),
            textures,
            neighbourhoods: Vec::new(),
            spatial: false,
        }
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
        self.named(name).map(|named| self.op(&named))
    }
}

#[cfg(test)]
mod tests {
    use super::Kept;

    #[test]
    fn what_was_used_longest_ago_goes_first_while_all_take_more_than_the_budget() {
        let mut kept: Kept<&str> = Kept::default();
        kept.put("a", "A", 40, 100);
        kept.put("b", "B", 40, 100);
        assert_eq!(kept.bytes, 80);
        // a is used again: b is then the one used longest ago.
        assert_eq!(kept.get("a"), Some("A"));
        kept.put("c", "C", 40, 100);
        assert_eq!(kept.bytes, 80);
        assert_eq!(kept.get("b"), None);
        assert_eq!(kept.get("a"), Some("A"));
        assert_eq!(kept.get("c"), Some("C"));
        // One larger than the budget is kept while it is the last used, alone.
        kept.put("d", "D", 150, 100);
        assert_eq!((kept.bytes, kept.entries.len()), (150, 1));
        assert_eq!(kept.get("d"), Some("D"));
        // The same key again counts once.
        kept.put("d", "D", 150, 1000);
        assert_eq!(kept.bytes, 150);
        // A smaller budget lets go of all but what is to be kept.
        kept.put("e", "E", 10, 1000);
        kept.evict(10, "e");
        assert_eq!((kept.bytes, kept.entries.len()), (10, 1));
        kept.evict(0, "");
        assert_eq!((kept.bytes, kept.entries.len()), (0, 0));
    }
}
