use super::*;
use columbina_view::{Cursor, Navigation, Pointer, Presentation, Viewport};
use windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;

pub(super) fn until_presentation(
    view: &View,
    wanted: impl Fn(Presentation) -> bool,
) -> Presentation {
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

fn reject_unpresented_picture_dimensions(in_flight: bool) {
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
        width: 256,
        height: 144,
    };
    let view = sdr_view(RawParent::Windows(parent.hwnd), rect, gpu(), report);
    let base = Viewport::Scale {
        scale: 2.0,
        center: [0.5, 0.5],
    };
    view.set_viewport(base).unwrap();
    view.take_pointer(Some(Cursor::Arrow)).unwrap();
    view.present_tagged(solid(128, 72, [120, 80, 190]).0, 0, 41);
    let first = until_presentation(&view, |p| p.tag == Some(41));
    let (_, shown) = view.navigate(None).unwrap();
    let pan = Navigation::Pan {
        base,
        surface: shown.surface,
        picture: shown.picture,
        offset: (12.0, 0.0),
    };
    view.show(false).unwrap(); // Keeps the submitted frame in Pending until explicitly shown.
    events.lock().unwrap().clear();
    view.stall_for(Duration::from_millis(1800));
    view.present_tagged(solid(256, 144, [80, 120, 190]).0, 0, 42);
    if in_flight {
        view.show(true).unwrap();
        let deadline = Instant::now() + WAIT;
        while !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, Event::Stalled))
        {
            assert!(
                Instant::now() < deadline,
                "drawer did not enter the held job"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let actual = view.navigate(Some(pan));
    assert!(actual.is_err(), "new dimensions must reject old pan before presentation (in_flight={in_flight}): {actual:?}");
    assert!(
        view.navigate(None).is_err(),
        "prepare cannot return old dimensions"
    );
    assert_eq!(pointer(&view, &events).presentation, Some(first));
    view.show(true).unwrap();
    let next = until_presentation(&view, |p| p.tag == Some(42));
    assert_eq!(
        next.map,
        base.map(shown.surface, (256, 144), 0),
        "refused pan leaves requested center unchanged"
    );
    assert_eq!(view.navigate(None).unwrap().0, base);
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn a_pending_new_picture_size_rejects_navigation_before_presentation() {
    reject_unpresented_picture_dimensions(false);
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn an_in_flight_new_picture_size_rejects_navigation_before_presentation() {
    reject_unpresented_picture_dimensions(true);
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn same_size_submissions_allow_navigation_and_clear_retires_it() {
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
        width: 256,
        height: 144,
    };
    let view = sdr_view(RawParent::Windows(parent.hwnd), rect, gpu(), report);
    let base = Viewport::Scale {
        scale: 2.0,
        center: [0.5, 0.5],
    };
    view.set_viewport(base).unwrap();
    view.take_pointer(Some(Cursor::Arrow)).unwrap();
    view.present_tagged(solid(128, 72, [120, 80, 190]).0, 0, 41);
    let first = until_presentation(&view, |p| p.tag == Some(41));
    let (_, shown) = view.navigate(None).unwrap();
    let pan = Navigation::Pan {
        base,
        surface: shown.surface,
        picture: shown.picture,
        offset: (12.0, 0.0),
    };
    view.show(false).unwrap();
    view.stall_for(Duration::from_millis(1800));
    // Stored dimensions change but the metadata-oriented picture size does not.
    view.present_tagged(solid(72, 128, [80, 120, 190]).0, 90, 42);
    let (panned, _) = view.navigate(Some(pan)).unwrap();
    assert_eq!(
        panned,
        Viewport::Scale {
            scale: 2.0,
            center: [0.453_125, 0.5]
        }
    );
    view.set_viewport(base).unwrap();
    events.lock().unwrap().clear();
    view.show(true).unwrap();
    let deadline = Instant::now() + WAIT;
    while !events
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, Event::Stalled))
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        view.navigate(None).unwrap().0,
        base,
        "normal same-size in-flight frame is allowed"
    );
    assert_eq!(view.navigate(Some(pan)).unwrap().0, panned);
    assert_eq!(pointer(&view, &events).presentation, Some(first));
    let next = until_presentation(&view, |p| {
        p.tag == Some(42) && p.map == panned.map(shown.surface, shown.picture, 0)
    });

    // Every RGB entry shares the submission path, including overlay-bearing timeline frames.
    let card = view.gpu();
    let original = columbina_compose::Target::preview(&card, 128, 72).unwrap();
    let larger = columbina_compose::Target::preview(&card, 256, 144).unwrap();
    view.show(false).unwrap();
    view.present_texture_overlay(Arc::clone(larger.texture()), Arc::clone(&card), 43, None)
        .unwrap();
    assert!(view.navigate(None).is_err());
    view.present_texture_tagged(Arc::clone(original.texture()), Arc::clone(&card), 44)
        .unwrap();
    assert!(
        view.navigate(None).is_ok(),
        "latest same-size submission supersedes the queued larger one"
    );
    view.present_texture(Arc::clone(larger.texture()), Arc::clone(&card))
        .unwrap();
    assert!(view.navigate(None).is_err());
    view.clear();
    assert!(view.navigate(None).is_err());
    assert_eq!(
        pointer(&view, &events).presentation,
        Some(next),
        "clear does not rewrite the actual pointer map before presentation"
    );
    view.show(true).unwrap();
    until_presentation(&view, |p| p.map.is_none());
    view.present(solid(128, 72, [120, 80, 190]).0, 17); // Unsupported rotation uses the existing unturned path.
    until_presentation(&view, |p| p.map.is_some_and(|map| map.picture == (128, 72)));
    assert!(view.navigate(None).is_ok());
    view.use_gpu(gpu());
    until_presentation(&view, |p| p.map.is_some_and(|map| map.picture == (128, 72)));
    assert!(
        view.navigate(None).is_ok(),
        "CPU recovery reuses its submitted dimensions without requiring another submit"
    );
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn pending_navigation_composes_while_pointer_geometry_stays_successfully_presented() {
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
        width: 101,
        height: 73,
    };
    let view = sdr_view(RawParent::Windows(parent.hwnd), rect, gpu(), report);
    view.take_pointer(Some(Cursor::Arrow)).unwrap();
    assert!(
        view.navigate(None).is_err(),
        "no picture cannot prepare navigation"
    );
    view.present_tagged(solid(37, 19, [120, 80, 190]).0, 90, 41);
    let first = until_presentation(&view, |p| p.tag == Some(41));
    let shown = first.map.unwrap();
    view.stall_for(Duration::from_millis(650));
    let zoom = Navigation::Zoom {
        factor: 1.1,
        at: Some((0.5, 10.5)),
    };
    let (once, _) = view.navigate(Some(zoom)).unwrap();
    let (twice, _) = view.navigate(Some(zoom)).unwrap();
    let Viewport::Scale { scale: s1, .. } = once else {
        panic!("zoom")
    };
    let Viewport::Scale { scale: s2, .. } = twice else {
        panic!("zoom")
    };
    assert!((s2 - s1 * 1.1).abs() < 1e-12);
    let pan = Navigation::Pan {
        base: twice,
        surface: shown.surface,
        picture: shown.picture,
        offset: (14.0, -9.0),
    };
    let (panned, _) = view.navigate(Some(pan)).unwrap();
    let pending = pointer(&view, &events);
    assert_eq!(pending.presentation, Some(first));
    assert_eq!(pending.picture, Some(shown.on_picture((40, 50)).0));
    let painted = until_presentation(&view, |p| {
        p.map == panned.map(shown.surface, shown.picture, 0)
    });
    assert!(painted.serial > first.serial);
    view.stall_for(Duration::from_millis(650));
    view.navigate(Some(Navigation::Fit)).unwrap();
    let (again, _) = view.navigate(Some(zoom)).unwrap();
    assert_eq!(
        again, once,
        "pending Fit then wheel uses Fit requested geometry"
    );
    assert_eq!(pointer(&view, &events).presentation, Some(painted));
    until_presentation(&view, |p| {
        p.map == once.map(shown.surface, shown.picture, 0)
    });
    view.place(Rect { width: 103, ..rect }).unwrap();
    assert!(
        view.navigate(Some(pan)).is_err(),
        "requested resize invalidates old geometry before the native size changes"
    );
    until_presentation(&view, |p| p.map.is_some_and(|map| map.surface == (103, 73)));
    assert!(
        view.navigate(Some(pan)).is_err(),
        "successful resize cannot reinterpret old pan displacement"
    );
    view.present_tagged(solid(38, 19, [80, 120, 190]).0, 90, 42);
    until_presentation(&view, |p| p.tag == Some(42));
    assert!(
        view.navigate(Some(pan)).is_err(),
        "new oriented picture size ends old pan"
    );
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
