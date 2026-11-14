//! Native messages exercise actual presentation and the real, sequential RPC connection.
use super::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
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
async fn native_transform_controls_preserve_left_keys_and_restore_the_whole_list() {
    for gradient in [false, true] {
        let dir = project_dir();
        let engine = engine(dir.path(), &[]).await;
        let parent = parent::Parent::open();
        let mut desktop = engine.client("desktop").await;
        let mut binding = fixture(&mut desktop).await;
        let mut all = windows(&project(&mut desktop).await, &binding);
        let mut other = all[0].clone();
        other["id"] = json!("w2");
        other["enabled"] = json!(false);
        all.as_array_mut().unwrap().push(other);
        let (field, path, down_at, moved_at, expected) = if gradient {
            all[0]["shape"] = json!({"kind":"gradient","softness":{"keys":[{"frame":0,"value":10,"interp":"linear"},{"frame":20,"value":50,"interp":"hold"}]}});
            ("softness", "shape", (82, 41), (82, 50), 60.0)
        } else {
            all[0]["transform"]["size"] = json!({"keys":[{"frame":0,"value":20,"interp":"linear"},{"frame":20,"value":80,"interp":"hold"}]});
            ("size", "transform", (82, 54), (91, 63), 45.0)
        };
        let changed = desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
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
        let mut bound = desktop.result("window.bind", Some(binding.clone())).await;
        let window = until_views(&parent, "transform view", |views| views.len() == 1).await[0].0;
        ready(&mut desktop, window, &bound["binding"]).await;
        let before = project(&mut desktop).await;
        let all = windows(&before, &binding);
        for cancel in [false, true] {
            let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, down_at.0, down_at.1).await;
            let started = desktop
                .result(
                    "window.input",
                    Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
                )
                .await;
            let armed = phase(&mut desktop, "armed").await;
            assert_eq!(armed["keys"][field], 0);
            assert!(armed["keys"].get("pan").is_none());
            let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, moved_at.0, moved_at.1).await;
            desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
            let actual = windows(&project(&mut desktop).await, &binding);
            assert!(
                (actual[0][path][field]["keys"][0]["value"].as_f64().unwrap() - expected).abs()
                    < 1e-8
            );
            assert_eq!(
                actual[0][path][field]["keys"][1],
                all[0][path][field]["keys"][1]
            );
            assert_eq!(actual[1], all[1]);
            if !cancel {
                pointer(&mut desktop, window, WM_LBUTTONUP, moved_at.0, moved_at.1).await;
            }
            desktop.result("window.input",Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":if cancel {"cancel"} else {"up"}}))).await;
            if !cancel {
                desktop.result("history.undo", None).await;
            }
            let restored = project(&mut desktop).await;
            assert_eq!(restored["project"], before["project"]);
            binding["revision"] = restored["revision"].clone();
            bound = desktop.result("window.bind", Some(binding.clone())).await;
            ready(&mut desktop, window, &bound["binding"]).await;
        }
        assert_eq!(
            desktop
                .result("transport.state", Some(json!({"monitor":"timeline"})))
                .await["playing"],
            false
        );
    }
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
    // The pixels of the outline are looked at: on a card, not on the software adapter, which
    // draws its edge otherwise.
    if columbina_compose::card_for_tests().is_none() {
        return;
    }
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
        pixels[27 * width + 79] > 220,
        "the white arc near (80,27) follows interpolated Pan=50"
    );
    assert!(
        pixels[27 * width + 66] < 150,
        "the left key's Pan=40 arc must not be drawn"
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
        pixels[27 * width + 87] > 220,
        "editing the left key by 12.5 gives displayed half-way Pan=56.25, arc near (88,27)"
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

#[allow(
    unsafe_code,
    reason = "synthetic middle-button messages to a test-owned child window"
)]
async fn middle(client: &mut Client, window: isize, message: u32, x: i32, y: i32) -> Value {
    client.take_notifications();
    unsafe {
        PostMessageW(
            Some(HWND(window as *mut _)),
            message,
            WPARAM(if message == WM_MBUTTONDOWN { 16 } else { 0 }),
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

async fn shown(client: &mut Client, window: isize, revision: &Value) {
    let deadline = Instant::now() + WAIT;
    loop {
        let p = pointer(client, window, WM_MOUSEMOVE, 64, 36).await;
        if p["snapshot"]["revision"] == *revision && p["snapshot"]["ready"] == true {
            return;
        }
        assert!(Instant::now() < deadline, "revision not presented: {p}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn polygon_points(project: &Value, binding: &Value) -> usize {
    windows(project, binding)[0]["shape"]["points"]
        .as_array()
        .unwrap()
        .len()
}

#[tokio::test]
async fn native_polygon_edge_click_inserts_once_and_a_middle_tap_deletes_whole_animation_points() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    all[0]["shape"] = json!({"kind":"polygon","inside":0,"outside":0,"points":[
        {"x":-0.25,"y":-0.25},{"x":0.25,"y":-0.25},{"x":0.25,"y":0.25},{"x":-0.25,"y":0.25}]});
    let changed = desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let window = until_views(&parent, "polygon view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &bound["binding"]).await;
    // A quick click on the top edge inserts exactly once, whether or not arming finished first.
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 64, 18).await;
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"down","event":down["event"]})),
        )
        .await;
    assert_eq!(started["keys"]["inserted"], 1);
    pointer(&mut desktop, window, WM_LBUTTONUP, 64, 18).await;
    let ended = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"capture":started["capture"],"kind":"up"})),
        )
        .await;
    assert_eq!(ended["phase"], "ended", "{ended}");
    let inserted = project(&mut desktop).await;
    assert_eq!(polygon_points(&inserted, &binding), 5);
    desktop.result("history.undo", None).await;
    let restored = project(&mut desktop).await;
    assert_eq!(polygon_points(&restored, &binding), 4);
    // A middle tap on a point deletes it; a moving middle drag never reaches this route.
    shown(&mut desktop, window, &restored["revision"]).await;
    let down = middle(&mut desktop, window, WM_MBUTTONDOWN, 82, 54).await;
    let up = middle(&mut desktop, window, WM_MBUTTONUP, 82, 54).await;
    assert_eq!(up["middle_moved"], false);
    let deleted = desktop
        .result(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"delete","event":down["event"]})),
        )
        .await;
    assert_eq!(
        (deleted["phase"].clone(), deleted["point"].clone()),
        (json!("deleted"), json!(2))
    );
    let three = project(&mut desktop).await;
    assert_eq!(polygon_points(&three, &binding), 3);
    // Below three points the whole delete is refused and nothing changes.
    shown(&mut desktop, window, &three["revision"]).await;
    let down = middle(&mut desktop, window, WM_MBUTTONDOWN, 46, 18).await;
    middle(&mut desktop, window, WM_MBUTTONUP, 46, 18).await;
    let refused = desktop
        .error(
            "window.input",
            Some(json!({"binding":bound["binding"],"kind":"delete","event":down["event"]})),
        )
        .await;
    assert_eq!(refused["data"]["topology"]["why"], "minimum", "{refused}");
    assert_eq!(project(&mut desktop).await, three);
}

fn curve_shape(project: &Value, binding: &Value) -> (Vec<Value>, bool) {
    let shape = &windows(project, binding)[0]["shape"];
    (
        shape["points"].as_array().unwrap().clone(),
        shape["closed"].as_bool().unwrap(),
    )
}

/// A coordinate of a static Curve point field, e.g. `("position", "x")`.
fn curve_value(point: &Value, field: &str, axis: &str) -> f64 {
    point[field][axis].as_f64().unwrap()
}

/// Bind again at the current revision and wait for its first frame, as the transform tests do
/// after a drag. Returns the new binding. The desktop's own flow, one binding for the whole
/// drawing, is the `native_curve_clicks_draw_in_a_row_…` test below.
async fn curve_rebind(client: &mut Client, window: isize, binding: &mut Value) -> (Value, Value) {
    let now = project(client).await;
    binding["revision"] = now["revision"].clone();
    let bound = client.result("window.bind", Some(binding.clone())).await;
    ready(client, window, &bound["binding"]).await;
    (bound["binding"].clone(), now)
}

/// A native left down at `at`, dragged to `through` once armed, and released there. Returns the
/// down's answer.
async fn curve_gesture(
    client: &mut Client,
    window: isize,
    binding: &Value,
    at: (i32, i32),
    through: Option<(i32, i32)>,
) -> Value {
    let down = pointer(client, window, WM_LBUTTONDOWN, at.0, at.1).await;
    let started = client
        .result(
            "window.input",
            Some(json!({"binding":binding,"kind":"down","event":down["event"]})),
        )
        .await;
    let end = through.unwrap_or(at);
    if let Some((x, y)) = through {
        phase(client, "armed").await;
        let moved = pointer(client, window, WM_MOUSEMOVE, x, y).await;
        client.result("window.input",Some(json!({"binding":binding,"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    }
    pointer(client, window, WM_LBUTTONUP, end.0, end.1).await;
    let ended = client
        .result(
            "window.input",
            Some(json!({"binding":binding,"capture":started["capture"],"kind":"up"})),
        )
        .await;
    assert_eq!(ended["phase"], "ended", "{ended}");
    started
}

/// A native middle tap at `at`, named for deletion.
async fn curve_tap(client: &mut Client, window: isize, binding: &Value, at: (i32, i32)) -> Value {
    let down = middle(client, window, WM_MBUTTONDOWN, at.0, at.1).await;
    let up = middle(client, window, WM_MBUTTONUP, at.0, at.1).await;
    assert_eq!(up["middle_moved"], false);
    json!({"binding":binding,"kind":"delete","event":down["event"]})
}

#[tokio::test]
async fn native_curve_clicks_draw_close_insert_on_the_edge_and_middle_taps_keep_two_points() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    all[0]["shape"] = json!({"kind":"curve","points":[],"closed":false,"inside":0,"outside":0});
    let changed = desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let window = until_views(&parent, "curve view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &bound["binding"]).await;
    // A click in the viewer draws a corner point where it lands (pixel centres, 72 px high).
    let started = curve_gesture(&mut desktop, window, &bound["binding"], (40, 36), None).await;
    assert_eq!(started["keys"]["inserted"], 0, "{started}");
    let (b, drawn) = curve_rebind(&mut desktop, window, &mut binding).await;
    let (points, closed) = curve_shape(&drawn, &binding);
    assert_eq!((points.len(), closed), (1, false));
    assert!((curve_value(&points[0], "position", "x") - (40.5 - 64.0) / 72.0).abs() < 1e-9);
    assert_eq!(curve_value(&points[0], "out", "x"), 0.0);
    // A drag pulls symmetric Bezier handles out of the new point, in the same undo step.
    let started = curve_gesture(&mut desktop, window, &b, (88, 36), Some((100, 45))).await;
    assert_eq!(started["keys"]["inserted"], 1);
    let pulled = project(&mut desktop).await;
    let (points, _) = curve_shape(&pulled, &binding);
    assert!((curve_value(&points[1], "out", "x") - 12.0 / 72.0).abs() < 1e-9);
    assert!((curve_value(&points[1], "in", "y") + 9.0 / 72.0).abs() < 1e-9);
    desktop.result("history.undo", None).await;
    assert_eq!(project(&mut desktop).await["project"], drawn["project"]);
    desktop.result("history.redo", None).await;
    let (b, _) = curve_rebind(&mut desktop, window, &mut binding).await;
    curve_gesture(&mut desktop, window, &b, (64, 60), None).await;
    // Clicking the first point closes the curve and adds no point.
    let (b, _) = curve_rebind(&mut desktop, window, &mut binding).await;
    let started = curve_gesture(&mut desktop, window, &b, (40, 36), None).await;
    assert!(started["keys"].get("inserted").is_none(), "{started}");
    let (b, closed_project) = curve_rebind(&mut desktop, window, &mut binding).await;
    let (points, closed) = curve_shape(&closed_project, &binding);
    assert_eq!((points.len(), closed), (3, true));
    // A click on the straight closing edge inserts a point on it, in one step.
    let started = curve_gesture(&mut desktop, window, &b, (52, 48), None).await;
    assert_eq!(started["keys"]["inserted"], 3, "{started}");
    let inserted = project(&mut desktop).await;
    let (points, _) = curve_shape(&inserted, &binding);
    assert_eq!(points.len(), 4);
    assert!((curve_value(&points[3], "position", "x") - (52.5 - 64.0) / 72.0).abs() < 1e-6);
    assert!((curve_value(&points[3], "position", "y") - (48.5 - 36.0) / 72.0).abs() < 1e-6);
    desktop.result("history.undo", None).await;
    let (b, restored) = curve_rebind(&mut desktop, window, &mut binding).await;
    assert_eq!(restored["project"], closed_project["project"]);
    // Dragging an out handle turns its linked in handle with it; Esc restores the whole list.
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 100, 45).await;
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"kind":"down","event":down["event"]})),
        )
        .await;
    let armed = phase(&mut desktop, "armed").await;
    assert_eq!(armed["keys"]["points"][0]["point"], 1, "{armed}");
    let moved = pointer(&mut desktop, window, WM_MOUSEMOVE, 100, 54).await;
    desktop.result("window.input",Some(json!({"binding":b,"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    let (points, _) = curve_shape(&project(&mut desktop).await, &binding);
    let out = [
        curve_value(&points[1], "out", "x"),
        curve_value(&points[1], "out", "y"),
    ];
    let into = [
        curve_value(&points[1], "in", "x"),
        curve_value(&points[1], "in", "y"),
    ];
    assert!((out[1] - 18.0 / 72.0).abs() < 1e-9, "{out:?}");
    assert!((into[0] + out[0]).abs() < 1e-9 && (into[1] + out[1]).abs() < 1e-9);
    desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"capture":started["capture"],"kind":"cancel"})),
        )
        .await;
    // A middle tap deletes a point; a closed curve keeps two, and then nothing changes.
    let (b, cancelled) = curve_rebind(&mut desktop, window, &mut binding).await;
    assert_eq!(cancelled["project"], closed_project["project"]);
    let tap = curve_tap(&mut desktop, window, &b, (64, 60)).await;
    let deleted = desktop.result("window.input", Some(tap)).await;
    assert_eq!(deleted["point"], 2, "{deleted}");
    let (b, two) = curve_rebind(&mut desktop, window, &mut binding).await;
    let (points, closed) = curve_shape(&two, &binding);
    assert_eq!((points.len(), closed), (2, true));
    let tap = curve_tap(&mut desktop, window, &b, (40, 36)).await;
    let refused = desktop.error("window.input", Some(tap)).await;
    assert_eq!(
        refused["data"]["topology"],
        json!({"why":"minimum","least":2,"shape":"curve"}),
        "{refused}"
    );
    assert_eq!(project(&mut desktop).await, two);
}

/// As the desktop does: one binding for a whole drawing, each click sent once the previous one's
/// revision is presented, never bound again in between, including right after a drag.
#[tokio::test]
async fn native_curve_clicks_draw_in_a_row_on_one_binding_waiting_only_for_each_presented_frame() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    all[0]["shape"] = json!({"kind":"curve","points":[],"closed":false,"inside":0,"outside":0});
    let changed = desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let b = bound["binding"].clone();
    let window = until_views(&parent, "curve row view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &b).await;
    // A click, a drag, two more clicks, the close, then an insertion on the closing edge.
    let steps = [
        ((40, 36), None),
        ((88, 36), Some((100, 45))),
        ((64, 60), None),
        ((40, 36), None),
        ((52, 48), None),
    ];
    let inserted = [json!(0), json!(1), json!(2), Value::Null, json!(3)];
    for (k, (at, through)) in steps.into_iter().enumerate() {
        let started = curve_gesture(&mut desktop, window, &b, at, through).await;
        assert_eq!(started["keys"]["inserted"], inserted[k], "{k}: {started}");
        let now = project(&mut desktop).await;
        shown(&mut desktop, window, &now["revision"]).await;
    }
    let (points, closed) = curve_shape(&project(&mut desktop).await, &binding);
    assert_eq!((points.len(), closed), (4, true));
    assert!((curve_value(&points[1], "out", "x") - 12.0 / 72.0).abs() < 1e-9);
    assert!((curve_value(&points[3], "position", "x") - (52.5 - 64.0) / 72.0).abs() < 1e-6);
}

/// A native left message with the given `MK_*` key bits (Shift is 4) in its WPARAM.
#[allow(
    unsafe_code,
    reason = "synthetic messages to a test-owned child window"
)]
async fn keyed(
    client: &mut Client,
    window: isize,
    message: u32,
    (x, y): (i32, i32),
    keys: usize,
) -> Value {
    client.take_notifications();
    unsafe {
        PostMessageW(
            Some(HWND(window as *mut _)),
            message,
            WPARAM(keys),
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

/// A native drag from `at` to `to` with `keys` held (`MK_LBUTTON` is 1, `MK_SHIFT` 4), once armed;
/// returns the down's answer and the up's.
async fn keyed_gesture(
    client: &mut Client,
    window: isize,
    binding: &Value,
    at: (i32, i32),
    to: (i32, i32),
    keys: usize,
) -> (Value, Value) {
    let down = keyed(client, window, WM_LBUTTONDOWN, at, keys).await;
    let started = client
        .result(
            "window.input",
            Some(json!({"binding":binding,"kind":"down","event":down["event"]})),
        )
        .await;
    phase(client, "armed").await;
    let moved = keyed(client, window, WM_MOUSEMOVE, to, keys).await;
    client.result("window.input",Some(json!({"binding":binding,"capture":started["capture"],"kind":"move","event":moved["event"]}))).await;
    keyed(client, window, WM_LBUTTONUP, to, keys & !1).await;
    let ended = client
        .result(
            "window.input",
            Some(json!({"binding":binding,"capture":started["capture"],"kind":"up"})),
        )
        .await;
    assert_eq!(ended["phase"], "ended", "{ended}");
    (started, ended)
}

/// Wait until the presented frame is at `revision` and shows `points` selected.
async fn selection_shown(client: &mut Client, window: isize, revision: &Value, points: &Value) {
    let deadline = Instant::now() + WAIT;
    loop {
        let p = pointer(client, window, WM_MOUSEMOVE, 64, 36).await;
        if p["snapshot"]["revision"] == *revision
            && p["snapshot"]["ready"] == true
            && p["snapshot"]["window"]["points"] == *points
        {
            return;
        }
        assert!(Instant::now() < deadline, "selection not presented: {p}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// As the desktop does, on one binding: a Shift-drag selects two anchors, the box moves them,
/// a corner rotation is cancelled, Backspace deletes them, and below two anchors the whole
/// delete is refused (RM20 p. 3195). Each action waits only for its frame to be presented.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one binding through a whole selection, its box drags and its deletions"
)]
async fn native_shift_band_selects_curve_points_the_box_moves_them_and_backspace_deletes_them() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut binding = fixture(&mut desktop).await;
    let mut all = windows(&project(&mut desktop).await, &binding);
    let corner =
        |x: f64, y: f64| json!({"position":{"x":x,"y":y},"in":{"x":0,"y":0},"out":{"x":0,"y":0}});
    all[0]["shape"] = json!({"kind":"curve","closed":true,"inside":0,"outside":0,"points":[
        corner(-0.25,-0.25),corner(0.25,-0.25),corner(0.25,0.25),corner(-0.25,0.25)]});
    let changed = desktop.apply(json!([{"op":"set_node","grade":binding["target"],"id":binding["node"],"params":{"windows":all}}])).await;
    binding["revision"] = changed["revision"].clone();
    desktop.result("view.attach",Some(json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72}))).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":binding["timeline"]})),
        )
        .await;
    let bound = desktop.result("window.bind", Some(binding.clone())).await;
    let b = bound["binding"].clone();
    let window = until_views(&parent, "multi view", |views| views.len() == 1).await[0].0;
    ready(&mut desktop, window, &b).await;
    let square = project(&mut desktop).await;
    // Shift-drag a band about the top anchors (device 46, 18 and 82, 18): pure selection.
    let (_, ended) = keyed_gesture(&mut desktop, window, &b, (40, 10), (88, 24), 5).await;
    assert_eq!(ended["selected_points"], json!([0, 1]), "{ended}");
    assert_eq!(project(&mut desktop).await["revision"], square["revision"]);
    selection_shown(&mut desktop, window, &square["revision"], &json!([0, 1])).await;
    // Inside the box moves both, as one undo step; the selection stays.
    let (started, ended) = keyed_gesture(&mut desktop, window, &b, (64, 18), (64, 27), 1).await;
    assert_eq!(started["keys"]["points"][1]["point"], 1, "{started}");
    assert_eq!(ended["selected_points"], json!([0, 1]));
    let moved = project(&mut desktop).await;
    let (points, _) = curve_shape(&moved, &binding);
    assert!((curve_value(&points[0], "position", "y") + 0.125).abs() < 1e-9);
    assert!((curve_value(&points[1], "position", "y") + 0.125).abs() < 1e-9);
    assert!((curve_value(&points[2], "position", "y") - 0.25).abs() < 1e-9);
    // The box is now 38..90 x 19..35: just beside its top right corner rotates them; Esc puts
    // the whole list back without a step.
    selection_shown(&mut desktop, window, &moved["revision"], &json!([0, 1])).await;
    let down = pointer(&mut desktop, window, WM_LBUTTONDOWN, 96, 13).await;
    let started = desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"kind":"down","event":down["event"]})),
        )
        .await;
    phase(&mut desktop, "armed").await;
    let turned = pointer(&mut desktop, window, WM_MOUSEMOVE, 70, 60).await;
    desktop.result("window.input",Some(json!({"binding":b,"capture":started["capture"],"kind":"move","event":turned["event"]}))).await;
    let rotated = project(&mut desktop).await;
    assert_ne!(rotated["project"], moved["project"]);
    desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"capture":started["capture"],"kind":"cancel"})),
        )
        .await;
    assert_eq!(project(&mut desktop).await["project"], moved["project"]);
    // Backspace deletes the selected anchors from the whole animation, in one step.
    let now = project(&mut desktop).await;
    selection_shown(&mut desktop, window, &now["revision"], &json!([0, 1])).await;
    let deleted = desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"kind":"delete_points"})),
        )
        .await;
    assert_eq!(
        (deleted["phase"].clone(), deleted["points"].clone()),
        (json!("deleted"), json!([0, 1])),
        "{deleted}"
    );
    let two = project(&mut desktop).await;
    let (points, closed) = curve_shape(&two, &binding);
    assert_eq!((points.len(), closed), (2, true));
    let refused = desktop
        .error(
            "window.input",
            Some(json!({"binding":b,"kind":"delete_points"})),
        )
        .await;
    assert_eq!(refused["data"]["reason"], "points", "{refused}");
    desktop.result("history.undo", None).await;
    let back = project(&mut desktop).await;
    assert_eq!(back["project"], moved["project"]);
    // All four selected: deleting would leave none, so nothing is deleted; Esc deselects.
    selection_shown(&mut desktop, window, &back["revision"], &json!([])).await;
    let (_, ended) = keyed_gesture(&mut desktop, window, &b, (40, 12), (88, 60), 5).await;
    assert_eq!(ended["selected_points"], json!([0, 1, 2, 3]), "{ended}");
    let refused = desktop
        .error(
            "window.input",
            Some(json!({"binding":b,"kind":"delete_points"})),
        )
        .await;
    assert_eq!(
        refused["data"]["topology"],
        json!({"why":"minimum","least":2,"shape":"curve"}),
        "{refused}"
    );
    assert_eq!(project(&mut desktop).await, back);
    let cleared = desktop
        .result(
            "window.input",
            Some(json!({"binding":b,"kind":"clear_points"})),
        )
        .await;
    assert_eq!(cleared["cleared"], 4, "{cleared}");
    selection_shown(&mut desktop, window, &back["revision"], &json!([])).await;
}
