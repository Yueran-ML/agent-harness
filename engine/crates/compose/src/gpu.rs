//! The graphics card of the engine: one instance, adapter, device and queue, which the views and
//! the compositor share, so that a picture composed on the card is shown from the card.
//!
//! A device can be lost (a driver reset, the card removed): then everything made on it is of no
//! more use, and a new [`Gpu`] is made in its place by whoever owns the engine's (the server),
//! which hands it to the views and makes the compositor again. Those who use a `Gpu` learn that
//! it was lost with [`Gpu::when_lost`], or ask [`Gpu::is_lost`]; nothing of this is an error of
//! theirs.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

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
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Make the instance, choose an adapter as `preference` says, and make a device of it,
    /// with 16-bit normalized textures where the adapter has them (for pictures of P010) and
    /// textures and buffers as large as it takes. This takes some 100 ms: call it where
    /// blocking is allowed.
    ///
    /// # Errors
    /// There is no adapter, not even in software, or no device can be made.
    pub fn new(preference: Preference) -> Result<Arc<Gpu>, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
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
            Preference::Software => ask(true),
        }
        .map_err(|error| GpuError::NoAdapter(error.to_string()))?;
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
        Ok(Arc::new(Gpu {
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
            watchers,
        }))
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

/// A [`Gpu`] for the tests that draw: none where there is no adapter, unless
/// `COLUMBINA_REQUIRE_GPU` is 1, when there must be one.
///
/// # Panics
/// `COLUMBINA_REQUIRE_GPU` is 1, and there is no adapter.
#[doc(hidden)]
#[must_use]
pub fn gpu_for_tests() -> Option<Arc<Gpu>> {
    match Gpu::new(Preference::HighPerformance) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
}
