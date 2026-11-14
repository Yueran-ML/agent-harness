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
