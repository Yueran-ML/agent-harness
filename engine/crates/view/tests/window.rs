//! A view in a real window on the desktop: a parent window of plain Win32 is made here, a view
//! is attached to it, and what the screen shows is read back (as the compositor shows it), and
//! a real click of the mouse is made over the view.
//!
//! The tests that need a desktop are ignored unless asked for:
//!
//! ```text
//! cargo test -p columbina-view --test window -- --ignored
//! ```
//!
//! They put a window on top of the others for a few seconds and move the mouse for a moment
//! (it is put back). They need the desktop to be seen: not locked, nothing of the system over
//! the window. Where it cannot be, `COLUMBINA_VIEW_LOCKED=1` reads what the window shows with
//! `PrintWindow` instead of from the screen, and makes no click: the drawing, the moving and
//! the hiding are checked, the input is not.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_view::{Event, Picture, RawParent, Rect, View, ViewError};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC,
    DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassNameW, GetClientRect, GetCursorPos,
    GetMessageW, GetParent, GetWindowLongW, IsWindow, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetCursorPos, TranslateMessage, WindowFromPoint, GWL_STYLE, MSG, WM_CLOSE,
    WM_DESTROY, WM_LBUTTONDOWN, WNDCLASSEXW, WS_CLIPCHILDREN, WS_DISABLED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

/// The colour of the parent's client area.
const BACKGROUND: [u8; 3] = [30, 160, 60];

/// How long anything that should happen soon is waited for.
const WAIT: Duration = Duration::from_secs(3);

static CLICKS: AtomicU32 = AtomicU32::new(0);
static CLICKED_X: AtomicI32 = AtomicI32::new(-1);
static CLICKED_Y: AtomicI32 = AtomicI32::new(-1);

unsafe extern "system" fn parent_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_LBUTTONDOWN => {
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = (
                i32::from((lparam.0 & 0xFFFF) as i16),
                i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            );
            CLICKED_X.store(x, Ordering::SeqCst);
            CLICKED_Y.store(y, Ordering::SeqCst);
            CLICKS.fetch_add(1, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

/// A top-level window, on top of the others, served by a thread of its own, as Electron's is.
struct Parent {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl Parent {
    fn open(x: i32, y: i32, width: i32, height: i32) -> Parent {
        let (made, making) = mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let [red, green, blue] = BACKGROUND.map(u32::from);
            let class = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap(),
                lpfnWndProc: Some(parent_procedure),
                hInstance: instance.into(),
                hbrBackground: CreateSolidBrush(COLORREF(red | green << 8 | blue << 16)),
                lpszClassName: w!("ColumbinaTestParent"),
                ..WNDCLASSEXW::default()
            };
            RegisterClassExW(&raw const class);
            let window = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("ColumbinaTestParent"),
                w!("columbina-view test"),
                WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            made.send(window.0 as isize).unwrap();
            let mut message = MSG::default();
            while GetMessageW(&raw mut message, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        });
        Parent {
            hwnd: making.recv().unwrap(),
            thread: Some(thread),
        }
    }

    /// Check that the window is what the screen shows at a point (where the user sees it): it is
    /// not when the desktop is locked, or a window of the system lies over it.
    fn assert_seen(&self, (x, y): (i32, i32)) {
        let deadline = Instant::now() + WAIT;
        loop {
            // The view is disabled: the window at the point is the parent.
            let found = unsafe { WindowFromPoint(POINT { x, y }) };
            if found.0 as isize == self.hwnd {
                return;
            }
            if Instant::now() >= deadline {
                let mut name = [0; 128];
                let length = unsafe { GetClassNameW(found, &mut name) };
                let name = String::from_utf16_lossy(&name[..usize::try_from(length).unwrap_or(0)]);
                panic!(
                    "the screen does not show the test's window at ({x}, {y}) but a window of \
                     class {name}: is the desktop locked?"
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Where a point of its client area is on the screen.
    fn on_screen(&self, x: i32, y: i32) -> (i32, i32) {
        let mut point = POINT { x, y };
        let _ = unsafe { ClientToScreen(hwnd(self.hwnd), &raw mut point) };
        (point.x, point.y)
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        let _ = unsafe { PostMessageW(Some(hwnd(self.hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The colour of a pixel of the screen, as the compositor shows it.
fn screen_pixel(x: i32, y: i32) -> [u8; 3] {
    let pixels = screen_area(x, y, 1, 1);
    [pixels[2], pixels[1], pixels[0]]
}

/// The pixels of an area of the screen, as the compositor shows it: BGRA, a row after another.
fn screen_area(x: i32, y: i32, width: i32, height: i32) -> Vec<u8> {
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap(),
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            Some(memory),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
        .unwrap();
        let old = SelectObject(memory, bitmap.into());
        BitBlt(memory, 0, 0, width, height, Some(screen), x, y, SRCCOPY).unwrap();
        let length = usize::try_from(width * height * 4).unwrap();
        let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec();
        SelectObject(memory, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
    }
}

/// The client area of a window as the compositor would show it, the drawing of DirectX in it
/// included, wherever the window is: `PrintWindow` with `PW_CLIENTONLY | PW_RENDERFULLCONTENT`.
/// BGRA, a row after another, and its width.
fn window_area(window: isize) -> (Vec<u8>, i32) {
    let mut client = windows::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(hwnd(window), &raw mut client) }.unwrap();
    let (width, height) = (client.right, client.bottom);
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap(),
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            Some(memory),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
        .unwrap();
        let old = SelectObject(memory, bitmap.into());
        assert!(PrintWindow(hwnd(window), memory, PRINT_WINDOW_FLAGS(1 | 2)).as_bool());
        let length = usize::try_from(width * height * 4).unwrap();
        let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec();
        SelectObject(memory, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        (pixels, width)
    }
}

/// How what is shown is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// From the screen, as the user sees it.
    Screen,
    /// From the window, with `PrintWindow`: where `COLUMBINA_VIEW_LOCKED` is 1, as when the
    /// desktop is locked. Then nothing is clicked.
    Window,
}

fn seen() -> Seen {
    if std::env::var("COLUMBINA_VIEW_LOCKED").as_deref() == Ok("1") {
        Seen::Window
    } else {
        Seen::Screen
    }
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3)
}

impl Parent {
    /// The colours shown at points of the client area.
    fn shown(&self, points: &[((i32, i32), [u8; 3])]) -> Vec<[u8; 3]> {
        match seen() {
            Seen::Screen => points
                .iter()
                .map(|((x, y), _)| {
                    let (x, y) = self.on_screen(*x, *y);
                    screen_pixel(x, y)
                })
                .collect(),
            Seen::Window => {
                let (pixels, width) = window_area(self.hwnd);
                points
                    .iter()
                    .map(|((x, y), _)| {
                        let at = usize::try_from((y * width + x) * 4).unwrap();
                        [pixels[at + 2], pixels[at + 1], pixels[at]]
                    })
                    .collect()
            }
        }
    }

    /// Wait until what is shown at each point of the client area is near its colour.
    fn until(&self, what: &str, points: &[((i32, i32), [u8; 3])]) {
        let deadline = Instant::now() + WAIT;
        loop {
            let shown = self.shown(points);
            if shown
                .iter()
                .zip(points)
                .all(|(shown, (_, wanted))| near(*shown, *wanted))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: {shown:?} is shown at {:?}",
                points.to_vec()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A real click of the left button, as the user makes it, at a point of the screen; the mouse
/// is put back where it was.
fn click(x: i32, y: i32) {
    let input = |flags: MOUSE_EVENT_FLAGS| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..MOUSEINPUT::default()
            },
        },
    };
    unsafe {
        let mut was = POINT::default();
        let _ = GetCursorPos(&raw mut was);
        SetCursorPos(x, y).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let sent = SendInput(
            &[input(MOUSEEVENTF_LEFTDOWN), input(MOUSEEVENTF_LEFTUP)],
            i32::try_from(std::mem::size_of::<INPUT>()).unwrap(),
        );
        assert_eq!(sent, 2, "the click is sent");
        std::thread::sleep(Duration::from_millis(100));
        let _ = SetCursorPos(was.x, was.y);
    }
}

/// A picture of one colour, of BT.709 of limited range, and the colour it is drawn as.
fn solid(width: u32, height: u32, codes: [u8; 3]) -> (Arc<Picture>, [u8; 3]) {
    let [y, cb, cr] = codes;
    let stride = (width as usize).next_multiple_of(64);
    let rows = height as usize;
    let mut chroma = vec![0; stride * rows.div_ceil(2)];
    for pair in chroma.as_chunks_mut::<2>().0 {
        *pair = [cb, cr];
    }
    let picture = Picture {
        frame: 0,
        width,
        height,
        bits: 8,
        luma: vec![y; stride * rows],
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    };
    let luma = (f64::from(y) - 16.0) / 219.0;
    let (cb, cr) = (
        (f64::from(cb) - 128.0) / 224.0,
        (f64::from(cr) - 128.0) / 224.0,
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rgb = [
        luma + 1.5748 * cr,
        luma - 0.187_324 * cb - 0.468_124 * cr,
        luma + 1.8556 * cb,
    ]
    .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
    (Arc::new(picture), rgb)
}

#[test]
fn a_parent_that_is_not_a_window_is_refused() {
    let rect = Rect {
        x: 0,
        y: 0,
        width: 10,
        height: 10,
    };
    // A handle that names no window.
    let error = View::attach(RawParent::Windows(0x7FFF_0001), rect).err();
    assert!(matches!(error, Some(ViewError::Parent(_))), "{error:?}");
    let error = View::attach(RawParent::MacOS(1), rect).err();
    assert!(
        matches!(error, Some(ViewError::Unsupported(_))),
        "{error:?}"
    );
}

#[test]
#[ignore = "needs a desktop that can be seen: run with --ignored"]
#[allow(clippy::too_many_lines)]
fn a_view_draws_in_its_parent_follows_it_and_lets_the_input_through() {
    columbina_view::prepare_process();
    let parent = Parent::open(160, 160, 800, 480);
    let events = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let events = Arc::clone(&events);
        Arc::new(move |event: Event| events.lock().unwrap().push(event))
    };
    let rect = Rect {
        x: 40,
        y: 30,
        width: 320,
        height: 180,
    };
    let view = View::attach_reporting(RawParent::Windows(parent.hwnd), rect, report).unwrap();
    eprintln!("drawn by {} with {}", view.adapter(), view.backend());
    let child = hwnd(isize::try_from(view.raw_handle()).unwrap());
    unsafe {
        assert_eq!(GetParent(child).unwrap().0 as isize, parent.hwnd);
        let style = u32::try_from(GetWindowLongW(child, GWL_STYLE)).unwrap();
        assert_ne!(style & WS_DISABLED.0, 0, "the view is disabled");
    }
    // Points of the parent's client area.
    let centre = (200, 120);
    let outside = [(36, 120), (364, 120), (200, 26), (200, 214)];
    if seen() == Seen::Screen {
        parent.assert_seen(parent.on_screen(centre.0, centre.1));
    } else {
        eprintln!("read with PrintWindow: COLUMBINA_VIEW_LOCKED is 1");
    }

    // Black at first, then the picture, over the rectangle and nowhere else.
    parent.until("black at first", &[(centre, [0, 0, 0])]);
    let (picture, colour) = solid(64, 36, [120, 80, 190]);
    let started = Instant::now();
    view.present(picture, 0);
    let mut points = vec![(centre, colour), ((43, 33), colour), ((356, 206), colour)];
    points.extend(outside.iter().map(|point| (*point, BACKGROUND)));
    parent.until("the picture over the rectangle", &points);
    eprintln!("presented and seen within {:?}", started.elapsed());

    // A real click over the view goes to the parent, at that point of its client area.
    if seen() == Seen::Screen {
        let clicks = CLICKS.load(Ordering::SeqCst);
        let (x, y) = parent.on_screen(centre.0, centre.1);
        click(x, y);
        let deadline = Instant::now() + WAIT;
        while CLICKS.load(Ordering::SeqCst) == clicks {
            assert!(Instant::now() < deadline, "the parent was never clicked");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            (
                CLICKED_X.load(Ordering::SeqCst),
                CLICKED_Y.load(Ordering::SeqCst)
            ),
            centre
        );
    } else {
        eprintln!("not clicked: the desktop is not seen");
    }

    // Hidden, the parent shows; shown, the picture again.
    view.show(false).unwrap();
    parent.until("hidden", &[(centre, BACKGROUND)]);
    view.show(true).unwrap();
    parent.until("shown again", &[(centre, colour)]);

    // Moved and made lower: it follows, the picture fitted with bars on its sides
    // (178 × 100 of 320 × 100, from 71).
    view.place(Rect {
        x: 400,
        y: 250,
        width: 320,
        height: 100,
    })
    .unwrap();
    parent.until(
        "moved",
        &[
            (centre, BACKGROUND),
            ((560, 300), colour),
            ((410, 300), [0, 0, 0]),
            ((710, 300), [0, 0, 0]),
            ((474, 300), colour),
            ((396, 300), BACKGROUND),
        ],
    );

    // The device lost, as in a driver reset: it is made again, and draws the next picture.
    view.lose_device();
    let (picture, other) = solid(64, 36, [180, 60, 140]);
    view.present(picture, 0);
    parent.until("drawn after the device was lost", &[((560, 300), other)]);
    {
        let events = events.lock().unwrap();
        let ready = events
            .iter()
            .filter(|event| matches!(event, Event::Ready { .. }))
            .count();
        assert!(ready >= 2, "{events:?}");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::DeviceLost { .. })),
            "{events:?}"
        );
    }

    // Cleared, black; dropped, the window is gone.
    view.clear();
    parent.until("cleared", &[((560, 300), [0, 0, 0])]);
    assert!(view.presented() >= 4, "{}", view.presented());
    drop(view);
    assert!(!unsafe { IsWindow(Some(child)) }.as_bool());
    parent.until("detached", &[((560, 300), BACKGROUND)]);
    let errors: Vec<Event> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, Event::Error { .. } | Event::Stalled))
        .cloned()
        .collect();
    assert_eq!(errors, []);
}

#[test]
#[ignore = "needs a desktop that can be seen: run with --ignored"]
fn a_view_whose_parent_is_destroyed_is_gone() {
    columbina_view::prepare_process();
    let parent = Parent::open(1000, 700, 300, 200);
    let rect = Rect {
        x: 10,
        y: 10,
        width: 100,
        height: 100,
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let events = Arc::clone(&events);
        Arc::new(move |event: Event| events.lock().unwrap().push(event))
    };
    let view = View::attach_reporting(RawParent::Windows(parent.hwnd), rect, report).unwrap();
    assert!(!view.is_gone());
    // A turn that is not a quarter is drawn as none, and told once while it lasts.
    for rotation in [45, 45, -90, 45] {
        view.present(solid(16, 16, [16, 128, 128]).0, rotation);
    }
    let odd = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, Event::Error { message } if message.contains("45")))
        .count();
    assert_eq!(odd, 2);
    drop(parent);
    let deadline = Instant::now() + WAIT;
    while view.place(rect) != Err(ViewError::Gone) {
        assert!(Instant::now() < deadline, "the view still has its window");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(view.show(true), Err(ViewError::Gone));
    assert!(view.is_gone());
    drop(view);
}

#[test]
#[ignore = "needs a desktop: run with --ignored"]
fn a_frame_that_hangs_hides_the_view_until_it_returns() {
    columbina_view::prepare_process();
    let parent = Parent::open(1000, 160, 400, 300);
    let events = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let events = Arc::clone(&events);
        Arc::new(move |event: Event| events.lock().unwrap().push(event))
    };
    let rect = Rect {
        x: 20,
        y: 20,
        width: 200,
        height: 150,
    };
    let view = View::attach_reporting(RawParent::Windows(parent.hwnd), rect, report).unwrap();
    let child = hwnd(isize::try_from(view.raw_handle()).unwrap());
    let visible = || {
        let style = unsafe { GetWindowLongW(child, GWL_STYLE) }.cast_unsigned();
        style & WS_VISIBLE.0 != 0
    };
    let until_event = |what: &str| {
        let deadline = Instant::now() + WAIT;
        while !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| format!("{event:?}").starts_with(what))
        {
            assert!(
                Instant::now() < deadline,
                "no {what}: {:?}",
                events.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let centre = (120, 95);
    let (first, colour) = solid(64, 48, [120, 80, 190]);
    view.present(first, 0);
    parent.until("drawn", &[(centre, colour)]);

    // The next frame hangs for 1.8 s: past 1 s the view is hidden, and the page shows.
    let started = Instant::now();
    view.stall_for(Duration::from_millis(1800));
    let (second, other) = solid(64, 48, [180, 60, 140]);
    view.present(second, 0);
    until_event("Stalled");
    assert!(started.elapsed() >= Duration::from_millis(900));
    parent.until("hidden while it hangs", &[(centre, BACKGROUND)]);
    assert!(!visible());
    // Shown again, as the page would after an overlay, while it still hangs: it stays hidden.
    view.show(false).unwrap();
    view.show(true).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!visible(), "shown while the frame still hangs");

    // Once the frame returns, it is shown, with the picture of that frame.
    until_event("Recovered");
    parent.until("shown again once it returned", &[(centre, other)]);
    assert!(visible());
    let events = events.lock().unwrap();
    let millis = events.iter().find_map(|event| match event {
        Event::Recovered { millis } => Some(*millis),
        _ => None,
    });
    assert!(millis.is_some_and(|millis| millis >= 1000), "{events:?}");
    drop(events);
    drop(view);
}
