//! Real native surfaces and synthetic window messages; no claim of physical `SendInput`.
use super::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEMOVE};

#[allow(
    unsafe_code,
    reason = "post a synthetic mouse message to the test-owned native window"
)]
async fn pointer(desktop: &mut Client, window: isize) -> Value {
    desktop.take_notifications();
    // Window messages exercise the same native mapping/reporter as a pointer, without moving
    // the user's mouse. They also work on a locked desktop where GetPixel cannot read a view.
    unsafe {
        PostMessageW(
            Some(HWND(window as *mut _)),
            WM_MOUSEMOVE,
            WPARAM(0),
            LPARAM(0x000a | (0x000a << 16)),
        )
    }
    .unwrap();
    loop {
        let message = desktop.message().await;
        if message["method"] == "view.pointer" {
            return message["params"].clone();
        }
    }
}

async fn until_pointer(
    desktop: &mut Client,
    window: isize,
    wanted: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let event = pointer(desktop, window).await;
        if wanted(&event) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "no matching presentation: {event}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one native session across pending frames, failed attach and replacement"
)]
async fn actual_presentation_keeps_old_revision_until_native_present_and_isolates_reattach() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut color = clip("$clip", "unused", 0, 30);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made=desktop.apply(json!([{"op":"add_timeline","timeline":{
        "id":"$timeline","name":"Presentation","width":64,"height":36,
        "rate":{"num":25,"den":1},"sample_rate":48000,
        "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,"clips":[color],"transitions":[]}]
    }}])).await;
    let timeline = made["names"]["$timeline"].clone();
    let attach = json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":64,"height":36});
    desktop.result("view.attach", Some(attach.clone())).await;
    desktop
        .result(
            "view.pointer",
            Some(json!({"monitor":"timeline","on":true})),
        )
        .await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":timeline})),
        )
        .await;
    let window = until_views(&parent, "timeline view", |views| views.len() == 1).await[0].0;
    let first = until_pointer(&mut desktop, window, |event| {
        event["snapshot"]["ready"] == true
    })
    .await;
    assert_eq!(first["snapshot"]["revision"], made["revision"]);
    assert_eq!(first["snapshot"]["frame"], 0);
    assert_eq!(first["presentation"]["map"]["picture"], json!([64, 36]));
    assert_eq!(first["x"], 10.5);

    desktop
        .result(
            "view.show",
            Some(json!({"monitor":"timeline","shown":false})),
        )
        .await;
    // A bin change advances history without changing the picture. The subsequent seek builds
    // a new-revision candidate, but the hidden native surface cannot present it yet.
    let changed = desktop
        .apply(json!([{"op":"add_bin","bin":{"id":"$bin","name":"New revision","parent":null}}]))
        .await;
    desktop
        .result(
            "transport.seek",
            Some(json!({"monitor":"timeline","frame":1})),
        )
        .await;
    desktop
        .until_state("new frame composed", |state| {
            state["monitor"] == "timeline" && state["shown"] == 1
        })
        .await;
    let hidden = pointer(&mut desktop, window).await;
    assert_eq!(hidden["presentation"], first["presentation"]);
    assert_eq!(hidden["snapshot"]["revision"], made["revision"]);
    desktop
        .result(
            "view.show",
            Some(json!({"monitor":"timeline","shown":true})),
        )
        .await;
    let next = until_pointer(&mut desktop, window, |event| {
        event["snapshot"]["frame"] == 1
    })
    .await;
    assert_eq!(next["snapshot"]["revision"], changed["revision"]);
    assert_ne!(next["presentation"]["tag"], first["presentation"]["tag"]);

    let mut invalid = attach.clone();
    invalid["parent"] = json!("1");
    desktop.error("view.attach", Some(invalid)).await;
    assert_eq!(
        until_views(&parent, "old view survives failed attach", |views| views
            .len()
            == 1)
        .await[0]
            .0,
        window
    );
    let survived = pointer(&mut desktop, window).await;
    assert_eq!(survived["instance"], first["instance"]);
    assert_eq!(survived["snapshot"]["revision"], changed["revision"]);

    desktop.result("view.attach", Some(attach)).await;
    let replacement = until_views(&parent, "replacement view", |views| {
        views.len() == 1 && views[0].0 != window
    })
    .await[0]
        .0;
    let replaced = until_pointer(&mut desktop, replacement, |event| {
        event["snapshot"]["ready"] == true
    })
    .await;
    assert_ne!(replaced["instance"], first["instance"]);
    assert_eq!(replaced["snapshot"]["revision"], changed["revision"]);
    assert!(!replaced.to_string().contains("path"));

    desktop.result("view.attach",Some(json!({"monitor":"source","parent":parent.hwnd.to_string(),"x":100,"y":0,"width":64,"height":36}))).await;
    desktop
        .result("view.pointer", Some(json!({"monitor":"source","on":true})))
        .await;
    let source = until_views(&parent, "separate source view", |views| views.len() == 2)
        .await
        .into_iter()
        .find(|view| view.2[0] == 100)
        .unwrap()
        .0;
    let untagged = pointer(&mut desktop, source).await;
    assert_eq!(untagged["monitor"], "source");
    assert!(untagged["presentation"]["tag"].is_null());
    assert!(untagged["snapshot"].is_null());
}

#[tokio::test]
async fn a_view_attached_where_nothing_is_loaded_shows_no_picture() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let monitors = [("source", 0), ("timeline", 100)];
    for (monitor, x) in monitors {
        desktop.result("view.attach",Some(json!({"monitor":monitor,"parent":parent.hwnd.to_string(),"x":x,"y":0,"width":64,"height":36}))).await;
        desktop
            .result("view.pointer", Some(json!({"monitor":monitor,"on":true})))
            .await;
    }
    let views = until_views(&parent, "both views", |views| views.len() == 2).await;
    for (monitor, x) in monitors {
        let window = views.iter().find(|view| view.2[0] == x).unwrap().0;
        // Drawn once, black: a presentation of no picture, of no frame.
        let black = until_pointer(&mut desktop, window, |event| {
            event["monitor"] == monitor && !event["presentation"].is_null()
        })
        .await;
        assert!(black["presentation"]["map"].is_null(), "{black}");
        assert!(black["presentation"]["tag"].is_null(), "{black}");
        assert!(black["snapshot"].is_null(), "{black}");
        // And nothing after it: the monitor has no frame to give the view.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let later = until_pointer(&mut desktop, window, |event| event["monitor"] == monitor).await;
        assert!(later["presentation"]["map"].is_null(), "{later}");
        assert!(later["snapshot"].is_null(), "{later}");
    }
}

#[tokio::test]
async fn a_view_made_again_shows_at_once_the_paused_frame_of_its_monitor() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut color = clip("$clip", "unused", 0, 30);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made=desktop.apply(json!([{"op":"add_timeline","timeline":{
        "id":"$timeline","name":"Made again","width":64,"height":36,
        "rate":{"num":25,"den":1},"sample_rate":48000,
        "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,"clips":[color],"transitions":[]}]
    }}])).await;
    let timeline = made["names"]["$timeline"].clone();
    let attach = json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":64,"height":36});
    desktop.result("view.attach", Some(attach.clone())).await;
    desktop
        .result(
            "view.pointer",
            Some(json!({"monitor":"timeline","on":true})),
        )
        .await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":timeline})),
        )
        .await;
    desktop
        .result(
            "transport.seek",
            Some(json!({"monitor":"timeline","frame":7})),
        )
        .await;
    // Compared with another frame of the timeline: the view made again shows the frame as the
    // monitor shows it, not the frame alone.
    desktop
        .result(
            "viewer.wipe",
            Some(json!({"style":"mix","position":1.0,"reference":{"kind":"frame","frame":1}})),
        )
        .await;
    let window = until_views(&parent, "timeline view", |views| views.len() == 1).await[0].0;
    let first = until_pointer(&mut desktop, window, |event| {
        event["snapshot"]["ready"] == true
            && event["snapshot"]["frame"] == 7
            && event["snapshot"]["comparison"] == "wipe"
    })
    .await;

    // The page is loaded again: its view is made anew where the old one lay, and nothing else
    // is asked of the engine, which is paused.
    desktop.result("view.attach", Some(attach)).await;
    let again = until_views(&parent, "view made again", |views| {
        views.len() == 1 && views[0].0 != window
    })
    .await[0]
        .0;
    let shown = until_pointer(&mut desktop, again, |event| {
        event["instance"] != first["instance"] && event["snapshot"]["ready"] == true
    })
    .await;
    assert_eq!(shown["snapshot"]["frame"], 7);
    assert_eq!(shown["snapshot"]["comparison"], "wipe");
    assert_eq!(shown["snapshot"]["revision"], first["snapshot"]["revision"]);
    assert_eq!(shown["presentation"]["map"]["picture"], json!([64, 36]));
    assert_ne!(shown["presentation"]["tag"], first["presentation"]["tag"]);
    // The playhead did not move for it.
    let state = desktop
        .result("transport.state", Some(json!({"monitor":"timeline"})))
        .await;
    assert_eq!(state["playing"], false, "{state}");
    assert_eq!(state["frame"], 7, "{state}");
}

/// Wait until the view, of 960 × 540 at the top left of `parent`, shows the frame `frame` of the
/// test media, as it is read back from the screen.
async fn until_number(parent: &parent::Parent, frame: i64, what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let (grey, width) = parent.grey();
        let shown = number_shown(&grey, width);
        if shown == Some(frame) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: the view shows {shown:?}, not frame {frame}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// What the view of `monitor` says of the picture it last presented, once `wanted` takes it:
/// its native `instance` and `geometry`, as `view.viewport` prepares a navigation, which changes
/// nothing and draws nothing (and is refused while no picture is presented).
async fn until_picture(
    desktop: &mut Client,
    monitor: &str,
    wanted: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let answer = desktop
            .call(
                "view.viewport",
                Some(json!({"monitor":monitor,"operation":"prepare"})),
            )
            .await;
        if let Some(result) = answer.get("result").filter(|result| wanted(result)) {
            return result.clone();
        }
        assert!(Instant::now() < deadline, "no picture presented: {answer}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_view_shows_its_frame_once_shown_again_and_once_made_again() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    let parent = parent::Parent::shown();
    let mut desktop = engine.client("desktop").await;
    // In SDR, as the screen is read back in 8 bits (as `the_view_shows_the_frame_sought`).
    desktop
        .result("view.display", Some(json!({ "hdr": false })))
        .await;
    let attach = json!({ "monitor": "source", "parent": parent.hwnd.to_string(), "x": 0, "y": 0, "width": 960, "height": 540 });
    desktop.result("view.attach", Some(attach.clone())).await;
    agent
        .source_result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .source_result("transport.seek", Some(json!({ "frame": 37 })))
        .await;
    agent
        .until_state("sought", |state| {
            state["shown"] == 37 && state["exact"] == true
        })
        .await;
    until_number(&parent, 37, "sought").await;
    let window = until_views(&parent, "source view", |views| views.len() == 1).await[0].0;

    // Hidden, as while a menu of the page lies over the viewer; a frame sought meanwhile is
    // drawn once it is shown again, at the same place and size.
    desktop
        .result(
            "view.show",
            Some(json!({ "monitor": "source", "shown": false })),
        )
        .await;
    until_views(&parent, "hidden", |views| views.len() == 1 && !views[0].3).await;
    agent
        .source_result("transport.seek", Some(json!({ "frame": 77 })))
        .await;
    agent
        .until_state("sought while hidden", |state| {
            state["shown"] == 77 && state["exact"] == true
        })
        .await;
    desktop
        .result(
            "view.show",
            Some(json!({ "monitor": "source", "shown": true })),
        )
        .await;
    until_views(&parent, "shown again", |views| {
        views.len() == 1 && views[0].3
    })
    .await;
    until_number(&parent, 77, "shown again").await;
    let before = until_picture(&mut desktop, "source", |_| true).await;

    // Made anew where it lay, as for a page loaded again, with nothing else asked: the paused
    // frame is drawn into it at once.
    desktop.result("view.attach", Some(attach)).await;
    until_views(&parent, "made again", |views| {
        views.len() == 1 && views[0].0 != window
    })
    .await;
    let presented = until_picture(&mut desktop, "source", |result| {
        result["instance"] != before["instance"]
    })
    .await;
    assert_eq!(presented["geometry"]["picture"], json!([1920, 1080]));
    assert_eq!(presented["geometry"]["surface"], json!([960, 540]));
    until_number(&parent, 77, "made again").await;
    let state = desktop.source_result("transport.state", None).await;
    assert_eq!(state["playing"], false, "{state}");
    assert_eq!(state["frame"], 77, "{state}");
}
