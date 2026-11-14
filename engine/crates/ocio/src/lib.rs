//! `OpenColorIO` (OCIO) for the engine: its configs (the built-in ACES configs, and ours on top of
//! them), its processors applied on the processor as the reference, and its shaders made into
//! WGSL for the compositor.
//!
//! # What is here
//!
//! - [`Config`]: a config opened by name (`ocio://studio-config-latest`, the built-in ACES Studio
//!   config; `ocio://cg-config-latest`), from a file, or from text; its colour spaces
//!   ([`ColorSpace`]: name, family, encoding, aliases, scene- or display-referred), its displays
//!   and views. [`Config::columbina`] is the studio config with the colour spaces of
//!   [`catalogue`] that it lacks added from our own fragment ([`catalogue::fragment`]).
//! - [`Processor`]: a transform between two colour spaces, to a display through a view (an ACES
//!   2.0 output transform), or one of OCIO's built-in transforms; applied to pixels on the
//!   processor ([`Processor::apply`]) as the reference, and made an operation for the card
//!   ([`Processor::gpu`], see [`gpu`]).
//! - [`catalogue`]: the colour spaces that the engine offers, by the names of `DaVinci Resolve`,
//!   each the colour space of OCIO that it is, with its family and whether it is scene- or
//!   display-referred.
//!
//! # Where this crate stands
//!
//! Like the crate of LUTs, it gives the compositor plain data and depends on no card: a
//! [`gpu::GpuOp`] is the WGSL of an operation (its body and library), its parameters and the
//! bytes of its textures, which `columbina-compose` uploads and wraps in its own `ColorOp`. So
//! compose depends on this crate and not the other way.
//!
//! # Building OCIO
//!
//! The crate links OCIO 2.x statically, found by the environment variable `OCIO_DIR` (as the
//! media crate finds libav by `FFMPEG_DIR`); `build.rs` says what is missing when it is not
//! there. The libraries are built with OCIO's own `CMake`, which fetches its dependencies, in
//! Release with the **static** C runtime (`/MT`), as the engine links it
//! (`engine/.cargo/config.toml`: `target-feature=+crt-static`); `cmake/msvc-static-crt.cmake`
//! makes every dependency's build use it too (expat and the projects of older `CMake` would
//! otherwise take `/MD`):
//!
//! ```text
//! vcvars64.bat
//! cmake -S OpenColorIO-2.6.0 -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
//!       -DCMAKE_INSTALL_PREFIX=install
//!       -DCMAKE_TOOLCHAIN_FILE=<engine>/crates/ocio/cmake/msvc-static-crt.cmake
//!       -DBUILD_SHARED_LIBS=OFF -DOCIO_BUILD_APPS=OFF -DOCIO_BUILD_TESTS=OFF
//!       -DOCIO_BUILD_GPU_TESTS=OFF -DOCIO_BUILD_PYTHON=OFF -DOCIO_BUILD_DOCS=OFF
//!       -DOCIO_BUILD_OPENFX=OFF -DOCIO_BUILD_JAVA=OFF -DOCIO_INSTALL_EXT_PACKAGES=ALL
//! cmake --build build --parallel 6
//! cmake --install build
//! copy build\ext\dist\lib\*.lib install\lib\
//! set OCIO_DIR=…\install
//! ```
//!
//! (About a minute and a half on a laptop of 2024 with six jobs.) `dumpbin /directives` on each
//! library shows `/DEFAULTLIB:LIBCMT`, never `MSVCRT`, when the runtime is right.
//!
//! The libraries linked are OCIO's and those of yaml-cpp, expat, Imath, pystring, minizip-ng and
//! zlib. OCIO is BSD-3-Clause; yaml-cpp and expat MIT; Imath and pystring BSD-3-Clause;
//! minizip-ng and zlib the zlib licence.

pub mod catalogue;
#[allow(unsafe_code)]
mod ffi;
pub mod gpu;
mod translate;

use std::sync::Arc;

pub use translate::TranslateError;

/// The built-in ACES Studio config, of the version that the linked OCIO has as its latest.
pub const STUDIO_CONFIG: &str = "ocio://studio-config-latest";
/// The built-in ACES CG config.
pub const CG_CONFIG: &str = "ocio://cg-config-latest";

/// What went wrong.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// OCIO's own message.
    #[error("OCIO: {0}")]
    Ocio(String),
    /// A shader that could not be made into WGSL, nor baked.
    #[error("the shader of the processor: {0}")]
    Shader(String),
}

/// The version of OCIO linked in, as `2.6.0`.
#[must_use]
pub fn version() -> String {
    ffi::version()
}

/// Whether a colour space is of the scene (light as it was) or of a display (light as it is to
/// be shown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Referred {
    Scene,
    Display,
}

/// A colour space of a config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorSpace {
    pub name: String,
    /// The family, as `Input/Sony` or `Utility`; may be empty.
    pub family: String,
    /// OCIO's encoding: `scene-linear`, `log`, `sdr-video`, `hdr-video`, `display-linear`,
    /// `data`…; may be empty.
    pub encoding: String,
    pub description: String,
    pub referred: Referred,
    /// Whether it is data, which no transform touches.
    pub data: bool,
    pub aliases: Vec<String>,
}

/// Which way a transform goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    Forward,
    Inverse,
}

/// A config of OCIO's. Cheap to clone, and to share between threads.
#[derive(Clone)]
pub struct Config {
    raw: Arc<ffi::Config>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("name", &self.name())
            .finish()
    }
}

impl Config {
    /// A config from a file, or a built-in one by its URI (`ocio://studio-config-latest`).
    ///
    /// # Errors
    /// OCIO's, when the file cannot be read or is not a valid config.
    pub fn open(path_or_uri: &str) -> Result<Config, Error> {
        ffi::Config::from_file(path_or_uri).map(Config::from)
    }

    /// The built-in ACES Studio config.
    ///
    /// # Errors
    /// OCIO's.
    pub fn studio() -> Result<Config, Error> {
        Config::open(STUDIO_CONFIG)
    }

    /// A config from the text of one (YAML), whose relative paths are from `working_dir`.
    ///
    /// # Errors
    /// OCIO's, when the text is not a config.
    pub fn from_text(text: &str, working_dir: &str) -> Result<Config, Error> {
        ffi::Config::from_text(text, working_dir).map(Config::from)
    }

    /// The studio config with the colour spaces of the catalogue that it lacks
    /// ([`catalogue::fragment`]): the config that the engine's colour management uses.
    ///
    /// # Errors
    /// OCIO's.
    pub fn columbina() -> Result<Config, Error> {
        let fragment = Config::from_text(&catalogue::fragment(), "")?;
        Config::studio()?.extend(&fragment)
    }

    /// A copy of this config with the colour spaces of `extra` added, validated. The two must
    /// share their reference spaces (ACES2065-1 for the scene, CIE XYZ D65 for displays), and no
    /// name of `extra` may be in this one already.
    ///
    /// # Errors
    /// OCIO's, or a name that is in both.
    pub fn extend(&self, extra: &Config) -> Result<Config, Error> {
        self.raw.extend(&extra.raw).map(Config::from)
    }

    /// Whether OCIO finds the config sound.
    ///
    /// # Errors
    /// What it finds wrong.
    pub fn validate(&self) -> Result<(), Error> {
        self.raw.validate()
    }

    #[must_use]
    pub fn name(&self) -> String {
        self.raw.name()
    }

    /// Every colour space, active or not, scene- and display-referred, in the config's order.
    ///
    /// # Errors
    /// OCIO's.
    pub fn color_spaces(&self) -> Result<Vec<ColorSpace>, Error> {
        Ok(self
            .raw
            .spaces()?
            .into_iter()
            .map(|s| ColorSpace {
                name: s.name,
                family: s.family,
                encoding: s.encoding,
                description: s.description,
                referred: if s.display {
                    Referred::Display
                } else {
                    Referred::Scene
                },
                data: s.data,
                aliases: s.aliases,
            })
            .collect())
    }

    /// The colour space called `name`, by its name or one of its aliases.
    ///
    /// # Errors
    /// OCIO's.
    pub fn color_space(&self, name: &str) -> Result<Option<ColorSpace>, Error> {
        let Some(canonical) = self.raw.canonical_name(name)? else {
            return Ok(None);
        };
        Ok(self
            .color_spaces()?
            .into_iter()
            .find(|space| space.name == canonical))
    }

    /// The active displays.
    #[must_use]
    pub fn displays(&self) -> Vec<String> {
        self.raw.displays()
    }

    /// The views of a display.
    ///
    /// # Errors
    /// OCIO's, for a display that is not there.
    pub fn views(&self, display: &str) -> Result<Vec<String>, Error> {
        self.raw.views(display)
    }

    /// From the colour space `src` to `dst` (names or aliases).
    ///
    /// # Errors
    /// OCIO's, when either is not there.
    pub fn processor(&self, src: &str, dst: &str) -> Result<Processor, Error> {
        self.raw.processor_spaces(src, dst).map(Processor::from)
    }

    /// From the colour space `src` to `display` through `view` (forward), or back (inverse).
    ///
    /// # Errors
    /// OCIO's.
    pub fn display_view(
        &self,
        src: &str,
        display: &str,
        view: &str,
        direction: Direction,
    ) -> Result<Processor, Error> {
        self.raw
            .processor_display_view(src, display, view, direction == Direction::Inverse)
            .map(Processor::from)
    }

    /// One of OCIO's built-in transforms, by its style (`APPLE_LOG_to_ACES2065-1`).
    ///
    /// # Errors
    /// OCIO's, for a style it does not know.
    pub fn builtin(&self, style: &str, direction: Direction) -> Result<Processor, Error> {
        self.raw
            .processor_builtin(style, direction == Direction::Inverse)
            .map(Processor::from)
    }
}

impl From<ffi::Config> for Config {
    fn from(raw: ffi::Config) -> Config {
        Config { raw: Arc::new(raw) }
    }
}

/// A transform of colour, made by a config. Cheap to clone, and to share between threads.
#[derive(Clone)]
pub struct Processor {
    raw: Arc<ffi::Processor>,
}

impl std::fmt::Debug for Processor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Processor")
            .field("cache_id", &self.cache_id())
            .finish()
    }
}

impl From<ffi::Processor> for Processor {
    fn from(raw: ffi::Processor) -> Processor {
        Processor { raw: Arc::new(raw) }
    }
}

impl Processor {
    /// OCIO's identifier of what it does: two processors that do the same have the same.
    #[must_use]
    pub fn cache_id(&self) -> String {
        self.raw.cache_id()
    }

    /// Whether it does nothing.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.raw.is_noop()
    }

    /// Applied to `rgb` in place, on the processor in 32-bit floats with OCIO's default
    /// optimization: the reference that the card is held to.
    ///
    /// # Errors
    /// OCIO's.
    pub fn apply(&self, rgb: &mut [[f32; 3]]) -> Result<(), Error> {
        self.raw.apply(rgb)
    }

    /// Applied to one colour.
    ///
    /// # Errors
    /// OCIO's.
    pub fn apply_one(&self, rgb: [f32; 3]) -> Result<[f32; 3], Error> {
        let mut one = [rgb];
        self.raw.apply(&mut one)?;
        Ok(one[0])
    }

    /// OCIO's own shader of it, in Vulkan GLSL 4.6, with its tables: what [`gpu`] translates.
    ///
    /// # Errors
    /// OCIO's.
    pub fn glsl(&self, prefix: &str) -> Result<gpu::Glsl, Error> {
        self.raw
            .shader(prefix, gpu::MAX_TEXTURE_WIDTH)
            .map(gpu::Glsl::from)
    }
}
