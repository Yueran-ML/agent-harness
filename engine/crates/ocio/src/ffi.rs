//! The C interface of the shim (`src/shim.cpp`), and the only unsafe code of the crate: each
//! function here wraps one call, copies what it hands back, and turns a failure into an
//! [`Error`] with OCIO's message.

use std::ffi::{c_char, c_int, c_uint, CStr, CString};
use std::ptr::NonNull;

use crate::Error;

#[repr(C)]
pub struct RawConfig {
    _private: [u8; 0],
}

#[repr(C)]
pub struct RawProcessor {
    _private: [u8; 0],
}

#[repr(C)]
pub struct RawShader {
    _private: [u8; 0],
}

extern "C" {
    fn ocio_last_error() -> *const c_char;
    fn ocio_version() -> *const c_char;

    fn ocio_config_from_file(path: *const c_char) -> *mut RawConfig;
    fn ocio_config_from_text(text: *const c_char, working_dir: *const c_char) -> *mut RawConfig;
    fn ocio_config_extend(base: *const RawConfig, extra: *const RawConfig) -> *mut RawConfig;
    fn ocio_config_free(config: *mut RawConfig);
    fn ocio_config_validate(config: *const RawConfig) -> c_int;
    fn ocio_config_name(config: *const RawConfig) -> *const c_char;
    fn ocio_config_num_spaces(config: *const RawConfig) -> c_int;
    fn ocio_config_space(
        config: *const RawConfig,
        index: c_int,
        name: *mut *const c_char,
        family: *mut *const c_char,
        encoding: *mut *const c_char,
        description: *mut *const c_char,
        display: *mut c_int,
        data: *mut c_int,
        aliases: *mut c_int,
    ) -> c_int;
    fn ocio_config_space_alias(
        config: *const RawConfig,
        index: c_int,
        alias: c_int,
    ) -> *const c_char;
    fn ocio_config_canonical_name(config: *const RawConfig, name: *const c_char) -> *const c_char;
    fn ocio_config_num_displays(config: *const RawConfig) -> c_int;
    fn ocio_config_display(config: *const RawConfig, index: c_int) -> *const c_char;
    fn ocio_config_num_views(config: *const RawConfig, display: *const c_char) -> c_int;
    fn ocio_config_view(
        config: *const RawConfig,
        display: *const c_char,
        index: c_int,
    ) -> *const c_char;

    fn ocio_processor_spaces(
        config: *const RawConfig,
        src: *const c_char,
        dst: *const c_char,
    ) -> *mut RawProcessor;
    fn ocio_processor_display_view(
        config: *const RawConfig,
        src: *const c_char,
        display: *const c_char,
        view: *const c_char,
        inverse: c_int,
    ) -> *mut RawProcessor;
    fn ocio_processor_builtin(
        config: *const RawConfig,
        style: *const c_char,
        inverse: c_int,
    ) -> *mut RawProcessor;
    fn ocio_processor_free(processor: *mut RawProcessor);
    fn ocio_processor_cache_id(processor: *const RawProcessor) -> *const c_char;
    fn ocio_processor_is_noop(processor: *const RawProcessor) -> c_int;
    fn ocio_processor_apply(processor: *const RawProcessor, rgb: *mut f32, count: i64) -> c_int;

    fn ocio_processor_shader(
        processor: *const RawProcessor,
        prefix: *const c_char,
        max_width: c_uint,
    ) -> *mut RawShader;
    fn ocio_shader_free(shader: *mut RawShader);
    fn ocio_shader_text(shader: *const RawShader) -> *const c_char;
    fn ocio_shader_num_uniforms(shader: *const RawShader) -> c_int;
    fn ocio_shader_num_textures(shader: *const RawShader) -> c_int;
    fn ocio_shader_texture(
        shader: *const RawShader,
        index: c_int,
        name: *mut *const c_char,
        sampler: *mut *const c_char,
        width: *mut c_uint,
        height: *mut c_uint,
        channels: *mut c_int,
        nearest: *mut c_int,
        values: *mut *const f32,
    ) -> c_int;
    fn ocio_shader_num_3d_textures(shader: *const RawShader) -> c_int;
    fn ocio_shader_3d_texture(
        shader: *const RawShader,
        index: c_int,
        name: *mut *const c_char,
        sampler: *mut *const c_char,
        edge: *mut c_uint,
        nearest: *mut c_int,
        values: *mut *const f32,
    ) -> c_int;
}

/// The message of the last failure on this thread.
fn last_error() -> Error {
    // SAFETY: the shim keeps a thread-local string, valid until its next call on this thread.
    Error::Ocio(unsafe { string(ocio_last_error()) }.unwrap_or_default())
}

/// A copy of a string of C, or none for a null pointer.
///
/// # Safety
/// `text` is null or points to a string ended by a zero.
unsafe fn string(text: *const c_char) -> Option<String> {
    if text.is_null() {
        None
    } else {
        Some(CStr::from_ptr(text).to_string_lossy().into_owned())
    }
}

fn c(text: &str) -> Result<CString, Error> {
    CString::new(text).map_err(|_| Error::Ocio(format!("{text:?} has a zero byte in it")))
}

fn status(code: c_int) -> Result<(), Error> {
    if code < 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn index(i: usize) -> c_int {
    c_int::try_from(i).unwrap_or(c_int::MAX)
}

fn count(n: c_int) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// The version of the OCIO linked in, as `2.6.0`.
pub fn version() -> String {
    // SAFETY: a static string of the library.
    unsafe { string(ocio_version()) }.unwrap_or_default()
}

/// A config of OCIO's, owned.
pub struct Config(NonNull<RawConfig>);

// SAFETY: the shim's config is a const OCIO config, which OCIO lets threads read at once (its
// processors are made under its own lock), and nothing of it is changed after it is made.
unsafe impl Send for Config {}
// SAFETY: as above.
unsafe impl Sync for Config {}

impl Drop for Config {
    fn drop(&mut self) {
        // SAFETY: made by the shim, freed once.
        unsafe { ocio_config_free(self.0.as_ptr()) }
    }
}

fn config(raw: *mut RawConfig) -> Result<Config, Error> {
    NonNull::new(raw).map(Config).ok_or_else(last_error)
}

/// What the shim says of a colour space.
pub struct RawSpace {
    pub name: String,
    pub family: String,
    pub encoding: String,
    pub description: String,
    pub display: bool,
    pub data: bool,
    pub aliases: Vec<String>,
}

impl Config {
    fn raw(&self) -> *const RawConfig {
        self.0.as_ptr()
    }

    pub fn from_file(path: &str) -> Result<Config, Error> {
        let path = c(path)?;
        // SAFETY: a string of C that outlives the call.
        config(unsafe { ocio_config_from_file(path.as_ptr()) })
    }

    pub fn from_text(text: &str, working_dir: &str) -> Result<Config, Error> {
        let text = c(text)?;
        let dir = c(working_dir)?;
        // SAFETY: strings of C that outlive the call.
        config(unsafe { ocio_config_from_text(text.as_ptr(), dir.as_ptr()) })
    }

    pub fn extend(&self, extra: &Config) -> Result<Config, Error> {
        // SAFETY: two live configs.
        config(unsafe { ocio_config_extend(self.raw(), extra.raw()) })
    }

    pub fn validate(&self) -> Result<(), Error> {
        // SAFETY: a live config.
        status(unsafe { ocio_config_validate(self.raw()) })
    }

    pub fn name(&self) -> String {
        // SAFETY: a string of the config's, copied at once.
        unsafe { string(ocio_config_name(self.raw())) }.unwrap_or_default()
    }

    pub fn spaces(&self) -> Result<Vec<RawSpace>, Error> {
        // SAFETY: a live config.
        let n = count(unsafe { ocio_config_num_spaces(self.raw()) });
        let mut spaces = Vec::with_capacity(n);
        for i in 0..n {
            let mut name = std::ptr::null();
            let mut family = std::ptr::null();
            let mut encoding = std::ptr::null();
            let mut description = std::ptr::null();
            let (mut display, mut data, mut aliases) = (0, 0, 0);
            // SAFETY: pointers to locals; the strings are the config's and copied before the
            // next call.
            unsafe {
                status(ocio_config_space(
                    self.raw(),
                    index(i),
                    &raw mut name,
                    &raw mut family,
                    &raw mut encoding,
                    &raw mut description,
                    &raw mut display,
                    &raw mut data,
                    &raw mut aliases,
                ))?;
                let mut space = RawSpace {
                    name: string(name).unwrap_or_default(),
                    family: string(family).unwrap_or_default(),
                    encoding: string(encoding).unwrap_or_default(),
                    description: string(description).unwrap_or_default(),
                    display: display != 0,
                    data: data != 0,
                    aliases: Vec::new(),
                };
                for alias in 0..aliases {
                    let text = ocio_config_space_alias(self.raw(), index(i), alias);
                    space.aliases.push(string(text).ok_or_else(last_error)?);
                }
                spaces.push(space);
            }
        }
        Ok(spaces)
    }

    pub fn canonical_name(&self, name: &str) -> Result<Option<String>, Error> {
        let name = c(name)?;
        // SAFETY: a string of C that outlives the call; the answer is copied at once.
        Ok(unsafe { string(ocio_config_canonical_name(self.raw(), name.as_ptr())) })
    }

    pub fn displays(&self) -> Vec<String> {
        // SAFETY: a live config; its strings are copied at once.
        unsafe {
            let n = count(ocio_config_num_displays(self.raw()));
            (0..n)
                .filter_map(|i| string(ocio_config_display(self.raw(), index(i))))
                .collect()
        }
    }

    pub fn views(&self, display: &str) -> Result<Vec<String>, Error> {
        let display = c(display)?;
        // SAFETY: a live config and a string of C that outlives the calls.
        unsafe {
            let n = ocio_config_num_views(self.raw(), display.as_ptr());
            status(n)?;
            (0..count(n))
                .map(|i| {
                    string(ocio_config_view(self.raw(), display.as_ptr(), index(i)))
                        .ok_or_else(last_error)
                })
                .collect()
        }
    }

    pub fn processor_spaces(&self, src: &str, dst: &str) -> Result<Processor, Error> {
        let (src, dst) = (c(src)?, c(dst)?);
        // SAFETY: a live config and strings of C that outlive the call.
        processor(unsafe { ocio_processor_spaces(self.raw(), src.as_ptr(), dst.as_ptr()) })
    }

    pub fn processor_display_view(
        &self,
        src: &str,
        display: &str,
        view: &str,
        inverse: bool,
    ) -> Result<Processor, Error> {
        let (src, display, view) = (c(src)?, c(display)?, c(view)?);
        // SAFETY: a live config and strings of C that outlive the call.
        processor(unsafe {
            ocio_processor_display_view(
                self.raw(),
                src.as_ptr(),
                display.as_ptr(),
                view.as_ptr(),
                c_int::from(inverse),
            )
        })
    }

    pub fn processor_builtin(&self, style: &str, inverse: bool) -> Result<Processor, Error> {
        let style = c(style)?;
        // SAFETY: a live config and a string of C that outlives the call.
        processor(unsafe {
            ocio_processor_builtin(self.raw(), style.as_ptr(), c_int::from(inverse))
        })
    }
}

/// A processor of OCIO's, with its CPU processor (of 32-bit floats, OCIO's default
/// optimization), owned.
pub struct Processor(NonNull<RawProcessor>);

// SAFETY: OCIO's processors are immutable once made, and their CPU processors may be applied
// from many threads at once.
unsafe impl Send for Processor {}
// SAFETY: as above.
unsafe impl Sync for Processor {}

impl Drop for Processor {
    fn drop(&mut self) {
        // SAFETY: made by the shim, freed once.
        unsafe { ocio_processor_free(self.0.as_ptr()) }
    }
}

fn processor(raw: *mut RawProcessor) -> Result<Processor, Error> {
    NonNull::new(raw).map(Processor).ok_or_else(last_error)
}

/// A 1D table of a shader, as OCIO hands it over.
pub struct RawTexture {
    pub name: String,
    pub sampler: String,
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub nearest: bool,
    pub values: Vec<f32>,
}

/// A 3D table of a shader, as OCIO hands it over.
pub struct RawTexture3d {
    pub name: String,
    pub sampler: String,
    pub edge: u32,
    pub nearest: bool,
    pub values: Vec<f32>,
}

/// A shader of OCIO's, copied out.
pub struct RawShaderInfo {
    pub text: String,
    pub uniforms: usize,
    pub textures: Vec<RawTexture>,
    pub textures_3d: Vec<RawTexture3d>,
}

impl Processor {
    fn raw(&self) -> *const RawProcessor {
        self.0.as_ptr()
    }

    pub fn cache_id(&self) -> String {
        // SAFETY: a string of the processor's, copied at once.
        unsafe { string(ocio_processor_cache_id(self.raw())) }.unwrap_or_default()
    }

    pub fn is_noop(&self) -> bool {
        // SAFETY: a live processor.
        unsafe { ocio_processor_is_noop(self.raw()) != 0 }
    }

    pub fn apply(&self, rgb: &mut [[f32; 3]]) -> Result<(), Error> {
        if rgb.is_empty() {
            return Ok(());
        }
        let n = i64::try_from(rgb.len()).map_err(|_| Error::Ocio("too many pixels".into()))?;
        // SAFETY: `rgb` is `n` packed triples of f32, borrowed mutably for the call.
        status(unsafe { ocio_processor_apply(self.raw(), rgb.as_mut_ptr().cast(), n) })
    }

    pub fn shader(&self, prefix: &str, max_width: u32) -> Result<RawShaderInfo, Error> {
        let prefix = c(prefix)?;
        // SAFETY: a live processor and a string of C that outlives the call; the shader is
        // freed below, after all it holds is copied.
        unsafe {
            let shader = ocio_processor_shader(self.raw(), prefix.as_ptr(), max_width);
            let shader = NonNull::new(shader).ok_or_else(last_error)?;
            let result = copy_shader(shader.as_ptr());
            ocio_shader_free(shader.as_ptr());
            result
        }
    }
}

/// # Safety
/// `shader` is live.
unsafe fn copy_shader(shader: *const RawShader) -> Result<RawShaderInfo, Error> {
    let text = string(ocio_shader_text(shader)).unwrap_or_default();
    let uniforms = count(ocio_shader_num_uniforms(shader));
    let mut textures = Vec::new();
    for i in 0..count(ocio_shader_num_textures(shader)) {
        let (mut name, mut sampler, mut values) =
            (std::ptr::null(), std::ptr::null(), std::ptr::null());
        let (mut width, mut height, mut channels, mut nearest) = (0, 0, 0, 0);
        status(ocio_shader_texture(
            shader,
            index(i),
            &raw mut name,
            &raw mut sampler,
            &raw mut width,
            &raw mut height,
            &raw mut channels,
            &raw mut nearest,
            &raw mut values,
        ))?;
        let channels = u32::try_from(channels).unwrap_or(0);
        let len = width as usize * height as usize * channels as usize;
        textures.push(RawTexture {
            name: string(name).unwrap_or_default(),
            sampler: string(sampler).unwrap_or_default(),
            width,
            height,
            channels,
            nearest: nearest != 0,
            values: floats(values, len)?,
        });
    }
    let mut textures_3d = Vec::new();
    for i in 0..count(ocio_shader_num_3d_textures(shader)) {
        let (mut name, mut sampler, mut values) =
            (std::ptr::null(), std::ptr::null(), std::ptr::null());
        let (mut edge, mut nearest) = (0, 0);
        status(ocio_shader_3d_texture(
            shader,
            index(i),
            &raw mut name,
            &raw mut sampler,
            &raw mut edge,
            &raw mut nearest,
            &raw mut values,
        ))?;
        let len = (edge as usize).pow(3) * 3;
        textures_3d.push(RawTexture3d {
            name: string(name).unwrap_or_default(),
            sampler: string(sampler).unwrap_or_default(),
            edge,
            nearest: nearest != 0,
            values: floats(values, len)?,
        });
    }
    Ok(RawShaderInfo {
        text,
        uniforms,
        textures,
        textures_3d,
    })
}

/// # Safety
/// `values` is null or points to `len` floats.
unsafe fn floats(values: *const f32, len: usize) -> Result<Vec<f32>, Error> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        return Err(Error::Ocio("a texture without values".into()));
    }
    Ok(std::slice::from_raw_parts(values, len).to_vec())
}
