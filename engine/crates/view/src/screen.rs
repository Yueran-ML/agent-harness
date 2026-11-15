//! The drawer: the thread of a view that owns the surface of its window and draws into it when
//! there is something new to draw (see the crate's docs).

use std::sync::{mpsc, Arc, PoisonError};
use std::time::{Duration, Instant};

use columbina_compose::Gpu;

use crate::display::{self, choose_surface, Content, Display, Monitor, Output};
use crate::marks::Marks;
use crate::native::{self, Syncer};
use crate::render::Painter;
use crate::window_overlay::render::OverlayPainter;
use crate::{frame_begun, frame_ended, Event, Frame, Pending, Shared, Viewport};

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
            // Drain completed submissions before replacing a live device's swapchain.
            // A single view can otherwise leave its last surface commands uncollected.
            // Do not wait for a lost device, or hold Shared while waiting for the GPU.
            if let Some(screen) = self.screen.as_ref().filter(|screen| !screen.is_lost()) {
                // The old screen is retained on timeout: this backoff must also apply
                // while screen is Some, including when the producer keeps sending frames.
                if self.remake_at.is_some_and(|at| Instant::now() < at) {
                    return None;
                }
                if screen
                    .gpu
                    .device()
                    .poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(RETRY),
                    })
                    .is_err()
                {
                    self.remake_at = Some(Instant::now() + REMAKE);
                    return None;
                }
            }
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
    let mut shown: Option<(Frame, u64)> = None;
    let mut given = 0_u64;
    let mut retries = 0;
    let mut again_at: Option<Instant> = None;
    loop {
        let check_at = card.screen.as_ref().map(Screen::check_at);
        let Some(job) = next(shared, &mut again_at, check_at) else {
            return;
        };
        if job.check {
            // The monitor read again, while nothing is drawn: drawn again if it changed.
            if let Some(screen) = card.screen.as_mut() {
                if screen.check_display() {
                    shared.change(|state| state.dirty = true);
                }
            }
            continue;
        }
        match job.pending {
            Pending::Nothing => {}
            Pending::Frame(frame) => {
                given += 1;
                shown = Some((frame, given));
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
        if screen.display_due(job.wish.hdr_allowed) {
            // Before the frame, which the watchdog watches.
            screen.check_display();
        }
        frame_begun(shared);
        if let Some(hang) = job.stall {
            // As a card that hangs: for the tests of the watchdog.
            std::thread::sleep(hang);
        }
        let outcome = screen.frame(&mut shown, job.size, job.wish, job.marks.as_deref());
        frame_ended(shared, syncer);
        if screen.is_lost() {
            // Lost in the frame: drawn again once the view is given another card.
            continue;
        }
        match outcome {
            Outcome::Presented => retries = 0,
            Outcome::Failed => {
                // Tried again at its own pace, not only when there is something new to draw.
                retries = 0;
                again_at = Some(Instant::now() + REMAKE);
            }
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

/// How the view is to show its pictures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wish {
    /// Whether it may show HDR.
    hdr_allowed: bool,
    /// What the pictures hold.
    content: Content,
    viewport: Viewport,
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
    /// How the pictures are to be shown.
    wish: Wish,
    /// What a page has the view draw over its picture.
    marks: Option<Arc<Marks>>,
    /// Only to read the monitor again (at `check_at`), and draw nothing.
    check: bool,
}

/// Wait for something to draw in a window that is seen and has pixels, or, at `again_at`, to
/// draw again what was not shown, or, at `check_at` while HDR is allowed, to read the monitor
/// again. None once the view ends or its window is destroyed.
fn next(shared: &Shared, again_at: &mut Option<Instant>, check_at: Option<Instant>) -> Option<Job> {
    let mut state = shared.lock();
    loop {
        if state.quit || state.gone {
            return None;
        }
        let wish = Wish {
            hdr_allowed: state.hdr_allowed,
            content: state.content,
            viewport: state.viewport,
        };
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
                wish,
                marks: state.marks.clone(),
                check: false,
            });
        }
        let check_at = check_at.filter(|_| state.hdr_allowed);
        if check_at.is_some_and(|at| Instant::now() >= at) {
            return Some(Job {
                pending: Pending::Nothing,
                size: state.size,
                lose: false,
                stall: None,
                gpu: Arc::clone(&state.gpu),
                wish,
                marks: None,
                check: true,
            });
        }
        let wake_at = match (*again_at, check_at) {
            (Some(again), Some(check)) => Some(again.min(check)),
            (again, check) => again.or(check),
        };
        state = match wake_at {
            Some(at) => {
                let wait = at.saturating_duration_since(Instant::now());
                let (mut state, _) = shared
                    .wake
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner);
                if again_at.is_some_and(|at| Instant::now() >= at) {
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
    /// Made only for frames with Window controls, and only for the final surface format.
    overlay: Option<OverlayPainter>,
    output: Output,
    config: wgpu::SurfaceConfiguration,
    /// What the surface takes, to choose its format and colour space again as the monitor
    /// changes.
    capabilities: wgpu::SurfaceCapabilities,
    /// What the monitor said of itself, and when it was last read.
    monitor: Option<Monitor>,
    checked_at: Instant,
    /// What was last reported of what the view shows.
    reported: Option<Display>,
    /// Whether the surface is configured as `config` says.
    configured: bool,
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
        let monitor = Monitor::of(&surface.display_hdr_info(adapter));
        let hdr_allowed = shared.lock().hdr_allowed;
        let choice = choose_surface(&capabilities, monitor.as_ref(), hdr_allowed)
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
            format: choice.format,
            color_space: choice.color_space,
            width: 1,
            height: 1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        Ok(Screen {
            shared: Arc::clone(shared),
            handle,
            painter: Painter::new(gpu.device(), gpu.queue(), choice.format),
            overlay: None,
            output: choice.output,
            gpu: Arc::clone(gpu),
            surface,
            config,
            capabilities,
            monitor,
            checked_at: Instant::now(),
            reported: None,
            configured: false,
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

    /// When the monitor is next read, while HDR is allowed.
    fn check_at(&self) -> Instant {
        self.checked_at + display::CHECK
    }

    /// Whether the monitor is to be read again before the next frame.
    fn display_due(&self, hdr_allowed: bool) -> bool {
        hdr_allowed && Instant::now() >= self.check_at()
    }

    /// Read the monitor that the window is on again; whether what it says changed.
    fn check_display(&mut self) -> bool {
        self.checked_at = Instant::now();
        let monitor = Monitor::of(&self.surface.display_hdr_info(self.gpu.adapter()));
        if monitor == self.monitor {
            return false;
        }
        self.monitor = monitor;
        true
    }

    /// Make the surface and the painter show the pictures as `wish` and the monitor say: the
    /// surface configured again, and the painter made again for its format, where it changes.
    fn apply(&mut self, wish: Wish) -> Result<(), String> {
        let choice = choose_surface(&self.capabilities, self.monitor.as_ref(), wish.hdr_allowed)
            .ok_or("the window takes no format that the view draws")?;
        if (choice.format, choice.color_space) != (self.config.format, self.config.color_space) {
            if choice.format != self.config.format {
                self.painter = Painter::new(self.gpu.device(), self.gpu.queue(), choice.format);
                self.overlay = None;
                self.uploaded = None;
            }
            self.config.format = choice.format;
            self.config.color_space = choice.color_space;
            self.configured = false;
        }
        if self
            .painter
            .set_mapping(display::mapping(wish.content, choice.output))
        {
            self.uploaded = None;
        }
        self.output = choice.output;
        let shown = Display {
            monitor: self.monitor,
            output: choice.output,
        };
        if self.reported != Some(shown) {
            self.reported = Some(shown);
            self.shared.lock().display = Some(shown);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "nits of a monitor, rounded"
            )]
            let nits = |nits: Option<f32>| nits.map(|nits| nits.round() as u32);
            self.shared.report(Event::Display {
                hdr_on: self.monitor.is_some_and(|monitor| monitor.hdr),
                hdr_shown: matches!(choice.output, Output::Hdr { .. }),
                peak_nits: nits(self.monitor.and_then(|monitor| monitor.peak_nits)),
                sdr_white_nits: nits(self.monitor.and_then(|monitor| monitor.sdr_white_nits)),
            });
        }
        Ok(())
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

    /// Draw `shown` into a window of `size`, and present it, with `marks` over it where it is a
    /// composed picture of their size. A picture that cannot be uploaded is let go, and black
    /// is drawn.
    fn frame(
        &mut self,
        shown: &mut Option<(Frame, u64)>,
        size: (u32, u32),
        wish: Wish,
        marks: Option<&Marks>,
    ) -> Outcome {
        if let Err(message) = self.apply(wish) {
            self.shared.report(Event::Error { message });
            return Outcome::Failed;
        }
        // CPU pictures can be uploaded again after recovery. A composed texture belongs to
        // its old device and must be replaced by the producer, never sampled on a new one.
        if shown.as_ref().is_some_and(|(frame, _)| matches!(frame, Frame::Texture { gpu, .. } if !Arc::ptr_eq(gpu, &self.gpu))) {
            *shown = None;
        }
        match shown {
            Some((frame, number)) => {
                let number = *number;
                if self.uploaded != Some(number) {
                    let uploaded = match frame {
                        Frame::Picture(picture, _, _) => self.painter.upload(picture),
                        Frame::Texture { texture, .. } => self.painter.use_texture(texture),
                    };
                    if let Err(message) = uploaded {
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
        let rotation = match shown {
            Some((Frame::Picture(_, rotation, _), _)) => *rotation,
            _ => 0,
        };
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
            let map = self.painter.viewport_map(size, rotation, wish.viewport);
            let overlay = shown.as_ref().and_then(|(frame, _)| match frame {
                Frame::Texture { overlay, .. } => overlay.as_deref(),
                Frame::Picture(..) => None,
            });
            // The marks are in pixels of a composed picture: a picture of media is not one.
            let marks =
                marks.filter(|_| matches!(shown.as_ref(), Some((Frame::Texture { .. }, _))));
            let commands = self.draw(&view, size, rotation, map, overlay, marks);
            self.painter.queue().submit([commands]);
            self.painter.queue().present(texture);
            let tag = shown.as_ref().and_then(|(frame, _)| match frame {
                Frame::Picture(_, _, tag) | Frame::Texture { tag, .. } => *tag,
            });
            self.shared.presented(&self.gpu, tag, map);
            return Outcome::Presented;
        }
        Outcome::Failed
    }

    fn draw(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        rotation: i16,
        map: Option<crate::ViewportMap>,
        overlay: Option<&crate::window_overlay::WindowOverlay>,
        marks: Option<&Marks>,
    ) -> wgpu::CommandBuffer {
        let Some(map) = map else {
            return self.painter.draw_mapped(target, size, rotation, map);
        };
        let marks = marks.filter(|marks| marks.fits(map));
        if overlay.is_none() && marks.is_none() {
            return self.painter.draw_mapped(target, size, rotation, Some(map));
        }
        let painter = self
            .overlay
            .get_or_insert_with(|| OverlayPainter::new(self.gpu.device(), self.config.format));
        self.painter
            .draw_mapped_with(target, size, rotation, Some(map), |encoder| {
                // The marks of the page lie under the Window controls, which take the pointer.
                if let Some(marks) = marks {
                    painter.encode_marks(
                        self.gpu.device(),
                        encoder,
                        target,
                        marks,
                        map,
                        self.output,
                    );
                }
                if let Some(overlay) = overlay {
                    painter.encode(
                        self.gpu.device(),
                        encoder,
                        target,
                        overlay,
                        map,
                        self.output,
                    );
                }
            })
    }

    /// Make the surface of the window again, on the same device.
    fn remake_surface(&mut self) -> Result<(), String> {
        self.shared.lock().presentation = None;
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
