//! What the palettes of the Color page need of the engine, through the real engine: a control
//! dragged is drawn as it goes and is one step to undo, told to the others at most 10 times a
//! second; the colour of a point of the picture and of a node there; what the pointer does on a
//! view, told only while it is asked for; and the figures of the picture at a node, and of a
//! frame drawn off screen by a headless engine.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, Client};

/// How long a first measure may take: the shaders of the scopes are compiled then, which FXC
/// takes most of a minute over in a build for tests.
const SLOW: Duration = Duration::from_secs(300);

/// A colour clip of `colour`, made `scale` as large and moved by `(x, y)` shares of the canvas.
fn colour(id: &str, colour: &str, length: i64, scale: f64, (x, y): (f64, f64)) -> Value {
    let mut made = clip(id, "unused", 0, length);
    made["source"] = json!({"kind": "color", "color": colour});
    made["transform"] = json!({"x": x, "y": y, "scale": scale, "rotation": 0.0, "opacity": 1.0});
    made
}

/// A timeline of 128 × 72 at 25 frames a second, a track for each clip; its names. Loaded in the
/// timeline monitor where `load` is set.
async fn timeline(client: &mut Client, clips: Vec<Value>, load: bool) -> Value {
    let tracks: Vec<Value> = clips
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "kind": "video", "name": format!("V{index}"),
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Palettes", "width": 128, "height": 72,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks,
        }}]))
        .await["names"]
        .clone();
    if load {
        client
            .result(
                "transport.load",
                Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
            )
            .await;
    }
    client.take_notifications();
    names
}

/// Colour bars of 75%: red, green, blue and grey, one in each quarter of the canvas.
fn bars() -> Vec<Value> {
    vec![
        colour("$red", "#bf0000ff", 50, 0.5, (-0.25, -0.25)),
        colour("$green", "#00bf00ff", 50, 0.5, (0.25, -0.25)),
        colour("$blue", "#0000bfff", 50, 0.5, (-0.25, 0.25)),
        colour("$grey", "#bfbfbfff", 50, 0.5, (0.25, 0.25)),
    ]
}

/// 75% of white, as a picture of 8 bits has it.
const BAR: f64 = 191.0 / 255.0;

/// Add a corrector of saturation `saturation` (50 changes nothing, 0 makes grey) between the
/// source and the output of the grade of clip `clip` of `timeline`; its target and id.
async fn grey_node(
    client: &mut Client,
    timeline: &Value,
    clip: &Value,
    saturation: f64,
) -> (Value, String) {
    let target = json!({"level": "clip", "timeline": timeline, "clip": clip});
    let mut node = columbina_model::Node::corrector("$node");
    let columbina_model::NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.saturation = columbina_model::Param::Fixed(saturation);
    let names = client
        .apply(
            json!([{"op": "add_node", "grade": target, "node": node, "links": [
            {"from": "source", "to": "$node", "kind": "rgb", "input": 0},
            {"from": "$node", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await["names"]
        .clone();
    (target, names["$node"].as_str().unwrap().to_owned())
}

/// The answer to a method that may be long in coming, keeping the notifications that come
/// before it.
async fn slow_answer(client: &mut Client, method: &str, params: Value) -> Value {
    use tokio::io::AsyncBufReadExt;
    let id = client.request(method, Some(params)).await;
    let deadline = Instant::now() + SLOW;
    loop {
        let mut line = String::new();
        tokio::time::timeout_at(deadline.into(), client.reader.read_line(&mut line))
            .await
            .expect("an answer in time")
            .unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        if message.get("id").is_some() {
            assert_eq!(message["id"], id, "{message}");
            return message;
        }
        client.notifications.push(message);
    }
}

/// The result of a method that may be long in coming.
async fn slow(client: &mut Client, method: &str, params: Value) -> Value {
    let answer = slow_answer(client, method, params).await;
    assert!(answer.get("error").is_none(), "{answer}");
    answer["result"].clone()
}

/// The error of a method that may be long in coming.
async fn slow_error(client: &mut Client, method: &str, params: Value) -> Value {
    let answer = slow_answer(client, method, params).await;
    assert!(answer.get("result").is_none(), "{answer}");
    answer["error"].clone()
}

/// The node of the grade of clip `clip` of the project as `project.get` answers it.
fn node_of(project: &Value, clip: &Value) -> Value {
    project["project"]["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|timeline| timeline["tracks"].as_array().unwrap())
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|found| found["id"] == *clip)
        .map(|found| found["grade"]["nodes"][0].clone())
        .unwrap()
}

fn near(value: &Value, expected: [f64; 3], within: f64) -> bool {
    (0..3).all(|i| (value[i].as_f64().unwrap() - expected[i]).abs() <= within)
}

/// The `project.changed` that came and come within `time`, keeping the others.
async fn changes_for(client: &mut Client, time: Duration) -> Vec<Value> {
    let end = Instant::now() + time;
    while let Ok(message) =
        tokio::time::timeout_at(end.into(), super::read_message(&mut client.reader)).await
    {
        client
            .notifications
            .push(message.expect("the engine closed the connection"));
    }
    let (changes, rest): (Vec<Value>, Vec<Value>) = std::mem::take(&mut client.notifications)
        .into_iter()
        .partition(|message| message["method"] == "project.changed");
    client.notifications = rest;
    changes
        .into_iter()
        .map(|message| message["params"].clone())
        .collect()
}

/// Check that changes told follow one another from `from`, each from the revision that the one
/// before made; give the revision of the last.
fn follow(changes: &[Value], from: u64) -> u64 {
    let mut at = from;
    for change in changes {
        let base = change.get("base").map_or_else(
            || change["revision"].as_u64().unwrap() - 1,
            |base| base.as_u64().unwrap(),
        );
        assert_eq!(base, at, "{changes:?}");
        at = change["revision"].as_u64().unwrap();
    }
    at
}

/// A transaction of a gesture that sets the saturation of a node.
fn saturation(
    target: &Value,
    node: &str,
    value: f64,
    gesture: &Value,
    revision: Option<u64>,
) -> Value {
    let mut params = json!({
        "ops": [{"op": "set_node", "grade": target, "id": node,
            "params": {"primaries": {"saturation": value}}}],
        "gesture": gesture,
    });
    if let Some(revision) = revision {
        params["revision"] = json!(revision);
    }
    params
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one drag, from its first change to its undo"
)]
async fn a_dragged_control_is_seen_as_it_goes_and_is_one_step_to_undo() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    let names = timeline(
        &mut desktop,
        vec![colour("$orange", "#c86e28ff", 50, 1.0, (0.0, 0.0))],
        true,
    )
    .await;
    // Saturation 50 is as it is: the node changes nothing yet.
    let (target, node) =
        grey_node(&mut desktop, &names["$timeline"], &names["$orange"], 50.0).await;
    let digest = |pixels: &Value| pixels["pixels"][0].clone();
    let orange = json!([200, 110, 40, 255]);
    let deadline = Instant::now() + super::WAIT;
    loop {
        let shown = desktop
            .result(
                "transport.frame_digest",
                Some(json!({"monitor": "timeline"})),
            )
            .await;
        if digest(&shown) == orange {
            break;
        }
        assert!(Instant::now() < deadline, "{shown}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let base = project(&mut desktop).await["revision"].as_u64().unwrap();
    changes_for(&mut agent, Duration::from_millis(50)).await;

    // 30 changes of one drag, each prepared at the revision before the drag, as a page whose copy
    // lags behind sends them; the picture changes as it goes.
    let mut last = 0;
    let mut seen_greyer = false;
    for step in 1..=30 {
        let value = 50.0 - f64::from(step) * 50.0 / 30.0;
        let gesture = json!({"id": "drag-1", "end": step == 30});
        let applied = desktop
            .result(
                "transaction.apply",
                Some(saturation(&target, &node, value, &gesture, Some(base))),
            )
            .await;
        last = applied["revision"].as_u64().unwrap();
        assert_eq!(last, base + u64::try_from(step).unwrap());
        if step == 15 {
            let deadline = Instant::now() + super::WAIT;
            loop {
                let shown = desktop
                    .result(
                        "transport.frame_digest",
                        Some(json!({"monitor": "timeline"})),
                    )
                    .await;
                let pixel = digest(&shown);
                let spread = pixel[0].as_i64().unwrap() - pixel[2].as_i64().unwrap();
                if spread > 20 && spread < 150 {
                    seen_greyer = true;
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "the picture did not change: {shown}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(seen_greyer);

    // The agent is told every change, in order, in far fewer notifications.
    let told = changes_for(&mut agent, Duration::from_millis(300)).await;
    assert!(
        told.len() < 30 && told.len() >= 2,
        "{} notifications: {told:?}",
        told.len()
    );
    assert!(
        told.iter().any(|change| change.get("base").is_some()),
        "{told:?}"
    );
    assert_eq!(follow(&told, base), last);
    let ops: usize = told
        .iter()
        .map(|change| change["done"].as_array().unwrap().len())
        .sum();
    assert_eq!(ops, 30);
    assert!(told.iter().all(|change| change["by"] == "desktop"));

    // One undo takes the whole drag back, and the picture with it; the next takes the node.
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"].as_array().unwrap().len(), 30);
    let saved = node_of(&project(&mut agent).await, &names["$orange"]);
    assert_eq!(saved["params"]["primaries"]["saturation"], 50.0, "{saved}");
    let deadline = Instant::now() + super::WAIT;
    loop {
        let shown = agent
            .result(
                "transport.frame_digest",
                Some(json!({"monitor": "timeline"})),
            )
            .await;
        if digest(&shown) == orange {
            break;
        }
        assert!(Instant::now() < deadline, "{shown}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"][0]["op"], "remove_node", "{undone}");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the rules of a gesture, one after another"
)]
async fn a_gesture_ends_as_another_change_comes_and_its_revisions_then_are_stale() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    let names = timeline(
        &mut desktop,
        vec![colour("$orange", "#c86e28ff", 50, 1.0, (0.0, 0.0))],
        false,
    )
    .await;
    let (target, node) = grey_node(&mut desktop, &names["$timeline"], &names["$orange"], 1.0).await;
    let base = project(&mut desktop).await["revision"].as_u64().unwrap();
    changes_for(&mut agent, Duration::from_millis(50)).await;

    // The id is checked.
    for id in [json!(""), json!("x".repeat(65)), json!(3)] {
        let error = desktop
            .error(
                "transaction.apply",
                Some(saturation(&target, &node, 0.5, &json!({"id": id}), None)),
            )
            .await;
        assert_eq!(error["code"], -32602, "{error}");
    }

    // Two changes of g1, both prepared at the revision before it.
    for value in [0.9, 0.8] {
        desktop
            .result(
                "transaction.apply",
                Some(saturation(
                    &target,
                    &node,
                    value,
                    &json!({"id": "g1"}),
                    Some(base),
                )),
            )
            .await;
    }
    // The agent changes the node: the gesture ends, and what it held is told first.
    let label = json!([{"op": "set_node", "grade": target, "id": node, "label": "Agent"}]);
    agent.apply(label).await;
    let told = changes_for(&mut agent, Duration::from_millis(50)).await;
    assert_eq!(follow(&told, base), base + 3);
    // A change of g1 prepared before the agent's is refused as stale, and changes nothing.
    let stale = desktop
        .error(
            "transaction.apply",
            Some(saturation(
                &target,
                &node,
                0.7,
                &json!({"id": "g1"}),
                Some(base + 1),
            )),
        )
        .await;
    assert_eq!(stale["data"]["reason"], "stale_revision", "{stale}");
    assert_eq!(stale["data"]["actual"], base + 3);
    // Prepared at the latest, it begins another gesture, of its own step; so does another id.
    desktop
        .result(
            "transaction.apply",
            Some(saturation(
                &target,
                &node,
                0.6,
                &json!({"id": "g1"}),
                Some(base + 3),
            )),
        )
        .await;
    desktop
        .result(
            "transaction.apply",
            Some(saturation(
                &target,
                &node,
                0.5,
                &json!({"id": "g2", "end": true}),
                Some(base + 4),
            )),
        )
        .await;
    // Another connection's gesture of the same id is another gesture.
    let mut other = engine.client("desktop").await;
    other
        .result(
            "transaction.apply",
            Some(saturation(&target, &node, 0.4, &json!({"id": "g2"}), None)),
        )
        .await;
    other
        .result(
            "transaction.apply",
            Some(saturation(&target, &node, 0.3, &json!({"id": "g2"}), None)),
        )
        .await;
    // It goes as its connection closes: what it held is told.
    drop(other);
    let told = changes_for(&mut agent, Duration::from_millis(400)).await;
    assert_eq!(follow(&told, base + 3), base + 7);

    // Undo, step by step: the other's gesture (two changes), g2, g1 again, the agent's label,
    // and the two changes of the first g1 together.
    let saturation_now = |project: &Value| {
        node_of(project, &names["$orange"])["params"]["primaries"]["saturation"]
            .as_f64()
            .unwrap()
    };
    for (count, after) in [(2, 0.5), (1, 0.6), (1, 0.8), (1, 0.8), (2, 1.0)] {
        let undone = agent.result("history.undo", None).await;
        assert_eq!(undone["done"].as_array().unwrap().len(), count, "{undone}");
        assert_eq!(saturation_now(&project(&mut agent).await), after);
    }
    let label = node_of(&project(&mut agent).await, &names["$orange"])["label"].clone();
    assert_eq!(label, "");

    // A drag let go where it began leaves no step: the next undo takes the node away.
    for (value, end) in [(0.2, false), (1.0, true)] {
        desktop
            .result(
                "transaction.apply",
                Some(saturation(
                    &target,
                    &node,
                    value,
                    &json!({"id": "g3", "end": end}),
                    None,
                )),
            )
            .await;
    }
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"][0]["op"], "remove_node", "{undone}");
}

#[tokio::test]
async fn failed_wire_transactions_preserve_a_gesture_until_its_explicit_end() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    let names = timeline(
        &mut desktop,
        vec![colour("$orange", "#c86e28ff", 50, 1.0, (0.0, 0.0))],
        false,
    )
    .await;
    let (target, node) =
        grey_node(&mut desktop, &names["$timeline"], &names["$orange"], 50.0).await;
    let base = project(&mut desktop).await["revision"].as_u64().unwrap();
    let gesture = json!({"id":"wire"});
    let first = desktop
        .result(
            "transaction.apply",
            Some(saturation(&target, &node, 45.0, &gesture, Some(base))),
        )
        .await;
    assert_eq!(
        first
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["clamped", "done", "names", "next_id", "revision"]
    );
    let before = project(&mut desktop).await;
    for (ops, data) in [
        (
            json!([{"op":"unknown"}]),
            json!({"index":0,"reason":"invalid_op"}),
        ),
        (
            json!([{"op":"set_node","grade":target,"id":"$unknown","label":"bad"}]),
            json!({"index":0,"name":"$unknown","reason":"unknown_name"}),
        ),
    ] {
        let error = agent
            .error("transaction.apply", Some(json!({"ops":ops})))
            .await;
        assert_eq!(error["code"], -32602);
        assert_eq!(error["data"], data);
        assert_eq!(project(&mut desktop).await, before);
    }
    let invalid = json!([{"op":"set_node","grade":target,"id":"absent","label":"bad"}]);
    let error = agent
        .error(
            "transaction.apply",
            Some(json!({"ops":invalid,"gesture":{"id":"other","end":true}})),
        )
        .await;
    assert_eq!(error["code"], -32001);
    assert_eq!(project(&mut desktop).await, before);
    // A continuation prepared at base succeeds only while the original wire gesture lives.
    desktop
        .result(
            "transaction.apply",
            Some(saturation(&target, &node, 40.0, &gesture, Some(base))),
        )
        .await;
    let failed_end = desktop
        .error(
            "transaction.apply",
            Some(json!({"ops":invalid,"gesture":{"id":"wire","end":true},"revision":base})),
        )
        .await;
    assert_eq!(failed_end["code"], -32001);
    let stale = desktop
        .error(
            "transaction.apply",
            Some(saturation(&target, &node, 35.0, &gesture, Some(base))),
        )
        .await;
    assert_eq!(
        stale["data"],
        json!({"reason":"stale_revision","expected":base,"actual":base+2})
    );
    // The ordinary wire contract still permits reopening the same id at the latest revision.
    desktop
        .result(
            "transaction.apply",
            Some(saturation(&target, &node, 35.0, &gesture, Some(base + 2))),
        )
        .await;
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"].as_array().unwrap().len(), 1);
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"].as_array().unwrap().len(), 2);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the samples of one frame, then their refusals"
)]
async fn a_sample_gives_the_colour_of_the_picture_and_of_a_node_there() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let names = timeline(&mut client, bars(), true).await;
    let timeline_id = names["$timeline"].clone();

    // Each bar where it is, on the monitor and off screen alike.
    for ((px, py), expected) in [
        ((32, 18), [BAR, 0.0, 0.0]),
        ((96, 18), [0.0, BAR, 0.0]),
        ((32, 54), [0.0, 0.0, BAR]),
        ((96, 54), [BAR, BAR, BAR]),
    ] {
        let (x, y) = (f64::from(px) + 0.5, f64::from(py) + 0.5);
        let on_monitor = slow(
            &mut client,
            "view.sample",
            json!({"monitor": "timeline", "x": x, "y": y, "size": 5}),
        )
        .await;
        assert!(near(&on_monitor["rgb"], expected, 2e-3), "{on_monitor}");
        assert_eq!(on_monitor["frame"], 0);
        assert_eq!(on_monitor["monitor"], "timeline");
        assert_eq!(
            on_monitor["region"],
            json!({"x": px - 2, "y": py - 2, "width": 5, "height": 5})
        );
        let off = slow(
            &mut client,
            "view.sample",
            json!({"timeline": timeline_id, "frame": 3, "x": x, "y": y}),
        )
        .await;
        assert!(near(&off["rgb"], expected, 2e-3), "{off}");
        assert_eq!(off["timeline"], timeline_id);
    }
    // A point on the edge of two pixels is the pixel to its right and below.
    let edge = slow(
        &mut client,
        "view.sample",
        json!({"monitor": "timeline", "x": 64.0, "y": 18.5, "size": 1}),
    )
    .await;
    assert!(near(&edge["rgb"], [0.0, BAR, 0.0], 2e-3), "{edge}");

    // A node that greys the red clip: what goes in is red, what comes out grey, and the monitor
    // shows what comes out.
    let (target, node) = grey_node(&mut client, &timeline_id, &names["$red"], 0.0).await;
    let luma = 0.2126 * BAR;
    let deadline = Instant::now() + super::WAIT;
    let sampled = loop {
        let sampled = slow(&mut client, "view.sample", json!({
            "monitor": "timeline", "x": 32.5, "y": 18.5, "size": 3, "grade": target, "node": node,
        }))
        .await;
        if near(&sampled["rgb"], [luma, luma, luma], 4e-3) {
            break sampled;
        }
        assert!(Instant::now() < deadline, "{sampled}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        near(&sampled["node"]["input"], [BAR, 0.0, 0.0], 2e-3),
        "{sampled}"
    );
    assert!(
        near(&sampled["node"]["output"], [luma, luma, luma], 4e-3),
        "{sampled}"
    );
    assert_eq!(sampled["node"]["node"], node);
    // Off screen too; and of a pixel that the red clip does not cover, what is there for it:
    // nothing, as only its layer is drawn.
    let off = slow(&mut client, "view.sample", json!({
        "timeline": timeline_id, "frame": 0, "x": 96.5, "y": 54.5, "grade": target, "node": node,
    }))
    .await;
    assert!(near(&off["rgb"], [BAR, BAR, BAR], 2e-3), "{off}");
    assert!(near(&off["node"]["input"], [0.0, 0.0, 0.0], 1e-3), "{off}");

    // Refused: off the frame, an even size, a node half named, a node of the source monitor, a
    // node that is not there, a clip not shown.
    for (params, reason) in [
        (
            json!({"monitor": "timeline", "x": 128.0, "y": 1.0}),
            "outside",
        ),
        (
            json!({"monitor": "timeline", "x": 1.0, "y": 1.0, "size": 2}),
            "size",
        ),
        (
            json!({"monitor": "timeline", "x": 1.0, "y": 1.0, "node": node}),
            "node",
        ),
        (
            json!({"monitor": "source", "x": 1.0, "y": 1.0, "grade": target, "node": node}),
            "node",
        ),
        (
            json!({"monitor": "timeline", "x": 1.0, "y": 1.0, "grade": target, "node": "n999"}),
            "node",
        ),
        (
            json!({"timeline": timeline_id, "frame": 60, "x": 1.0, "y": 1.0, "grade": target, "node": node}),
            "clip",
        ),
        (
            json!({"monitor": "timeline", "timeline": timeline_id, "frame": 0, "x": 1.0, "y": 1.0}),
            "monitor",
        ),
    ] {
        let error = slow_error(&mut client, "view.sample", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
}

#[tokio::test]
async fn the_figures_at_a_node_are_those_of_its_input_and_its_output() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let names = timeline(&mut client, bars(), true).await;
    let timeline_id = names["$timeline"].clone();
    let (target, node) = grey_node(&mut client, &timeline_id, &names["$red"], 0.0).await;
    let luma = 0.2126 * BAR;
    let max = |figures: &Value, channel: &str| figures[channel]["max"].as_f64().unwrap();

    // What the monitor shows: every bar.
    let shown = slow(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline"}),
    )
    .await;
    assert!((max(&shown, "green") - BAR).abs() < 2e-3, "{shown}");
    // Into the node: the red clip alone, red; out of it, grey.
    let input = slow(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline", "grade": target, "node": node, "at": "input"}),
    )
    .await;
    assert!((max(&input, "red") - BAR).abs() < 2e-3, "{input}");
    assert!(
        max(&input, "green") < 1e-3 && max(&input, "blue") < 1e-3,
        "{input}"
    );
    assert_eq!(
        input["node"],
        json!({"grade": target, "node": node, "at": "input"})
    );
    assert_eq!(input["view"], "timeline");
    assert_eq!(input["timeline"], timeline_id);
    let output = slow(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline", "grade": target, "node": node}),
    )
    .await;
    for channel in ["red", "green", "blue"] {
        assert!((max(&output, channel) - luma).abs() < 4e-3, "{output}");
    }
    assert_eq!(output["node"]["at"], "output");
    // The red clip covers a quarter of the canvas, and is all that is drawn.
    assert!(
        (input["red"]["mean"].as_f64().unwrap() - BAR / 4.0).abs() < 2e-3,
        "{input}"
    );
    // Off screen, the same frame gives the figures that the monitor gives.
    let off = slow(
        &mut client,
        "scopes.statistics",
        json!({"timeline": timeline_id, "frame": 0}),
    )
    .await;
    for channel in ["red", "green", "blue", "luma"] {
        let mean = |figures: &Value| figures[channel]["mean"].as_f64().unwrap();
        assert!((mean(&off) - mean(&shown)).abs() < 1e-3, "{off} {shown}");
    }
    assert!(off.get("view").is_none(), "{off}");
    // Refused: the source monitor, which shows media ungraded; `at` without a node.
    for params in [
        json!({"view": "source", "grade": target, "node": node}),
        json!({"view": "timeline", "at": "input"}),
        json!({"timeline": timeline_id}),
    ] {
        let error = slow_error(&mut client, "scopes.statistics", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
    }
}

#[tokio::test]
async fn a_headless_engine_measures_a_frame_drawn_off_screen() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let names = timeline(&mut agent, bars(), false).await;
    let timeline_id = names["$timeline"].clone();
    let figures = slow(
        &mut agent,
        "scopes.statistics",
        json!({"timeline": timeline_id, "frame": 0, "bins": 64}),
    )
    .await;
    assert_eq!(figures["frame"], 0);
    assert_eq!(
        (figures["width"].clone(), figures["height"].clone()),
        (json!(128), json!(72))
    );
    assert_eq!(figures["histogram"]["bins"], 64);
    for channel in ["red", "green", "blue"] {
        // Each channel is 75% on two of the four quarters (its bar and the grey).
        let mean = figures[channel]["mean"].as_f64().unwrap();
        assert!((mean - BAR / 2.0).abs() < 2e-3, "{figures}");
    }
    let sampled = slow(
        &mut agent,
        "view.sample",
        json!({"timeline": timeline_id, "frame": 0, "x": 32.5, "y": 54.5}),
    )
    .await;
    assert!(near(&sampled["rgb"], [0.0, 0.0, BAR], 2e-3), "{sampled}");
    // What a monitor shows, and the pointer on a view, there are none of.
    for (method, params) in [
        ("scopes.statistics", json!({"view": "timeline"})),
        (
            "view.sample",
            json!({"monitor": "timeline", "x": 1.0, "y": 1.0}),
        ),
        ("view.pointer", json!({"monitor": "timeline", "on": true})),
    ] {
        let error = agent.error(method, Some(params)).await;
        assert_eq!(error["code"], -32601, "{method}: {error}");
    }
}

#[cfg(windows)]
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the pointer, from asked for to let go"
)]
async fn the_pointer_on_a_view_is_told_only_while_it_is_asked_for() {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let parent = super::parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    desktop
        .result("view.display", Some(json!({"hdr": false})))
        .await;
    // A view of 256 × 144 for a timeline of 128 × 72: two pixels of the window to one.
    let attach = json!({"monitor": "timeline", "parent": parent.hwnd.to_string(), "x": 0, "y": 0, "width": 256, "height": 144});
    desktop.result("view.attach", Some(attach)).await;
    timeline(&mut desktop, bars(), true).await;
    let views = super::until_views(&parent, "the view", |views| views.len() == 1).await;
    let view = views[0].0;
    // Posted as the system would post them: buttons and keys in wparam, the point in lparam.
    let post = |message: u32, keys: usize, (x, y): (isize, isize)| {
        let point = (x & 0xFFFF) | ((y & 0xFFFF) << 16);
        #[allow(unsafe_code, reason = "a message posted to a window, which only fails")]
        unsafe {
            PostMessageW(
                Some(HWND(view as *mut core::ffi::c_void)),
                message,
                WPARAM(keys),
                LPARAM(point),
            )
        }
        .unwrap();
    };
    let pointer_events = |client: &mut Client| -> Vec<Value> {
        let (pointer, rest): (Vec<Value>, Vec<Value>) = std::mem::take(&mut client.notifications)
            .into_iter()
            .partition(|message| message["method"] == "view.pointer");
        client.notifications = rest;
        pointer
            .into_iter()
            .map(|message| message["params"].clone())
            .collect()
    };
    // Wait until a frame is shown, so that a point is found on it.
    let deadline = Instant::now() + super::WAIT;
    while desktop
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await["shown"]
        .is_null()
    {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Not asked for: nothing is told.
    post(WM_LBUTTONDOWN, 1, (20, 40));
    post(WM_LBUTTONUP, 0, (20, 40));
    changes_for(&mut desktop, Duration::from_millis(300)).await;
    assert_eq!(pointer_events(&mut desktop), Vec::<Value>::new());

    // Asked for, by the desktop only.
    let error = agent
        .error(
            "view.pointer",
            Some(json!({"monitor": "timeline", "on": true})),
        )
        .await;
    assert_eq!(error["code"], -32600, "{error}");
    let taken = desktop
        .result(
            "view.pointer",
            Some(json!({"monitor": "timeline", "on": true, "cursor": "crosshair"})),
        )
        .await;
    assert_eq!(
        taken,
        json!({"monitor": "timeline", "on": true, "cursor": "crosshair"})
    );
    // A press with Shift, a drag, a release.
    post(WM_LBUTTONDOWN, 1 | 4, (20, 40));
    for x in [30, 40, 50, 60] {
        post(WM_MOUSEMOVE, 1, (x, 40));
    }
    post(WM_LBUTTONUP, 0, (64, 40));
    let deadline = Instant::now() + super::WAIT;
    let mut told = Vec::new();
    while told.last().is_none_or(|last: &Value| last["kind"] != "up") {
        changes_for(&mut desktop, Duration::from_millis(50)).await;
        told.extend(pointer_events(&mut desktop));
        assert!(Instant::now() < deadline, "{told:?}");
    }
    assert_eq!(told[0]["kind"], "down", "{told:?}");
    assert_eq!(told[0]["button"], "left");
    assert_eq!(told[0]["shift"], true);
    assert_eq!(told[0]["ctrl"], false);
    assert_eq!(
        (told[0]["x"].as_f64(), told[0]["y"].as_f64()),
        (Some(10.25), Some(20.25)),
        "{told:?}"
    );
    assert_eq!(told[0]["window"], json!({"x": 20, "y": 40}));
    assert_eq!(told[0]["inside"], true);
    let moves: Vec<&Value> = told
        .iter()
        .filter(|params| params["kind"] == "move")
        .collect();
    assert!(!moves.is_empty() && moves.len() <= 4, "{told:?}");
    assert_eq!(
        moves.last().unwrap()["x"].as_f64(),
        Some(30.25),
        "the last move is always told: {told:?}"
    );
    let up = told.last().unwrap();
    assert_eq!(
        (up["x"].as_f64(), up["buttons"].clone()),
        (Some(32.25), json!([]))
    );
    assert!(agent
        .take_notifications()
        .iter()
        .all(|message| message["method"] != "view.pointer"));

    // Let go: nothing more is told.
    let off = desktop
        .result(
            "view.pointer",
            Some(json!({"monitor": "timeline", "on": false})),
        )
        .await;
    assert_eq!(
        off,
        json!({"monitor": "timeline", "on": false, "cursor": null})
    );
    post(WM_LBUTTONDOWN, 1, (20, 40));
    post(WM_LBUTTONUP, 0, (20, 40));
    changes_for(&mut desktop, Duration::from_millis(300)).await;
    assert_eq!(pointer_events(&mut desktop), Vec::<Value>::new());
}
