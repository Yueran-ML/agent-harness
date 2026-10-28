//! The drawer: the thread of a view that owns the surface of its window and draws into it when
//! there is something new to draw (see the crate's docs).

use std::sync::{mpsc, Arc, PoisonError};
use std::time::{Duration, Instant};

use columbina_compose::{Gpu, Watch};
use columbina_media::Picture;

use crate::native::{self, Syncer};
use crate::render::Painter;
use crate::{frame_begun, frame_ended, Event, Pending, Shared};

/// How long a frame that was not shown waits before it is tried again.
const RETRY: Duration = Duration::from_millis(50);

/// How many times in a row a frame that was not shown is tried again.
const RETRIES: u32 = 10;

/// How long the drawer waits before it tries again to draw with a card that it could not draw
/// in the window with.
const REMAKE: Duration = Duration::from_secs(1);

/// How often a card that still cannot draw in the window is reported.
const REPORT_REMAKING: Duration = Duration::from_secs(30);

/// The name of the adapter and of its API, or why there is none.
pub(crate) type Ready = Result<(String, String), String>;

/// The surface and what draws in it, on the card that the view was given; made again on the
/// next card that it is given, once that one is lost.
struct Card {
    screen: Option<Screen>,
    /// When a card that could not draw in the window is tried again: pictures that come before
    /// only take the place of the last one.
    remake_at: Option<Instant>,
    /// How many times in a row it could not draw, and when that was last reported.
    failures: u32,
    reported_at: Option<Instant>,
}

impl Card {
    /// What draws with `gpu`, made again if it was made on another card or on one that was
    /// lost; none while `gpu` is lost (until the view is given another), or until it can draw
    /// in the window.
    fn ready(
        &mut self,
        shared: &Arc<Shared>,
        handle: usize,
        gpu: &Arc<Gpu>,
    ) -> Option<&mut Screen> {
        if self
            .screen
            .as_ref()
            .is_some_and(|screen| screen.is_lost() || !Arc::ptr_eq(&screen.gpu, gpu))
        {
            self.screen = None;
            self.remake_at = None;
        }
        if self.screen.is_none() {
            if gpu.is_lost() || self.remake_at.is_some_and(|at| Instant::now() < at) {
                return None;
            }
            match Screen::new(shared, handle, gpu) {
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
                                "the graphics card cannot draw in the window ({} tries, one a \
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

/// Draw in the window of `handle` with the card that the view was given, and then with each one
/// that it is given, until the view ends or its window is destroyed. Says on `ready` once the
/// card is ready to draw in the window, or why it cannot be.
///
/// A card is held only by the state and by what draws with it, so that one that was lost is let
/// go, with the swap chain of the window, as soon as another takes its place: a window has one
/// swap chain at a time.
pub(crate) fn run(
    shared: &Arc<Shared>,
    syncer: &Syncer,
    handle: usize,
    ready: &mpsc::Sender<Ready>,
) {
    let first = Arc::clone(&shared.lock().gpu);
    let made = Screen::new(shared, handle, &first);
    drop(first);
    let mut card = match made {
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
            // As a driver reset would: the device is destroyed, and the view waits for another
            // card, as for a device that is lost.
            if let Some(screen) = card.screen.take() {
                screen.gpu.device().destroy();
                screen.gpu.mark_lost("lost on purpose, by a test");
            }
        }
        let Some(screen) = card.ready(shared, handle, &job.gpu) else {
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
            // Lost in the frame: drawn again once the view is given another card.
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
    /// The card to draw with.
    gpu: Arc<Gpu>,
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
                gpu: Arc::clone(&state.gpu),
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

/// The surface of a window, and what draws in it, on a card.
struct Screen {
    shared: Arc<Shared>,
    handle: usize,
    gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    painter: Painter,
    config: wgpu::SurfaceConfiguration,
    /// Whether the surface is configured as `config` says.
    configured: bool,
    /// Tells the view when the card is lost.
    _watch: Watch,
    /// The number of the picture whose planes the painter holds.
    uploaded: Option<u64>,
    adapter: String,
    backend: String,
}

impl Screen {
    fn new(shared: &Arc<Shared>, handle: usize, gpu: &Arc<Gpu>) -> Result<Screen, String> {
        if let Some(why) = gpu.lost_reason() {
            return Err(format!("the graphics card was lost: {why}"));
        }
        let surface = native::create_surface(gpu.instance(), handle)?;
        let adapter = gpu.adapter();
        if !adapter.is_surface_supported(&surface) {
            return Err(format!(
                "the graphics adapter {} does not draw in the window",
                gpu.name()
            ));
        }
        let capabilities = surface.get_capabilities(adapter);
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
        let watch = gpu.when_lost({
            let shared = Arc::downgrade(shared);
            move |why: &str| {
                if let Some(shared) = shared.upgrade() {
                    shared.report(Event::DeviceLost {
                        message: why.to_owned(),
                    });
                    // Woken, to let the surface go.
                    shared.change(|state| state.dirty = true);
                }
            }
        });
        Ok(Screen {
            shared: Arc::clone(shared),
            handle,
            painter: Painter::new(gpu.device(), gpu.queue(), format),
            gpu: Arc::clone(gpu),
            surface,
            config,
            configured: false,
            _watch: watch,
            uploaded: None,
            adapter: gpu.name().to_owned(),
            backend: gpu.backend().to_owned(),
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
        self.gpu.is_lost()
    }

    /// Configure the surface as `config` says, catching what the card refuses.
    fn configure(&mut self) -> Result<(), String> {
        let scope = self
            .gpu
            .device()
            .push_error_scope(wgpu::ErrorFilter::Validation);
        self.surface.configure(self.gpu.device(), &self.config);
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
        let surface = native::create_surface(self.gpu.instance(), self.handle)?;
        if !self.gpu.adapter().is_surface_supported(&surface) {
            // The adapter no longer draws in the window: the card is taken as lost, and the
            // owner of the engine's makes another.
            self.gpu
                .mark_lost("the adapter no longer draws in the window");
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
