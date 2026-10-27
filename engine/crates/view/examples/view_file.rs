//! Play the picture of a media file in a view, in a window of its own: for checks by hand, and
//! for those of Task 19, without Electron.
//!
//! ```text
//! cargo run -p columbina-view --example view_file -- <file> [--seconds <N>] [--shot <file.bgra>]
//!     [--software]
//! ```
//!
//! A window of 1280 × 800 device pixels opens, dark grey, with the view in it 24 pixels from
//! each side; it follows the window as the window is sized. The file is decoded with
//! columbina-media (on the graphics card where it can, unless `--software`) and each frame is
//! presented at the rate of the media, from the start again at its end. It ends when the window
//! is closed, or after `--seconds`; with `--shot`, the window is captured as it is shown
//! (`PrintWindow` with `PW_RENDERFULLCONTENT`, as raw BGRA, its size printed) halfway through.
//! What it did is printed at the end: the adapter, the decoder, the frames presented and their
//! rate.
#![allow(unsafe_code)]

#[cfg(not(windows))]
fn main() {
    eprintln!("view_file runs on Windows only");
}

#[cfg(windows)]
fn main() {
    if let Err(message) = windows_main::run() {
        eprintln!("view_file: {message}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_main {
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use columbina_media::{Hardware, VideoDecoder};
    use columbina_view::{RawParent, Rect, View};
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
        PostMessageW, PostQuitMessage, RegisterClassExW, TranslateMessage, CW_USEDEFAULT, MSG,
        WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_SIZE, WNDCLASSEXW, WS_CLIPCHILDREN,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    /// Space between the view and the sides of the window.
    const MARGIN: i32 = 24;

    /// What the window tells the player.
    enum Told {
        Sized(i32, i32),
        Closed,
    }

    static TELL: OnceLock<Mutex<mpsc::Sender<Told>>> = OnceLock::new();

    fn tell(told: Told) {
        if let Some(sender) = TELL.get() {
            let _ = sender.lock().unwrap().send(told);
        }
    }

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_SIZE => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let (width, height) = (
                    (lparam.0 & 0xFFFF) as i32,
                    ((lparam.0 >> 16) & 0xFFFF) as i32,
                );
                tell(Told::Sized(width, height));
                LRESULT(0)
            }
            WM_DESTROY => {
                tell(Told::Closed);
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    fn view_rect(width: i32, height: i32) -> Rect {
        Rect {
            x: MARGIN,
            y: MARGIN,
            width: u32::try_from(width - 2 * MARGIN).unwrap_or(0),
            height: u32::try_from(height - 2 * MARGIN).unwrap_or(0),
        }
    }

    struct Options {
        file: PathBuf,
        seconds: Option<f64>,
        shot: Option<PathBuf>,
        hardware: Hardware,
    }

    fn options() -> Result<Options, String> {
        let mut args = std::env::args_os().skip(1);
        let mut file = None;
        let mut seconds = None;
        let mut shot = None;
        let mut hardware = Hardware::Auto;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--seconds") => {
                    let value = args.next().and_then(|value| value.into_string().ok());
                    seconds = Some(
                        value
                            .and_then(|value| value.parse().ok())
                            .ok_or("--seconds needs a number")?,
                    );
                }
                Some("--shot") => {
                    shot = Some(PathBuf::from(args.next().ok_or("--shot needs a file")?));
                }
                Some("--software") => hardware = Hardware::Off,
                _ => file = Some(PathBuf::from(arg)),
            }
        }
        Ok(Options {
            file: file.ok_or("usage: view_file <file> [--seconds N] [--shot FILE] [--software]")?,
            seconds,
            shot,
            hardware,
        })
    }

    /// Open the window on a thread of its own, which serves it until it is closed.
    fn open_window() -> isize {
        let (made, making) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap(),
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                hbrBackground: CreateSolidBrush(COLORREF(0x0030_3030)),
                lpszClassName: w!("ColumbinaViewFile"),
                ..WNDCLASSEXW::default()
            };
            RegisterClassExW(&raw const class);
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ColumbinaViewFile"),
                w!("columbina view_file"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1280,
                800,
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
        making.recv().unwrap()
    }

    fn client_size(window: isize) -> (i32, i32) {
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(HWND(window as *mut _), &raw mut rect) };
        (rect.right - rect.left, rect.bottom - rect.top)
    }

    /// Capture the window as it is shown, with what the card drew in it, as raw BGRA.
    fn capture(window: isize, path: &PathBuf) -> Result<(i32, i32), String> {
        let (width, height) = client_size(window);
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
            .map_err(|error| error.to_string())?;
            let old = SelectObject(memory, bitmap.into());
            // The client area only (1), with what DirectX drew (2).
            let printed = PrintWindow(HWND(window as *mut _), memory, PRINT_WINDOW_FLAGS(1 | 2));
            let length = usize::try_from(width * height * 4).unwrap();
            let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec();
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            if !printed.as_bool() {
                return Err("PrintWindow failed".into());
            }
            std::fs::write(path, pixels).map_err(|error| error.to_string())?;
        }
        Ok((width, height))
    }

    #[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
    pub fn run() -> Result<(), String> {
        let options = options()?;
        columbina_view::prepare_process();
        let facts = columbina_media::probe(&options.file).map_err(|error| error.to_string())?;
        let video = facts.video.ok_or("the file has no picture")?;
        let rotation = u16::try_from(video.rotation.rem_euclid(360)).unwrap_or(0);
        let mut decoder = VideoDecoder::open(&options.file, options.hardware)
            .map_err(|error| error.to_string())?;
        let rate = decoder.rate();
        let frame_time = Duration::from_secs_f64(f64::from(rate.den) / f64::from(rate.num));

        let (told, telling) = mpsc::channel();
        TELL.set(Mutex::new(told)).ok();
        let window = open_window();
        let (width, height) = client_size(window);
        let view = View::attach_reporting(
            RawParent::Windows(window),
            view_rect(width, height),
            Arc::new(|event| eprintln!("view: {event:?}")),
        )
        .map_err(|error| error.to_string())?;
        println!(
            "file {} ({}×{}, {}/{} frames a second, turned {rotation}); view drawn by {} with {}; \
             decoded with {}",
            options.file.display(),
            video.width,
            video.height,
            rate.num,
            rate.den,
            view.adapter(),
            view.backend(),
            decoder.path_used(),
        );

        let started = Instant::now();
        let mut next_at = started;
        let mut shown = 0_u64;
        let mut late = 0_u64;
        let mut shot_taken = options.shot.is_none();
        'playing: loop {
            while let Ok(told) = telling.try_recv() {
                match told {
                    Told::Sized(width, height) => view
                        .place(view_rect(width, height))
                        .map_err(|error| error.to_string())?,
                    Told::Closed => break 'playing,
                }
            }
            let elapsed = started.elapsed().as_secs_f64();
            if options.seconds.is_some_and(|seconds| elapsed >= seconds) {
                break;
            }
            if !shot_taken && elapsed >= options.seconds.unwrap_or(4.0) / 2.0 {
                shot_taken = true;
                if let Some(path) = &options.shot {
                    let (width, height) = capture(window, path)?;
                    println!("shot {} of {width}×{height}", path.display());
                }
            }
            let picture = match decoder.next().map_err(|error| error.to_string())? {
                Some(picture) => picture,
                None => decoder.frame_at(0).map_err(|error| error.to_string())?,
            };
            let now = Instant::now();
            if next_at > now {
                std::thread::sleep(next_at - now);
            } else if now - next_at > frame_time {
                late += 1;
                next_at = now;
            }
            view.present(Arc::new(picture), rotation);
            shown += 1;
            next_at += frame_time;
        }
        let elapsed = started.elapsed().as_secs_f64();
        // Let the last frames be drawn before they are counted.
        std::thread::sleep(Duration::from_millis(100));
        println!(
            "{shown} pictures presented in {elapsed:.2} s ({:.2} a second, {late} late); {} \
             frames drawn",
            shown as f64 / elapsed,
            view.presented(),
        );
        drop(view);
        let _ =
            unsafe { PostMessageW(Some(HWND(window as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        Ok(())
    }
}
