//! The window of a view on Windows: a child `HWND` of the parent, made and served by a thread
//! of its own that does nothing but take its messages (see the crate's docs).
//!
//! The other threads never touch the window: they post `WM_SYNC` to it, and its thread makes it
//! match what the view's state says (where it is, whether it is seen, whether it takes the
//! pointer). Posting never waits.
//!
//! While the view takes the pointer, the window is enabled, answers `HTCLIENT` to hit tests and
//! is not `WS_EX_TRANSPARENT`, so that the mouse over it is its own; a button pressed over it
//! captures the mouse until the last button is released, so that a drag goes on beyond it. Each
//! press, move and release is reported with where it is on the picture. Otherwise the window is as
//! it always was, and a mouse message that reaches it anyway (posted to it) is not reported.

use std::cell::{Cell, RefCell};
use std::num::NonZeroIsize;
use std::sync::{mpsc, Arc, Mutex, Once, OnceLock, PoisonError};
use std::time::Duration;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, ScreenToClient, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetAsyncKeyState, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL,
    VK_LBUTTON, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, IsWindow, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetWindowLongPtrW, SetWindowPos, TranslateMessage, CS_DBLCLKS,
    GWL_EXSTYLE, HTCLIENT, HTTRANSPARENT, HWND_TOP, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZENS,
    IDC_SIZEWE, MA_NOACTIVATE, MSG, SM_SWAPBUTTON, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    WM_APP, WM_CAPTURECHANGED, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETCURSOR, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_EX_NOACTIVATE,
    WS_EX_NOPARENTNOTIFY, WS_EX_TRANSPARENT, WS_VISIBLE,
};

use crate::pointer::{Button, Pointer, PointerKind};
use crate::{Cursor, Event, RawParent, Rect, Shared, ViewError, Worker};

/// The class of the windows of views.
const CLASS: PCWSTR = w!("ColumbinaView");

/// Make the window match the view's state.
const WM_SYNC: u32 = WM_APP + 1;
/// Destroy the window, and end its thread.
const WM_END: u32 = WM_APP + 2;

/// How long the window's thread is waited for to make the window.
const MAKING: Duration = Duration::from_secs(10);

thread_local! {
    /// The state of the view whose window this thread serves.
    static SHARED: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
    /// The buttons that were pressed over the window and are held, a bit each, as
    /// [`Pointer::buttons`] has them.
    static HELD: Cell<u8> = const { Cell::new(0) };
    /// Buttons whose capture ended at pointer-off, but whose physical release the next
    /// owner still needs. These bits never own or reacquire native capture.
    static RELEASE_PENDING: Cell<u8> = const { Cell::new(0) };
    /// Where the mouse last was in the window.
    static LAST: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static MIDDLE_ORIGIN: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
    static MIDDLE_MOVED: Cell<bool> = const { Cell::new(false) };
}

/// The keys and buttons held, as the messages of the mouse give them in `wparam`.
const MK_LBUTTON: u32 = 0x1;
const MK_RBUTTON: u32 = 0x2;
const MK_SHIFT: u32 = 0x4;
const MK_CONTROL: u32 = 0x8;
const MK_MBUTTON: u32 = 0x10;

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
            style: CS_DBLCLKS,
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
    let (rect, visible, pointer) = {
        let state = shared.lock();
        (state.rect(), state.visible(), state.pointer().is_some())
    };
    take_pointer(window, pointer);
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

/// Make the window the target of the mouse, or let the mouse through it to the page beneath.
fn take_pointer(window: HWND, take: bool) {
    // SAFETY: the window is this thread's own; its style is read and written as a whole number.
    unsafe {
        let extended = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let transparent = isize::try_from(WS_EX_TRANSPARENT.0).expect("a small constant");
        let wanted = if take {
            extended & !transparent
        } else {
            extended | transparent
        };
        if !take && retire_held() {
            // Clear owned capture before EnableWindow/ReleaseCapture can reenter through
            // WM_CAPTURECHANGED. The pending physical release is a separate lifetime.
            let _ = ReleaseCapture();
        }
        if wanted != extended {
            SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
        }
        let _ = EnableWindow(window, take);
        if take && wanted != extended && RELEASE_PENDING.with(Cell::get) != 0 {
            // One snapshot at the handoff boundary, not a poll. A release while disabled
            // may have gone to the parent rather than this child.
            let buttons = held_buttons();
            let released = synchronized_releases(buttons);
            let keys = (u32::from(GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0) * MK_SHIFT)
                | (u32::from(GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0) * MK_CONTROL);
            for button in [Button::Left, Button::Right, Button::Middle] {
                if released & bit(button) != 0 {
                    report(
                        PointerKind::Up,
                        Some(button),
                        buttons,
                        LAST.with(Cell::get),
                        keys,
                        0,
                        0,
                    );
                }
            }
        }
    }
}

fn retire_held() -> bool {
    let held = HELD.with(|held| held.replace(0));
    RELEASE_PENDING.with(|pending| pending.set(pending.get() | held));
    MIDDLE_ORIGIN.with(|origin| origin.set(None));
    MIDDLE_MOVED.with(|moved| moved.set(false));
    held != 0
}

fn synchronized_releases(buttons: u8) -> u8 {
    RELEASE_PENDING.with(|pending| {
        let released = pending.get() & !buttons;
        pending.set(pending.get() & buttons);
        released
    })
}

fn release_button(button: Button) -> bool {
    let bit = bit(button);
    let held = HELD.with(|held| held.replace(held.get() & !bit));
    let pending = RELEASE_PENDING.with(|pending| pending.replace(pending.get() & !bit));
    (held | pending) & bit != 0
}

fn logical_buttons(physical: u8, swapped: bool) -> u8 {
    if swapped {
        (physical & 4) | ((physical & 1) << 1) | ((physical & 2) >> 1)
    } else {
        physical
    }
}

/// Current logical mouse buttons, sampled only at a handoff or explicit read-only request.
pub(crate) fn held_buttons() -> u8 {
    // SAFETY: these only read the physical high bits and the current primary-button mapping.
    unsafe {
        logical_buttons(
            u8::from(GetAsyncKeyState(i32::from(VK_LBUTTON.0)) < 0)
                | u8::from(GetAsyncKeyState(i32::from(VK_RBUTTON.0)) < 0) << 1
                | u8::from(GetAsyncKeyState(i32::from(VK_MBUTTON.0)) < 0) << 2,
            GetSystemMetrics(SM_SWAPBUTTON) != 0,
        )
    }
}

/// Whether the view takes the pointer, and the cursor it shows then.
fn pointer_taken() -> Option<Cursor> {
    let mut taken = None;
    with_shared(|shared| taken = shared.lock().pointer());
    taken
}

/// The button of a message of a button, and whether it was pressed.
fn button_of(message: u32) -> Option<(Button, bool)> {
    match message {
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => Some((Button::Left, true)),
        WM_LBUTTONUP => Some((Button::Left, false)),
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => Some((Button::Right, true)),
        WM_RBUTTONUP => Some((Button::Right, false)),
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => Some((Button::Middle, true)),
        WM_MBUTTONUP => Some((Button::Middle, false)),
        _ => None,
    }
}

fn bit(button: Button) -> u8 {
    match button {
        Button::Left => 1,
        Button::Right => 2,
        Button::Middle => 4,
    }
}

/// Report what a message of the mouse did, where the view takes the pointer: a press captures
/// the mouse, and the release of the last button held lets it go.
fn mouse(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the coordinates are the two signed words of lparam"
    )]
    let mut point = (
        i32::from((lparam.0.cast_unsigned() as u16).cast_signed()),
        i32::from(((lparam.0.cast_unsigned() >> 16) as u16).cast_signed()),
    );
    if message == WM_MOUSEWHEEL {
        let mut client = POINT {
            x: point.0,
            y: point.1,
        };
        // SAFETY: WM_MOUSEWHEEL carries screen coordinates; this thread owns the child HWND.
        if !unsafe { ScreenToClient(window, &raw mut client) }.as_bool() {
            return;
        }
        point = (client.x, client.y);
    }
    LAST.with(|last| last.set(point));
    if MIDDLE_ORIGIN
        .with(Cell::get)
        .is_some_and(|origin| origin != point)
    {
        MIDDLE_MOVED.with(|moved| moved.set(true));
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the flags are in the low word"
    )]
    let keys = wparam.0 as u32;
    let buttons = u8::from(keys & MK_LBUTTON != 0)
        | u8::from(keys & MK_RBUTTON != 0) << 1
        | u8::from(keys & MK_MBUTTON != 0) << 2;
    let (kind, button) = match button_of(message) {
        Some((button, true)) => {
            RELEASE_PENDING.with(|pending| pending.set(pending.get() & !bit(button)));
            if button == Button::Middle {
                MIDDLE_ORIGIN.with(|origin| origin.set(Some(point)));
                MIDDLE_MOVED.with(|moved| moved.set(false));
            }
            if HELD.with(Cell::get) == 0 {
                // SAFETY: the window is this thread's own.
                unsafe { SetCapture(window) };
            }
            HELD.with(|held| held.set(held.get() | bit(button)));
            (PointerKind::Down, Some(button))
        }
        Some((button, false)) => {
            if !release_button(button) {
                // Pressed elsewhere, released here: not a gesture of the view's.
                return;
            }
            (PointerKind::Up, Some(button))
        }
        None if message == WM_MOUSEWHEEL => (PointerKind::Wheel, None),
        None => (PointerKind::Move, None),
    };
    RELEASE_PENDING.with(|pending| pending.set(pending.get() & buttons));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the native wheel delta is the signed high word"
    )]
    let delta = if message == WM_MOUSEWHEEL {
        ((wparam.0 >> 16) as u16).cast_signed()
    } else {
        0
    };
    let clicks = if matches!(
        message,
        WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK
    ) {
        2
    } else {
        u8::from(kind == PointerKind::Down)
    };
    report(kind, button, buttons, point, keys, delta, clicks);
    if kind == PointerKind::Up {
        if button == Some(Button::Middle) {
            MIDDLE_ORIGIN.with(|origin| origin.set(None));
            MIDDLE_MOVED.with(|moved| moved.set(false));
        }
        if HELD.with(Cell::get) == 0 {
            // SAFETY: it lets go of the mouse that this thread captured, after reporting Up.
            let _ = unsafe { ReleaseCapture() };
        }
    }
}

/// Report what the pointer did at `point` of the window.
fn report(
    kind: PointerKind,
    button: Option<Button>,
    buttons: u8,
    point: (i32, i32),
    keys: u32,
    delta: i16,
    clicks: u8,
) {
    // SAFETY: it reads the state of a key of this thread's input, which is joined to the page's.
    let alt = unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
    with_shared(|shared| {
        let (on, presentation) = {
            let state = shared.lock();
            (state.on_picture(point), state.presentation)
        };
        shared.report(Event::Pointer(Pointer {
            kind,
            button,
            buttons,
            delta,
            clicks,
            middle_moved: MIDDLE_MOVED.with(Cell::get),
            picture: on.map(|(at, _)| at),
            presentation,
            inside: on.is_some_and(|(_, inside)| inside),
            window: point,
            shift: keys & MK_SHIFT != 0,
            ctrl: keys & MK_CONTROL != 0,
            alt,
        }));
    });
}

/// The cursor of the system for `cursor`.
fn cursor_of(cursor: Cursor) -> PCWSTR {
    match cursor {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Crosshair => IDC_CROSS,
        Cursor::Move => IDC_SIZEALL,
        Cursor::ResizeEw => IDC_SIZEWE,
        Cursor::ResizeNs => IDC_SIZENS,
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
        // Never the target of the mouse (it is disabled besides), unless it takes the pointer;
        // never activated by it.
        WM_NCHITTEST => {
            if pointer_taken().is_some() {
                LRESULT(isize::try_from(HTCLIENT).expect("a small constant"))
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_MOUSEACTIVATE => LRESULT(isize::try_from(MA_NOACTIVATE).expect("a small constant")),
        WM_SETCURSOR => match pointer_taken() {
            Some(cursor) => {
                // SAFETY: a cursor of the system, which is never freed.
                unsafe { SetCursor(LoadCursorW(None, cursor_of(cursor)).ok()) };
                LRESULT(1)
            }
            // SAFETY: the arguments are the system's, handed on as they came.
            None => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        },
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MOUSEWHEEL | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK
        | WM_MBUTTONDBLCLK => {
            if pointer_taken().is_some() {
                mouse(window, message, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            // Another window took the mouse while a button was held: the drag ends there.
            let held = HELD.with(|held| held.replace(0));
            if held != 0 && lparam.0 != window.0 as isize && pointer_taken().is_some() {
                report(PointerKind::Cancel, None, 0, LAST.with(Cell::get), 0, 0, 0);
                RELEASE_PENDING.with(|pending| pending.set(0));
            }
            MIDDLE_ORIGIN.with(|origin| origin.set(None));
            MIDDLE_MOVED.with(|moved| moved.set(false));
            LRESULT(0)
        }
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

#[cfg(test)]
mod button_tests {
    use super::*;

    #[test]
    fn physical_buttons_follow_the_logical_primary_mapping() {
        for middle in [false, true] {
            let middle_bit = u8::from(middle) << 2;
            assert_eq!(logical_buttons(1 | middle_bit, false), 1 | middle_bit);
            assert_eq!(logical_buttons(1 | middle_bit, true), 2 | middle_bit);
            assert_eq!(logical_buttons(2 | middle_bit, false), 2 | middle_bit);
            assert_eq!(logical_buttons(2 | middle_bit, true), 1 | middle_bit);
            assert_eq!(logical_buttons(3 | middle_bit, true), 3 | middle_bit);
            assert_eq!(logical_buttons(middle_bit, true), middle_bit);
        }
    }

    #[test]
    fn a_retired_capture_keeps_only_unobserved_physical_releases() {
        for button in [Button::Left, Button::Right, Button::Middle] {
            HELD.with(|held| held.set(bit(button)));
            RELEASE_PENDING.with(|pending| pending.set(0));
            assert!(retire_held());
            assert_eq!(HELD.with(Cell::get), 0);
            assert_eq!(synchronized_releases(bit(button)), 0); // Still physically held at on.
            assert!(release_button(button)); // A later native Up remains recognizable.
            assert!(!release_button(button)); // No unrelated/duplicate release.
            HELD.with(|held| held.set(bit(button)));
            assert!(retire_held());
            assert_eq!(synchronized_releases(0), bit(button)); // Released while off.
            assert_eq!(synchronized_releases(0), 0);
            assert!(!release_button(button));
        }
    }
}
