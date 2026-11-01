//! Native messages exercise actual presentation and the real, sequential RPC connection.
use super::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

#[allow(
    unsafe_code,
    reason = "synthetic messages to a test-owned child window"
)]
async fn pointer(client: &mut Client, window: isize, message: u32, x: i32, y: i32) -> Value {
    client.take_notifications();
    unsafe {
        PostMessageW(
            Some(HWND(window as *mut _)),
            message,
            WPARAM(usize::from(message != WM_LBUTTONUP)),
            LPARAM(((x & 0xffff) | ((y & 0xffff) << 16)) as isize),
        )
    }
    .unwrap();
    loop {
        let event = client.message().await;
        if event["method"] == "view.pointer" {
            return event["params"].clone();
        }
    }
}

async fn ready(client: &mut Client, window: isize, binding: &Value) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let event = pointer(client, window, WM_MOUSEMOVE, 64, 36).await;
        if event["snapshot"]["window"]["binding"] == *binding && event["snapshot"]["ready"] == true
        {
            return event;
        }
        assert!(Instant::now() < deadline, "no overlay: {event}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn phase(client: &mut Client, wanted: &str) -> Value {
    loop {
        if let Some(note) = client
            .take_notifications()
            .into_iter()
            .find(|n| n["method"] == "window.state" && n["params"]["phase"] == wanted)
        {
            return note["params"].clone();
        }
        let note = client.message().await;
        if note["method"] == "window.state" && note["params"]["phase"] == wanted {
            return note["params"].clone();
        }
    }
}
fn windows(project: &Value, binding: &Value) -> Value {
    let t = project["project"]["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == binding["timeline"])
        .unwrap();
    t["tracks"][0]["clips"][0]["grade"]["nodes"][0]["params"]["windows"].clone()
}

#[tokio::test]
async fn native_drag_freezes_clicked_playback_frame_survives_hold_and_has_one_undo() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let binding = fixture(&mut desktop).await;
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let window = until_views(&parent, "window view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &bound["binding"]).await;
    let original = project(&mut desktop).await;
    desktop
        .result("transport.play", Some(json!({"monitor":"timeline"})))
        .await;
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 36).await;
    let clicked = down["snapshot"]["frame"].as_i64().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ahead = desktop
        .result("transport.state", Some(json!({"monitor":"timeline"})))
        .await;
    assert!(
        ahead["frame"].as_i64().unwrap() >= clicked + 3,
        "worker should have advanced: {ahead}"
    );
    let buffered = pointer(&mut desktop, window, WM_MOUSEMOVE, 72, 36).await;
    assert!(
        buffered["snapshot"]["frame"].as_i64().unwrap() > clicked,
        "the retained move must carry a later playback presentation: {buffered}"
    );
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        )
        .await;
    let armed = phase(&mut desktop, "armed").await;
    assert_eq!(armed["keys"]["clock"], "clip");
    let moved = desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":buffered["event"]}))).await;
    assert_eq!(moved["phase"], "armed");
    assert_eq!(
        windows(&project(&mut desktop).await, &binding)[0]["transform"]["pan"],
        56.25,
        "the pre-arm position must use the clicked frame's locked mapping"
    );
    let paused = desktop
        .result("transport.state", Some(json!({"monitor":"timeline"})))
        .await;
    assert_eq!(paused["frame"], clicked);
    assert_eq!(paused["playing"], false);
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 80, 36).await;
    desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 144, 36).await;
    assert_eq!(
        moved["inside"], false,
        "captured move intentionally crosses the picture edge"
    );
    desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    pointer(&mut desktop, window, WM_LBUTTONUP, 160, 36).await;
    desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"up"})),
        )
        .await;
    let after = project(&mut desktop).await;
    assert_eq!(
        windows(&after, &binding)[0]["transform"]["pan"],
        112.5,
        "up adds no position"
    );
    assert_eq!(
        after["revision"].as_u64(),
        Some(original["revision"].as_u64().unwrap() + 3)
    );
    desktop.result("history.undo", None).await;
    assert_eq!(project(&mut desktop).await["project"], original["project"]);
}

#[tokio::test]
async fn esc_restores_entire_keyed_windows_without_a_new_undo_step() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    all[0]["transform"]["pan"] = json!({"keys":[{"frame":0,"value":40.0,"interp":{"bezier":{"out":[0.2,2.0],"in":[0.4,-1.0]}}},{"frame":20,"value":60.0,"interp":"hold"}]});
    all[0]["transform"]["tilt"] = json!({"keys":[{"frame":3,"value":50.0,"interp":"linear"},{"frame":25,"value":70.0,"interp":"hold"}]});
    let changed=desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    desktop
        .result(
            "transport.seek",
            Some(json!({"monitor":"timeline","frame":10})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let window = until_views(&parent, "window view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &bound["binding"]).await;
    let before = project(&mut desktop).await;
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 36).await;
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        )
        .await;
    let armed = phase(&mut desktop, "armed").await;
    assert_eq!(armed["keys"]["pan"], 0);
    assert_eq!(armed["keys"]["tilt"], 3);
    assert_eq!(armed["keys"]["frame"], 10);
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 80, 43).await;
    desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    let changed = project(&mut desktop).await;
    let actual = windows(&changed, &binding);
    assert_eq!(actual[0]["transform"]["pan"]["keys"][0]["value"], 52.5);
    assert_eq!(
        actual[0]["transform"]["pan"]["keys"][1],
        all[0]["transform"]["pan"]["keys"][1]
    );
    desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"cancel"})),
        )
        .await;
    assert_eq!(project(&mut desktop).await["project"], before["project"]);
    desktop.result("history.undo", None).await;
    assert_eq!(
        windows(&project(&mut desktop).await, &binding)[0]["transform"]["pan"],
        50.0,
        "Esc removed the empty gesture, so undo reaches the prior key-edit"
    );
}

#[tokio::test]
async fn native_outline_is_interpolated_while_numeric_movement_writes_the_left_key() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::shown();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    all[0]["transform"]["pan"] = json!({"keys":[{"frame":0,"value":40.0,"interp":"linear"},{"frame":20,"value":60.0,"interp":"hold"}]});
    let changed=desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop
        .result("view.display", Some(json!({"hdr":false})))
        .await;
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    desktop
        .result(
            "transport.seek",
            Some(json!({"monitor":"timeline","frame":10})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let window = until_views(&parent, "window view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &bound["binding"]).await;
    let (pixels, width) = parent.grey();
    assert!(
        pixels[36 * width + 81] > 220,
        "the circle's right edge is at x=82, interpolated Pan=50"
    );
    assert!(
        pixels[36 * width + 69] < 150,
        "the left key's Pan=40 edge must not be drawn"
    );
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 36).await;
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        )
        .await;
    phase(&mut desktop, "armed").await;
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 80, 36).await;
    desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    let changed = project(&mut desktop).await;
    assert_eq!(
        windows(&changed, &binding)[0]["transform"]["pan"]["keys"][0]["value"],
        52.5
    );
    let deadline = Instant::now() + WAIT;
    loop {
        let p = pointer(&mut desktop, window, WM_MOUSEMOVE, 80, 36).await;
        if p["snapshot"]["revision"] == changed["revision"] {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (pixels, width) = parent.grey();
    assert!(
        pixels[36 * width + 89] > 220,
        "editing the left key by 12.5 gives the displayed half-way Pan=56.25, edge x=90"
    );
    desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"cancel"})),
        )
        .await;
}

async fn fixture(client: &mut Client) -> Value {
    let mut color = clip("$clip", "unused", 0, 100);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made = client.apply(json!([{"op":"add_timeline","timeline":{
        "id":"$timeline","name":"Window","width":128,"height":72,
        "rate":{"num":25,"den":1},"sample_rate":48000,
        "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,"clips":[color],"transitions":[]}]
    }}])).await;
    let timeline = made["names"]["$timeline"].clone();
    let clip = made["names"]["$clip"].clone();
    let target = json!({"level":"clip","timeline":timeline,"clip":clip});
    let mut node = columbina_model::Node::corrector("$node");
    let columbina_model::NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    c.windows.push(columbina_model::grade::Window::new(
        "w1",
        columbina_model::grade::WindowShape::circular(),
    ));
    let added = client
        .apply(json!([{"op":"add_node","grade":target,"node":node,"links":[
        {"from":"source","to":"$node","kind":"rgb","input":0},
        {"from":"$node","to":"output","kind":"rgb","input":0}]}]))
        .await;
    json!({"monitor":"timeline","revision":added["revision"],"timeline":timeline,
        "target":target,"clip":clip,"node":added["names"]["$node"],"selected":"w1","visible":true,"outline":true,"tool":"window"})
}

#[tokio::test]
async fn same_connection_cancel_during_native_presentation_wait_never_revives_capture() {
    for kind in ["up", "cancel"] {
        same_connection_end_during_wait(kind).await;
    }
}

async fn same_connection_end_during_wait(kind: &str) {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let binding = fixture(&mut desktop).await;
    desktop.result("view.attach", Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding)).await;
    let window = until_views(&parent, "window view", |views| views.len() == 1).await[0].0;
    let deadline = Instant::now() + WAIT;
    loop {
        let event = pointer(&mut desktop, window, WM_MOUSEMOVE, 64, 36).await;
        if event["snapshot"]["window"]["binding"] == bound["binding"] {
            break;
        }
        assert!(Instant::now() < deadline, "no selected overlay: {event}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A hidden native view keeps the clicked presentation but cannot present FreezeAt's draw.
    desktop
        .result(
            "view.show",
            Some(json!({"monitor":"timeline","shown":false})),
        )
        .await;
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 36).await;
    let before = project(&mut desktop).await;
    let arming = tokio::time::timeout(
        Duration::from_secs(2),
        desktop.result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        ),
    )
    .await
    .expect("down must not await native presentation on this connection");
    assert_eq!(arming["phase"], "arming");
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 80, 36).await;
    let waiting=desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":arming["capture"],"kind":"move","event":moved["event"]}))).await;
    assert_eq!(waiting["phase"], "arming");
    assert_eq!(
        project(&mut desktop).await,
        before,
        "an acknowledged worker draw with no native presentation cannot write history"
    );
    let ended = tokio::time::timeout(
        Duration::from_secs(2),
        desktop.result(
            "window.input",
            Some(json!({"binding":bound["binding"],"capture":arming["capture"],"kind":kind})),
        ),
    )
    .await
    .expect("same connection cancel must remain reachable");
    assert_eq!(ended["phase"], "ended");
    desktop
        .result(
            "view.show",
            Some(json!({"monitor":"timeline","shown":true})),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        project(&mut desktop).await,
        before,
        "late freeze/present must not write or revive"
    );
    let error = desktop.error("window.input", Some(json!({"binding":bound["binding"],"capture":arming["capture"],"kind":"move","event":down["event"]}))).await;
    assert_eq!(error["data"]["reason"], "capture");
}

#[tokio::test]
async fn pure_reference_frame_wipe_has_neither_window_hit_nor_capture() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let binding = fixture(&mut desktop).await;
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding)).await;
    desktop
        .result(
            "viewer.wipe",
            Some(json!({"style":"mix","position":1.0,"reference":{"kind":"frame","frame":1}})),
        )
        .await;
    let window = until_views(&parent, "window view", |views| views.len() == 1).await[0].0;
    let deadline = Instant::now() + WAIT;
    loop {
        let p = pointer(&mut desktop, window, WM_MOUSEMOVE, 64, 36).await;
        if p["snapshot"]["comparison"] == "wipe" && p["snapshot"]["ready"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "wipe not presented: {p}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let before = project(&mut desktop).await;
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 36).await;
    let answer = desktop
        .call(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        )
        .await;
    assert_eq!(
        answer["error"]["data"]["reason"], "hit",
        "pure reference B must not capture: {answer}"
    );
    assert_eq!(project(&mut desktop).await, before);
}
