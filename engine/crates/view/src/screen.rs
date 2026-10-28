//! The drawer: the thread of a view that owns the graphics card and draws into the window when
//! there is something new to draw (see the crate's docs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, PoisonError};
use std::time::{Duration, Instant};

use columbina_media::Picture;

use crate::native::{self, Syncer};
use crate::render::{self, Painter};
use crate::{frame_begun, frame_ended, Event, Pending, Shared};

/// How long a frame that was not shown waits before it is tried again.
const RETRY: Duration = Duration::from_millis(50);

/// How many times in a row a frame that was not shown is tried again.
const RETRIES: u32 = 10;

/// How long the drawer waits before it tries again to make a device that it could not make.
const REMAKE: Duration = Duration::from_secs(1);

/// How often a device that still cannot be made is reported.
const REPORT_REMAKING: Duration = Duration::from_secs(30);

/// The name of the adapter and of its API, or why there is none.
pub(crate) type Ready = Result<(String, String), String>;

/// The device, once made, and when it is made again after it is lost.
struct Card {
    screen: Option<Screen>,
    /// When a device that could not be made is tried again: pictures that come before only
    /// take the place of the last one.
    remake_at: Option<Instant>,
    /// How many times in a row it could not be made, and when that was last reported.
    failures: u32,
    reported_at: Option<Instant>,
}

impl Card {
    /// The device, made again if it was lost and it is time to; none until it can be.
    fn ready(&mut self, shared: &Arc<Shared>, handle: usize) -> Option<&mut Screen> {
        if self.screen.as_ref().is_some_and(Screen::is_lost) {
            self.screen = None;
        }
        if self.screen.is_none() {
            if self.remake_at.is_some_and(|at| Instant::now() < at) {
                return None;
            }
            match Screen::new(shared, handle) {
                Ok(made) => {
                    shared.report(made.ready_event());
                    self.screen = Some(made);
                    self.remake_at = None;
                    self.failures = 0;
                    self.reported_at = None;
                }
                Err(message) => {
                    self.failures += 1;
                    self.remake_at = Some(Instant::now() + REMAKE);
                    if self
                        .reported_at
                        .is_none_or(|at| at.elapsed() >= REPORT_REMAKING)
                    {
                        self.reported_at = Some(Instant::now());
                        shared.report(Event::Error {
                            message: format!(
                                "the graphics card cannot be made ready again ({} tries, one a \
                                 second): {message}",
                                self.failures
                            ),
                        });
                    }
                    return None;
                }
            }
        }
        self.screen.as_mut()
    }
}

/// Draw in the window of `handle` until the view ends or its window is destroyed. Says on
/// `ready` once the card is ready, or why it cannot be.
pub(crate) fn run(
    shared: &Arc<Shared>,
    syncer: &Syncer,
    handle: usize,
    ready: &mpsc::Sender<Ready>,
) {
    let mut card = match Screen::new(shared, handle) {
        Ok(screen) => {
            let _ = ready.send(Ok((screen.adapter.clone(), screen.backend.clone())));
            shared.report(screen.ready_event());
            Card {
                screen: Some(screen),
                remake_at: None,
                failures: 0,
                reported_at: None,
            }
        }
        Err(message) => {
            let _ = ready.send(Err(message));
            return;
        }
    };
    // The picture shown, how it is turned, and its number among the pictures given.
    let mut shown: Option<(Arc<Picture>, i16, u64)> = None;
    let mut given = 0_u64;
    let mut retries = 0;
    let mut again_at: Option<Instant> = None;
    loop {
        let Some(job) = next(shared, &mut again_at) else {
            return;
        };
        match job.pending {
            Pending::Nothing => {}
            Pending::Picture(picture, rotation) => {
                given += 1;
                shown = Some((picture, rotation, given));
            }
            Pending::Clear => shown = None,
        }
        if job.lose {
            // As a driver reset would: the device is destroyed, and everything on it is made
            // again, as for a device that is lost.
            if let Some(screen) = card.screen.take() {
                screen.device.destroy();
                shared.report(Event::DeviceLost {
                    message: "lost on purpose, by a test".into(),
                });
            }
        }
        let Some(screen) = card.ready(shared, handle) else {
            again_at = card.remake_at;
            continue;
        };
        if job.size.0 == 0 || job.size.1 == 0 {
            continue;
        }
        frame_begun(shared);
        if let Some(hang) = job.stall {
            // As a card that hangs: for the tests of the watchdog.
            std::thread::sleep(hang);
        }
        let outcome = screen.frame(&mut shown, job.size);
        frame_ended(shared, syncer, outcome == Outcome::Presented);
        if screen.is_lost() {
            // Lost in the frame: made again at once, and the frame drawn again.
            again_at = Some(Instant::now() + RETRY);
            continue;
        }
        match outcome {
            Outcome::Presented | Outcome::Failed => retries = 0,
            Outcome::NotShown => {
                if retries < RETRIES {
                    retries += 1;
                    again_at = Some(Instant::now() + RETRY);
                } else {
                    retries = 0;
                }
            }
        }
    }
}

/// What there is to do.
struct Job {
    /// The picture asked for since the last frame.
    pending: Pending,
    /// The size of the window.
    size: (u32, u32),
    /// Whether a test asked for the device to be lost.
    lose: bool,
    /// How long a test asked this frame to hang.
    stall: Option<Duration>,
}

/// Wait for something to draw in a window that is seen and has pixels, or, at `again_at`, to
/// draw again what was not shown. None once the view ends or its window is destroyed.
fn next(shared: &Shared, again_at: &mut Option<Instant>) -> Option<Job> {
    let mut state = shared.lock();
    loop {
        if state.quit || state.gone {
            return None;
        }
        let drawable = state.dirty && state.visible() && state.size.0 > 0 && state.size.1 > 0;
        if drawable || state.lose_device {
            state.dirty = false;
            *again_at = None;
            return Some(Job {
                pending: std::mem::replace(&mut state.pending, Pending::Nothing),
                size: state.size,
                lose: std::mem::take(&mut state.lose_device),
                stall: state.stall.take(),
            });
        }
        state = match *again_at {
            Some(at) => {
                let wait = at.saturating_duration_since(Instant::now());
                let (mut state, waited) = shared
                    .wake
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner);
                if waited.timed_out() {
                    *again_at = None;
                    state.dirty = true;
                }
                state
            }
            None => shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// What came of drawing a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Presented,
    /// The window is covered, or the card was slow to give an image: to be tried again.
    NotShown,
    /// An error, which was reported.
    Failed,
}

/// The graphics card, ready to draw in a window.
struct Screen {
    shared: Arc<Shared>,
    handle: usize,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter_handle: wgpu::Adapter,
    device: wgpu::Device,
    painter: Painter,
    config: wgpu::SurfaceConfiguration,
    /// Whether the surface is configured as `config` says.
    configured: bool,
    /// Set when the device is lost.
    lost: Arc<AtomicBool>,
    /// The number of the picture whose planes the painter holds.
    uploaded: Option<u64>,
    adapter: String,
    backend: String,
}

impl Screen {
    fn new(shared: &Arc<Shared>, handle: usize) -> Result<Screen, String> {
        // wgpu panics at an instance made where it has none of the backends of the platform
        // (on Linux, where the view is never made: `Native::open` refuses it first).
        if !wgpu::Instance::enabled_backend_features().intersects(native::BACKENDS) {
            return Err("wgpu has no backend to draw with on this platform".into());
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: native::BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = native::create_surface(&instance, handle)?;
        let adapter_handle =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            }))
            .map_err(|error| format!("no graphics adapter draws in the window: {error}"))?;
        let (device, queue) =
            pollster::block_on(adapter_handle.request_device(&wgpu::DeviceDescriptor {
                label: Some("columbina-view"),
                required_features: render::wanted_features(&adapter_handle),
                required_limits: render::wanted_limits(&adapter_handle),
                ..wgpu::DeviceDescriptor::default()
            }))
            .map_err(|error| format!("the graphics device cannot be made: {error}"))?;
        let lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let lost = Arc::clone(&lost);
            let shared = Arc::clone(shared);
            move |reason, message| {
                lost.store(true, Ordering::SeqCst);
                shared.report(Event::DeviceLost {
                    message: format!("{reason:?}: {message}"),
                });
            }
        });
        device.on_uncaptured_error(Arc::new({
            let shared = Arc::clone(shared);
            move |error: wgpu::Error| {
                shared.report(Event::Error {
                    message: error.to_string(),
                });
            }
        }));
        let capabilities = surface.get_capabilities(&adapter_handle);
        let format = surface_format(&capabilities.formats)
            .ok_or("the window takes no format that the view draws")?;
        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: 1,
            height: 1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        let info = adapter_handle.get_info();
        Ok(Screen {
            shared: Arc::clone(shared),
            handle,
            painter: Painter::new(&device, &queue, format),
            instance,
            surface,
            adapter_handle,
            device,
            config,
            configured: false,
            lost,
            uploaded: None,
            adapter: info.name,
            backend: format!("{:?}", info.backend),
        })
    }

    /// The event that says it is ready.
    fn ready_event(&self) -> Event {
        Event::Ready {
            adapter: self.adapter.clone(),
            backend: self.backend.clone(),
            dpi_awareness: native::dpi_awareness(),
        }
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Configure the surface as `config` says, catching what the card refuses.
    fn configure(&mut self) -> Result<(), String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.surface.configure(&self.device, &self.config);
        let error = pollster::block_on(scope.pop());
        self.configured = error.is_none();
        if let Some(error) = error {
            return Err(format!("the surface cannot be configured: {error}"));
        }
        Ok(())
    }

    /// Draw `shown` into a window of `size`, and present it. A picture that cannot be uploaded
    /// is let go, and black is drawn.
    fn frame(&mut self, shown: &mut Option<(Arc<Picture>, i16, u64)>, size: (u32, u32)) -> Outcome {
        match shown {
            Some((picture, _, number)) => {
                let number = *number;
                if self.uploaded != Some(number) {
                    if let Err(message) = self.painter.upload(picture) {
                        self.shared.report(Event::Error { message });
                        self.painter.forget();
                        *shown = None;
                        self.uploaded = None;
                    } else {
                        self.uploaded = Some(number);
                    }
                }
            }
            None => {
                if self.uploaded.take().is_some() {
                    self.painter.forget();
                }
            }
        }
        let rotation = shown.as_ref().map_or(0, |(_, rotation, _)| *rotation);
        if !self.configured || (self.config.width, self.config.height) != size {
            self.config.width = size.0;
            self.config.height = size.1;
            if let Err(message) = self.configure() {
                self.shared.report(Event::Error { message });
                return Outcome::Failed;
            }
        }
        // An outdated surface is configured again and a lost one made again, once each.
        for _ in 0..3 {
            let texture = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture) => texture,
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    // Configured again for the next frame.
                    self.configured = false;
                    texture
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Outcome::NotShown;
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if let Err(message) = self.configure() {
                        self.shared.report(Event::Error { message });
                        return Outcome::Failed;
                    }
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    if self.is_lost() {
                        return Outcome::Failed;
                    }
                    self.shared.report(Event::SurfaceLost);
                    if let Err(message) = self.remake_surface() {
                        self.shared.report(Event::Error { message });
                        return Outcome::Failed;
                    }
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    self.configured = false;
                    return Outcome::Failed;
                }
            };
            let view = texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let commands = self.painter.draw(&view, size, rotation);
            self.painter.queue().submit([commands]);
            self.painter.queue().present(texture);
            return Outcome::Presented;
        }
        Outcome::Failed
    }

    /// Make the surface of the window again, on the same device.
    fn remake_surface(&mut self) -> Result<(), String> {
        self.configured = false;
        let surface = native::create_surface(&self.instance, self.handle)?;
        if !self.adapter_handle.is_surface_supported(&surface) {
            // The adapter no longer draws in the window: the device is made again.
            self.lost.store(true, Ordering::SeqCst);
            return Err("the adapter no longer draws in the window".into());
        }
        self.surface = surface;
        self.configure()
    }
}

/// The format of a surface that the view draws into: 8 bits that are not sRGB, else the first
/// that is not sRGB, else none.
fn surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|format| formats.contains(format))
    .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
}
