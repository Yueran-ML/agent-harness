use super::*;
use columbina_view::{Cursor, Pointer, Presentation, Viewport};
use windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;

fn until_presentation(view: &View, wanted: impl Fn(Presentation) -> bool) -> Presentation {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(presented) = view.presentation() {
            if wanted(presented) {
                return presented;
            }
        }
        assert!(
            Instant::now() < deadline,
            "presentation: {:?}",
            view.presentation()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pointer(view: &View, events: &Mutex<Vec<Event>>) -> Pointer {
    events.lock().unwrap().clear();
    let child = hwnd(isize::try_from(view.raw_handle()).unwrap());
    unsafe {
        PostMessageW(
            Some(child),
            WM_MOUSEMOVE,
            WPARAM(0),
            LPARAM(40 + (50 << 16)),
        )
        .unwrap();
    }
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(pointer) = events.lock().unwrap().iter().find_map(|event| match event {
            Event::Pointer(pointer) => Some(*pointer),
            _ => None,
        }) {
            return pointer;
        }
        assert!(Instant::now() < deadline, "no native pointer event");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn a_pointer_uses_the_last_presented_frame_until_a_queued_frame_or_viewport_is_shown() {
    columbina_view::prepare_process();
    let parent = Parent::open(800, 300, 400, 300);
    let events = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let events = Arc::clone(&events);
        Arc::new(move |event| events.lock().unwrap().push(event))
    };
    let rect = Rect {
        x: 20,
        y: 20,
        width: 200,
        height: 150,
    };
    let view = sdr_view(RawParent::Windows(parent.hwnd), rect, gpu(), report);
    view.take_pointer(Some(Cursor::Crosshair)).unwrap();
    let (picture, colour) = solid(64, 32, [120, 80, 190]);
    view.present_tagged(picture, 0, 41);
    let first = until_presentation(&view, |p| p.tag == Some(41));
    parent.until("the first tagged picture", &[((120, 95), colour)]);
    assert_eq!(
        pointer(&view, &events).picture,
        Some(first.map.unwrap().on_picture((40, 50)).0)
    );

    view.stall_for(Duration::from_millis(650));
    let (picture, _) = solid(30, 80, [180, 60, 140]);
    view.present_tagged(picture, 90, 42);
    let viewport = Viewport::Scale {
        scale: 2.0,
        center: [0.25, 0.75],
    };
    view.set_viewport(viewport).unwrap();
    let pending = pointer(&view, &events);
    assert_eq!(pending.presentation.unwrap().tag, Some(41));
    assert_eq!(
        pending.picture,
        Some(first.map.unwrap().on_picture((40, 50)).0)
    );
    let second = until_presentation(&view, |p| {
        p.tag == Some(42) && p.map == viewport.map((200, 150), (30, 80), 90)
    });
    let shown = pointer(&view, &events);
    assert_eq!(shown.presentation.unwrap().tag, Some(42));
    assert_eq!(
        shown.picture,
        Some(second.map.unwrap().on_picture((40, 50)).0)
    );

    view.set_viewport(Viewport::Fit).unwrap();
    let fitted = until_presentation(&view, |p| {
        p.serial > second.serial && p.map == Viewport::Fit.map((200, 150), (30, 80), 90)
    });
    assert_eq!(
        fitted.tag,
        Some(42),
        "a viewport redraw keeps the producer identity"
    );
    view.stall_for(Duration::from_millis(650));
    view.place(Rect { width: 180, ..rect }).unwrap();
    let child = hwnd(isize::try_from(view.raw_handle()).unwrap());
    let deadline = Instant::now() + WAIT;
    loop {
        let mut actual = windows::Win32::Foundation::RECT::default();
        unsafe {
            GetClientRect(child, &raw mut actual).unwrap();
        }
        if actual.right == 180 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let resizing = pointer(&view, &events);
    assert!(resizing.picture.is_none());
    assert!(!resizing.inside);
    let resized = until_presentation(&view, |p| {
        p.map.is_some_and(|map| map.surface == (180, 150))
    });
    assert_eq!(resized.tag, Some(42));
    assert!(pointer(&view, &events).picture.is_some());

    view.stall_for(Duration::from_millis(650));
    view.clear();
    assert_eq!(pointer(&view, &events).presentation.unwrap().tag, Some(42));
    let cleared = until_presentation(&view, |p| p.serial > resized.serial && p.map.is_none());
    assert_eq!(cleared.tag, None);
    assert!(pointer(&view, &events).picture.is_none());
    view.present_tagged(Arc::new(NativePicture::default()), 0, 99);
    let bad = until_presentation(&view, |p| p.serial > cleared.serial);
    assert!(bad.map.is_none());
    assert_eq!(
        bad.tag, None,
        "an upload that failed is not the displayed frame"
    );
}
