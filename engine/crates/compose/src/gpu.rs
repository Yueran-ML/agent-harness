//! The graphics card of the engine: one instance, adapter, device and queue, which the views and
//! the compositor share, so that a picture composed on the card is shown from the card.
//!
//! A device can be lost (a driver reset, the card removed): then everything made on it is of no
//! more use, and a new [`Gpu`] is made in its place by whoever owns the engine's (the server),
//! which hands it to the views and makes the compositor again. Those who use a `Gpu` learn that
//! it was lost with [`Gpu::when_lost`], or ask [`Gpu::is_lost`]; nothing of this is an error of
//! theirs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

/// The backends that the engine draws with: Direct3D 12 on Windows, Metal on macOS. Elsewhere
/// (Linux, for the tests of the pure parts) whatever wgpu was built with, which may be none.
#[cfg(windows)]
const BACKENDS: wgpu::Backends = wgpu::Backends::DX12;
#[cfg(target_os = "macos")]
const BACKENDS: wgpu::Backends = wgpu::Backends::METAL;
#[cfg(not(any(windows, target_os = "macos")))]
const BACKENDS: wgpu::Backends = wgpu::Backends::all();

/// Which adapter is asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    /// The fastest card, a discrete one where there are two; the software adapter of the system
    /// (WARP on Windows) where there is no card at all.
    #[default]
    HighPerformance,
    /// A card only, never the software adapter: while a card that was lost may come back (a
    /// driver that resets), and to go back to a card from the software adapter.
    Hardware,
    /// The software adapter of the system only: slow, but there wherever the system draws.
    Software,
}

/// Why there is no [`Gpu`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GpuError {
    /// No adapter, not even one in software.
    #[error("there is no graphics adapter: {0}")]
    NoAdapter(String),
    /// An adapter, but no device could be made of it.
    #[error("the graphics device cannot be made: {0}")]
    Device(String),
}

/// What is told when the device is lost or the card reports an error: a message.
type Listener = Arc<dyn Fn(&str) + Send + Sync>;

/// Who is told what happens to the device, kept apart from the [`Gpu`] so that the callbacks of
/// the device do not keep it.
#[derive(Default)]
struct Watchers {
    state: Mutex<Watched>,
}

#[derive(Default)]
struct Watched {
    /// Why the device was lost, once it was.
    lost: Option<String>,
    /// The number of the next listener.
    next: u64,
    lost_listeners: Vec<(u64, Listener)>,
    error_listeners: Vec<(u64, Listener)>,
    /// How many errors the card reported that no one asked for (uncaptured).
    errors: u64,
    last_error: Option<String>,
}

impl Watchers {
    fn lock(&self) -> MutexGuard<'_, Watched> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The device is lost: those who listen are told, once.
    fn lose(&self, why: &str) {
        let listeners: Vec<Listener> = {
            let mut state = self.lock();
            if state.lost.is_some() {
                return;
            }
            state.lost = Some(why.to_owned());
            state
                .lost_listeners
                .iter()
                .map(|(_, listener)| Arc::clone(listener))
                .collect()
        };
        for listener in listeners {
            listener(why);
        }
    }

    fn error(&self, message: &str) {
        let listeners: Vec<Listener> = {
            let mut state = self.lock();
            state.errors += 1;
            state.last_error = Some(message.to_owned());
            state
                .error_listeners
                .iter()
                .map(|(_, listener)| Arc::clone(listener))
                .collect()
        };
        for listener in listeners {
            listener(message);
        }
    }

    fn remove(&self, id: u64) {
        let mut state = self.lock();
        state.lost_listeners.retain(|(each, _)| *each != id);
        state.error_listeners.retain(|(each, _)| *each != id);
    }
}

/// A listener of a [`Gpu`], which is no longer told once this is dropped.
#[must_use = "the listener is let go when this is dropped"]
pub struct Watch {
    watchers: Weak<Watchers>,
    id: u64,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(watchers) = self.watchers.upgrade() {
            watchers.remove(self.id);
        }
    }
}

/// The graphics card: an instance of wgpu, the adapter chosen, and a device of it with its
/// queue. It is made once for the engine and shared ([`Arc`]); surfaces of windows are made from
/// its instance, and every texture is made on its device, so that one made by the compositor can
/// be drawn in a view.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    backend: String,
    software: bool,
    sixteen_norm: bool,
    compiler: ShaderCompiler,
    dxc_failure: Option<String>,
    watchers: Arc<Watchers>,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Gpu")
            .field("name", &self.name)
            .field("backend", &self.backend)
            .field("software", &self.software)
            .field("sixteen_norm", &self.sixteen_norm)
            .field("compiler", &self.compiler)
            .field("dxc_failure", &self.dxc_failure)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Make the instance, choose an adapter as `preference` says, and make a device of it,
    /// with 16-bit normalized textures where the adapter has them (for pictures of P010) and
    /// textures and buffers as large as it takes. This takes some 100 ms: call it where
    /// blocking is allowed.
    ///
    /// The shaders are compiled as [`ShaderCompiler::chosen`] says. Where DXC makes no device
    /// (a `dxcompiler.dll` that does not load, or a `dxil.dll` that does not sign what it
    /// compiles), the device is made again with FXC; if FXC makes one, DXC is not tried again
    /// while the process lasts, and [`Gpu::dxc_failure`] says why it was given up.
    ///
    /// # Errors
    /// There is no backend or adapter, not even in software, or no device can be made, with
    /// FXC either.
    pub fn new(preference: Preference) -> Result<Arc<Gpu>, GpuError> {
        Gpu::new_with(preference, ShaderCompiler::chosen(), &DXC_GIVEN_UP).map(Arc::new)
    }

    /// [`Gpu::new`] with `compiler`, and where a DXC that failed is remembered.
    fn new_with(
        preference: Preference,
        compiler: ShaderCompiler,
        given_up: &OnceLock<String>,
    ) -> Result<Gpu, GpuError> {
        let dxc = compiler.path().map(Path::to_path_buf);
        let (error, dxc) = match (Gpu::made(preference, compiler), dxc) {
            (Err(error), Some(dxc)) if cfg!(windows) => (error, dxc),
            (made, _) => return made,
        };
        let fxc = ShaderCompiler {
            kind: CompilerKind::Fxc,
            by: ChosenBy::DxcFailed,
        };
        // FXC failing too, there is no card to make a device of, whatever compiles: its error
        // is told, and DXC may be tried again.
        let mut gpu = Gpu::made(preference, fxc)?;
        let why = format!("{}: {error}", dxc.display());
        let _ = given_up.set(why.clone());
        gpu.dxc_failure = Some(why);
        Ok(gpu)
    }

    /// Make the instance with `compiler`, the adapter and the device.
    fn made(preference: Preference, compiler: ShaderCompiler) -> Result<Gpu, GpuError> {
        let instance = instance(BACKENDS, &compiler)?;
        let ask = |fallback: bool| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                compatible_surface: None,
                apply_limit_buckets: false,
            }))
        };
        let adapter = match preference {
            Preference::HighPerformance => ask(false).or_else(|_| ask(true)),
            Preference::Hardware => ask(false),
            Preference::Software => ask(true),
        }
        .map_err(|error| GpuError::NoAdapter(error.to_string()))?;
        if preference == Preference::Hardware
            && adapter.get_info().device_type == wgpu::DeviceType::Cpu
        {
            return Err(GpuError::NoAdapter(
                "there is no graphics card, only the software adapter".into(),
            ));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("columbina"),
            required_features: wanted_features(&adapter),
            required_limits: wanted_limits(&adapter),
            ..wgpu::DeviceDescriptor::default()
        }))
        .map_err(|error| GpuError::Device(error.to_string()))?;
        let watchers = Arc::new(Watchers::default());
        device.set_device_lost_callback({
            let watchers = Arc::clone(&watchers);
            move |reason, message| {
                let why = if message.is_empty() {
                    format!("{reason:?}")
                } else {
                    format!("{reason:?}: {message}")
                };
                watchers.lose(&why);
            }
        });
        // Without a handler of its own, wgpu panics on an error that no scope caught.
        device.on_uncaptured_error(Arc::new({
            let watchers = Arc::clone(&watchers);
            move |error: wgpu::Error| watchers.error(&error.to_string())
        }));
        let info = adapter.get_info();
        Ok(Gpu {
            sixteen_norm: device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            software: info.device_type == wgpu::DeviceType::Cpu,
            name: info.name,
            backend: format!("{:?}", info.backend),
            instance,
            adapter,
            device,
            queue,
            compiler,
            dxc_failure: None,
            watchers,
        })
    }

    /// The instance, which the surfaces of the views are made from.
    #[must_use]
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    #[must_use]
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The name of the adapter.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The API that it draws with (`Dx12`, `Metal`).
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Whether the adapter is the system's in software (WARP on Windows): it draws and renders,
    /// slowly.
    #[must_use]
    pub fn is_software(&self) -> bool {
        self.software
    }

    /// Whether the device has `R16Unorm` and `Rg16Unorm`, for the planes of P010.
    #[must_use]
    pub fn has_sixteen_norm(&self) -> bool {
        self.sixteen_norm
    }

    /// The compiler of the shaders, for Direct3D 12; none for another API (Metal compiles its
    /// own).
    #[must_use]
    pub fn shader_compiler(&self) -> Option<&ShaderCompiler> {
        (self.backend == "Dx12").then_some(&self.compiler)
    }

    /// Why DXC made no device (its `dxcompiler.dll`, then the error), where FXC compiles the
    /// shaders of this card in its place.
    #[must_use]
    pub fn dxc_failure(&self) -> Option<&str> {
        self.dxc_failure.as_deref()
    }

    /// Whether the device was lost: nothing made on it is of use any more.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.watchers.lock().lost.is_some()
    }

    /// Why the device was lost, if it was.
    #[must_use]
    pub fn lost_reason(&self) -> Option<String> {
        self.watchers.lock().lost.clone()
    }

    /// Take the device as lost, as one that can no longer do what is asked of it (an adapter
    /// that no longer draws in a window): those who listen are told, once, as when the device
    /// is lost in fact.
    pub fn mark_lost(&self, why: &str) {
        self.watchers.lose(why);
    }

    /// Tell `listener` once the device is lost, with why; at once, if it was lost already. It is
    /// called on whatever thread finds the device lost, and must not block.
    pub fn when_lost(&self, listener: impl Fn(&str) + Send + Sync + 'static) -> Watch {
        let listener: Listener = Arc::new(listener);
        let (id, lost) = {
            let mut state = self.watchers.lock();
            let id = state.next;
            state.next += 1;
            state.lost_listeners.push((id, Arc::clone(&listener)));
            (id, state.lost.clone())
        };
        if let Some(why) = lost {
            listener(&why);
        }
        Watch {
            watchers: Arc::downgrade(&self.watchers),
            id,
        }
    }

    /// Tell `listener` of each error of the card that no one caught with an error scope. It is
    /// called on whatever thread the error is found on, and must not block.
    pub fn when_error(&self, listener: impl Fn(&str) + Send + Sync + 'static) -> Watch {
        let mut state = self.watchers.lock();
        let id = state.next;
        state.next += 1;
        state.error_listeners.push((id, Arc::new(listener)));
        Watch {
            watchers: Arc::downgrade(&self.watchers),
            id,
        }
    }

    /// How many errors the card reported that no one caught, and the last of them.
    #[must_use]
    pub fn uncaught_errors(&self) -> (u64, Option<String>) {
        let state = self.watchers.lock();
        (state.errors, state.last_error.clone())
    }
}

/// Make an instance for the backends that the engine uses on this platform, whose Direct3D 12
/// compiles shaders with `compiler`.
fn instance(
    backends: wgpu::Backends,
    compiler: &ShaderCompiler,
) -> Result<wgpu::Instance, GpuError> {
    // wgpu panics when no backend was compiled for this platform (as on Linux, where only
    // the pure parts run). Nor can an instance draw if none of the requested backends exist.
    if !wgpu::Instance::enabled_backend_features().intersects(backends) {
        return Err(GpuError::NoAdapter(
            "wgpu has no backend to draw with on this platform".into(),
        ));
    }
    let mut descriptor = wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    descriptor.backend_options.dx12.shader_compiler = compiler.wgpu();
    if matches!(compiler.kind, CompilerKind::Dxc(_)) {
        descriptor.backend_options.dx12.force_shader_model = dxc_shader_model();
    }
    Ok(wgpu::Instance::new(descriptor))
}

/// The variable that names the compiler of Direct3D 12 shaders.
pub const COMPILER_VARIABLE: &str = "COLUMBINA_DX12_COMPILER";

/// Why DXC was given up, once it made no device and FXC made one: from then on FXC compiles
/// the shaders of every card that the process makes.
static DXC_GIVEN_UP: OnceLock<String> = OnceLock::new();

/// The compiler of the Direct3D 12 shaders of a [`Gpu`], and how it was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderCompiler {
    /// What compiles them.
    pub kind: CompilerKind,
    /// Why it does.
    pub by: ChosenBy,
}

/// What compiles the Direct3D 12 shaders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompilerKind {
    /// FXC, the `d3dcompiler_47.dll` that every Windows has.
    Fxc,
    /// Microsoft's DXC, the `dxcompiler.dll` of this path, compiling shaders of model 6.0.
    Dxc(PathBuf),
    /// What wgpu takes: a `dxcompiler.dll` found by its name, else FXC.
    Auto,
}

/// Why a compiler compiles the Direct3D 12 shaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChosenBy {
    /// `COLUMBINA_DX12_COMPILER` names it.
    Variable,
    /// `dxcompiler.dll` and `dxil.dll` lie beside the executable.
    BesideExecutable,
    /// Neither: FXC.
    Default,
    /// DXC made no device, and FXC did.
    DxcFailed,
}

impl ShaderCompiler {
    /// The compiler that a [`Gpu`] made now compiles its shaders with, in this order:
    ///
    /// 1. what `COLUMBINA_DX12_COMPILER` names: `auto` (wgpu's choice), `fxc`, or the path of a
    ///    `dxcompiler.dll`; any other value is FXC;
    /// 2. else Microsoft's DXC, where `dxcompiler.dll` and `dxil.dll` lie beside the executable
    ///    (as the package brings them, from the Windows SDK), by the absolute path of that
    ///    `dxcompiler.dll`;
    /// 3. else FXC, which every Windows has.
    ///
    /// DXC that made no device once in this process ([`Gpu::new`]) is not taken again: FXC is.
    ///
    /// wgpu would otherwise take whatever `dxcompiler.dll` is found on the `PATH`, and builds of
    /// it that other programs ship (as Obsidian and Electron ship Dawn's) crash the process as
    /// some of our shaders are compiled, with no error that could be caught. So DXC is always
    /// loaded by its full path, never by the search of Windows.
    ///
    /// DXC compiles the shaders of grades of many nodes some twice as fast as FXC. It signs them
    /// with the `dxil.dll` that Windows finds by its name: the one beside the executable, else
    /// in the folders of the system, else on the `PATH`, and not the one beside a
    /// `dxcompiler.dll` named elsewhere (2026-10-31: Obsidian's, on the `PATH`, of version 1.7,
    /// which would not sign shaders of model 6.8, and every device failed to be made, "Parent
    /// device is lost"). So the shaders that it compiles are of model 6.0, which every
    /// `dxil.dll` signs and is all that ours need.
    #[must_use]
    pub fn chosen() -> ShaderCompiler {
        let variable = std::env::var_os(COMPILER_VARIABLE);
        let executable = std::env::current_exe().ok();
        let beside = executable.as_deref().and_then(Path::parent);
        let chosen = choose(variable.as_deref(), beside);
        match (&chosen.kind, DXC_GIVEN_UP.get()) {
            (CompilerKind::Dxc(_), Some(_)) => ShaderCompiler {
                kind: CompilerKind::Fxc,
                by: ChosenBy::DxcFailed,
            },
            _ => chosen,
        }
    }

    /// Its name in the log: `fxc`, `dxc` or `auto`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.kind {
            CompilerKind::Fxc => "fxc",
            CompilerKind::Dxc(_) => "dxc",
            CompilerKind::Auto => "auto",
        }
    }

    /// The `dxcompiler.dll` of DXC.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.kind {
            CompilerKind::Dxc(path) => Some(path),
            CompilerKind::Fxc | CompilerKind::Auto => None,
        }
    }

    /// Why it was chosen, in the log: `COLUMBINA_DX12_COMPILER`, `beside the executable`,
    /// `default` or `dxc failed`.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self.by {
            ChosenBy::Variable => COMPILER_VARIABLE,
            ChosenBy::BesideExecutable => "beside the executable",
            ChosenBy::Default => "default",
            ChosenBy::DxcFailed => "dxc failed",
        }
    }

    /// As wgpu names it.
    fn wgpu(&self) -> wgpu::Dx12Compiler {
        match &self.kind {
            CompilerKind::Fxc => wgpu::Dx12Compiler::Fxc,
            CompilerKind::Dxc(path) => wgpu::Dx12Compiler::DynamicDxc {
                dxc_path: path.to_string_lossy().into_owned(),
            },
            CompilerKind::Auto => wgpu::Dx12Compiler::Auto,
        }
    }
}

/// The compiler that `variable` (the value of `COLUMBINA_DX12_COMPILER`) names, else DXC if
/// `dxcompiler.dll` and `dxil.dll` are in `beside`, else FXC.
fn choose(variable: Option<&OsStr>, beside: Option<&Path>) -> ShaderCompiler {
    let named = |kind| ShaderCompiler {
        kind,
        by: ChosenBy::Variable,
    };
    if let Some(value) = variable.filter(|value| !value.is_empty()) {
        let text = value.to_string_lossy();
        return if text.eq_ignore_ascii_case("auto") {
            named(CompilerKind::Auto)
        } else if text.to_ascii_lowercase().ends_with("dxcompiler.dll") {
            named(CompilerKind::Dxc(PathBuf::from(value)))
        } else {
            named(CompilerKind::Fxc)
        };
    }
    if let Some(folder) = beside {
        let compiler = folder.join("dxcompiler.dll");
        if compiler.is_file() && folder.join("dxil.dll").is_file() {
            return ShaderCompiler {
                kind: CompilerKind::Dxc(compiler),
                by: ChosenBy::BesideExecutable,
            };
        }
    }
    ShaderCompiler {
        kind: CompilerKind::Fxc,
        by: ChosenBy::Default,
    }
}

/// The compiler of Direct3D 12 shaders that a [`Gpu`] made now takes, as wgpu names it (see
/// [`ShaderCompiler::chosen`]), for the tests that say what they measure with.
#[cfg(test)]
#[must_use]
pub(crate) fn dx12_compiler() -> wgpu::Dx12Compiler {
    ShaderCompiler::chosen().wgpu()
}

/// The shader model of the shaders that DXC compiles: 6.0 (see [`ShaderCompiler::chosen`]).
#[allow(
    unsafe_code,
    reason = "the token of wgpu that lowers the shader model is made by an unsafe function"
)]
fn dxc_shader_model() -> wgpu::ForceShaderModelToken {
    // SAFETY: the token is to promise that the shaders compiled use no feature that the
    // compiler cannot compile: 6.0 is the least model that DXC compiles, of every version, and
    // our shaders, naga's HLSL of WGSL that asks for no feature, need no more.
    unsafe { wgpu::ForceShaderModelToken::with_shader_model(wgpu::DxcShaderModel::V6_0) }
}

/// The features that are asked for where the adapter has them.
fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}

/// The limits that are asked for: the defaults, with textures as large as the adapter takes,
/// for pictures beyond 8K, and buffers as large, for the planes of pictures that are rendered.
fn wanted_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let most = adapter.limits();
    wgpu::Limits {
        max_texture_dimension_2d: most.max_texture_dimension_2d,
        max_storage_buffer_binding_size: most.max_storage_buffer_binding_size,
        max_buffer_size: most.max_buffer_size,
        ..wgpu::Limits::default()
    }
}

/// What `COLUMBINA_TEST_ADAPTER` asks of the tests: `software` has them draw on the software
/// adapter though the machine has a card, as they do on a runner of CI; `software-as-card`
/// does too, and has [`card_for_tests`] give that adapter as if it were a card, to see what
/// the tests of a card say of it.
fn test_adapter() -> (Preference, bool) {
    match std::env::var("COLUMBINA_TEST_ADAPTER").as_deref() {
        Ok("software") => (Preference::Software, false),
        Ok("software-as-card") => (Preference::Software, true),
        _ => (Preference::HighPerformance, false),
    }
}

/// A [`Gpu`] for the tests that draw: none where there is no backend or adapter, unless
/// `COLUMBINA_REQUIRE_GPU` is 1, when there must be one. `COLUMBINA_TEST_ADAPTER=software`
/// gives the software adapter of the system where there is a card too.
///
/// # Panics
/// `COLUMBINA_REQUIRE_GPU` is 1, and there is no backend or adapter.
#[doc(hidden)]
#[must_use]
pub fn gpu_for_tests() -> Option<Arc<Gpu>> {
    match Gpu::new(test_adapter().0) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            assert!(
                std::env::var("COLUMBINA_REQUIRE_GPU").as_deref() != Ok("1"),
                "COLUMBINA_REQUIRE_GPU is 1, and there is no graphics card: {error}"
            );
            eprintln!("skipped: no graphics card: {error}");
            None
        }
    }
}

/// A [`Gpu`] of a graphics card for the tests that hold what is drawn to the numbers that the
/// processor computes, or to the bit: none where [`gpu_for_tests`] gives none, and none on the
/// software adapter of the system (WARP on Windows), which is all that a runner of CI has. That
/// adapter draws, and the tests that only ask for a picture run on it; but it rounds and
/// interpolates otherwise than the cards whose error these tests bound, so that they would
/// fail there for what is no fault of the engine.
///
/// # Panics
/// `COLUMBINA_REQUIRE_GPU` is 1, and there is no card: no adapter, or only the software one.
#[doc(hidden)]
#[must_use]
pub fn card_for_tests() -> Option<Arc<Gpu>> {
    let gpu = gpu_for_tests()?;
    if gpu.is_software() && !test_adapter().1 {
        assert!(
            std::env::var("COLUMBINA_REQUIRE_GPU").as_deref() != Ok("1"),
            "COLUMBINA_REQUIRE_GPU is 1, and there is only the software adapter"
        );
        eprintln!("skipped: no graphics card, only the software adapter");
        return None;
    }
    Some(gpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn an_instance_without_a_usable_backend_is_refused_before_wgpu_is_called() {
        // Exercise the boundary even on Windows and macOS, which normally have a backend.
        let unavailable = wgpu::Backends::all() - wgpu::Instance::enabled_backend_features();
        for backends in [wgpu::Backends::empty(), unavailable] {
            assert_eq!(
                instance(backends, &fxc()).unwrap_err(),
                GpuError::NoAdapter("wgpu has no backend to draw with on this platform".into())
            );
        }
    }

    #[test]
    fn a_device_lost_is_told_once_to_each_listener_and_at_once_to_those_who_come_after() {
        let Some(gpu) = gpu_for_tests() else { return };
        assert!(!gpu.is_lost());
        let told = Arc::new(AtomicUsize::new(0));
        let count = |told: &Arc<AtomicUsize>| {
            let told = Arc::clone(told);
            move |_: &str| {
                told.fetch_add(1, Ordering::SeqCst);
            }
        };
        let first = gpu.when_lost(count(&told));
        let dropped = gpu.when_lost(count(&told));
        drop(dropped);
        gpu.mark_lost("as a test");
        gpu.mark_lost("again");
        assert_eq!(told.load(Ordering::SeqCst), 1);
        assert_eq!(gpu.lost_reason().as_deref(), Some("as a test"));
        let _late = gpu.when_lost(count(&told));
        assert_eq!(told.load(Ordering::SeqCst), 2);
        drop(first);
    }

    #[test]
    fn a_destroyed_device_is_lost() {
        let Some(gpu) = gpu_for_tests() else { return };
        let told = Arc::new(Mutex::new(None));
        let _watch = gpu.when_lost({
            let told = Arc::clone(&told);
            move |why: &str| *told.lock().unwrap() = Some(why.to_owned())
        });
        gpu.device().destroy();
        let _ = gpu.device().poll(wgpu::PollType::wait_indefinitely());
        assert!(gpu.is_lost());
        assert_eq!(told.lock().unwrap().as_deref(), Some("Destroyed"));
    }

    #[test]
    fn the_software_adapter_is_said_to_be_software() {
        // WARP is in every Windows; elsewhere there may be no software adapter.
        match Gpu::new(Preference::Software) {
            Ok(gpu) => assert!(gpu.is_software(), "{gpu:?}"),
            Err(error) if cfg!(windows) => panic!("no WARP: {error}"),
            Err(error) => eprintln!("skipped: no software adapter: {error}"),
        }
    }

    fn fxc() -> ShaderCompiler {
        ShaderCompiler {
            kind: CompilerKind::Fxc,
            by: ChosenBy::Default,
        }
    }

    /// A folder of its own under the temporary one, with these files, empty.
    fn folder_with(name: &str, files: &[&str]) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "columbina-compose-gpu-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for file in files {
            std::fs::write(folder.join(file), b"").unwrap();
        }
        folder
    }

    #[test]
    fn the_compiler_named_comes_first_then_dxc_beside_the_executable_then_fxc() {
        let both = folder_with("both", &["dxcompiler.dll", "dxil.dll"]);
        let alone = folder_with("alone", &["dxcompiler.dll"]);
        let named = |value: &str| choose(Some(OsStr::new(value)), Some(&both));
        let by_variable = |kind| ShaderCompiler {
            kind,
            by: ChosenBy::Variable,
        };
        assert_eq!(named("auto"), by_variable(CompilerKind::Auto));
        assert_eq!(named("AUTO"), by_variable(CompilerKind::Auto));
        assert_eq!(named("fxc"), by_variable(CompilerKind::Fxc));
        // Any other value is FXC, as it always was, whatever lies beside the executable.
        assert_eq!(named("dxc"), by_variable(CompilerKind::Fxc));
        assert_eq!(
            named(r"C:\SDK\x64\DXCompiler.dll"),
            by_variable(CompilerKind::Dxc(PathBuf::from(
                r"C:\SDK\x64\DXCompiler.dll"
            )))
        );
        // Empty, it names nothing.
        assert_eq!(
            choose(Some(OsStr::new("")), Some(&both)),
            ShaderCompiler {
                kind: CompilerKind::Dxc(both.join("dxcompiler.dll")),
                by: ChosenBy::BesideExecutable,
            }
        );
        assert_eq!(
            choose(None, Some(&both)),
            ShaderCompiler {
                kind: CompilerKind::Dxc(both.join("dxcompiler.dll")),
                by: ChosenBy::BesideExecutable,
            }
        );
        // dxcompiler.dll without the dxil.dll that signs what it compiles is not taken.
        assert_eq!(choose(None, Some(&alone)), fxc());
        assert_eq!(choose(None, None), fxc());
        let _ = std::fs::remove_dir_all(both);
        let _ = std::fs::remove_dir_all(alone);
    }

    #[test]
    fn the_compiler_is_named_in_the_log_with_why() {
        let dxc = ShaderCompiler {
            kind: CompilerKind::Dxc(PathBuf::from(r"C:\engine\dxcompiler.dll")),
            by: ChosenBy::BesideExecutable,
        };
        assert_eq!(
            (dxc.name(), dxc.path(), dxc.reason()),
            (
                "dxc",
                Some(Path::new(r"C:\engine\dxcompiler.dll")),
                "beside the executable"
            )
        );
        assert!(
            matches!(dxc.wgpu(), wgpu::Dx12Compiler::DynamicDxc { dxc_path } if dxc_path == r"C:\engine\dxcompiler.dll")
        );
        assert_eq!(
            (fxc().name(), fxc().path(), fxc().reason()),
            ("fxc", None, "default")
        );
        assert!(matches!(fxc().wgpu(), wgpu::Dx12Compiler::Fxc));
    }

    #[test]
    fn the_tests_compile_with_fxc_as_nothing_lies_beside_them() {
        // The DLLs are never copied into target/ (see tests/dxc.rs for a test with them).
        if std::env::var_os(COMPILER_VARIABLE).is_none() {
            assert_eq!(ShaderCompiler::chosen(), fxc());
        }
    }

    #[test]
    fn a_dxc_that_makes_no_device_gives_way_to_fxc_for_the_rest_of_the_process() {
        // A dxcompiler.dll that is not there: Direct3D 12 cannot be had with it at all. WARP,
        // which is in every Windows, draws, so that no card is needed. Only Windows compiles
        // with DXC or FXC.
        if !cfg!(windows) {
            return;
        }
        let missing = ShaderCompiler {
            kind: CompilerKind::Dxc(std::env::temp_dir().join("no-such-folder/dxcompiler.dll")),
            by: ChosenBy::Variable,
        };
        let given_up = OnceLock::new();
        match Gpu::new_with(Preference::Software, missing, &given_up) {
            Ok(gpu) => {
                assert_eq!(
                    gpu.shader_compiler(),
                    Some(&ShaderCompiler {
                        kind: CompilerKind::Fxc,
                        by: ChosenBy::DxcFailed,
                    })
                );
                let why = gpu.dxc_failure().expect("why DXC was given up");
                assert_eq!(given_up.get().map(String::as_str), Some(why));
                // The card draws with FXC.
                let module = gpu
                    .device()
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: None,
                        source: wgpu::ShaderSource::Wgsl(
                            "@compute @workgroup_size(1) fn main() {}".into(),
                        ),
                    });
                let scope = gpu.device().push_error_scope(wgpu::ErrorFilter::Validation);
                let _ = gpu
                    .device()
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: None,
                        layout: None,
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        cache: None,
                    });
                assert!(pollster::block_on(scope.pop()).is_none());
            }
            Err(error) => panic!("no WARP with FXC: {error}"),
        }
    }
}
