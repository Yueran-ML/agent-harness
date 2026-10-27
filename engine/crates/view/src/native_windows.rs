//! The window of a view on Windows: a child `HWND` of the parent, made and served by a thread
//! of its own that does nothing but take its messages (see the crate's docs).
//!
//! The other threads never touch the window: they post `WM_SYNC` to it, and its thread makes it
//! match what the view's state says (where it is, whether it is seen). Posting never waits.

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::{mpsc, Arc, Mutex, Once, OnceLock, PoisonError};
use std::time::Duration;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindow,
    LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowPos, TranslateMessage,
    HTTRANSPARENT, HWND_TOP, IDC_ARROW, MA_NOACTIVATE, MSG, SWP_HIDEWINDOW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY,
    WS_EX_TRANSPARENT, WS_VISIBLE,
};

use crate::{RawParent, Rect, Shared, ViewError, Worker};

/// The class of the windows of views.
const CLASS: PCWSTR = w!("ColumbinaView");

/// Make the window match the view's state.
const WM_SYNC: u32 = WM_APP + 1;
/// Destroy the window, and end its thread.
const WM_END: u32 = WM_APP + 2;

/// How long the window's thread is waited for to make the window.
const MAKING: Duration = Duration::from_secs(10);

/// The backends that the view draws with on Windows.
pub(crate) const BACKENDS: wgpu::Backends = wgpu::Backends::DX12;

thread_local! {
    /// The state of the view whose window this thread serves.
    static SHARED: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}

pub(crate) fn prepare_process() {
    AWARENESS.get_or_init(|| {
        // SAFETY: it takes a constant and changes the process; it fails, harmlessly, where the
        // awareness was set already (by the manifest, or by a caller before).
        let set =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        // SAFETY: they read the awareness of this thread, which a new thread takes from the
        // process.
        let is_v2 = unsafe {
            AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
        }
        .as_bool();
        match (set, is_v2) {
            (Ok(()), _) => "per_monitor_v2".into(),
            (Err(_), true) => "per_monitor_v2 (set before)".into(),
            (Err(error), false) => {
                // SAFETY: as above.
                let awareness =
                    unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
                format!(
                    "not per_monitor_v2 (awareness {}): SetProcessDpiAwarenessContext failed: \
                     {error}; rectangles in device pixels will be off",
                    awareness.0
                )
            }
        }
    });
}

/// How the process is aware of the DPI of monitors, as [`prepare_process`] left it.
static AWARENESS: OnceLock<String> = OnceLock::new();

/// How the process is aware of the DPI of monitors, as it is reported.
pub(crate) fn dpi_awareness() -> String {
    AWARENESS
        .get()
        .cloned()
        .unwrap_or_else(|| "not set: prepare_process was not called".into())
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

/// The window of a view, and its thread.
pub(crate) struct Native {
    hwnd: isize,
    pump: Option<Worker>,
}

/// What asks the window's thread to make the window match the view's state.
#[derive(Clone)]
pub(crate) struct Syncer {
    hwnd: isize,
}

impl Syncer {
    pub(crate) fn sync(&self) {
        // SAFETY: posting to a window that is no more fails, and nothing else is done.
        let _ = unsafe { PostMessageW(Some(hwnd(self.hwnd)), WM_SYNC, WPARAM(0), LPARAM(0)) };
    }
}

impl Native {
    /// Make the window in `parent`, on a thread of its own, which serves it until it is
    /// destroyed.
    pub(crate) fn open(
        parent: RawParent,
        rect: Rect,
        shared: &Arc<Shared>,
    ) -> Result<Native, ViewError> {
        let RawParent::Windows(parent) = parent else {
            return Err(ViewError::Unsupported(
                "on Windows the parent of a view is an HWND".into(),
            ));
        };
        // SAFETY: IsWindow takes any value, and only says whether it is a window.
        if parent == 0 || !unsafe { IsWindow(Some(hwnd(parent))) }.as_bool() {
            return Err(ViewError::Parent(format!("{parent} is not a window")));
        }
        let (made, making) = mpsc::channel();
        // Taken away once the window is no longer waited for: the window's thread then destroys
        // the window it makes, rather than leave it in the parent.
        let made = Arc::new(Mutex::new(Some(made)));
        let pump = Worker::spawn("columbina-view-window", {
            let shared = Arc::clone(shared);
            let made = Arc::clone(&made);
            move || serve(parent, rect, &shared, &made)
        })?;
        let answer = match making.recv_timeout(MAKING) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Whatever was sent before this is taken; after it, nothing more is.
                made.lock().unwrap_or_else(PoisonError::into_inner).take();
                making.try_recv().ok()
            }
            answer => answer.ok(),
        };
        match answer {
            Some(Ok(window)) => Ok(Native {
                hwnd: window,
                pump: Some(pump),
            }),
            Some(Err(message)) => {
                pump.finish(MAKING);
                Err(ViewError::Window(message))
            }
            None => Err(ViewError::Window(format!(
                "it was not made within {} s",
                MAKING.as_secs()
            ))),
        }
    }

    pub(crate) fn handle(&self) -> usize {
        self.hwnd.cast_unsigned()
    }

    pub(crate) fn syncer(&self) -> Syncer {
        Syncer { hwnd: self.hwnd }
    }

    /// Destroy the window, and wait for its thread to end for at most `within`.
    pub(crate) fn close(&mut self, within: Duration) {
        if let Some(pump) = self.pump.take() {
            // SAFETY: as in `Syncer::sync`; a window destroyed already has ended its thread.
            let _ = unsafe { PostMessageW(Some(hwnd(self.hwnd)), WM_END, WPARAM(0), LPARAM(0)) };
            pump.finish(within);
        }
    }
}

/// The window's thread: make the window, say so, and take its messages until it is destroyed.
fn serve(
    parent: isize,
    rect: Rect,
    shared: &Arc<Shared>,
    made: &Mutex<Option<mpsc::Sender<Result<isize, String>>>>,
) {
    SHARED.with(|slot| *slot.borrow_mut() = Some(Arc::clone(shared)));
    let window = make(parent, rect);
    // Said while the waiter cannot stop waiting, so that a window is either taken or destroyed.
    let waited = {
        let made = made.lock().unwrap_or_else(PoisonError::into_inner);
        match (&*made, &window) {
            (Some(made), Ok(window)) => {
                shared.change(|state| state.sized((rect.width, rect.height)));
                made.send(Ok(window.0 as isize)).is_ok()
            }
            (Some(made), Err(message)) => {
                let _ = made.send(Err(message.clone()));
                false
            }
            (None, _) => false,
        }
    };
    if !waited {
        if let Ok(window) = window {
            // No one waits for it any more.
            // SAFETY: the window is this thread's own.
            let _ = unsafe { DestroyWindow(window) };
        }
        SHARED.with(|slot| slot.borrow_mut().take());
        return;
    }
    let mut message = MSG::default();
    // SAFETY: the message is this thread's own, and is handed back as it came.
    unsafe {
        while GetMessageW(&raw mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    SHARED.with(|slot| slot.borrow_mut().take());
}

/// Make the window, shown, on top of the parent's other children.
fn make(parent: isize, rect: Rect) -> Result<HWND, String> {
    static CLASS_ONCE: Once = Once::new();
    // SAFETY: the module of the process is always there.
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    CLASS_ONCE.call_once(|| {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).expect("a small struct"),
            lpfnWndProc: Some(window_procedure),
            hInstance: instance.into(),
            // SAFETY: a cursor of the system, which is never freed.
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpszClassName: CLASS,
            // No brush: the window is never painted by the system, only drawn by the card.
            ..WNDCLASSEXW::default()
        };
        // SAFETY: the class names a procedure of this module and a name that lives for ever.
        // Where it fails, making the window fails, and says so.
        unsafe { RegisterClassExW(&raw const class) };
    });
    // Disabled, so that the input of the user goes to the page beneath it; never activated;
    // and the parent, in another process, is not sent WM_PARENTNOTIFY, which would wait for it.
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_DISABLED;
    let extended = WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOPARENTNOTIFY;
    // SAFETY: the parent was a window a moment ago; if it is not now, this fails. The class was
    // registered above.
    let window = unsafe {
        CreateWindowExW(
            extended,
            CLASS,
            w!("Columbina view"),
            style,
            rect.x,
            rect.y,
            i32::try_from(rect.width).unwrap_or(i32::MAX),
            i32::try_from(rect.height).unwrap_or(i32::MAX),
            Some(hwnd(parent)),
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    // SAFETY: the window is this thread's own.
    let _ = unsafe {
        SetWindowPos(
            window,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                | SWP_NOACTIVATE,
        )
    };
    Ok(window)
}

/// Make the window match the view's state: where it is, and whether it is seen. It is put on top
/// of the parent's other children again, as the page's own windows may have come over it.
fn sync(window: HWND, shared: &Shared) {
    let (rect, visible) = {
        let state = shared.lock();
        (state.rect(), state.visible())
    };
    let shown = if visible {
        SWP_SHOWWINDOW
    } else {
        SWP_HIDEWINDOW
    };
    // SAFETY: the window is this thread's own.
    let _ = unsafe {
        SetWindowPos(
            window,
            Some(HWND_TOP),
            rect.x,
            rect.y,
            i32::try_from(rect.width).unwrap_or(i32::MAX),
            i32::try_from(rect.height).unwrap_or(i32::MAX),
            SWP_NOACTIVATE | shown,
        )
    };
    shared.change(|state| state.sized((rect.width, rect.height)));
}

fn with_shared(work: impl FnOnce(&Shared)) {
    if let Some(shared) = SHARED.with(|slot| slot.borrow().clone()) {
        work(&shared);
    }
}

/// The procedure of the window. It never waits for another thread: it changes the view's state
/// and wakes the drawer, at most.
unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Never the target of the mouse, nor activated by it (it is disabled besides).
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(isize::try_from(MA_NOACTIVATE).expect("a small constant")),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: the window is this thread's own, and is painted between the two.
            unsafe {
                BeginPaint(window, &raw mut paint);
                let _ = EndPaint(window, &raw const paint);
            }
            with_shared(Shared::expose);
            LRESULT(0)
        }
        WM_SYNC => {
            with_shared(|shared| sync(window, shared));
            LRESULT(0)
        }
        WM_END => {
            // SAFETY: the window is this thread's own.
            let _ = unsafe { DestroyWindow(window) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // Destroyed by the view, or with its parent: either way its thread ends.
            with_shared(Shared::window_gone);
            // SAFETY: it only posts to this thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the arguments are the system's, handed on as they came.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// The surface of the window of `handle`, for the card to draw in.
pub(crate) fn create_surface(
    instance: &wgpu::Instance,
    handle: usize,
) -> Result<wgpu::Surface<'static>, String> {
    let window = NonZeroIsize::new(handle.cast_signed()).ok_or("the view has no window")?;
    // SAFETY: the window outlives the surface: the view lets its drawer, which holds the
    // surface, end before it destroys the window, and a drawer whose window was destroyed with
    // its parent ends at once, presenting nothing more. A surface of a window destroyed while
    // it draws only reports itself lost.
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(window)),
        })
    }
    .map_err(|error| format!("the window takes no surface: {error}"))
}
