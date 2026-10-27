//! The picture, drawn in a window of its own inside the window of the application.
//!
//! The desktop app is an Electron window whose page lays out the editor; the engine, a process of
//! its own, draws the picture of the viewer with the graphics card ([wgpu]) into a native child
//! window that it makes inside the page's window, over the rectangle that the page keeps for the
//! viewer. The page is told nothing of the picture, and no frame crosses between the processes.
//!
//! - [`View::attach`] makes the child window in a parent, given by its native handle (an `HWND`
//!   on Windows), at a [`Rect`] of the parent's client area in **device pixels**.
//! - [`View::place`] moves it, as the page's layout moves; [`View::show`] hides it while an
//!   overlay of the page (a menu, a dialog) lies over the viewer, since a native window always
//!   covers the page beneath it ("airspace"): the page hides the view as the overlay opens.
//! - [`View::present`] hands it a picture, newest first: it returns at once, and the picture is
//!   drawn by a thread of the view. [`View::clear`] draws black.
//!
//! # Threads
//!
//! A view has three threads of its own, none of them the caller's:
//!
//! - **the window's**, which made the window and does nothing but take its messages. A child
//!   window of another process ties the input of its thread to the parent's thread, so that a
//!   thread that stopped taking messages would stop the app's input: this one only ever takes
//!   them. It moves, shows and hides the window when the other threads ask it to with a posted
//!   message; no other thread touches the window, and no one waits for this one.
//! - **the drawer's**, which owns the graphics card (an `Instance`, `Adapter`, `Device` and the
//!   `Surface` of the window) and draws when there is something new to draw: a picture, a new
//!   size, the window shown again or uncovered (`WM_PAINT`). It never draws on a clock, so an idle
//!   view takes no time at all, and a playing one is drawn as often as pictures come, at the rate
//!   of the media (pictures that come faster than it draws are skipped: the newest wins). It
//!   draws with v-sync (`Fifo`), one frame queued at most.
//! - **the watchdog's**, which sleeps until a frame is begun. If a frame (uploading the picture,
//!   taking an image of the window, drawing, presenting it) takes more than [`STALL`] (1 s), as
//!   when the card hangs or a driver resets, the window is hidden, so that the page beneath it
//!   shows and nothing the user sees is frozen, and [`Event::Stalled`] is reported. Once the frame
//!   returns, the window is shown again, if it is meant to be shown, and drawn
//!   ([`Event::Recovered`]).
//!
//! # The card failing
//!
//! A surface that is outdated (the window changed) is configured again; one that is lost is made
//! again ([`Event::SurfaceLost`]). A device that is lost (a driver reset, the card removed) is
//! made again, with all that was on it, from a new adapter, and the last picture is drawn again
//! ([`Event::DeviceLost`]); if no device can be made, it is tried again a second later, while
//! there is something to draw. A frame that is not shown because the window is covered or the
//! card is slow to give an image is tried again a few times, 50 ms apart, then left until the
//! next thing to draw. Nothing of this is an error of the caller's.
//!
//! # Colour
//!
//! A picture is NV12 or P010 ([`Picture`]), taken as BT.709 of limited range (phase 0: colour
//! is managed in phase 2), turned into R'G'B' as it is drawn, and written as it is, its gamma
//! kept, to a surface that is not sRGB (`Bgra8Unorm`), so that it is not encoded twice. Pixels
//! are taken as square. A picture is fitted into the window, turned as its media says, its shape
//! kept, and centred, with black bars ([`letterbox`]).
//!
//! # Platforms
//!
//! On **Windows** the window is a child `HWND` (`WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS |
//! WS_DISABLED`, with `WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOPARENTNOTIFY`), drawn with
//! Direct3D 12. It is disabled so that the clicks and keys of the user go to the page beneath
//! it: a window that only answers `HTTRANSPARENT` to hit tests lets through the input of its own
//! thread, not the real input of the user. The process is made aware of the DPI of each monitor
//! ([`prepare_process`]), as Electron is, so that rectangles are in device pixels on both sides.
//!
//! On **macOS** the view is an `NSView` added to the parent `NSView`, drawn with Metal, that
//! answers no hit test, so that the page gets the input. It is **not tested**, and it works only
//! where the parent view lives in the same process (see `native_macos.rs`).
//!
//! Elsewhere [`View::attach`] fails with [`ViewError::Unsupported`].
//!
//! Untested on Windows: the border of the parent dragged by the user (the modal loop of sizing),
//! a change of DPI as the parent moves to another monitor, fullscreen, the loss of the device in
//! fact (it is made to be lost by a test), and sleep and resume.

use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

mod render;
mod screen;

#[cfg(windows)]
#[allow(unsafe_code)]
mod native_windows;
#[cfg(windows)]
use native_windows as native;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod native_macos;
#[cfg(target_os = "macos")]
use native_macos as native;

#[cfg(not(any(windows, target_os = "macos")))]
mod native_none;
#[cfg(not(any(windows, target_os = "macos")))]
use native_none as native;

pub use columbina_media::Picture;
pub use render::{letterbox, quarter_turns, Placement};

/// How long a frame may take before the window is hidden.
pub const STALL: Duration = Duration::from_secs(1);

/// How long [`View::attach`] waits for the graphics card to be ready.
const READY: Duration = Duration::from_secs(20);

/// How long a view that is dropped waits for each of its threads to end, before it leaves it.
const ENDING: Duration = Duration::from_secs(2);

/// A rectangle of the parent's client area, in device pixels, from its top left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// The widest and highest a view may be, as the surfaces of the cards are.
    pub const MOST: u32 = 16384;

    fn checked(self) -> Result<Rect, ViewError> {
        if self.width > Self::MOST || self.height > Self::MOST {
            return Err(ViewError::Rect(format!(
                "a view of {}×{} is larger than {} on a side",
                self.width,
                self.height,
                Self::MOST
            )));
        }
        Ok(self)
    }
}

/// The native handle of the window that a view is made in, as the platform has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawParent {
    /// An `HWND`.
    Windows(isize),
    /// An `NSView *`, of this process.
    MacOS(usize),
}

impl RawParent {
    /// The handle of a parent of this platform, written in decimal, as a client sends it: an
    /// `HWND` on Windows, which may be written as signed or unsigned, and an `NSView *` on macOS.
    /// None if it is not a whole number of this platform's handles, or is 0.
    #[must_use]
    pub fn from_decimal(text: &str) -> Option<RawParent> {
        let value: i128 = text.parse().ok()?;
        if value == 0 {
            return None;
        }
        if cfg!(target_os = "macos") {
            usize::try_from(value).ok().map(RawParent::MacOS)
        } else {
            // A handle written unsigned, as a Node Buffer of 8 bytes reads, is the same bits.
            let bits = u64::try_from(value)
                .ok()
                .or_else(|| i64::try_from(value).ok().map(i64::cast_unsigned))?;
            isize::try_from(bits.cast_signed())
                .ok()
                .map(RawParent::Windows)
        }
    }
}

/// Why a view cannot be made or used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// The parent is not a window.
    #[error("the parent is not a window: {0}")]
    Parent(String),
    /// The rectangle is not one that a view can have.
    #[error("{0}")]
    Rect(String),
    /// The window could not be made.
    #[error("the window of the view cannot be made: {0}")]
    Window(String),
    /// Nothing can be drawn: no graphics card, or none that draws in the window.
    #[error("the view cannot draw: {0}")]
    Gpu(String),
    /// The window was destroyed, with its parent.
    #[error("the window of the view was destroyed with its parent")]
    Gone,
    /// Views are not made on this platform, or of this kind of parent.
    #[error("{0}")]
    Unsupported(String),
}

impl ViewError {
    /// A word for the error, for a program to read.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            ViewError::Parent(_) => "parent",
            ViewError::Rect(_) => "rect",
            ViewError::Window(_) => "window",
            ViewError::Gpu(_) => "gpu",
            ViewError::Gone => "gone",
            ViewError::Unsupported(_) => "unsupported",
        }
    }
}

/// What happened to a view, as it is reported to whoever made it, for its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The graphics card is ready to draw: the name of its adapter and of the API.
    Ready { adapter: String, backend: String },
    /// A frame took more than [`STALL`]: the window is hidden.
    Stalled,
    /// The frame that stalled returned, after so many milliseconds.
    Recovered { millis: u64 },
    /// The device was lost, and is made again.
    DeviceLost { message: String },
    /// The surface of the window was lost, and is made again.
    SurfaceLost,
    /// Something went wrong that the view goes on from: a picture that cannot be drawn, an
    /// error of the card.
    Error { message: String },
}

/// What reports the [`Event`]s of a view. It is called on the view's threads, and must not
/// block.
pub type Report = Arc<dyn Fn(Event) + Send + Sync>;

/// Make the process ready for views, once, before it makes any window: on Windows it is made
/// aware of the DPI of each monitor (`PER_MONITOR_AWARE_V2`), as Electron is, so that the
/// rectangles that the page sends in device pixels are the window's. [`View::attach`] does it
/// too; a program that makes windows of its own calls it first.
pub fn prepare_process() {
    native::prepare_process();
}

/// A picture to draw, as the caller last asked.
enum Pending {
    Nothing,
    Picture(Arc<Picture>, u16),
    Clear,
}

/// What the threads of a view share.
pub(crate) struct Shared {
    state: Mutex<State>,
    /// Wakes the drawer.
    wake: Condvar,
    /// Wakes the watchdog.
    watch: Condvar,
    report: Report,
}

/// What is changed only while it is locked.
#[allow(
    clippy::struct_excessive_bools,
    reason = "flags of the view, each set by another hand"
)]
pub(crate) struct State {
    /// The picture to draw next, if a new one was asked for.
    pending: Pending,
    /// Where the caller wants the window.
    rect: Rect,
    /// Whether the caller wants the window shown.
    shown: bool,
    /// Whether the watchdog hid the window.
    stalled: bool,
    /// The size that the window has, once its thread has made it so.
    size: (u32, u32),
    /// Whether there is something new to draw.
    dirty: bool,
    /// Whether the view is ending.
    quit: bool,
    /// Whether the window was destroyed.
    gone: bool,
    /// Whether a test asked for the device to be lost.
    lose_device: bool,
    /// When the frame being drawn was begun.
    busy_since: Option<Instant>,
    /// How many frames were presented.
    presented: u64,
}

impl State {
    /// Whether the window is to be seen.
    pub(crate) fn visible(&self) -> bool {
        self.shown && !self.stalled
    }

    pub(crate) fn rect(&self) -> Rect {
        self.rect
    }

    /// The window has been given the size of its rectangle by its thread.
    pub(crate) fn sized(&mut self, size: (u32, u32)) {
        self.size = size;
        self.dirty = true;
    }
}

impl Shared {
    fn new(rect: Rect, report: Report) -> Self {
        Self {
            state: Mutex::new(State {
                pending: Pending::Nothing,
                rect,
                shown: true,
                stalled: false,
                size: (0, 0),
                dirty: true,
                quit: false,
                gone: false,
                lose_device: false,
                busy_since: None,
                presented: 0,
            }),
            wake: Condvar::new(),
            watch: Condvar::new(),
            report,
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Change the state, and wake the drawer.
    pub(crate) fn change(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.wake.notify_all();
    }

    /// The window must be drawn again: it was uncovered, or shown.
    pub(crate) fn expose(&self) {
        self.change(|state| state.dirty = true);
    }

    /// The window was destroyed, by its parent or by the view.
    pub(crate) fn window_gone(&self) {
        self.change(|state| state.gone = true);
        self.watch.notify_all();
    }

    pub(crate) fn report(&self, event: Event) {
        (self.report)(event);
    }
}

/// A thread of a view, which can be waited for a while and then left.
struct Worker {
    handle: JoinHandle<()>,
    /// Closed when the thread ends, however it ends.
    ended: mpsc::Receiver<()>,
}

impl Worker {
    fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> Result<Worker, ViewError> {
        let (alive, ended) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let _alive = alive;
                work();
            })
            .map_err(|error| ViewError::Window(format!("cannot start a thread: {error}")))?;
        Ok(Worker { handle, ended })
    }

    /// Wait for the thread to end, for at most `within`; past that, it is left to end alone.
    /// Whether it ended.
    fn finish(self, within: Duration) -> bool {
        if let Err(mpsc::RecvTimeoutError::Timeout) = self.ended.recv_timeout(within) {
            return false;
        }
        let _ = self.handle.join();
        true
    }
}

/// A child window that a picture is drawn in. Dropping it destroys the window and ends its
/// threads.
pub struct View {
    shared: Arc<Shared>,
    native: native::Native,
    syncer: native::Syncer,
    drawer: Option<Worker>,
    watchdog: Option<Worker>,
    adapter: String,
    backend: String,
}

impl View {
    /// Make a view in `parent`, at `rect` of its client area, and make the graphics card ready
    /// to draw in it; it is shown, black. This waits for the window to be made, by a thread of
    /// the view, and for the card, which takes some 100 ms: call it where blocking is allowed.
    ///
    /// # Errors
    /// The parent is not a window of this platform ([`ViewError::Parent`],
    /// [`ViewError::Unsupported`]), the rectangle is too large, the window cannot be made, or no
    /// graphics card can draw in it ([`ViewError::Gpu`]).
    pub fn attach(parent: RawParent, rect: Rect) -> Result<View, ViewError> {
        Self::attach_reporting(parent, rect, Arc::new(|_| {}))
    }

    /// As [`View::attach`], reporting what happens to the view to `report`.
    ///
    /// # Errors
    /// As [`View::attach`].
    pub fn attach_reporting(
        parent: RawParent,
        rect: Rect,
        report: Report,
    ) -> Result<View, ViewError> {
        let rect = rect.checked()?;
        prepare_process();
        let shared = Arc::new(Shared::new(rect, report));
        let native = native::Native::open(parent, rect, &shared)?;
        let syncer = native.syncer();
        let (ready, readied) = mpsc::channel();
        let drawer = Worker::spawn("columbina-view-draw", {
            let shared = Arc::clone(&shared);
            let syncer = syncer.clone();
            let handle = native.handle();
            move || screen::run(&shared, &syncer, handle, &ready)
        });
        let mut view = View {
            shared,
            native,
            syncer,
            drawer: None,
            watchdog: None,
            adapter: String::new(),
            backend: String::new(),
        };
        view.drawer = Some(drawer?);
        match readied.recv_timeout(READY) {
            Ok(Ok((adapter, backend))) => {
                view.adapter = adapter;
                view.backend = backend;
            }
            Ok(Err(message)) => return Err(ViewError::Gpu(message)),
            Err(_) => {
                return Err(ViewError::Gpu(format!(
                    "the graphics card was not ready within {} s",
                    READY.as_secs()
                )))
            }
        }
        view.watchdog = Some(Worker::spawn("columbina-view-watchdog", {
            let shared = Arc::clone(&view.shared);
            let syncer = view.syncer.clone();
            move || watchdog(&shared, &syncer)
        })?);
        Ok(view)
    }

    /// Move the view to `rect` of the parent's client area. It returns at once; the window's
    /// thread moves the window, and the picture is drawn again at its new size.
    ///
    /// # Errors
    /// The rectangle is too large, or the window was destroyed with its parent.
    pub fn place(&self, rect: Rect) -> Result<(), ViewError> {
        let rect = rect.checked()?;
        self.alive(|state| state.rect = rect)
    }

    /// Show the view, or hide it, as while an overlay of the page lies over it. It returns at
    /// once. A view that is hidden draws nothing; it draws the newest picture once it is shown.
    ///
    /// # Errors
    /// The window was destroyed with its parent.
    pub fn show(&self, shown: bool) -> Result<(), ViewError> {
        self.alive(|state| {
            state.shown = shown;
            state.dirty = true;
        })
    }

    /// Change the state of a view whose window is there, and have the window's thread make the
    /// window match it.
    fn alive(&self, change: impl FnOnce(&mut State)) -> Result<(), ViewError> {
        let mut state = self.shared.lock();
        if state.gone {
            return Err(ViewError::Gone);
        }
        change(&mut state);
        drop(state);
        self.shared.wake.notify_all();
        self.syncer.sync();
        Ok(())
    }

    /// Draw `picture`, turned `rotation` degrees clockwise (0, 90, 180 or 270), fitted into the
    /// view. It returns at once: the picture is drawn by the view's drawer, and a picture that
    /// comes before the last one was drawn takes its place. The view keeps the picture it drew
    /// last, to draw it again as the window changes, until the next one or [`View::clear`].
    pub fn present(&self, picture: Arc<Picture>, rotation: u16) {
        self.shared.change(|state| {
            state.pending = Pending::Picture(picture, rotation);
            state.dirty = true;
        });
    }

    /// Draw black, and let the last picture go.
    pub fn clear(&self) {
        self.shared.change(|state| {
            state.pending = Pending::Clear;
            state.dirty = true;
        });
    }

    /// The name of the graphics adapter that draws.
    #[must_use]
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The API that it draws with (`Dx12`, `Metal`).
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// The native handle of the view's own window (an `HWND`, an `NSView *`): for checks of the
    /// window itself, as the tests and Task 19 make.
    #[must_use]
    pub fn raw_handle(&self) -> usize {
        self.native.handle()
    }

    /// How many frames were presented so far.
    #[must_use]
    pub fn presented(&self) -> u64 {
        self.shared.lock().presented
    }

    /// Lose the device, as a driver reset would: for the tests, which see it made again.
    #[doc(hidden)]
    pub fn lose_device(&self) {
        self.shared.change(|state| {
            state.lose_device = true;
            state.dirty = true;
        });
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.shared.change(|state| state.quit = true);
        self.shared.watch.notify_all();
        // The drawer lets the surface go before the window is destroyed. A drawer that hangs
        // in the card is left, and lets it go when it returns.
        if let Some(drawer) = self.drawer.take() {
            drawer.finish(ENDING);
        }
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.finish(ENDING);
        }
        self.native.close(ENDING);
    }
}

/// Hide the window while a frame takes more than [`STALL`], until it returns.
fn watchdog(shared: &Shared, syncer: &native::Syncer) {
    let mut state = shared.lock();
    loop {
        if state.quit || state.gone {
            return;
        }
        let Some(since) = state.busy_since else {
            state = shared
                .watch
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if state.stalled || now < since + STALL {
            // Wait for the frame to end, or to stall.
            let wait = (since + STALL).saturating_duration_since(now);
            let wait = if state.stalled { STALL } else { wait };
            state = shared
                .watch
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        state.stalled = true;
        drop(state);
        shared.report(Event::Stalled);
        syncer.sync();
        state = shared.lock();
    }
}

/// A frame is begun: the watchdog watches it.
pub(crate) fn frame_begun(shared: &Shared) {
    shared.lock().busy_since = Some(Instant::now());
    shared.watch.notify_all();
}

/// A frame has ended, presented or not: a window that the watchdog hid is shown again, if it is
/// meant to be, and drawn.
pub(crate) fn frame_ended(shared: &Shared, syncer: &native::Syncer, presented: bool) {
    let mut state = shared.lock();
    let since = state.busy_since.take();
    if presented {
        state.presented += 1;
    }
    let recovered = state.stalled;
    if recovered {
        state.stalled = false;
        state.dirty = true;
    }
    drop(state);
    shared.watch.notify_all();
    if recovered {
        let millis = since.map_or(0, |since| {
            u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
        });
        shared.report(Event::Recovered { millis });
        syncer.sync();
        shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_is_read_in_decimal_as_this_platform_has_it() {
        assert_eq!(RawParent::from_decimal("0"), None);
        assert_eq!(RawParent::from_decimal(""), None);
        assert_eq!(RawParent::from_decimal("12x"), None);
        assert_eq!(RawParent::from_decimal("1.5"), None);
        if cfg!(target_os = "macos") {
            assert_eq!(
                RawParent::from_decimal("4096"),
                Some(RawParent::MacOS(4096))
            );
            assert_eq!(RawParent::from_decimal("-1"), None);
        } else {
            assert_eq!(
                RawParent::from_decimal("11407328"),
                Some(RawParent::Windows(11_407_328))
            );
            // Signed and unsigned, the same bits.
            assert_eq!(RawParent::from_decimal("-2"), Some(RawParent::Windows(-2)));
            assert_eq!(
                RawParent::from_decimal("18446744073709551614"),
                Some(RawParent::Windows(-2))
            );
            assert_eq!(RawParent::from_decimal("18446744073709551616"), None);
        }
    }

    #[test]
    fn a_rect_larger_than_a_surface_is_refused() {
        let rect = Rect {
            x: -5,
            y: 0,
            width: Rect::MOST,
            height: 1,
        };
        assert_eq!(rect.checked(), Ok(rect));
        let error = Rect {
            width: Rect::MOST + 1,
            ..rect
        }
        .checked()
        .unwrap_err();
        assert_eq!(error.reason(), "rect");
    }
}
